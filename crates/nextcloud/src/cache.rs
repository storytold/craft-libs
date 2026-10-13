//! Downloaded originals on local disk, for apps that need a real file (a raw decoder, a video demuxer).
//!
//! Layout: `<dir>/<hex16(sha256(account_key))>/<hex64(sha256(path))>-<hex16(sha256(etag))>.<ext>`. Every name is
//! hex or an allow-listed extension, so a hostile remote name or etag can't escape `dir`. Sizes, last use and pins
//! live in `<dir>/index.json`; the files on disk are the truth, so a lost or corrupt index costs only last use
//! times and pins.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Client, Error, RemotePath};

const INDEX: &str = "index.json";
const LOCK: &str = ".lock";
const STAGE_SUFFIX: &str = ".craft_tmp";

/// Local copies of remote files, evicted least recently used first above a byte budget. Clones share one cache.
#[derive(Clone, Debug)]
pub struct OriginalsCache {
    inner: Arc<Mutex<Inner>>,
}

/// A local copy returned by [`OriginalsCache::fetch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub path: PathBuf,
    /// The server couldn't be asked (unreachable, busy or its certificate not trusted), so this is the newest copy on
    /// disk, which may be out of date.
    pub stale: bool,
}

/// Bytes on disk, see [`OriginalsCache::usage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    /// The part of `total` that [`OriginalsCache::pin`] keeps.
    pub pinned: u64,
}

/// Keeps a path's copies from eviction until dropped; see [`OriginalsCache::hold`]. A [`OriginalsCache::fetch`] that
/// downloads a newer version of the path still deletes the older copies, held or not.
#[derive(Debug)]
#[must_use = "dropping a Hold releases it at once"]
pub struct Hold {
    inner: Arc<Mutex<Inner>>,
    owner: String,
}

#[derive(Debug)]
struct Inner {
    dir: PathBuf,
    budget: u64,
    index: Index,
    /// Hold counts by owner (`<account hash>/<path hash>`); in memory only.
    holds: HashMap<String, usize>,
    /// The exclusive lock on `<dir>/.lock` (`None` where it can't be taken), released when the last clone is dropped.
    _lock: Option<fs::File>,
}

/// `index.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    /// By `<account hash>/<file name>`.
    entries: BTreeMap<String, Item>,
    /// `nextcloud://` path strings.
    pins: BTreeSet<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Item {
    size: u64,
    /// Unix seconds.
    last_use: u64,
}

