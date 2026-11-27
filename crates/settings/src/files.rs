//! [`SettingsFiles`]: the file access a [`SettingsStore`](crate::SettingsStore) goes through, so an
//! app can route its settings elsewhere (PhotoCraft's `Services` file closures, browser storage).

#[cfg(target_arch = "wasm32")]
mod local_storage;

#[cfg(target_arch = "wasm32")]
pub use local_storage::LocalStorageFiles;

use std::any::Any;
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
    /// Takes a `.lock` file beside the settings file at `path`, held from the disk check to the write; dropping the
    /// box releases it. Files with nothing to lock keep the default, which holds none.
    fn lock(&self, path: &Path) -> Option<Box<dyn Any + Send>> {
        let _ = path;
        None
    }
}

/// The file system, through `craft-atomic` and [`recovery`](crate::recovery): what the store does
/// today, unchanged.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskFiles;

#[cfg(not(target_arch = "wasm32"))]
impl SettingsFiles for DiskFiles {
    /// A file larger than [`MAX_CONFIG_BYTES`](crate::store::MAX_CONFIG_BYTES) fails with
    /// `ErrorKind::FileTooLarge` without being read into memory; text that is not UTF-8 fails with
    /// `ErrorKind::InvalidData`, as `std::fs::read_to_string` does.
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        use std::io::Read as _;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let limit = crate::store::MAX_CONFIG_BYTES;
        if file.metadata()?.len() > limit {
            return Err(crate::store::too_large());
        }
        // The length can change after the check, or be unknown (a pipe), so the read is capped too.
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(crate::store::too_large());
        }
        String::from_utf8(bytes).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
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

    /// An advisory lock on `.<file>.lock` beside `path`, so two processes saving at once (logging out closes every
    /// window together) take turns at checking, merging and replacing the file instead of one losing the other's
    /// changes. `None` (save unlocked) when the lock file cannot be opened or is not free within `LOCK_WAIT`.
    fn lock(&self, path: &Path) -> Option<Box<dyn Any + Send>> {
        let name = path.file_name().map_or_else(|| "config.toml".into(), |n| n.to_string_lossy().into_owned());
        let lock = path.with_file_name(format!(".{name}.lock"));
        let file = match std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("settings: cannot open {}, saving without it: {e}", lock.display());
                return None;
            }
        };
        let start = std::time::Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Some(Box::new(file)),
                Err(std::fs::TryLockError::WouldBlock) if start.elapsed() < LOCK_WAIT => std::thread::sleep(LOCK_POLL),
                Err(e) => {
                    log::warn!("settings: {} is held, saving without it: {e}", lock.display());
                    return None;
                }
            }
        }
    }
}

/// How long a save waits for another process's save to finish before going ahead anyway: a save takes
/// milliseconds, and a process stuck holding the lock must not stop this one from saving.
#[cfg(not(target_arch = "wasm32"))]
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// How often a waiting save tries the lock again.
#[cfg(not(target_arch = "wasm32"))]
const LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(10);

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
