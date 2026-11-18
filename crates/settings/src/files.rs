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
    /// ([`broken_path`](crate::recovery::broken_path)'s naming).
    fn move_aside(&self, path: &Path) -> io::Result<PathBuf>;
    /// Writes the backup beside `path`, at [`backup_path`](crate::recovery::backup_path).
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

/// Settings kept in the browser's localStorage, keyed by path: the files of a web build.
///
/// Open the store with a [`Location::Dir`](crate::Location::Dir) naming the key root
/// (`config.toml` is stored under `<dir>/config.toml`); a storage the browser refuses or a full
/// quota comes back as an error, which the store reports like a disk's.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
pub struct BrowserFiles;

#[cfg(target_arch = "wasm32")]
impl BrowserFiles {
    /// The page's localStorage; an error where the page has none or the browser blocks it.
    fn storage() -> io::Result<web_sys::Storage> {
        let window = web_sys::window().ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no browser window"))?;
        window.local_storage().map_err(js_error)?.ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "browser storage is unavailable"))
    }

    /// The storage key of `path`: its text.
    fn key(path: &Path) -> io::Result<&str> {
        path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not valid text"))
    }
}

#[cfg(target_arch = "wasm32")]
impl SettingsFiles for BrowserFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        Self::storage()?.get_item(Self::key(path)?).map_err(js_error)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        Self::storage()?.set_item(Self::key(path)?, text).map_err(js_error)
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        let storage = Self::storage()?;
        let key = Self::key(path)?;
        let Some(text) = storage.get_item(key).map_err(js_error)? else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "nothing to move aside"));
        };
        let target = crate::recovery::free_broken_path(path, |target| Ok(storage.get_item(Self::key(target)?).map_err(js_error)?.is_some()))?;
        storage.set_item(Self::key(&target)?, &text).map_err(js_error)?;
        storage.remove_item(key).map_err(js_error)?;
        Ok(target)
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        let backup = crate::recovery::backup_path(path);
        Self::storage()?.set_item(Self::key(&backup)?, text).map_err(js_error)
    }

    /// Storage has no folders: any key root accepts items while the page has storage at all.
    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        let _ = dir;
        Self::storage().map(drop)
    }
}

/// A browser exception as an I/O error, so the store treats it like a disk's.
#[cfg(target_arch = "wasm32")]
fn js_error(e: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("browser storage: {e:?}"))
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