impl OriginalsCache {
    /// Opens (creating) the cache in `dir`: locks it, loads or rebuilds the index, deletes stage files a crash left
    /// behind and evicts down to `budget` bytes. Fails if `dir` can't be created or read, or is already open.
    ///
    /// Holds an exclusive lock on `<dir>/.lock` for as long as any clone of the cache (or a [`Hold`] taken from it)
    /// lives. A second `open` of the directory meanwhile, from another process or this one, fails with an
    /// [`Error::Io`] of kind [`io::ErrorKind::WouldBlock`]; clone the open cache to share it. The lock is best-effort:
    /// where `.lock` can't be opened (even read-only) or the filesystem can't lock, `open` goes on without it, so a
    /// read-only cache stays usable.
    pub fn open(dir: &Path, budget: u64) -> Result<OriginalsCache, Error> {
        fs::create_dir_all(dir)?;
        // Before the sweep and the index rebuild, so a second opener never deletes the first one's stage files.
        let path = dir.join(LOCK);
        let lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path).or_else(|_| fs::File::open(&path)).ok();
        if let Some(Err(fs::TryLockError::WouldBlock)) = lock.as_ref().map(fs::File::try_lock) {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "The originals cache at {} is already open (another process, or a second OriginalsCache in this one); clone the open cache to share it",
                    dir.display()
                ),
            )));
        }
        let old = fs::read(dir.join(INDEX)).ok().and_then(|b| serde_json::from_slice::<Index>(&b).ok()).unwrap_or_default();
        let mut entries = BTreeMap::new();
        for acct in fs::read_dir(dir)?.flatten() {
            let name = acct.file_name().to_string_lossy().into_owned();
            if name.ends_with(STAGE_SUFFIX) {
                remove(&acct.path());
                continue;
            }
            if name.len() != 16 || !is_hex(&name) {
                continue;
            }
            // Not a folder, or unreadable: its files stay out of the cache rather than failing it.
            let Ok(files) = fs::read_dir(acct.path()) else { continue };
            for file in files.flatten() {
                let file_name = file.file_name().to_string_lossy().into_owned();
                if file_name.ends_with(STAGE_SUFFIX) {
                    remove(&file.path());
                    continue;
                }
                let Ok(meta) = file.metadata() else { continue };
                if !meta.is_file() || !is_entry_name(&file_name) {
                    continue;
                }
                let key = format!("{name}/{file_name}");
                let last_use = old.entries.get(&key).map_or_else(|| mtime(&meta), |i| i.last_use);
                entries.insert(key, Item { size: meta.len(), last_use });
            }
        }
        let mut inner = Inner { dir: dir.to_path_buf(), budget, index: Index { entries, pins: old.pins }, holds: HashMap::new(), _lock: lock };
        inner.evict(None);
        // A full disk or read-only folder must not cut the app off from its offline copies.
        let _ = inner.save();
        Ok(OriginalsCache { inner: Arc::new(Mutex::new(inner)) })
    }

    /// The local copy of `path` at `etag`, if there is one; no network. Counts as a use. Take a
    /// [`hold`](Self::hold) first if the file must stay on disk while it is read.
    pub fn cached(&self, path: &RemotePath, etag: &str) -> Option<PathBuf> {
        self.lock().use_entry(&entry_key(path, etag))
    }

    /// The newest local copy of `path` (most recent last use, any etag); no network. `None` when no copy is on disk.
    /// Counts as a use. For offline lookups when the app has no etag (e.g. the server is unreachable). Take a
    /// [`hold`](Self::hold) first, as with [`fetch`](Self::fetch) and [`cached`](Self::cached).
    pub fn newest(&self, path: &RemotePath) -> Option<PathBuf> {
        self.lock().use_newest(&owner(path))
    }

    /// A local copy of `path`'s current version, downloading it on a miss (see [`Client::download_to`] for `cancel`
    /// and `progress`). A server that is unreachable, busy (`RateLimited`, e.g. in maintenance mode) or whose
    /// certificate isn't trusted (`CertificateChanged`, `UnknownCertificate`) returns the newest local copy marked
    /// stale, or the error if there is none. A crash, cancel or full disk never leaves a partial file under the final
    /// name. Take a
    /// [`hold`](Self::hold) first, so another fetch's eviction can't delete the file before it is opened.
    pub fn fetch(
        &self,
        client: &Client,
        path: &RemotePath,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Fetched, Error> {
        let etag = match client.stat(path) {
            Ok(entry) => entry.etag,
            // None of these mean the file is gone, and the local bytes don't depend on trusting the server.
            Err(e @ (Error::Unreachable(_) | Error::RateLimited | Error::CertificateChanged | Error::UnknownCertificate { .. })) => {
                return self.newest(path).map(|path| Fetched { path, stale: true }).ok_or(e);
            }
            Err(e) => return Err(e),
        };
        let key = entry_key(path, &etag);
        let file = {
            let mut inner = self.lock();
            if let Some(path) = inner.use_entry(&key) {
                return Ok(Fetched { path, stale: false });
            }
            inner.dir.join(&key)
        };
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        // ponytail: concurrent fetches of one path both download (last rename wins); track in-flight paths if that
        // shows up.
        let mut size = 0;
        let mut failure = None;
        let written = craft_atomic::atomic_write_with(&file, |f| match client.download_to(path, f, cancel, progress) {
            Ok(n) => {
                size = n;
                Ok(())
            }
            Err(e) => {
                let io = io::Error::other(e.to_string());
                failure = Some(e);
                Err(io)
            }
        });
        if let Err(e) = written {
            return Err(failure.unwrap_or(Error::Io(e)));
        }

        let mut inner = self.lock();
        let owner = owner(path);
        let others: Vec<String> = inner.index.entries.keys().filter(|k| owner_of(k) == owner && **k != key).cloned().collect();
        for other in others {
            if remove(&inner.dir.join(&other)) {
                inner.index.entries.remove(&other);
            }
        }
        inner.index.entries.insert(key.clone(), Item { size, last_use: now() });
        inner.evict(Some(&key));
        // The download is in place; a failed index write loses only last use times.
        let _ = inner.save();
        Ok(Fetched { path: file, stale: false })
    }

    /// Keeps every copy of `path` from eviction, across restarts ("Make Available Offline").
    pub fn pin(&self, path: &RemotePath) -> Result<(), Error> {
        let mut inner = self.lock();
        inner.index.pins.insert(path.to_string());
        inner.save()
    }

    /// Undoes [`OriginalsCache::pin`].
    pub fn unpin(&self, path: &RemotePath) -> Result<(), Error> {
        let mut inner = self.lock();
        inner.index.pins.remove(&path.to_string());
        inner.save()
    }

    /// Keeps `path`'s copies from eviction until the returned [`Hold`] is dropped (a file open in a decoder). Take it
    /// before [`fetch`](Self::fetch) or [`cached`](Self::cached), so a concurrent fetch's eviction can't delete the
    /// file in between. A newer version fetched meanwhile still replaces the older copies.
    pub fn hold(&self, path: &RemotePath) -> Hold {
        let owner = owner(path);
        *self.lock().holds.entry(owner.clone()).or_default() += 1;
        Hold { inner: self.inner.clone(), owner }
    }

    /// Bytes on disk, and how many of them are pinned.
    pub fn usage(&self) -> Usage {
        let inner = self.lock();
        let pinned = inner.pinned();
        let sum = |pinned_only: bool| {
            inner.index.entries.iter().filter(|(k, _)| !pinned_only || pinned.contains(owner_of(k))).fold(0u64, |t, (_, i)| t.saturating_add(i.size))
        };
        Usage { total: sum(false), pinned: sum(true) }
    }

    /// Changes the budget, evicting down to it.
    pub fn set_budget(&self, budget: u64) {
        let mut inner = self.lock();
        inner.budget = budget;
        inner.evict(None);
        let _ = inner.save();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut inner = lock(&self.inner);
        if let Some(n) = inner.holds.get_mut(&self.owner) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                inner.holds.remove(&self.owner);
            }
        }
    }
}

