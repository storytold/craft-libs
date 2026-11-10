//! The suite folder on disk. Reads take no lock: every write is an atomic rename, so a reader
//! sees either the old or the new file.

use std::{
    cell::Cell,
    fs::{self, File, OpenOptions, Permissions, TryLockError},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use craft_atomic::{AtomicFs, NewFilePermissions, RenameRetry, WriteOptions, retry_rename};

use crate::backend::LockError;
use crate::backend::{FileStamp, LibraryFile, WriteError};

const SELECTION: &str = "selection.json";
const LOCK: &str = "selection.lock";
const LIBRARY: &str = "library";
const LOCK_RETRY: Duration = Duration::from_millis(20);

pub(crate) struct Disk {
    pub dir: PathBuf,
    pub fs: Box<dyn AtomicFs + Send + Sync>,
}

fn private() -> WriteOptions {
    WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private)
}

impl Disk {
    pub fn selection_path(&self) -> PathBuf {
        self.dir.join(SELECTION)
    }

    pub fn theme_path(&self, stem: &str) -> PathBuf {
        self.dir.join(LIBRARY).join(format!("{stem}.json"))
    }

    /// `selection.lock` is never deleted: removing it while another process holds it would let a
    /// third lock a fresh file.
    pub fn lock(&self, timeout: Duration) -> Result<File, LockError> {
        fs::create_dir_all(&self.dir).map_err(LockError::Io)?;
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.dir.join(LOCK)).map_err(LockError::Io)?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) if start.elapsed() < timeout => std::thread::sleep(LOCK_RETRY),
                Err(TryLockError::WouldBlock) => return Err(LockError::Busy),
                // No locking here (some network file systems): write anyway; the re-check before
                // the rename still catches a concurrent writer.
                Err(TryLockError::Error(e)) if locking_unsupported(&e) => return Ok(file),
                Err(TryLockError::Error(e)) => return Err(LockError::Io(e)),
            }
        }
    }

    pub fn read_capped(&self, path: &Path, cap: usize) -> io::Result<Option<Vec<u8>>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take((cap as u64).saturating_add(1)).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    pub fn stamp(&self, path: &Path) -> io::Result<Option<FileStamp>> {
        match fs::metadata(path) {
            Ok(meta) => Ok(Some(stamp_of(&meta))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn write_selection(&self, bytes: &[u8], expected: Option<&[u8]>) -> Result<(), WriteError> {
        let path = self.selection_path();
        let guard = Guarded { inner: &*self.fs, path: &path, expected, raced: Cell::new(false) };
        match private().atomic_write_custom(&guard, RenameRetry::platform(), &path, bytes) {
            Ok(()) => Ok(()),
            Err(_) if guard.raced.get() => Err(WriteError::Raced),
            Err(e) => Err(WriteError::Io(e)),
        }
    }

    pub fn move_aside_selection(&self) -> io::Result<PathBuf> {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for n in 0..1000u32 {
            let name = if n == 0 { format!("selection.broken-{ts}.json") } else { format!("selection.broken-{ts}-{n}.json") };
            let target = self.dir.join(name);
            if !target.exists() {
                retry_rename(RenameRetry::platform(), || self.fs.rename(&self.selection_path(), &target))?;
                return Ok(target);
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many broken copies of selection.json"))
    }

    pub fn list(&self) -> io::Result<Vec<LibraryFile>> {
        let entries = match fs::read_dir(self.dir.join(LIBRARY)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            // Hidden names are craft-atomic's staging files; other extensions aren't themes.
            let Some(stem) = name.to_str().filter(|n| !n.starts_with('.')).and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            // A file removed after the listing is simply not listed.
            if let Ok(meta) = fs::metadata(entry.path())
                && meta.is_file()
            {
                out.push(LibraryFile { stem: stem.to_owned(), stamp: stamp_of(&meta) });
            }
        }
        out.sort_by(|a, b| a.stem.cmp(&b.stem));
        Ok(out)
    }

    pub fn restore_selection(&self, moved: &Path) -> io::Result<()> {
        retry_rename(RenameRetry::platform(), || self.fs.rename(moved, &self.selection_path()))
    }

    /// `library/.<stem>.json.deleting`: hidden, so listings skip it.
    fn hidden_theme_path(&self, stem: &str) -> PathBuf {
        self.dir.join(LIBRARY).join(format!(".{stem}.json.deleting"))
    }

    pub fn hide_theme(&self, stem: &str) -> io::Result<()> {
        retry_rename(RenameRetry::platform(), || self.fs.rename(&self.theme_path(stem), &self.hidden_theme_path(stem)))
    }

    pub fn unhide_theme(&self, stem: &str) -> io::Result<()> {
        retry_rename(RenameRetry::platform(), || self.fs.rename(&self.hidden_theme_path(stem), &self.theme_path(stem)))
    }

    pub fn purge_hidden_theme(&self, stem: &str) -> io::Result<()> {
        self.fs.remove(&self.hidden_theme_path(stem))
    }

    pub fn create_theme(&self, stem: &str, bytes: &[u8]) -> io::Result<()> {
        fs::create_dir_all(self.dir.join(LIBRARY))?;
        private().write_new_custom(&*self.fs, &self.theme_path(stem), bytes)
    }

    pub fn remove_theme(&self, stem: &str) -> io::Result<()> {
        self.fs.remove(&self.theme_path(stem))
    }
}

/// `try_lock` reports that the file system has no locks (`Unsupported`, or `ENOLCK`).
fn locking_unsupported(e: &io::Error) -> bool {
    #[cfg(target_os = "linux")]
    const ENOLCK: i32 = 37;
    #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
    const ENOLCK: i32 = 77;
    #[cfg(not(any(
        target_os = "linux",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    const ENOLCK: i32 = -1;
    e.kind() == io::ErrorKind::Unsupported || e.raw_os_error() == Some(ENOLCK)
}

fn stamp_of(meta: &fs::Metadata) -> FileStamp {
    let modified_nanos = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos());
    #[cfg(unix)]
    let identity = std::os::unix::fs::MetadataExt::ino(meta);
    #[cfg(not(unix))]
    let identity = 0;
    FileStamp { len: meta.len(), modified_nanos, identity }
}

/// Checks `selection.json` once more just before the rename that replaces it. Advisory locks
/// can be unreliable on network file systems; a file that changed under the lock means another
/// writer, and the write is abandoned instead of overwriting its change.
struct Guarded<'a> {
    inner: &'a dyn AtomicFs,
    path: &'a Path,
    expected: Option<&'a [u8]>,
    raced: Cell<bool>,
}

impl AtomicFs for Guarded<'_> {
    fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
        self.inner.create_new(path, permissions)
    }

    fn sync(&self, file: &File) -> io::Result<()> {
        self.inner.sync(file)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let now = match fs::read(self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if now.as_deref() != self.expected {
            self.raced.set(true);
            return Err(io::Error::other("selection.json changed while it was being written"));
        }
        self.inner.rename(from, to)
    }

    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.inner.hard_link(from, to)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.inner.remove(path)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.inner.sync_dir(dir)
    }

    fn set_permissions(&self, file: &File, perms: Permissions) -> io::Result<()> {
        self.inner.set_permissions(file, perms)
    }
}
