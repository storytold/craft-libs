//! Where recovery copies go and the folder of copies: sidecars, orphans and removal.

use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use std::time::SystemTime;

#[cfg(not(target_arch = "wasm32"))]
use super::*;

/// The recovery folder's name.
pub const RECOVERY_DIR: &str = "Recovery";

/// How the app's settings folder was chosen, which decides where recovery copies go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsMode {
    /// The platform's own folders.
    Platform,
    /// Portable mode or an override folder: everything stays in the settings folder.
    SelfContained,
}

/// The recovery folder. In [`SettingsMode::Platform`]: `%LOCALAPPDATA%\<Folder>\Recovery` on
/// Windows; `$XDG_STATE_HOME/<app_id>/Recovery` on Linux and the BSDs (an absolute
/// `XDG_STATE_HOME` only, else `$HOME/.local/state`); `<config>/Recovery` on macOS. Otherwise, or
/// when those variables are missing, `<config>/Recovery`. `folder` is the Windows folder name
/// ("Photocraft"), `app_id` the XDG one ("photocraft"); `env` reads the environment.
// pc: apps/photocraft/src/app_dirs.rs:94 resolve_recovery
pub fn recovery_dir(
    folder: &str,
    app_id: &str,
    config_dir: Option<&Path>,
    mode: SettingsMode,
    env: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    if mode == SettingsMode::Platform {
        if cfg!(windows) {
            if let Some(local) = env("LOCALAPPDATA").filter(|l| !l.is_empty()) {
                return Some(PathBuf::from(local).join(folder).join(RECOVERY_DIR));
            }
        } else if !cfg!(target_os = "macos") {
            let state = env("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|s| s.is_absolute())
                .or_else(|| env("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local").join("state")));
            if let Some(state) = state {
                return Some(state.join(app_id).join(RECOVERY_DIR));
            }
        }
    }
    config_dir.map(|d| d.join(RECOVERY_DIR))
}

/// Moves copies from an older recovery folder (`legacy`) into `dir`, so a crash just before an
/// update is still offered. Returns the folder this process uses: `legacy` only when `dir`
/// doesn't exist and nothing can be moving (the move failed, or its lock can't be made). Two
/// launches at once are kept apart by `recovery-migration.lock` beside `dir`, waiting at most 5 s;
/// entries move through `.moving` and `.held` names so a half-done move is finished by the next
/// launch.
// pc: apps/photocraft/src/app_dirs.rs:133 adopt_legacy_recovery, :138 adopt_legacy_recovery_waiting (MIGRATION_LOCK :115, MIGRATION_WAIT :118)
#[cfg(not(target_arch = "wasm32"))]
pub fn adopt_legacy_recovery(legacy: &Path, dir: &Path) -> PathBuf {
    migrate::adopt_waiting(legacy, dir, migrate::WAIT)
}

/// What the sidecar records about a copy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryMeta {
    /// The document's name ("poster.psd", "Untitled-2").
    pub name: String,
    /// The file it was opened from or last saved to.
    pub original: Option<PathBuf>,
    /// The copy's extension, without the dot ("pcraft").
    pub extension: String,
    /// The edit revision the copy holds.
    pub revision: u64,
}

/// A copy an earlier run left.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryEntry {
    /// The file stem shared by the copy and its sidecar.
    pub key: String,
    /// What the sidecar says about the copy.
    pub meta: RecoveryMeta,
    /// When the copy was written (the sidecar's modification time).
    pub saved: SystemTime,
    /// The copy's path.
    pub path: PathBuf,
}

/// The recovery folder of one app run, after PhotoCraft's and PdfCraft's stores.
///
/// Each copy is `<key>.<extension>` plus `<key>.json`, both written atomically, the copy first.
/// A new document's key is `doc-<session>-<id>`, `<session>` unique to this run (ids restart
/// every launch); a recovered document [adopts](Self::adopt) the key it was loaded from, so its
/// autosaves replace that copy. Nothing is deleted because it was recovered: a second crash
/// before the next autosave loses nothing. Keys are sanitized ([`sanitize_key`]).
///
/// Every method takes `&self`, so the app keeps one store in an `Arc` and shares it with its
/// autosave jobs. The folder is created owner-only (0o700 on Unix). [`open`](Self::open) takes an exclusive
/// `instance-<session>-<pid>-<n>.lock`, so [`leftovers`](Self::leftovers) offers only copies whose run is
/// gone, never those of another instance still running (PdfCraft).
#[cfg(not(target_arch = "wasm32"))]
pub struct RecoveryStore {
    dir: PathBuf,
    session: String,
    adopted: std::sync::Mutex<HashMap<u64, String>>,
    /// The name of this run's `instance-<session>-<pid>-<n>.lock`, recorded in every sidecar it writes.
    instance: String,
    lock: std::fs::File,
}

