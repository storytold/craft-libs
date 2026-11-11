//! [`SettingsFiles`]: the file access a [`SettingsStore`](crate::SettingsStore) goes through, so an
//! app can route its settings elsewhere (PhotoCraft's `Services` file closures, browser storage).

use std::io;
use std::path::{Path, PathBuf};

/// How the store reads and writes its files. [`DiskFiles`] is the file system, the default; an
/// app plugs in its own through [`OpenOptions::files`](crate::OpenOptions::files).
///
/// Every method is called on the thread that ticks the store. The store keeps its rules (backup
/// before overwrite, a damaged file moved aside, owner-only permissions) and asks this trait only
/// for the operations.
pub trait SettingsFiles: Send + Sync {
    /// The file's text; `Ok(None)` when it doesn't exist.
    fn read(&self, path: &Path) -> io::Result<Option<String>>;
    /// Replaces the file atomically; a new file is owner-only.
    fn write(&self, path: &Path, text: &str) -> io::Result<()>;
    /// Renames a damaged file aside and returns the new path
    /// ([`move_aside`](crate::recovery::move_aside)'s naming).
    fn move_aside(&self, path: &Path) -> io::Result<PathBuf>;
    /// Writes the backup beside `path` ([`write_backup`](crate::recovery::write_backup)'s rules).
    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()>;
    /// Makes sure the folder exists before the store checks it. Files that have no folders
    /// (browser storage) keep the default, which does nothing.
    fn create_dir(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        Ok(())
    }
    /// Whether `dir` accepts a new file.
    fn check_writable(&self, dir: &Path) -> io::Result<()>;
}

/// The file system, through `craft-atomic` and [`recovery`](crate::recovery): what the store does
/// today, unchanged.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskFiles;

#[cfg(not(target_arch = "wasm32"))]
impl SettingsFiles for DiskFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        use std::io::Write as _;
        craft_atomic::WriteOptions::default().with_new_file_permissions(craft_atomic::NewFilePermissions::Private).atomic_write_with(path, |file| {
            restrict_permissions(file)?;
            file.write_all(text.as_bytes())
        })
    }

    fn create_dir(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        crate::recovery::move_aside(path)
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        crate::recovery::write_backup(path, text)
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        crate::recovery::check_writable(dir)
    }
}

/// New files are owner-only; on a replace `craft-atomic` keeps the existing file's mode.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn restrict_permissions(file: &std::fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn restrict_permissions(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}
