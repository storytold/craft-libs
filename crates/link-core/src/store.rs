use crate::{Change, Error, Event, Id, Manifest, Object, Receipt, Result, SCHEMA, Transaction, affected_many, validate_edge};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_BLOB_BYTES: u64 = 128 * 1024 * 1024;

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}
fn encode(v: &impl Serialize) -> Result<String> {
    serde_json::to_string(v).map_err(storage)
}
fn decode<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(storage)
}

pub struct Store {
    db: Connection,
    root: PathBuf,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).map_err(storage)?;
        let root = std::fs::canonicalize(root).map_err(storage)?;
        std::fs::create_dir_all(root.join("blobs")).map_err(storage)?;
        let blobs = std::fs::canonicalize(root.join("blobs")).map_err(storage)?;
        if blobs.parent() != Some(root.as_path()) {
            return Err(Error::Invalid("redirected blob directory".into()));
        }
        for name in ["craftlink.sqlite", "craftlink.sqlite-wal", "craftlink.sqlite-shm", "craftlink.sqlite-journal"] {
            let path = root.join(name);
            if path.exists() && std::fs::canonicalize(&path).map_err(storage)? != path {
                return Err(Error::Invalid("redirected database file".into()));
            }
        }
        let mut db = Connection::open(root.join("craftlink.sqlite")).map_err(storage)?;
        db.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        // SQLite's journal-mode transition can return BUSY without invoking its
        // busy handler when two processes first open a new database together.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match db.execute_batch("PRAGMA journal_mode=WAL;") {
                Ok(()) => break,
                Err(e)
                    if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(e) => return Err(storage(e)),
            }
        }
        db.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;").map_err(storage)?;
        // A first-open migration is a writer too: a second opener must re-read
        // the version after the first has finished creating the schema.
        let migration = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage)?;
        let version: u32 = migration.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(storage)?;
        if version > SCHEMA {
            return Err(Error::Capability(format!("database schema {version}")));
        }
        if version == 0 {
            migration.execute_batch("
                CREATE TABLE applications (id TEXT PRIMARY KEY, manifest TEXT NOT NULL);
                CREATE TABLE objects (id TEXT PRIMARY KEY, data TEXT NOT NULL, undo TEXT NOT NULL, redo TEXT NOT NULL);
                CREATE TABLE revisions (object TEXT NOT NULL, revision INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(object,revision));
                CREATE TABLE links (id TEXT PRIMARY KEY, source TEXT NOT NULL REFERENCES objects(id), target TEXT NOT NULL REFERENCES objects(id), mode TEXT NOT NULL, UNIQUE(source,target));
                CREATE TABLE events (seq INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, data TEXT NOT NULL);
                CREATE TABLE operations (id TEXT PRIMARY KEY, request TEXT NOT NULL, receipt TEXT NOT NULL);
                CREATE TABLE conflicts (id INTEGER PRIMARY KEY AUTOINCREMENT, request TEXT NOT NULL, reason TEXT NOT NULL);
                CREATE TABLE bindings (id TEXT PRIMARY KEY, data TEXT NOT NULL);
                PRAGMA user_version=1;").map_err(storage)?;
        }
        migration.commit().map_err(storage)?;
        Ok(Self { db, root })
    }
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(Error::Invalid("blob exceeds 128 MiB".into()));
        }
        let hash = format!("{:x}", Sha256::digest(bytes));
        let path = self.root.join("blobs").join(&hash);
        if path.exists() {
            self.blob(&hash)?;
        } else {
            // Publish once; concurrent publishers of identical content may win,
            // but no publisher replaces a previously committed immutable blob.
            match craft_atomic::write_new(&path, bytes) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.blob(&hash)?;
                }
                Err(e) => return Err(storage(e)),
            }
        }
        Ok(hash)
    }
    pub fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(Error::Invalid("invalid content hash".into()));
        }
        let path = self.root.join("blobs").join(hash);
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| Error::Missing(e.to_string()))?;
        if !metadata.is_file() || metadata.len() > MAX_BLOB_BYTES || std::fs::canonicalize(&path).map_err(storage)? != path {
            return Err(Error::Invalid("blob must be a bounded regular file inside the store".into()));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path).map_err(storage)?.take(MAX_BLOB_BYTES + 1).read_to_end(&mut bytes).map_err(storage)?;
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(Error::Invalid("blob exceeds 128 MiB".into()));
        }
        if format!("{:x}", Sha256::digest(&bytes)) != hash {
            return Err(Error::Storage("blob checksum mismatch".into()));
        }
        Ok(bytes)
    }
    pub fn register(&mut self, manifest: Manifest) -> Result<()> {
        manifest.validate()?;
        self.db
            .execute(
                "INSERT INTO applications VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET manifest=excluded.manifest",
                params![manifest.application, encode(&manifest)?],
            )
            .map_err(storage)?;
        Ok(())
    }
    pub fn manifests(&self) -> Result<Vec<Manifest>> {
        self.strings("SELECT manifest FROM applications ORDER BY id", [])?.iter().map(|s| decode(s)).collect()
    }
    fn strings<const N: usize>(&self, sql: &str, p: [&str; N]) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare(sql).map_err(storage)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(p), |r| r.get::<_, String>(0)).map_err(storage)?;
        rows.map(|r| r.map_err(storage)).collect()
    }
    pub fn object(&self, id: Id) -> Result<Object> {
        let s: Option<String> =
            self.db.query_row("SELECT data FROM objects WHERE id=?", [id.to_string()], |r| r.get(0)).optional().map_err(storage)?;
        decode(&s.ok_or_else(|| Error::Missing(id.to_string()))?)
    }
    pub fn objects(&self) -> Result<Vec<Object>> {
        self.strings("SELECT data FROM objects ORDER BY id", [])?.iter().map(|s| decode(s)).collect()
    }
    pub fn edges(&self) -> Result<Vec<(Id, Id)>> {
        let mut stmt = self.db.prepare("SELECT source,target FROM links").map_err(storage)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(storage)?;
        rows.map(|row| {
            let (s, t) = row.map_err(storage)?;
            Ok((s.parse().map_err(storage)?, t.parse().map_err(storage)?))
        })
        .collect()
    }
    pub fn link(&mut self, id: Id, source: Id, target: Id, mode: &str) -> Result<()> {
        if !["semantic", "source", "preview"].contains(&mode) {
            return Err(Error::Capability(mode.into()));
        }
        let tx = self.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage)?;
        let mut stmt = tx.prepare("SELECT source,target FROM links").map_err(storage)?;
        let pairs = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(storage)?;
        let mut edges = Vec::new();
        for p in pairs {
            let (s, t) = p.map_err(storage)?;
            edges.push((s.parse().map_err(storage)?, t.parse().map_err(storage)?));
        }
        drop(stmt);
        validate_edge(&edges, source, target)?;
        let existing: Option<(String, String, String)> = tx
            .query_row("SELECT source,target,mode FROM links WHERE id=?", [id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()
            .map_err(storage)?;
        if let Some(old) = existing {
            if old != (source.to_string(), target.to_string(), mode.to_string()) {
                return Err(Error::Conflict("link id reused".into()));
            }
            return Ok(());
        }
        tx.execute("INSERT INTO links VALUES (?1,?2,?3,?4)", params![id.to_string(), source.to_string(), target.to_string(), mode])
            .map_err(storage)?;
        tx.execute(
            "INSERT INTO events(kind,data) VALUES ('linked',?)",
            [encode(&serde_json::json!({"id":id,"source":source,"target":target,"mode":mode}))?],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)
    }
    pub fn commit(&mut self, request: Transaction) -> Result<Receipt> {
        self.commit_mode(request, "edit")
    }
    fn commit_mode(&mut self, request: Transaction, mode: &str) -> Result<Receipt> {
        self.commit_mode_checked(request, mode, || Ok(()))
    }
    /// Fault/validation seam after all transactional writes, before COMMIT.
    /// An error rolls back metadata; immutable unreferenced blobs are harmless.
    pub fn commit_with_precommit(&mut self, request: Transaction, before_commit: impl FnOnce() -> Result<()>) -> Result<Receipt> {
        self.commit_mode_checked(request, "edit", before_commit)
    }
    fn commit_mode_checked(&mut self, request: Transaction, mode: &str, before_commit: impl FnOnce() -> Result<()>) -> Result<Receipt> {
        if request.changes.is_empty() || request.changes.len() > 1024 {
            return Err(Error::Invalid("transaction size".into()));
        }
        let request_json = encode(&(mode, &request))?;
        let previous: Option<(String, String)> = self
            .db
            .query_row("SELECT request,receipt FROM operations WHERE id=?", [request.operation.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(storage)?;
        if let Some((old, receipt)) = previous {
            if old != request_json {
                return Err(Error::Conflict("operation id reused with different request".into()));
            }
            return decode(&receipt);
        }
        let mut ids = BTreeSet::new();
        for c in &request.changes {
            if !ids.insert(c.object) {
                return Err(Error::Invalid("duplicate object in transaction".into()));
            }
            if let Some(h) = &c.native {
                if h.format.is_empty() || h.application.is_empty() {
                    return Err(Error::Invalid("native payload origin required".into()));
                }
                self.blob(&h.hash)?;
            }
        }
        let tx = self.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage)?;
        // Recheck idempotency under the SQLite writer lock (independent Store/process writers).
        let replay: Option<(String, String)> = tx
            .query_row("SELECT request,receipt FROM operations WHERE id=?", [request.operation.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(storage)?;
        if let Some((old, receipt)) = replay {
            if old != request_json {
                return Err(Error::Conflict("operation id reused".into()));
            }
            return decode(&receipt);
        }
        // Capabilities are authoritative only under the writer lock. Revocation
        // by another process while we wait must be observed before any writes.
        for c in &request.changes {
            let manifest: Option<String> =
                tx.query_row("SELECT manifest FROM applications WHERE id=?", [&c.application], |r| r.get(0)).optional().map_err(storage)?;
            let manifest: Manifest = decode(&manifest.ok_or_else(|| Error::Capability("application not registered".into()))?)?;
            let props =
                c.value.as_object().ok_or_else(|| Error::Invalid("semantic value must be an object".into()))?.keys().cloned().collect::<Vec<_>>();
            manifest.negotiate(&c.kind, &props, true)?;
        }
        let mut revisions = BTreeMap::new();
        let mut changed = BTreeSet::new();
        for c in &request.changes {
            let row: Option<(String, String, String)> = tx
                .query_row("SELECT data,undo,redo FROM objects WHERE id=?", [c.object.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()
                .map_err(storage)?;
            let old: Option<Object> = row.as_ref().map(|r| decode(&r.0)).transpose()?;
            let actual = old.as_ref().map_or(0, |o| o.revision);
            if actual != c.expected {
                let reason = format!("{} expected {}, actual {}", c.object, c.expected, actual);
                drop(tx); // Roll back every earlier object update, then retain the rejected intent.
                self.record_conflict(&serde_json::from_str(&request_json).map_err(storage)?, &reason)?;
                return Err(Error::Conflict(reason));
            }
            if let Some(o) = &old {
                if o.document != c.document || o.application != c.application || o.kind != c.kind {
                    return Err(Error::Invalid("object identity is immutable".into()));
                }
                if o.value == c.value && o.native == c.native {
                    revisions.insert(c.object, actual);
                    continue;
                }
            }
            let mut undo: Vec<Object> = row.as_ref().map(|r| decode(&r.1)).transpose()?.unwrap_or_default();
            let mut redo: Vec<Object> = row.as_ref().map(|r| decode(&r.2)).transpose()?.unwrap_or_default();
            match mode {
                "undo" => {
                    undo.pop();
                    if let Some(o) = old {
                        redo.push(o);
                    }
                }
                "redo" => {
                    redo.pop();
                    if let Some(o) = old {
                        undo.push(o);
                    }
                }
                _ => {
                    if let Some(o) = old {
                        undo.push(o);
                    }
                    redo.clear();
                }
            }
            let revision = actual.checked_add(1).filter(|r| *r <= i64::MAX as u64).ok_or_else(|| Error::Invalid("revision overflow".into()))?;
            let object = Object {
                object: c.object,
                document: c.document,
                application: c.application.clone(),
                kind: c.kind.clone(),
                revision,
                value: c.value.clone(),
                native: c.native.clone(),
            };
            tx.execute("INSERT INTO revisions VALUES (?1,?2,?3)", params![c.object.to_string(), revision as i64, encode(&object)?])
                .map_err(storage)?;
            tx.execute(
                "INSERT INTO objects VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET data=excluded.data,undo=excluded.undo,redo=excluded.redo",
                params![c.object.to_string(), encode(&object)?, encode(&undo)?, encode(&redo)?],
            )
            .map_err(storage)?;
            revisions.insert(c.object, revision);
            changed.insert(c.object);
        }
        let mut stmt = tx.prepare("SELECT source,target FROM links").map_err(storage)?;
        let pairs = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(storage)?;
        let mut edges = Vec::new();
        for p in pairs {
            let (s, t) = p.map_err(storage)?;
            edges.push((s.parse().map_err(storage)?, t.parse().map_err(storage)?));
        }
        drop(stmt);
        let invalidated = affected_many(&edges, changed.iter().copied());
        let mut receipt = Receipt { operation: request.operation, sequence: 0, revisions, invalidated };
        if !changed.is_empty() {
            tx.execute("INSERT INTO events(kind,data) VALUES (?1,?2)", params![mode, encode(&receipt)?]).map_err(storage)?;
            receipt.sequence = tx.last_insert_rowid() as u64;
            tx.execute("UPDATE events SET data=? WHERE seq=?", params![encode(&receipt)?, receipt.sequence as i64]).map_err(storage)?;
        } else {
            receipt.sequence = tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get::<_, i64>(0)).map_err(storage)? as u64;
        }
        tx.execute("INSERT INTO operations VALUES (?1,?2,?3)", params![request.operation.to_string(), request_json, encode(&receipt)?])
            .map_err(storage)?;
        before_commit()?;
        tx.commit().map_err(storage)?;
        Ok(receipt)
    }
    pub fn history(&mut self, operation: Id, object: Id, expected: u64, redo: bool) -> Result<Receipt> {
        let mode = if redo { "redo" } else { "undo" };
        // History retries use the original committed request, not the now-moved stack.
        let saved: Option<String> =
            self.db.query_row("SELECT request FROM operations WHERE id=?", [operation.to_string()], |r| r.get(0)).optional().map_err(storage)?;
        if let Some(s) = saved {
            let (prior_mode, request): (String, Transaction) = decode(&s)?;
            if prior_mode != mode || request.changes.first().is_none_or(|c| c.object != object || c.expected != expected) {
                return Err(Error::Conflict("history operation id reused".into()));
            }
            return self.commit_mode(request, mode);
        }
        let current = self.object(object)?;
        if current.revision != expected {
            return Err(Error::Conflict("history head changed".into()));
        }
        let sql = if redo { "SELECT redo FROM objects WHERE id=?" } else { "SELECT undo FROM objects WHERE id=?" };
        let s: String = self.db.query_row(sql, [object.to_string()], |r| r.get(0)).map_err(storage)?;
        let stack: Vec<Object> = decode(&s)?;
        let target = stack.last().ok_or_else(|| Error::Invalid(format!("nothing to {mode}")))?;
        self.commit_mode(
            Transaction {
                operation,
                changes: vec![Change {
                    object,
                    document: current.document,
                    application: current.application,
                    kind: current.kind,
                    expected,
                    value: target.value.clone(),
                    native: target.native.clone(),
                }],
            },
            mode,
        )
    }
    pub fn events(&self, after: u64) -> Result<Vec<Event>> {
        let mut stmt = self.db.prepare("SELECT seq,kind,data FROM events WHERE seq>? ORDER BY seq LIMIT 256").map_err(storage)?;
        let after = i64::try_from(after).map_err(|_| Error::Invalid("cursor overflow".into()))?;
        let rows = stmt.query_map([after], |r| Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get::<_, String>(2)?))).map_err(storage)?;
        rows.map(|row| {
            let (sequence, kind, s) = row.map_err(storage)?;
            Ok(Event { sequence: sequence as u64, kind, data: decode(&s)? })
        })
        .collect()
    }
    pub fn record_conflict(&self, request: &Value, reason: &str) -> Result<()> {
        self.db.execute("INSERT INTO conflicts(request,reason) VALUES (?1,?2)", params![encode(request)?, reason]).map_err(storage)?;
        Ok(())
    }
    pub fn conflicts(&self) -> Result<Vec<Value>> {
        self.strings("SELECT json_object('id',id,'request',json(request),'reason',reason) FROM conflicts ORDER BY id", [])?
            .iter()
            .map(|s| decode(s))
            .collect()
    }
    pub fn binding(&self, id: Id) -> Result<Value> {
        let s: Option<String> =
            self.db.query_row("SELECT data FROM bindings WHERE id=?", [id.to_string()], |r| r.get(0)).optional().map_err(storage)?;
        decode(&s.ok_or_else(|| Error::Missing(id.to_string()))?)
    }
    pub fn bindings(&self) -> Result<Vec<Value>> {
        self.strings("SELECT data FROM bindings ORDER BY id", [])?.iter().map(|s| decode(s)).collect()
    }
    pub fn set_binding(&self, id: Id, data: &Value) -> Result<()> {
        self.db
            .execute("INSERT INTO bindings VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data", params![id.to_string(), encode(data)?])
            .map_err(storage)?;
        Ok(())
    }
}
