//! Browser settings storage: [`LocalStorageFiles`].

use std::io;
use std::path::{Path, PathBuf};

use super::SettingsFiles;

/// How many `.aside-<n>` names `move_aside` tries before giving up.
const MAX_ASIDE: u32 = 1000;

/// Browser settings storage under a byte cap. A stored value over the cap reads as absent, so the store uses
/// defaults, and is not reported as damaged. The backup is keyed by
/// [`backup_path`](crate::recovery::backup_path).
///
/// Each file is one localStorage item keyed by the path's text, so [`Location::Dir`](crate::Location::Dir) names
/// the key prefix. The storage is looked up on every call rather than held, which keeps the type `Send + Sync`; a
/// browser page has one thread, so nothing is shared across threads.
#[derive(Clone, Copy, Debug)]
pub struct LocalStorageFiles {
    cap: usize,
}

impl LocalStorageFiles {
    /// Storage that ignores a stored value over `cap` bytes and refuses to write one.
    pub fn new(cap: usize) -> Self {
        Self { cap }
    }

    /// Writes `text` under `key` when it fits the cap; over it, the old item stays and the write fails.
    fn put(&self, storage: &web_sys::Storage, key: &str, text: &str) -> io::Result<()> {
        if text.len() > self.cap {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!("{} bytes is over the {} byte limit for browser settings", text.len(), self.cap),
            ));
        }
        storage.set_item(key, text).map_err(|e| io::Error::other(format!("localStorage refused the write: {e:?}")))
    }
}

/// The page's localStorage; an error when the browser has none or blocks it (private modes, a sandboxed frame).
fn storage() -> io::Result<web_sys::Storage> {
    let window = web_sys::window().ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no browser window"))?;
    match window.local_storage() {
        Ok(Some(storage)) => Ok(storage),
        Ok(None) => Err(io::Error::new(io::ErrorKind::Unsupported, "localStorage is not available")),
        Err(e) => Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("localStorage is blocked: {e:?}"))),
    }
}

/// The item key of `path`.
fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn get(storage: &web_sys::Storage, key: &str) -> io::Result<Option<String>> {
    storage.get_item(key).map_err(|e| io::Error::other(format!("localStorage refused the read: {e:?}")))
}

impl SettingsFiles for LocalStorageFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        let text = get(&storage()?, &key(path))?;
        // An oversized item is ignored rather than reported as damaged: the next save replaces it.
        Ok(text.filter(|text| text.len() <= self.cap))
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        self.put(&storage()?, &key(path), text)
    }

    /// Copies the item to `<stem>.aside-<n>.<ext>` and removes it. A counter stands in for the native rule's
    /// timestamp, which the browser's `SystemTime` cannot give.
    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        let storage = storage()?;
        let from = key(path);
        let Some(text) = get(&storage, &from)? else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "nothing to move aside"));
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("config");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("toml");
        for n in 1..=MAX_ASIDE {
            let target = path.with_file_name(format!("{stem}.aside-{n}.{ext}"));
            if get(&storage, &key(&target))?.is_some() {
                continue;
            }
            // The damaged text is kept whole even over the cap: it is the user's to recover.
            storage.set_item(&key(&target), &text).map_err(|e| io::Error::other(format!("localStorage refused the copy: {e:?}")))?;
            storage.remove_item(&from).map_err(|e| io::Error::other(format!("localStorage refused the removal: {e:?}")))?;
            return Ok(target);
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many damaged copies"))
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        self.put(&storage()?, &key(&crate::recovery::backup_path(path)), text)
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        storage().map(|_| ())
    }
}
