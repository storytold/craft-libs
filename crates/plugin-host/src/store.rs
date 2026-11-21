//! Persistence: installed packages with their grants, and the per plugin `state.*` key-value store.
//!
//! [`MemoryStore`] keeps everything in process and works on every target. [`DirStore`] writes
//! under the app settings directory on native targets. Both obey the same traits, and the same
//! tests run on both.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::MaybeSendSync;

/// Default `state.*` quota per plugin.
pub const DEFAULT_STATE_QUOTA_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The write would take the plugin past its quota.
    QuotaExceeded {
        needed: usize,
        quota: usize,
    },
    /// The plugin id cannot name a directory.
    BadId(String),
    Io(String),
    /// A file exists and does not parse.
    Corrupt(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::QuotaExceeded { needed, quota } => write!(f, "state would use {needed} bytes, quota is {quota}"),
            StoreError::BadId(id) => write!(f, "plugin id {id:?} cannot name a directory"),
            StoreError::Io(m) => write!(f, "store I/O: {m}"),
            StoreError::Corrupt(m) => write!(f, "store file is corrupt: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Where an installed package came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    File,
    DevFolder,
}

/// `{plugin_id, publisher?, package_sha256, permission, scope?, granted_at, decided_by: user}`.
/// The scope lives inside the permission string, such as `commands.run:effect.*`.
///
/// A grant belongs to the package the user decided on. The manager counts it only beside a
/// package with the same plugin id, the same publisher and the same [`package_sha256()`], and drops
/// it otherwise, so code that a store pairs with another package's grants runs without them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredGrant {
    pub plugin_id: String,
    /// The manifest `author`. Unverified in v1 because `craft.sig` is reserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// [`package_sha256()`] of the package the grant was given to. `None` matches no package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_sha256: Option<String>,
    pub permission: String,
    pub granted_at_ms: u64,
    pub decided_by: Decider,
}

/// The SHA-256 of a package, as 64 lowercase hex digits.
pub fn package_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decider {
    User,
}

/// An installed plugin as persisted. Key: `id`. One version per id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPlugin {
    pub id: String,
    pub version: String,
    pub bytes: Vec<u8>,
    pub enabled: bool,
    pub installed_at_ms: u64,
    pub source: Source,
    pub grants: Vec<StoredGrant>,
}

/// An entry of the package store that could not be read. The rest of the store still loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedPackage {
    /// What names the entry: the directory name for a disk store.
    pub name: String,
    pub reason: String,
}

/// What `load_all_lenient` found.
#[derive(Clone, Debug, Default)]
pub struct LoadedPackages {
    pub plugins: Vec<StoredPlugin>,
    pub skipped: Vec<SkippedPackage>,
}

pub trait PackageStore: MaybeSendSync {
    /// Insert or replace by id. Atomic: a failure leaves the old record whole.
    fn save(&self, plugin: &StoredPlugin) -> Result<(), StoreError>;
    /// Every stored plugin. The first unreadable entry fails the whole call.
    fn load_all(&self) -> Result<Vec<StoredPlugin>, StoreError>;
    /// Every readable plugin, plus a list of the entries that were not. One bad entry never hides
    /// the others. The app start path uses this one. The default wraps `load_all`,
    /// so a store that cannot tell entries apart still works, with one failure hiding all.
    fn load_all_lenient(&self) -> Result<LoadedPackages, StoreError> {
        Ok(LoadedPackages { plugins: self.load_all()?, skipped: Vec::new() })
    }
    /// Remove the package and its grants. Absent is not an error.
    fn remove(&self, id: &str) -> Result<(), StoreError>;
}

pub trait StateStore: MaybeSendSync {
    fn get(&self, plugin: &str, key: &str) -> Result<Option<Value>, StoreError>;
    /// Write `key`. Fails with `QuotaExceeded` when the plugin's keys and values would pass `quota`
    /// bytes after the write. Atomic with the quota check.
    fn put(&self, plugin: &str, key: &str, value: &Value, quota: usize) -> Result<(), StoreError>;
    /// Remove all state of one plugin.
    fn clear(&self, plugin: &str) -> Result<(), StoreError>;
}

/// Bytes one entry costs against the quota.
fn entry_cost(key: &str, value: &Value) -> Result<usize, StoreError> {
    let encoded = serde_json::to_vec(value).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    Ok(key.len() + encoded.len())
}