impl Inner {
    /// The file for `key` if it is still on disk, with its last use bumped; drops the entry if the file is gone.
    fn use_entry(&mut self, key: &str) -> Option<PathBuf> {
        let file = self.dir.join(key);
        if !file.is_file() {
            self.index.entries.remove(key);
            return None;
        }
        // In memory only: the next index write (a download, pin, unpin or budget change) persists it.
        self.index.entries.get_mut(key)?.last_use = now();
        Some(file)
    }

    /// The most recently used entry of `owner`, any etag, that is still on disk (see [`Inner::use_entry`]).
    fn use_newest(&mut self, owner: &str) -> Option<PathBuf> {
        loop {
            let mine = self.index.entries.iter().filter(|(k, _)| owner_of(k) == owner);
            let key = mine.max_by_key(|(k, i)| (i.last_use, *k)).map(|(k, _)| k.clone())?;
            // A gone file drops its entry, so the next pass sees the runner-up.
            if let Some(file) = self.use_entry(&key) {
                return Some(file);
            }
        }
    }

    /// Owners of pinned entries.
    fn pinned(&self) -> HashSet<String> {
        self.index.pins.iter().filter_map(|p| RemotePath::parse(p).ok()).map(|p| owner(&p)).collect()
    }

    /// Deletes least recently used entries until within budget, sparing pinned, held and `keep`.
    fn evict(&mut self, keep: Option<&str>) {
        let pinned = self.pinned();
        let mut total = self.index.entries.values().fold(0u64, |t, i| t.saturating_add(i.size));
        let mut victims: Vec<(u64, String)> = (self.index.entries.iter())
            .filter(|(k, _)| Some(k.as_str()) != keep && !pinned.contains(owner_of(k)) && !self.holds.contains_key(owner_of(k)))
            .map(|(k, i)| (i.last_use, k.clone()))
            .collect();
        // Same-second last uses fall back to file name order, so eviction is deterministic.
        victims.sort();
        for (_, key) in victims {
            if total <= self.budget {
                break;
            }
            if remove(&self.dir.join(&key))
                && let Some(item) = self.index.entries.remove(&key)
            {
                total = total.saturating_sub(item.size);
            }
        }
    }

