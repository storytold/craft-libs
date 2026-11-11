//! Instance records. A live process holds `<app>-<pid>.lock`. `scan` deletes
//! the record, the socket, and the lock when `try_lock` acquires that file.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::auth::{self, is_lower_hex};
use crate::error::LinkError;
use crate::wire::PROTOCOL_MAX;

use super::RuntimeDir;

/// Where clients connect. Serialized with an internal `kind` tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Endpoint {
    Unix { path: String },
    Pipe { name: String },
    Tcp { port: u16 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Protocol {
    pub link: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands_schema: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocInfo {
    pub doc_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// One running app. Unknown JSON fields are kept in `extra`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub v: u32,
    pub app: String,
    pub app_version: String,
    pub pid: u32,
    pub instance: String,
    pub endpoint: Endpoint,
    pub token: String,
    pub started: u64,
    pub protocol: Protocol,
    pub docs: Vec<DocInfo>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl std::fmt::Debug for InstanceRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceRecord")
            .field("v", &self.v)
            .field("app", &self.app)
            .field("app_version", &self.app_version)
            .field("pid", &self.pid)
            .field("instance", &self.instance)
            .field("endpoint", &self.endpoint)
            .field("token", &"redacted")
            .field("started", &self.started)
            .field("protocol", &self.protocol)
            .field("docs", &self.docs)
            .finish_non_exhaustive()
    }
}

impl InstanceRecord {
    pub fn new(app: &str, app_version: &str, pid: u32, endpoint: Endpoint) -> Result<Self, LinkError> {
        let record = Self {
            v: 1,
            app: app.to_string(),
            app_version: app_version.to_string(),
            pid,
            instance: auth::random_uuid()?,
            endpoint,
            token: auth::generate_token()?,
            started: unix_now()?,
            protocol: Protocol { link: PROTOCOL_MAX, commands_schema: None, extra: Map::new() },
            docs: Vec::new(),
            extra: Map::new(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), LinkError> {
        if self.v != 1 {
            return Err(LinkError::Invalid(format!("record v is {}, want 1", self.v)));
        }
        if !valid_app_name(&self.app) {
            return Err(LinkError::Invalid(format!("app {:?} does not match the record pattern", self.app)));
        }
        if self.app_version.is_empty() {
            return Err(LinkError::Invalid("app_version is empty".into()));
        }
        if self.pid < 1 {
            return Err(LinkError::Invalid("pid must be >= 1".into()));
        }
        if !valid_uuid(&self.instance) {
            return Err(LinkError::Invalid("instance is not a uuid".into()));
        }
        if self.token.len() != 64 || !is_lower_hex(&self.token) {
            return Err(LinkError::Invalid("token must be 64 lowercase hex characters".into()));
        }
        if self.protocol.link < 1 {
            return Err(LinkError::Invalid("protocol.link must be >= 1".into()));
        }
        if let Some(schema) = &self.protocol.commands_schema {
            let Some(hex) = schema.strip_prefix("sha256:") else {
                return Err(LinkError::Invalid("commands_schema must start with sha256:".into()));
            };
            if hex.len() != 64 || !is_lower_hex(hex) {
                return Err(LinkError::Invalid("commands_schema digest is not 64 lowercase hex characters".into()));
            }
        }
        match &self.endpoint {
            Endpoint::Unix { path } => {
                if path.len() > 100 || path.is_empty() || path.contains('\0') || !Path::new(path).is_absolute() {
                    return Err(LinkError::Invalid("unix endpoint path must be absolute and at most 100 bytes".into()));
                }
            }
            Endpoint::Pipe { name } => {
                if !name.starts_with("craft-") || name.contains('/') || name.contains('\\') {
                    return Err(LinkError::Invalid("pipe name must start with craft-".into()));
                }
            }
            Endpoint::Tcp { port } => {
                if *port < 1 {
                    return Err(LinkError::Invalid("tcp port must be >= 1".into()));
                }
            }
        }
        for doc in &self.docs {
            if doc.doc_id.is_empty() || doc.title.is_empty() {
                return Err(LinkError::Invalid("doc_id and title are required".into()));
            }
        }
        self.check_caps()?;
        self.check_tcp_port()?;
        Ok(())
    }

    /// Roles and optional verbs from the optional `caps` field (stored in
    /// `extra`, so the struct shape and old struct literals stay valid).
    /// Empty when the field is absent or malformed. [`Self::validate`] rejects
    /// a malformed field.
    pub fn caps(&self) -> Vec<String> {
        self.extra
            .get("caps")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// Add `cap` to `caps` when it is not there yet.
    pub fn add_cap(&mut self, cap: &str) -> Result<(), LinkError> {
        if !valid_cap(cap) {
            return Err(LinkError::Invalid(format!("cap {cap:?} must match [a-z][a-z0-9._-]{{0,63}}")));
        }
        let mut caps = self.caps();
        if !caps.iter().any(|c| c == cap) {
            caps.push(cap.to_string());
        }
        self.extra.insert("caps".to_string(), Value::from(caps));
        Ok(())
    }

    /// Loopback TCP control port, when the app opened one (`--control <port>`). Stored in
    /// `extra`. The record file is 0600, so only the owner can read this port and the token
    /// that the TCP listener requires.
    pub fn tcp_port(&self) -> Option<u16> {
        self.extra.get("tcp_port").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()).filter(|port| *port >= 1)
    }

    /// Publish the TCP control port in the record.
    pub fn set_tcp_port(&mut self, port: u16) {
        self.extra.insert("tcp_port".to_string(), Value::from(port));
    }

    fn check_tcp_port(&self) -> Result<(), LinkError> {
        match self.extra.get("tcp_port") {
            None => Ok(()),
            Some(value) if value.as_u64().is_some_and(|port| (1..=65535).contains(&port)) => Ok(()),
            Some(_) => Err(LinkError::Invalid("tcp_port must be an integer from 1 to 65535".into())),
        }
    }

    fn check_caps(&self) -> Result<(), LinkError> {
        let Some(value) = self.extra.get("caps") else {
            return Ok(());
        };
        let Some(list) = value.as_array() else {
            return Err(LinkError::Invalid("caps must be an array of strings".into()));
        };
        let mut seen = BTreeSet::new();
        for item in list {
            let Some(cap) = item.as_str().filter(|c| valid_cap(c)) else {
                return Err(LinkError::Invalid("each cap must match [a-z][a-z0-9._-]{0,63}".into()));
            };
            if !seen.insert(cap) {
                return Err(LinkError::Invalid(format!("cap {cap:?} is listed twice")));
            }
        }
        Ok(())
    }
}

fn valid_cap(cap: &str) -> bool {
    let bytes = cap.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// Holds the lock file for this process. Drop releases the lock and leaves the
/// files in place. [`Discovery::remove`] deletes them. [`Discovery::scan`]
/// deletes them when the holder is gone.
pub struct Discovery {
    dir: PathBuf,
    stem: String,
    record: InstanceRecord,
    lock: File,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discovery").field("stem", &self.stem).field("record", &self.record).finish_non_exhaustive()
    }
}

impl Discovery {
    /// Write the record and hold `<app>-<pid>.lock` until drop.
    ///
    /// `try_lock` is the acquire. A successful `try_lock` is the same exclusive
    /// lock as `File::lock`, and it returns [`LinkError::Busy`] when a live
    /// process already holds the file instead of waiting.
    pub fn publish(dir: &RuntimeDir, record: InstanceRecord) -> Result<Self, LinkError> {
        record.validate()?;
        let stem = format!("{}-{}", record.app, record.pid);
        let lock_path = dir.path().join(format!("{stem}.lock"));
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let lock = opts.open(&lock_path)?;
        #[cfg(unix)]
        crate::discovery::enforce_mode(&lock_path, 0o600)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(LinkError::Busy(format!("{stem} is already live")));
            }
            Err(TryLockError::Error(err)) => return Err(err.into()),
        }
        let keep = unix_path(&record);
        clear_payload(dir.path(), &stem, keep.as_deref())?;
        let bytes = serde_json::to_vec(&record)?;
        write_atomic(dir.path(), &stem, &bytes)?;
        Ok(Self { dir: dir.path().to_path_buf(), stem, record, lock })
    }

    pub fn record(&self) -> &InstanceRecord {
        &self.record
    }

    /// Delete this process's record, socket, and lock.
    pub fn remove(self) -> Result<(), LinkError> {
        reap_dead(&self.dir, &self.stem, &self.lock)
    }

    /// Live records in `dir`. A lock that `try_lock` acquires belongs to a dead
    /// process: this deletes its record, socket, and lock.
    pub fn scan(dir: &RuntimeDir) -> Result<Vec<InstanceRecord>, LinkError> {
        let mut stems = BTreeSet::new();
        let entries = std::fs::read_dir(dir.path())?;
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().into_string().ok() else { continue };
            if let Some(stem) = stem_from_entry(&name) {
                stems.insert(stem);
            }
        }
        let mut live = Vec::new();
        for stem in stems {
            if let Some(record) = settle(dir.path(), &stem)? {
                live.push(record);
            }
        }
        Ok(live)
    }
}