/// Apply a put to one plugin's map under the quota.
fn put_into(map: &mut BTreeMap<String, Value>, key: &str, value: &Value, quota: usize) -> Result<(), StoreError> {
    let mut total = 0usize;
    for (k, v) in map.iter().filter(|(k, _)| k.as_str() != key) {
        total += entry_cost(k, v)?;
    }
    let needed = total + entry_cost(key, value)?;
    if needed > quota {
        return Err(StoreError::QuotaExceeded { needed, quota });
    }
    map.insert(key.to_owned(), value.clone());
    Ok(())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// In-process store.
#[derive(Default)]
pub struct MemoryStore {
    packages: Mutex<BTreeMap<String, StoredPlugin>>,
    state: Mutex<HashMap<String, BTreeMap<String, Value>>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }
}

impl PackageStore for MemoryStore {
    fn save(&self, plugin: &StoredPlugin) -> Result<(), StoreError> {
        lock(&self.packages).insert(plugin.id.clone(), plugin.clone());
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<StoredPlugin>, StoreError> {
        Ok(lock(&self.packages).values().cloned().collect())
    }

    fn remove(&self, id: &str) -> Result<(), StoreError> {
        lock(&self.packages).remove(id);
        Ok(())
    }
}

impl StateStore for MemoryStore {
    fn get(&self, plugin: &str, key: &str) -> Result<Option<Value>, StoreError> {
        Ok(lock(&self.state).get(plugin).and_then(|m| m.get(key)).cloned())
    }

    fn put(&self, plugin: &str, key: &str, value: &Value, quota: usize) -> Result<(), StoreError> {
        let mut all = lock(&self.state);
        put_into(all.entry(plugin.to_owned()).or_default(), key, value, quota)
    }

    fn clear(&self, plugin: &str) -> Result<(), StoreError> {
        lock(&self.state).remove(plugin);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use dir::DirStore;

#[cfg(not(target_arch = "wasm32"))]
mod dir {
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use serde_json::Value;

    use serde::{Deserialize, Serialize};

    use super::{LoadedPackages, PackageStore, SkippedPackage, Source, StateStore, StoreError, StoredGrant, StoredPlugin, lock, put_into};

    /// The part of a [`StoredPlugin`] that is not the package, as written to `meta.json`.
    #[derive(Serialize, Deserialize)]
    struct Meta {
        id: String,
        version: String,
        enabled: bool,
        installed_at_ms: u64,
        source: Source,
        grants: Vec<StoredGrant>,
    }

    /// The file in a plugin directory that names its live generation.
    const CURRENT: &str = "current";
    const PACKAGE: &str = "package.wasm";
    const META: &str = "meta.json";
    /// `g` and the 20 digits of the largest `u64`. A longer `current` is corrupt, so a read stops there.
    const MAX_POINTER_LEN: u64 = 21;

    /// Files under the app settings directory:
    ///
    /// ```text
    /// plugins/<id>/current            names the live generation, such as `g7`
    /// plugins/<id>/g<n>/package.wasm  the code of generation n
    /// plugins/<id>/g<n>/meta.json     the record of generation n, with the grants given to that code
    /// state/<id>/state.json
    /// ```
    ///
    /// A generation is written once and never changed. `save` writes a new generation in full,
    /// then replaces `current` with a rename. That rename is the commit: a reader sees the old
    /// code with the old grants, or the new code with the new grants, never the code of one beside
    /// the grants of the other. A plugin directory without `current` holds no committed record (a
    /// first save that stopped before its commit, or a `remove` that stopped part way) and is not
    /// listed. A generation that `current` does not name is a leftover. The next save of that id
    /// deletes it after its own commit.
    pub struct DirStore {
        root: PathBuf,
        // One lock for every read-modify-write. Package writes are rare.
        io: Mutex<()>,
    }

    fn io_err(path: &Path, e: std::io::Error) -> StoreError {
        StoreError::Io(format!("{}: {e}", path.display()))
    }

    /// A plugin id is `[A-Za-z0-9._-]{1,64}`. A leading dot would make `.` or `..`, so refuse it.
    fn checked_id(id: &str) -> Result<&str, StoreError> {
        if id.is_empty() || id.starts_with('.') || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return Err(StoreError::BadId(id.to_owned()));
        }
        Ok(id)
    }

    fn generation_name(n: u64) -> String {
        format!("g{n}")
    }

    /// The number in a generation name: `g` and decimal digits without a leading zero. Anything
    /// else is `None`, so the text of `current` never becomes a path of its own.
    fn parse_generation(name: &str) -> Option<u64> {
        let digits = name.strip_prefix('g')?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || (digits.len() > 1 && digits.starts_with('0')) {
            return None;
        }
        digits.parse().ok()
    }

    /// Make the entries of `dir` durable. Best effort, as in `craft-atomic`: some file systems
    /// refuse to sync a directory, and only Unix can open one.
    fn sync_dir(dir: &Path) {
        #[cfg(unix)]
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        #[cfg(not(unix))]
        let _ = dir;
    }

    /// Write `bytes` to `path` and flush them to disk. `create_new` refuses a file that exists.
    fn write_synced(path: &Path, bytes: &[u8], create_new: bool) -> Result<(), StoreError> {
        let mut options = fs::OpenOptions::new();
        options.write(true);
        if create_new {
            options.create_new(true);
        } else {
            options.create(true).truncate(true);
        }
        let mut file = options.open(path).map_err(|e| io_err(path, e))?;
        file.write_all(bytes).map_err(|e| io_err(path, e))?;
        file.sync_all().map_err(|e| io_err(path, e))
    }

    /// Write through a flushed temp file and a rename, so a crash leaves the old file whole.
    fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = path.parent().ok_or_else(|| StoreError::Io(format!("{} has no parent", path.display())))?;
        fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
        let tmp = path.with_extension("tmp");
        if let Err(e) = write_synced(&tmp, bytes, false) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(io_err(path, e));
        }
        sync_dir(dir);
        Ok(())
    }

