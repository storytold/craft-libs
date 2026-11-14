//! Test helpers for crates that open a [`SettingsStore`](crate::SettingsStore): one in-memory
//! [`SettingsFiles`] and the store's retry schedule.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::files::SettingsFiles;
pub use crate::store::retry_delay;

/// Settings files held in memory, for tests that open the store with
/// [`OpenOptions::files`](crate::OpenOptions::files). [`fail_writes`](Self::fail_writes) makes every write
/// fail until it is switched off, for the save-failure and retry cases.
// P3(top-w30): top-w19 decision, `craft_settings::testing::MemoryFiles`
#[derive(Debug, Default)]
pub struct MemoryFiles {
    #[allow(dead_code)] // P3(top-w30): MemoryFiles
    files: Mutex<BTreeMap<PathBuf, String>>,
    #[allow(dead_code)] // P3(top-w30): MemoryFiles
    fail_writes: AtomicBool,
}

impl MemoryFiles {
    /// No files, writes succeed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The text last written to `path`; `None` when nothing was.
    pub fn text(&self, path: &Path) -> Option<String> {
        let _ = path;
        todo!("top-w30")
    }

    /// While `fail` is on, every write and backup fails with an I/O error and keeps the old text.
    pub fn fail_writes(&self, fail: bool) {
        let _ = fail;
        todo!("top-w30")
    }
}

impl SettingsFiles for MemoryFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        let _ = path;
        todo!("top-w30")
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        let _ = (path, text);
        todo!("top-w30")
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        let _ = path;
        todo!("top-w30")
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        let _ = (path, text);
        todo!("top-w30")
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        todo!("top-w30")
    }
}
