//! Browser settings storage: [`LocalStorageFiles`].

use std::io;
use std::path::{Path, PathBuf};

use super::SettingsFiles;

/// Browser settings storage under a byte cap. A stored value over the cap reads as absent, so the store uses
/// defaults, and is not reported as damaged. The backup is keyed by
/// [`backup_path`](crate::recovery::backup_path).
#[derive(Clone, Copy, Debug)]
pub struct LocalStorageFiles {
    cap: usize,
}

impl LocalStorageFiles {
    /// Storage that ignores a stored value over `cap` bytes.
    pub fn new(cap: usize) -> Self {
        let _ = cap;
        todo!("pf.C-S4")
    }
}

impl SettingsFiles for LocalStorageFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        let _ = (self.cap, path);
        todo!("pf.C-S4")
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        let _ = (path, text);
        todo!("pf.C-S4")
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        let _ = path;
        todo!("pf.C-S4")
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        let _ = (path, text);
        todo!("pf.C-S4")
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        todo!("pf.C-S4")
    }
}