    impl DirStore {
        pub fn new(root: impl Into<PathBuf>) -> DirStore {
            DirStore { root: root.into(), io: Mutex::new(()) }
        }

        fn plugin_dir(&self, id: &str) -> Result<PathBuf, StoreError> {
            Ok(self.root.join("plugins").join(checked_id(id)?))
        }

        /// The generation `current` names. `None` when there is no `current`: nothing was committed.
        fn live_generation(dir: &Path) -> Result<Option<u64>, StoreError> {
            let path = dir.join(CURRENT);
            let file = match fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(io_err(&path, e)),
            };
            let mut text = Vec::new();
            file.take(MAX_POINTER_LEN + 1).read_to_end(&mut text).map_err(|e| io_err(&path, e))?;
            let n = std::str::from_utf8(&text).ok().and_then(parse_generation);
            n.map(Some).ok_or_else(|| StoreError::Corrupt(format!("{} names no generation", path.display())))
        }

        /// A generation number above every generation directory in `dir`, so a save never writes
        /// into a leftover.
        fn next_generation(dir: &Path) -> Result<u64, StoreError> {
            let mut last = 0u64;
            for entry in fs::read_dir(dir).map_err(|e| io_err(dir, e))? {
                let entry = entry.map_err(|e| io_err(dir, e))?;
                if let Some(n) = entry.file_name().to_str().and_then(parse_generation) {
                    last = last.max(n);
                }
            }
            last.checked_add(1).ok_or_else(|| StoreError::Corrupt(format!("{} has no generation number left", dir.display())))
        }

