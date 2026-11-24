//! `window.json`: the saved [`Placement`] of each window, beside the app's `config.toml`.
//!
//! ```json
//! { "schema_version": 1, "windows": { "main": { ... }, "<key>": { ... } } }
//! ```
//!
//! The folder is chosen by `craft_settings::paths`, so portable installs keep the file in the
//! portable folder and `<APP>_CONFIG_DIR` moves it with `config.toml`. Writes are atomic and
//! private. A file that does not parse is moved aside (`window.broken-<unix-ts>.json`, the
//! `craft_settings::recovery` convention) and the store starts empty. A file with a newer
//! `schema_version` is read but never written. Keys this version does not know are written back
//! unchanged. Two running instances of one app overwrite each other; the last writer wins.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use craft_atomic::{NewFilePermissions, WriteOptions};
use craft_settings::paths::{self, Location, Portable, Probe};
use craft_settings::recovery;
use serde_json::{Map, Value, json};

use super::place::{Placement, sane};

/// The file name beside `config.toml`.
pub const FILE_NAME: &str = "window.json";
/// The `schema_version` this code writes.
pub const SCHEMA_VERSION: u64 = 1;
/// The key of the app's first window.
pub const MAIN: &str = "main";

/// Why a store does not write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnly {
    /// The file has a newer `schema_version` than this code knows.
    NewerSchema(u64),
    /// The file exists but could not be read (permissions, say); it is left alone.
    Unreadable,
}

/// A write [`Store::take_pending`] prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// Pass to [`Store::mark_saved`] after the write succeeded.
    pub generation: u64,
}

/// Writes `pending` atomically and privately, creating the folder. Needs no access to the store.
pub fn write_pending(pending: &Pending) -> io::Result<()> {
    if let Some(dir) = pending.path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private).atomic_write(&pending.path, &pending.bytes)
}

/// The saved placements of an app's windows.
#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// Entries stay raw so one that fails to parse is neither lost nor fatal to the others.
    windows: BTreeMap<String, Value>,
    /// Top-level keys other than `schema_version` and `windows`.
    extra: Map<String, Value>,
    read_only: Option<ReadOnly>,
    /// Counts changes; the store is dirty while it is ahead of `saved`.
    generation: u64,
    saved: u64,
    moved_aside: Option<PathBuf>,
}

impl Store {
    /// The store for `app_id` in the folder `craft_settings` picks for its `config.toml`, with
    /// the suite's default portable marker. In an isolated or in-memory configuration
    /// (`<APP>_CONFIG_DIR` empty) nothing is read or written.
    pub fn open(app_id: &str) -> Self {
        Self::open_with(app_id, &Portable::DEFAULT, &Location::Auto, &Probe::default())
    }

    /// [`open`](Self::open) with the app's own portable names, location override and probe: the
    /// same arguments its `craft_settings` options use.
    pub fn open_with(app_id: &str, portable: &Portable, location: &Location, probe: &Probe) -> Self {
        match paths::resolve(app_id, portable, location, probe).dir {
            Some(dir) => Self::at(&dir.join(FILE_NAME)),
            None => Self::in_memory(),
        }
    }

    /// A store backed by the file at `path`. A missing file is an empty store.
    pub fn at(path: &Path) -> Self {
        let mut store = Self { path: Some(path.to_path_buf()), ..Self::in_memory() };
        store.load();
        store
    }

    /// A store that reads and writes nothing: tests, agents, the web.
    pub fn in_memory() -> Self {
        Self { path: None, windows: BTreeMap::new(), extra: Map::new(), read_only: None, generation: 0, saved: 0, moved_aside: None }
    }

    /// The file this store writes, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Where a damaged file was moved to while opening, if one was.
    pub fn moved_aside(&self) -> Option<&Path> {
        self.moved_aside.as_deref()
    }

    /// Why [`save`](Self::save) writes nothing, if so.
    pub fn read_only(&self) -> Option<&ReadOnly> {
        self.read_only.as_ref()
    }

    /// The placement saved under `key`; `None` when absent or not a placement.
    pub fn get(&self, key: &str) -> Option<Placement> {
        serde_json::from_value(self.windows.get(key)?.clone()).ok()
    }

    /// Replaces the placement under `key`. Fields of the old entry that this version does not know
    /// stay. A placement with a non-finite size is ignored. Nothing is written until
    /// [`save`](Self::save) or [`take_pending`](Self::take_pending).
    pub fn set(&mut self, key: &str, placement: &Placement) {
        let Some(placement) = sane(placement) else { return };
        let Ok(Value::Object(new)) = serde_json::to_value(&placement) else { return };
        let merged = match self.windows.get(key) {
            Some(Value::Object(old)) => {
                let mut m = old.clone();
                m.extend(new);
                m
            }
            _ => new,
        };
        let merged = Value::Object(merged);
        if self.windows.get(key) != Some(&merged) {
            self.windows.insert(key.to_owned(), merged);
            self.generation += 1;
        }
    }

