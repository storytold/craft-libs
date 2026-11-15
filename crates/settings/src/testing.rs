//! Test helpers for crates that open a [`SettingsStore`](crate::SettingsStore): one in-memory
//! [`SettingsFiles`] and the store's retry schedule.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::files::SettingsFiles;
pub use crate::store::retry_delay;

/// Settings files held in memory, for tests that open the store with
/// [`OpenOptions::files`](crate::OpenOptions::files). [`fail_writes`](Self::fail_writes) makes every write
/// fail until it is switched off, for the save-failure and retry cases.
#[derive(Debug, Default)]
pub struct MemoryFiles {
    files: Mutex<BTreeMap<PathBuf, String>>,
    fail_writes: AtomicBool,
}

impl MemoryFiles {
    /// No files, writes succeed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The text last written to `path`; `None` when nothing was.
    pub fn text(&self, path: &Path) -> Option<String> {
        self.files().get(path).cloned()
    }

    /// While `fail` is on, every write and backup fails with an I/O error and keeps the old text.
    pub fn fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }

    /// A test that panicked while holding the lock leaves the map usable for the next one.
    fn files(&self) -> MutexGuard<'_, BTreeMap<PathBuf, String>> {
        self.files.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_write(&self) -> io::Result<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(io::Error::other("writes are failing (MemoryFiles::fail_writes)"));
        }
        Ok(())
    }
}

impl SettingsFiles for MemoryFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        Ok(self.files().get(path).cloned())
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        self.check_write()?;
        self.files().insert(path.to_path_buf(), text.to_owned());
        Ok(())
    }

    /// Renames to `<stem>.broken-<n>.<ext>`, the first `n` not taken; the disk version's
    /// timestamp is left out so tests can name the result.
    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        self.check_write()?;
        let mut files = self.files();
        let Some(text) = files.remove(path) else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "nothing to move aside"));
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("config");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("toml");
        let target = (0u32..)
            .map(|n| path.with_file_name(format!("{stem}.broken-{n}.{ext}")))
            .find(|candidate| !files.contains_key(candidate))
            .unwrap_or_else(|| path.with_file_name(format!("{stem}.broken.{ext}")));
        files.insert(target.clone(), text);
        Ok(target)
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        self.check_write()?;
        self.files().insert(crate::recovery::backup_path(path), text.to_owned());
        Ok(())
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        Ok(())
    }
}