pub fn valid_app_name(app: &str) -> bool {
    crate::ids::valid_app_id(app)
}

fn valid_uuid(text: &str) -> bool {
    crate::ids::valid_uuid(text)
}

fn unix_now() -> Result<u64, LinkError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).map_err(LinkError::io)?.as_secs())
}

fn stem_from_entry(name: &str) -> Option<String> {
    if let Some(stem) = name.strip_suffix(".json").or_else(|| name.strip_suffix(".lock")).or_else(|| name.strip_suffix(".sock")) {
        return accept_stem(stem);
    }
    let rest = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let (stem, hex) = rest.rsplit_once('.')?;
    if hex.len() < 2 || !is_lower_hex(hex) {
        return None;
    }
    accept_stem(stem)
}

fn accept_stem(stem: &str) -> Option<String> {
    let (app, pid) = stem.rsplit_once('-')?;
    let pid: u32 = pid.parse().ok()?;
    if pid < 1 || !valid_app_name(app) {
        return None;
    }
    Some(stem.to_string())
}

fn settle(dir: &Path, stem: &str) -> Result<Option<InstanceRecord>, LinkError> {
    let lock_path = dir.join(format!("{stem}.lock"));
    let json_path = dir.join(format!("{stem}.json"));
    if lock_path.is_file() {
        let lock = OpenOptions::new().read(true).write(true).open(&lock_path)?;
        match lock.try_lock() {
            Err(TryLockError::WouldBlock) => return read_live(&json_path),
            Err(TryLockError::Error(err)) => return Err(err.into()),
            Ok(()) => {
                reap_dead(dir, stem, &lock)?;
                return Ok(None);
            }
        }
    }
    if json_path.exists() || dir.join(format!("{stem}.sock")).exists() {
        clear_payload(dir, stem, None)?;
    } else {
        remove_tmps(dir, stem)?;
    }
    Ok(None)
}