    /// Forgets `key`.
    pub fn remove(&mut self, key: &str) {
        if self.windows.remove(key).is_some() {
            self.generation += 1;
        }
    }

    /// The keys with an entry, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.windows.keys().map(String::as_str)
    }

    /// Whether something changed since the last successful write.
    pub fn is_dirty(&self) -> bool {
        self.generation != self.saved
    }

    /// The write that would bring the file up to date, or `None` when there is nothing to write
    /// (in memory, read-only, or unchanged). Write it with [`write_pending`], without holding any
    /// lock on the store, then report success with [`mark_saved`](Self::mark_saved).
    pub fn take_pending(&self) -> Option<Pending> {
        let path = self.path.clone().filter(|_| self.read_only.is_none() && self.is_dirty())?;
        let mut doc = self.extra.clone();
        doc.insert("schema_version".into(), json!(SCHEMA_VERSION));
        doc.insert("windows".into(), Value::Object(self.windows.clone().into_iter().collect()));
        let mut bytes = serde_json::to_vec_pretty(&Value::Object(doc)).ok()?;
        bytes.push(b'\n');
        Some(Pending { path, bytes, generation: self.generation })
    }

    /// Records that [`Pending::generation`] reached the disk. The store stays dirty if it changed
    /// again meanwhile, and after a failed write (do not call this then), so the next
    /// [`take_pending`](Self::take_pending) retries.
    pub fn mark_saved(&mut self, generation: u64) {
        self.saved = self.saved.max(generation.min(self.generation));
    }

    /// [`take_pending`](Self::take_pending), [`write_pending`] and [`mark_saved`](Self::mark_saved)
    /// in one call. `Ok(true)` when written; `Ok(false)` when there was nothing to write.
    pub fn save(&mut self) -> io::Result<bool> {
        let Some(pending) = self.take_pending() else { return Ok(false) };
        write_pending(&pending)?;
        self.mark_saved(pending.generation);
        Ok(true)
    }

    fn load(&mut self) {
        let Some(path) = self.path.clone() else { return };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            // Invalid UTF-8 is damage; anything else may be transient, so leave the file be.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => return self.start_over(&path),
            Err(_) => {
                self.read_only = Some(ReadOnly::Unreadable);
                return;
            }
        };
        let Some(mut doc) = serde_json::from_str::<Value>(&text).ok().and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        }) else {
            return self.start_over(&path);
        };
        let Some(version) = doc.remove("schema_version").and_then(|v| v.as_u64()) else {
            return self.start_over(&path);
        };
        // A newer file is never judged by this version's shape rules, so never moved aside.
        if version > SCHEMA_VERSION {
            self.read_only = Some(ReadOnly::NewerSchema(version));
            if let Some(Value::Object(m)) = doc.remove("windows") {
                self.windows = m.into_iter().collect();
            }
            self.extra = doc;
            return;
        }
        let Some(Value::Object(windows)) = doc.remove("windows") else {
            return self.start_over(&path);
        };
        self.windows = windows.into_iter().collect();
        self.extra = doc;
    }

    fn start_over(&mut self, path: &Path) {
        match recovery::move_aside(path) {
            Ok(to) => self.moved_aside = Some(to),
            // Cannot rename it away: do not overwrite what we could not set aside.
            Err(_) => self.read_only = Some(ReadOnly::Unreadable),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::restore::{Monitor, MonitorKey};

    fn sample(x: i32) -> Placement {
        let m = Monitor::new(MonitorKey::new("DP-1", [1920, 1080]), [0, 0], [1920, 1080], 1.0);
        let mut p = crate::restore::place(None, &[m], Some(0), &crate::restore::Defaults::new([800.0, 600.0], [300.0, 200.0]));
        p.position_px = Some([x, 10]);
        p
    }

    fn siblings(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);
        let mut s = Store::at(&path);
        assert!(s.get(MAIN).is_none());
        s.set(MAIN, &sample(5));
        s.set("doc-2", &sample(6));
        assert!(s.save().unwrap());
        assert!(!s.save().unwrap(), "nothing changed");
        let again = Store::at(&path);
        assert_eq!(again.get(MAIN), Some(sample(5)));
        assert_eq!(again.get("doc-2"), Some(sample(6)));
        assert_eq!(again.keys().collect::<Vec<_>>(), ["doc-2", MAIN]);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["schema_version"], 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut s = Store::at(&path);
        s.set(MAIN, &sample(1));
        s.save().unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
    }

    #[test]
    fn corrupt_file_is_moved_aside_and_the_store_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "{ not json").unwrap();
        let mut s = Store::at(&path);
        assert!(s.get(MAIN).is_none() && s.read_only().is_none());
        let aside = s.moved_aside().unwrap().to_path_buf();
        assert_eq!(std::fs::read_to_string(aside).unwrap(), "{ not json");
        assert!(!path.exists());
        s.set(MAIN, &sample(1));
        s.save().unwrap();
        assert!(Store::at(&path).get(MAIN).is_some());
        assert_eq!(siblings(dir.path()).len(), 2);
    }

    #[test]
    fn json_without_a_schema_version_counts_as_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, r#"{"windows":{}}"#).unwrap();
        assert!(Store::at(&path).moved_aside().is_some());
    }

    #[test]
    fn newer_schema_is_read_but_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut doc = json!({ "schema_version": 2, "windows": {}, "future": [1] });
        doc["windows"][MAIN] = serde_json::to_value(sample(7)).unwrap();
        let text = serde_json::to_string(&doc).unwrap();
        std::fs::write(&path, &text).unwrap();
        let mut s = Store::at(&path);
        assert_eq!(s.read_only(), Some(&ReadOnly::NewerSchema(2)));
        assert_eq!(s.get(MAIN), Some(sample(7)));
        s.set(MAIN, &sample(8));
        assert!(!s.save().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(siblings(dir.path()), [FILE_NAME]);
    }

    #[test]
    fn unknown_keys_survive_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut doc = json!({ "schema_version": 1, "note": {"a": 1}, "windows": { "odd": {"unexpected": true} } });
        doc["windows"][MAIN] = serde_json::to_value(sample(1)).unwrap();
        std::fs::write(&path, doc.to_string()).unwrap();
        let mut s = Store::at(&path);
        assert!(s.get("odd").is_none());
        s.set(MAIN, &sample(2));
        s.save().unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["note"], json!({"a": 1}));
        assert_eq!(v["windows"]["odd"], json!({"unexpected": true}));
        assert_eq!(Store::at(&path).get(MAIN), Some(sample(2)));
    }

    #[test]
    fn newer_schema_with_an_unexpected_shape_is_not_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, r#"{"schema_version":3,"windows":[1,2]}"#).unwrap();
        let s = Store::at(&path);
        assert_eq!(s.read_only(), Some(&ReadOnly::NewerSchema(3)));
        assert!(s.moved_aside().is_none() && path.exists());
    }

    #[test]
    fn non_finite_placements_are_never_stored() {
        let mut s = Store::in_memory();
        let mut p = sample(1);
        p.normal.size[0] = f32::NAN;
        s.set(MAIN, &p);
        assert!(s.get(MAIN).is_none() && !s.is_dirty());
        let mut p = sample(1);
        p.normal.pos[1] = f32::INFINITY;
        s.set(MAIN, &p);
        assert_eq!(s.get(MAIN).unwrap().normal.pos, [0.0, 0.0]);
    }

    #[test]
    fn set_keeps_unknown_fields_of_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut entry = serde_json::to_value(sample(1)).unwrap();
        entry["later"] = json!("x");
        std::fs::write(&path, json!({"schema_version": 1, "windows": {MAIN: entry}}).to_string()).unwrap();
        let mut s = Store::at(&path);
        s.set(MAIN, &sample(2));
        s.save().unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["windows"][MAIN]["later"], "x");
        assert_eq!(Store::at(&path).get(MAIN), Some(sample(2)));
    }

    #[test]
    fn dirty_clears_only_after_the_write_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::at(&dir.path().join(FILE_NAME));
        s.set(MAIN, &sample(1));
        let first = s.take_pending().unwrap();
        s.set(MAIN, &sample(2));
        write_pending(&first).unwrap();
        s.mark_saved(first.generation);
        assert!(s.is_dirty(), "changed again while the first write was in flight");
        let second = s.take_pending().unwrap();
        assert!(s.is_dirty(), "taking a write does not clear it");
        write_pending(&second).unwrap();
        s.mark_saved(second.generation);
        assert!(!s.is_dirty() && s.take_pending().is_none());
    }

    #[test]
    fn in_memory_store_keeps_values_and_writes_nothing() {
        let mut s = Store::in_memory();
        s.set(MAIN, &sample(3));
        assert_eq!(s.get(MAIN), Some(sample(3)));
        assert!(!s.save().unwrap());
        assert!(s.path().is_none());
    }

    #[test]
    fn open_puts_the_file_beside_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_with("app", &Portable::DEFAULT, &Location::Dir(dir.path().to_path_buf()), &Probe::default());
        assert_eq!(s.path(), Some(dir.path().join(FILE_NAME).as_path()));
        let mem = Store::open_with("app", &Portable::DEFAULT, &Location::Memory, &Probe::default());
        assert!(mem.path().is_none());
    }
}