// The app shares one store with its autosave jobs (`Arc<RecoveryStore>`).
#[cfg(not(target_arch = "wasm32"))]
const _: () = {
    const fn shared<T: Send + Sync>() {}
    shared::<RecoveryStore>();
};

#[cfg(not(target_arch = "wasm32"))]
impl RecoveryStore {
    // pc: crates/format/src/autosave.rs:348 RecoveryStore::new; pdfcraft: crates/ui-egui/src/persist/recovery.rs:103 claim
    /// Opens the store in `dir`, creating the folder with private permissions, and locks an
    /// instance file in it that marks this run as alive. Removes orphaned temporary files that
    /// are older than the store's age limit.
    pub fn open(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static STORES: AtomicU64 = AtomicU64::new(0);
        let dir = dir.into();
        create_private_dir(&dir)?;
        let n = STORES.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let session = format!("{nanos:x}_{pid:x}_{n:x}");
        // The session string is in the name: a PID namespace can hand a new run its crashed
        // predecessor's PID and counter.
        let instance = format!("instance-{nanos:x}-{pid:x}-{n:x}.lock");
        let path = dir.join(&instance);
        let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        if let Err(e) = lock.try_lock() {
            let _ = std::fs::remove_file(&path);
            return Err(match e {
                std::fs::TryLockError::Error(e) => e,
                std::fs::TryLockError::WouldBlock => std::io::Error::new(std::io::ErrorKind::WouldBlock, "the instance lock is held"),
            });
        }
        sweep_temp_files(&dir, ORPHAN_AGE);
        Ok(Self { dir, session, adopted: std::sync::Mutex::new(HashMap::new()), instance, lock })
    }

    /// The recovery folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The key document `id` autosaves under.
    // pc: crates/format/src/autosave.rs:380 (doc-<session>-<id>)
    pub fn key(&self, id: u64) -> String {
        let adopted = self.adopted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        adopted.get(&id).cloned().unwrap_or_else(|| format!("doc-{}-{id}", self.session))
    }

    /// Writes a copy of document `id`; call from a job.
    pub fn write(&self, id: u64, meta: &RecoveryMeta, bytes: &[u8]) -> std::io::Result<()> {
        let key = self.key(id);
        let mut meta = meta.clone();
        meta.extension = sanitize_extension(&meta.extension);
        let copy = self.dir.join(format!("{key}.{}", meta.extension));
        craft_atomic::atomic_write(&copy, bytes)?;
        let sidecar = Sidecar { meta, instance: Some(self.instance.clone()) };
        let json = serde_json::to_vec(&sidecar).map_err(std::io::Error::other)?;
        craft_atomic::atomic_write(&self.dir.join(format!("{key}.json")), &json)
    }

    /// Removes document `id`'s copy (saved or closed).
    // pc: crates/format/src/autosave.rs:411 discard
    pub fn discard(&self, id: u64) -> std::io::Result<()> {
        let adopted = self.adopted.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
        let key = adopted.unwrap_or_else(|| format!("doc-{}-{id}", self.session));
        remove_key(&self.dir, &key)
    }