fn read_live(json_path: &Path) -> Result<Option<InstanceRecord>, LinkError> {
    if !json_path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(json_path)?;
    let record: InstanceRecord = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(_) => return Ok(None),
    };
    if record.validate().is_err() {
        return Ok(None);
    }
    Ok(Some(record))
}

fn reap_dead(dir: &Path, stem: &str, lock: &File) -> Result<(), LinkError> {
    clear_payload(dir, stem, None)?;
    let lock_path = dir.join(format!("{stem}.lock"));
    match std::fs::remove_file(&lock_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    lock.unlock()?;
    Ok(())
}

fn unix_path(record: &InstanceRecord) -> Option<PathBuf> {
    match &record.endpoint {
        Endpoint::Unix { path } => Some(PathBuf::from(path)),
        Endpoint::Pipe { .. } | Endpoint::Tcp { .. } => None,
    }
}

/// Remove the json file, temp files, and socket files. `keep` is the socket the
/// caller just bound and is about to publish. Deleting that name would unlist
/// a live listener.
fn clear_payload(dir: &Path, stem: &str, keep: Option<&Path>) -> Result<(), LinkError> {
    let json_path = dir.join(format!("{stem}.json"));
    if let Ok(bytes) = std::fs::read(&json_path)
        && let Ok(record) = serde_json::from_slice::<InstanceRecord>(&bytes)
        && let Some(old) = unix_path(&record)
        && keep.is_none_or(|path| path != old.as_path())
    {
        remove_socket_inside(dir, &old);
    }
    let conventional = dir.join(format!("{stem}.sock"));
    if keep.is_none_or(|path| path != conventional.as_path()) {
        remove_socket_inside(dir, &conventional);
    }
    match std::fs::remove_file(&json_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    remove_tmps(dir, stem)
}

fn remove_tmps(dir: &Path, stem: &str) -> Result<(), LinkError> {
    let prefix = format!(".{stem}.");
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().into_string().ok() else { continue };
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn remove_socket_inside(runtime: &Path, candidate: &Path) {
    use std::os::unix::fs::FileTypeExt;
    let Ok(meta) = std::fs::symlink_metadata(candidate) else { return };
    if !meta.file_type().is_socket() {
        return;
    }
    let Some(parent) = candidate.parent() else { return };
    let Ok(parent_real) = std::fs::canonicalize(parent) else { return };
    let Ok(runtime_real) = std::fs::canonicalize(runtime) else { return };
    if parent_real == runtime_real {
        let _ = std::fs::remove_file(candidate);
    }
}

/// Only Unix endpoints leave socket files behind. A named pipe has no file to remove.
#[cfg(not(unix))]
fn remove_socket_inside(runtime: &Path, candidate: &Path) {
    let _ = (runtime, candidate);
}

/// `.<app>-<pid>.<random>.tmp`, then rename onto `<app>-<pid>.json`.
fn write_atomic(dir: &Path, stem: &str, bytes: &[u8]) -> Result<(), LinkError> {
    let mut rand = [0u8; 8];
    getrandom::getrandom(&mut rand).map_err(LinkError::io)?;
    let tmp = dir.join(format!(".{stem}.{}.tmp", auth::encode_hex(&rand)));
    let final_path = dir.join(format!("{stem}.json"));
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(err) = std::fs::rename(&tmp, &final_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> (PathBuf, RuntimeDir) {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = PathBuf::from(format!("/tmp/cl-{label}-{}", auth::encode_hex(&bytes)));
        let _ = std::fs::remove_dir_all(&path);
        let dir = RuntimeDir::at(&path).unwrap();
        (path, dir)
    }

    #[test]
    fn schema_accepts_a_record_and_rejects_a_bad_token() {
        let (_path, dir) = scratch("schema");
        let endpoint = dir.unix_endpoint("effectcraft", 4242).unwrap();
        let record = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint).unwrap();
        let mut value = serde_json::to_value(&record).unwrap();
        value["note"] = Value::String("kept".into());
        let restored: InstanceRecord = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.extra.get("note").and_then(Value::as_str), Some("kept"));
        let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/instance-record.schema.json");
        let schema: Value = serde_json::from_slice(&std::fs::read(&schema_path).unwrap()).unwrap();
        let validator = jsonschema::options().should_validate_formats(true).build(&schema).unwrap();
        let errors: Vec<_> = validator.iter_errors(&value).map(|err| err.to_string()).collect();
        assert!(errors.is_empty(), "{errors:?}");
        value["token"] = Value::String("nope".into());
        assert!(!validator.is_valid(&value));
        let _ = std::fs::remove_dir_all(dir.path());
    }

    #[test]
    fn scan_keeps_a_held_record_and_drops_it_after_unlock() {
        let _no_fork = crate::SPAWN_GUARD.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (path, dir) = scratch("scan");
        let endpoint = dir.unix_endpoint("effectcraft", 4242).unwrap();
        let record = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint).unwrap();
        let decoy = dir.socket_file("effectcraft", 4242);
        std::fs::write(&decoy, b"not a socket").unwrap();
        let held = Discovery::publish(&dir, record.clone()).unwrap();
        let live = Discovery::scan(&dir).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].instance, record.instance);
        assert_eq!(live[0].token, record.token);
        drop(held);
        assert!(Discovery::scan(&dir).unwrap().is_empty());
        assert!(!dir.record_file("effectcraft", 4242).exists());
        assert!(!dir.lock_file("effectcraft", 4242).exists());
        assert!(decoy.exists(), "scan must not delete a regular file");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[cfg(unix)]
    #[test]
    fn scan_unlinks_a_dead_socket() {
        let _no_fork = crate::SPAWN_GUARD.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        use interprocess::local_socket::prelude::*;
        use interprocess::local_socket::{GenericFilePath, ListenerOptions};

        let (path, dir) = scratch("sock");
        let endpoint = dir.unix_endpoint("filmcraft", 5151).unwrap();
        let sock = match &endpoint {
            Endpoint::Unix { path } => path.clone(),
            Endpoint::Pipe { .. } | Endpoint::Tcp { .. } => panic!("unix endpoint"),
        };
        let name = Path::new(&sock).to_fs_name::<GenericFilePath>().unwrap();
        let listener = ListenerOptions::new().name(name).reclaim_name(false).create_sync().unwrap();
        crate::discovery::enforce_mode(Path::new(&sock), 0o600).unwrap();
        let record = InstanceRecord::new("filmcraft", "0.9.1", 5151, endpoint).unwrap();
        let held = Discovery::publish(&dir, record).unwrap();
        assert!(Path::new(&sock).exists(), "publish must leave the bound socket in place");
        drop(held);
        assert!(Path::new(&sock).exists(), "dropping the lock must not unlink the socket");
        assert!(Discovery::scan(&dir).unwrap().is_empty());
        assert!(!Path::new(&sock).exists(), "scan removes the socket once the lock is free");
        drop(listener);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn second_publish_of_a_live_stem_is_busy() {
        let _no_fork = crate::SPAWN_GUARD.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (path, dir) = scratch("busy");
        let endpoint = dir.unix_endpoint("effectcraft", 4242).unwrap();
        let first = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint.clone()).unwrap();
        let held = Discovery::publish(&dir, first).unwrap();
        let second = InstanceRecord::new("effectcraft", "0.8.3", 4242, endpoint).unwrap();
        let err = Discovery::publish(&dir, second).unwrap_err();
        assert!(matches!(err, LinkError::Busy(_)), "{err}");
        drop(held);
        let _ = std::fs::remove_dir_all(&path);
    }
}