        /// Delete every generation in `dir` except `keep`. It runs after the commit, so it never
        /// deletes the live one, and a stop part way leaves leftovers for the next save.
        fn sweep(dir: &Path, keep: u64) {
            let Ok(entries) = fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                if entry.file_name().to_str().and_then(parse_generation).is_some_and(|n| n != keep) {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }

        /// Read the live generation of one plugin directory. The directory name must equal the id
        /// in `meta.json`. `None` when the directory holds no committed record.
        fn read_entry(dir: &Path) -> Result<Option<StoredPlugin>, StoreError> {
            let Some(n) = Self::live_generation(dir)? else {
                return Ok(None);
            };
            let generation = dir.join(generation_name(n));
            let meta_path = generation.join(META);
            let meta: Meta = serde_json::from_slice(&fs::read(&meta_path).map_err(|e| io_err(&meta_path, e))?)
                .map_err(|e| StoreError::Corrupt(format!("{}: {e}", meta_path.display())))?;
            let wasm_path = generation.join(PACKAGE);
            let bytes = fs::read(&wasm_path).map_err(|e| io_err(&wasm_path, e))?;
            if dir.file_name().and_then(|n| n.to_str()) != Some(meta.id.as_str()) {
                return Err(StoreError::Corrupt(format!("{} holds plugin {}", dir.display(), meta.id)));
            }
            Ok(Some(StoredPlugin {
                id: meta.id,
                version: meta.version,
                bytes,
                enabled: meta.enabled,
                installed_at_ms: meta.installed_at_ms,
                source: meta.source,
                grants: meta.grants,
            }))
        }

        /// Read every plugin directory. An unreadable directory goes to `skipped`. A failure to
        /// list the root is the only error.
        fn scan(&self) -> Result<LoadedPackages, StoreError> {
            let root = self.root.join("plugins");
            let entries = match fs::read_dir(&root) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LoadedPackages::default()),
                Err(e) => return Err(io_err(&root, e)),
            };
            let mut out = LoadedPackages::default();
            for entry in entries {
                let dir = match entry {
                    Ok(entry) => entry.path(),
                    Err(e) => {
                        out.skipped.push(SkippedPackage { name: root.display().to_string(), reason: e.to_string() });
                        continue;
                    }
                };
                if !dir.is_dir() {
                    continue;
                }
                match Self::read_entry(&dir) {
                    Ok(Some(plugin)) => out.plugins.push(plugin),
                    Ok(None) => {}
                    Err(e) => {
                        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
                        out.skipped.push(SkippedPackage { name, reason: e.to_string() });
                    }
                }
            }
            out.plugins.sort_by(|a, b| a.id.cmp(&b.id));
            out.skipped.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }

        fn state_file(&self, id: &str) -> Result<PathBuf, StoreError> {
            Ok(self.root.join("state").join(checked_id(id)?).join("state.json"))
        }

        fn read_state(&self, id: &str) -> Result<std::collections::BTreeMap<String, Value>, StoreError> {
            let path = self.state_file(id)?;
            match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt(format!("{}: {e}", path.display()))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
                Err(e) => Err(io_err(&path, e)),
            }
        }
    }

    impl PackageStore for DirStore {
        fn save(&self, plugin: &StoredPlugin) -> Result<(), StoreError> {
            let dir = self.plugin_dir(&plugin.id)?;
            let meta = Meta {
                id: plugin.id.clone(),
                version: plugin.version.clone(),
                enabled: plugin.enabled,
                installed_at_ms: plugin.installed_at_ms,
                source: plugin.source,
                grants: plugin.grants.clone(),
            };
            let meta_bytes = serde_json::to_vec_pretty(&meta).map_err(|e| StoreError::Corrupt(e.to_string()))?;
            let _io = lock(&self.io);
            fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
            let n = Self::next_generation(&dir)?;
            let name = generation_name(n);
            let staged = dir.join(&name);
            fs::create_dir(&staged).map_err(|e| io_err(&staged, e))?;
            let written = write_synced(&staged.join(PACKAGE), &plugin.bytes, true).and_then(|()| write_synced(&staged.join(META), &meta_bytes, true));
            if let Err(e) = written {
                let _ = fs::remove_dir_all(&staged);
                return Err(e);
            }
            sync_dir(&staged);
            sync_dir(&dir);
            // The commit. If it fails, `current` still names the old generation, and the next
            // save deletes the staged one.
            write_atomic(&dir.join(CURRENT), name.as_bytes())?;
            Self::sweep(&dir, n);
            Ok(())
        }

        fn load_all(&self) -> Result<Vec<StoredPlugin>, StoreError> {
            let loaded = self.scan()?;
            if let Some(first) = loaded.skipped.first() {
                return Err(StoreError::Corrupt(format!("{}: {}", first.name, first.reason)));
            }
            Ok(loaded.plugins)
        }

        fn load_all_lenient(&self) -> Result<LoadedPackages, StoreError> {
            self.scan()
        }

        fn remove(&self, id: &str) -> Result<(), StoreError> {
            let dir = self.plugin_dir(id)?;
            let _io = lock(&self.io);
            // `current` goes first, so a stop part way leaves a directory with no record, never a
            // record that has lost some of its files.
            let current = dir.join(CURRENT);
            match fs::remove_file(&current) {
                Ok(()) => sync_dir(&dir),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(&current, e)),
            }
            match fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_err(&dir, e)),
            }
        }
    }

    impl StateStore for DirStore {
        fn get(&self, plugin: &str, key: &str) -> Result<Option<Value>, StoreError> {
            let _io = lock(&self.io);
            Ok(self.read_state(plugin)?.remove(key))
        }

        fn put(&self, plugin: &str, key: &str, value: &Value, quota: usize) -> Result<(), StoreError> {
            let _io = lock(&self.io);
            let mut map = self.read_state(plugin)?;
            put_into(&mut map, key, value, quota)?;
            let bytes = serde_json::to_vec(&map).map_err(|e| StoreError::Corrupt(e.to_string()))?;
            write_atomic(&self.state_file(plugin)?, &bytes)
        }

        fn clear(&self, plugin: &str) -> Result<(), StoreError> {
            let file = self.state_file(plugin)?;
            let dir = file.parent().ok_or_else(|| StoreError::Io("state file has no parent".into()))?.to_owned();
            let _io = lock(&self.io);
            match fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_err(&dir, e)),
            }
        }
    }
}