    /// Copies of runs that are gone, newest first; a copy without a readable sidecar is listed
    /// under its key with what the file system knows.
    // pc: crates/format/src/autosave.rs:361 recover
    pub fn leftovers(&self) -> Vec<RecoveryEntry> {
        let Ok(read) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        // Per key: the copies and whether a sidecar exists.
        let mut keys: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
        for e in read.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            let Some((stem, ext)) = name.split_once('.') else { continue };
            if stem.is_empty() || ext == "lock" || sanitize_key(stem) != stem {
                continue;
            }
            keys.entry(stem.to_owned()).or_default().push((ext.to_owned(), e.path()));
        }
        let mut out = Vec::new();
        for (key, files) in keys {
            let sidecar = files.iter().find(|(ext, _)| ext == "json").map(|(_, p)| p);
            let copies: Vec<&(String, PathBuf)> = files.iter().filter(|(ext, _)| ext != "json").collect();
            let parsed = sidecar.and_then(|p| read_sidecar(p));
            if let Some((side, _)) = &parsed
                && side.instance.as_deref().is_some_and(|i| i == self.instance || self.held_by_another(i))
            {
                continue;
            }
            let entry = match parsed {
                Some((side, saved)) => {
                    let wanted = format!("{key}.{}", side.meta.extension);
                    let path = copies
                        .iter()
                        .map(|(_, p)| p)
                        .find(|p| p.file_name().is_some_and(|n| *n == *wanted))
                        .or_else(|| copies.first().map(|(_, p)| p));
                    path.map(|path| RecoveryEntry { key, meta: side.meta, saved, path: path.clone() })
                }
                // No sidecar: the copy may be a first write in progress. It is a leftover only
                // when it is not this run's and no other instance is alive to be writing it.
                None if self.owns_key(&key) || self.other_instance_alive() => None,
                None => copies.first().map(|(ext, path)| {
                    let saved = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    let meta = RecoveryMeta { name: key.clone(), original: None, extension: ext.clone(), revision: 0 };
                    RecoveryEntry { key: key.clone(), meta, saved, path: path.clone() }
                }),
            };
            out.extend(entry);
        }
        out.sort_by(|a, b| b.saved.cmp(&a.saved).then_with(|| a.key.cmp(&b.key)));
        out
    }

    /// Whether `key` is one this run writes: a new document's or an adopted one.
    fn owns_key(&self, key: &str) -> bool {
        let adopted = self.adopted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        key.starts_with(&format!("doc-{}-", self.session)) || adopted.values().any(|k| k == key)
    }

    /// Whether another run holds its instance lock in this folder.
    fn other_instance_alive(&self) -> bool {
        let Ok(read) = std::fs::read_dir(&self.dir) else { return false };
        let names: Vec<String> = read.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
        names.iter().any(|n| n.starts_with("instance-") && n.ends_with(".lock") && *n != self.instance && self.held_by_another(n))
    }

    /// Whether `instance` (a name read from an untrusted sidecar) is the lock of another run that
    /// is still alive. A lock nobody holds is a crash's leftover and is removed.
    fn held_by_another(&self, instance: &str) -> bool {
        let plain = instance.starts_with("instance-")
            && instance.ends_with(".lock")
            && instance.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && !instance.contains("..");
        if !plain {
            return false;
        }
        let path = self.dir.join(instance);
        let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { return false };
        match file.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => true,
            Ok(()) => {
                let _ = file.unlock();
                drop(file);
                let _ = std::fs::remove_file(&path);
                false
            }
            // No file locks on this file system: offered, as if the run were gone.
            Err(std::fs::TryLockError::Error(_)) => false,
        }
    }

    /// Document `id` was recovered from `entry`: its autosaves go to `entry.key` from now on.
    // pc: crates/format/src/autosave.rs:367 adopt
    pub fn adopt(&self, id: u64, entry: &RecoveryEntry) {
        let key = sanitize_key(&entry.key);
        self.adopted.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, key);
    }

    /// Deletes a leftover the user discarded.
    pub fn remove(&self, entry: &RecoveryEntry) -> std::io::Result<()> {
        if sanitize_key(&entry.key) != entry.key {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "recovery key contains path components"));
        }
        remove_key(&self.dir, &entry.key)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RecoveryStore {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
        let _ = std::fs::remove_file(self.dir.join(&self.instance));
    }
}

/// The sidecar on disk: [`RecoveryMeta`] plus the lock name of the run that wrote it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(super) struct Sidecar {
    #[serde(flatten)]
    meta: RecoveryMeta,
    #[serde(default)]
    instance: Option<String>,
}

/// A sidecar larger than this is not read.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const MAX_SIDECAR: u64 = 1 << 20;

/// How long a temp file of an interrupted write must have been left before it is removed, far
/// longer than any write, so another run's write in progress is never touched.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const ORPHAN_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The parsed sidecar with its modification time, if it is a plain readable file.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_sidecar(path: &Path) -> Option<(Sidecar, SystemTime)> {
    let meta = std::fs::symlink_metadata(path).ok().filter(|m| m.is_file() && m.len() <= MAX_SIDECAR)?;
    let side: Sidecar = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((side, meta.modified().ok()?))
}

/// An extension safe in a file name: ASCII letters and digits; empty becomes `bin`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sanitize_extension(ext: &str) -> String {
    let s: String = ext.chars().filter(char::is_ascii_alphanumeric).collect();
    if s.is_empty() { "bin".into() } else { s }
}

/// Removes every file of `key` (copy, sidecar, whatever extension the copy had).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn remove_key(dir: &Path, key: &str) -> std::io::Result<()> {
    let mut result = Ok(());
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.split_once('.').is_some_and(|(stem, ext)| stem == key && ext != "lock")
            && let Err(err) = std::fs::remove_file(e.path())
            && err.kind() != std::io::ErrorKind::NotFound
        {
            result = Err(err);
        }
    }
    result
}

/// Creates `dir` readable by its owner only (0o700 on Unix).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Removes temp files of an interrupted atomic write that were last touched `min_age` ago.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sweep_temp_files(dir: &Path, min_age: Duration) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let is_temp = e.file_name().to_str().is_some_and(|n| n.starts_with('.') && n.ends_with(".craft_tmp"));
        let old = e
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= min_age);
        if is_temp && old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A key safe as a file stem: ASCII letters, digits, `-` and `_` stay, everything else becomes
/// `_`; an empty key becomes `untitled`.
// pc: crates/format/src/autosave.rs:67 sanitize
pub fn sanitize_key(key: &str) -> String {
    let s: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "untitled".into() } else { s }
}