    fn save(&self) -> Result<(), Error> {
        let json = serde_json::to_vec(&self.index).map_err(|e| Error::Io(io::Error::other(e)))?;
        Ok(craft_atomic::atomic_write(&self.dir.join(INDEX), &json)?)
    }
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `<account hash>/<path hash>`: the prefix shared by every copy of `path`.
fn owner(path: &RemotePath) -> String {
    format!("{}/{}", hash(&path.account_key(), 8), hash(&path.to_string(), 32))
}

/// The owner part of an entry key.
fn owner_of(key: &str) -> &str {
    key.split_once('-').map_or(key, |(owner, _)| owner)
}

/// `<account hash>/<path hash>-<etag hash>.<ext>`.
fn entry_key(path: &RemotePath, etag: &str) -> String {
    format!("{}-{}.{}", owner(path), hash(etag, 8), extension(path.name().unwrap_or_default()))
}

/// Lowercase hex of the first `bytes` bytes of `sha256(s)`.
fn hash(s: &str, bytes: usize) -> String {
    Sha256::digest(s).iter().take(bytes).map(|b| format!("{b:02x}")).collect()
}

/// The lowercased extension of `name` if it is 1–8 of `[a-z0-9]`, else `bin`.
fn extension(name: &str) -> String {
    let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase());
    if is_ext(&ext) { ext } else { "bin".into() }
}

fn is_ext(s: &str) -> bool {
    (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `^[0-9a-f]{64}-[0-9a-f]{16}\.[a-z0-9]{1,8}$`.
fn is_entry_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else { return false };
    let Some((path, etag)) = stem.split_once('-') else { return false };
    path.len() == 64 && is_hex(path) && etag.len() == 16 && is_hex(etag) && is_ext(ext)
}

/// Deletes a file; true if it is gone (already gone counts).
fn remove(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn mtime(meta: &Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_allow_listed() {
        for (name, ext) in [("_DSC1748.ARW", "arw"), ("a.tar.GZ", "gz"), ("noext", "bin"), ("x.A R W", "bin"), ("x.verylongext", "bin")] {
            assert_eq!(extension(name), ext, "{name}");
        }
    }

    #[test]
    fn handles_cross_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<OriginalsCache>();
        shareable::<Hold>();
    }

    #[test]
    fn entry_names() {
        let good = format!("{}-{}.arw", "a".repeat(64), "0".repeat(16));
        assert!(is_entry_name(&good));
        assert!(!is_entry_name(&good.replace(".arw", ".ARW")));
        assert!(!is_entry_name(&good.replace(".arw", ".tar.gz")));
        assert!(!is_entry_name(&good.replacen('a', "A", 1)));
        assert!(!is_entry_name("index.json"));
    }
}
