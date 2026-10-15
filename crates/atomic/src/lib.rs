#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Crash-safe file replacement: the unified atomic save engine for the Craft suite.
//!
//! [`atomic_write`] never truncates the destination file in place. It writes a temporary file
//! in the **same directory** as the destination, flushes it to non-volatile storage (`sync_all`),
//! renames it over the destination atomically, and on Unix, attempts to flush the directory so the rename
//! survives a power loss. If write or rename fails, the destination retains its original bytes and
//! the temporary staging file is removed.
//!
//! ### Guarantees: Atomicity vs. Durability
//! - **Atomicity Guarantee (All-or-Nothing)**:
//!   [`atomic_write`] guarantees that external readers never observe a partial write. Either the target
//!   file retains its previous bytes, or it is replaced completely with the new contents. If an error occurs
//!   during staging or rename, the target remains unmodified and the staging file is removed in user space.
//!   An abnormal process termination (e.g. `SIGKILL`, crash, power loss) bypasses user-space cleanup and
//!   leaves the temporary staging file on disk. Sweeping orphaned staging files is the responsibility of the
//!   consuming application; the library does not perform background sweeping. The destination file itself is
//!   never corrupted.
//! - **Durability & Ordering Boundaries**:
//!   Data persistence across power failures involves two sync points:
//!   1. **Content Flush (`sync_file`)**: The staging file's payload is forced to non-volatile storage
//!      (`F_FULLFSYNC` on macOS, `sync_all` on Windows/Linux) *before* the atomic rename.
//!   2. **Directory Flush (`sync_dir`, Unix only)**: On Unix, flushing the parent directory table is
//!      attempted *after* the rename on a best-effort basis. If directory sync fails, the error is ignored
//!      because the rename has already committed and the destination has already been replaced.
//!   3. **Hardware Durability Limits**: While this enforces strict kernel dispatch order
//!      (`sync_file < rename < sync_dir`), software flushes cannot guarantee data survival across
//!      unbuffered drive controller write-caching or physical power cutoffs.
//!
//! ### Platform Characteristics
//! - **Windows**: Antivirus scanners, search indexers, and backup utilities briefly hold files
//!   open with sharing violations. The rename is retried with exponential backoff (`RenameRetry`).
//! - **Cross-Device (`EXDEV`)**: Staging files are always created beside the destination in the
//!   same folder, guaranteeing same-filesystem replace semantics.
//! - **Apple Network Shares (macOS)**: `sync_all` issues `F_FULLFSYNC`. Where SMB shares or
//!   virtual filesystems reject this with `ENOTSUP` or `ENOTTY`, [`sync_file`] falls back to standard
//!   `fsync`.
//! - **Web / WASM**: On `wasm32`, returns standard unsupported `io::Error` rather than panicking.
//! - **Failure Injection**: [`atomic_write_custom`] accepts an [`AtomicFs`] implementation so
//!   callers and test suites can inject disk-full, lock-contention, or permission errors at each stage.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// File-system operations performed by atomic writes.
/// [`RealFs`] provides the production implementation; test suites implement this to inject faults.
pub trait AtomicFs {
    /// Create `path`, which must not exist, and open it for writing.
    /// On Unix, create it with owner-only permissions so payload written afterward is private.
    fn create_new(&self, path: &Path) -> io::Result<File>;
    /// Flush a written file to durable storage.
    fn sync(&self, file: &File) -> io::Result<()>;
    /// Rename `from` over `to`, replacing it atomically.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Give the file `from` the second name `to`; fails with `AlreadyExists` when `to` exists.
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Remove a file.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// Flush a directory entry change to durable storage (Unix); no-op elsewhere.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Give the open temporary file the permissions of the file it replaces.
    fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()>;
}

/// The production file system implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl AtomicFs for RealFs {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Set the mode at creation: chmod afterward leaves a window in which another
            // user can open the stage, including a stage left behind by an abrupt exit.
            options.mode(0o600);
        }
        options.open(path)
    }

    fn sync(&self, file: &File) -> io::Result<()> {
        sync_file(file)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::hard_link(from, to)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            sync_file(&File::open(dir)?)
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
            Ok(())
        }
    }

    fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()> {
        file.set_permissions(perms)
    }
}

/// Flush `file` to stable storage (`sync_all`).
/// On Apple platforms, this issues an `F_FULLFSYNC`. Where the file system rejects full sync
/// (such as SMB network shares or virtual file systems), it falls back to a plain `fsync`.
pub fn sync_file(file: &File) -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        or_plain_fsync(file.sync_all(), || rustix::fs::fsync(file).map_err(io::Error::from))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        file.sync_all()
    }
}

/// Returns `full`, or calls `fsync()` if `full` failed only because `F_FULLFSYNC` is unsupported.
pub fn or_plain_fsync(full: io::Result<()>, fsync: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    match full {
        Err(e) if full_flush_unsupported(&e) => fsync(),
        other => other,
    }
}

fn full_flush_unsupported(e: &io::Error) -> bool {
    const ENOTTY: i32 = 25;
    const ENOTSUP: i32 = 45;
    cfg!(target_vendor = "apple") && matches!(e.raw_os_error(), Some(ENOTSUP | ENOTTY))
}

/// Retry configuration for renames that encounter transient file locks (see [`retry_rename`]).
#[derive(Debug, Clone, Copy)]
pub struct RenameRetry {
    /// Number of retries after the initial attempt (0 = no retry).
    pub attempts: u32,
    /// Delay before the first retry in milliseconds; doubles on each subsequent attempt, up to
    /// 200 ms per attempt.
    pub first_delay_ms: u64,
}

impl RenameRetry {
    /// Platform default: retry on Windows (7 attempts with 10ms exponential backoff), no-op elsewhere.
    pub fn platform() -> Self {
        if cfg!(windows) { Self { attempts: 7, first_delay_ms: 10 } } else { Self { attempts: 0, first_delay_ms: 0 } }
    }
}

/// Atomically writes `bytes` to `path`, replacing any existing content.
///
/// If write or rename fails, the target file is untouched and any temporary staging file is removed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_custom(&RealFs, RenameRetry::platform(), path, bytes)
}

/// Atomically writes to `path` using a streaming writer callback `F`.
///
/// On Unix, the staging file starts with mode `0600` (restricted by the process umask).
/// Existing destination permissions are restored after the callback. For a new destination,
/// callback-selected permissions are kept, or the file remains private by default.
pub fn atomic_write_with<F>(path: &Path, write_fn: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    replace(&RealFs, RenameRetry::platform(), path, write_fn)
}

/// Atomically writes `bytes` to `path` through custom [`AtomicFs`] and [`RenameRetry`].
pub fn atomic_write_custom(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace(fs, retry, path, |f| f.write_all(bytes))
}

/// The replace path behind every `atomic_write*` function: stage the new content beside the
/// target, then rename it over the target.
fn replace(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let target = resolve_links(path)?;
    let (dir, name) = split(&target, path)?;
    let existing = std::fs::metadata(&target).ok();
    let existed = existing.is_some();
    if existing.as_ref().is_some_and(|m| m.is_file()) {
        check_writable(&target).map_err(|e| read_only_error(e, path))?;
    }

    let tmp = stage(fs, &dir, &name, path, existing.as_ref(), fill)?;

    if let Err(e) = retry_rename(retry, || fs.rename(&tmp, &target)) {
        let _ = fs.remove(&tmp);
        return Err(rename_error(e, path, existed));
    }

    // The new bytes are in place; a failed directory fsync can't be undone and must not report
    // the save as failed, so it is best effort.
    let _ = fs.sync_dir(&dir);
    Ok(())
}

/// A rename replaces a file the user can't write (Unix only needs the folder to be writable), so
/// refuse, as writing it in place would. Opening for writing without truncating changes nothing.
#[cfg(unix)]
fn check_writable(target: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(target).map(drop)
}

/// Windows refuses to rename over a read-only file by itself.
#[cfg(not(unix))]
fn check_writable(_: &Path) -> io::Result<()> {
    Ok(())
}

/// The folder `target` is in and its file name. `path` is the caller's path, for the error.
fn split(target: &Path, path: &Path) -> io::Result<(PathBuf, String)> {
    let name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: not a file path", path.display())))?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((dir, name.to_string_lossy().into_owned()))
}

/// Create a temporary file in `dir`, fill it with `fill`, give it the permissions of the file it
/// replaces (`replaced`, Unix) and flush it. A leftover file with the same name is skipped. When
/// any step fails, the temporary file is removed.
fn stage(
    fs: &dyn AtomicFs,
    dir: &Path,
    name: &str,
    path: &Path,
    replaced: Option<&std::fs::Metadata>,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let existed = replaced.is_some();
    // Windows files carry only a read-only flag, and a read-only file can't be replaced.
    let perms = replaced.filter(|_| cfg!(unix)).map(|m| m.permissions());
    for _ in 0..8 {
        let tmp = dir.join(temp_name(name));
        let mut file = match fs.create_new(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(write_error(e, path, dir, existed)),
        };
        let written = fill(&mut file)
            .map_err(|e| write_error(e, path, dir, existed))
            .and_then(|()| perms.map_or(Ok(()), |p| fs.set_permissions(&file, p).map_err(|e| permissions_error(e, path))))
            .and_then(|()| fs.sync(&file).map_err(|e| write_error(e, path, dir, existed)));
        drop(file);
        if written.is_err() {
            let _ = fs.remove(&tmp);
        }
        return written.map(|()| tmp);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{}: could not create a temporary file", dir.display())))
}

/// Exclusively creates and writes `path` without replacing existing files.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_new_with(path, |f| f.write_all(bytes))
}

/// Exclusively creates and writes `path` with a streaming writer callback `F`.
/// On Unix, the file starts private (`0600`, restricted by the process umask); the callback
/// may choose its final permissions.
pub fn write_new_with<F>(path: &Path, write_fn: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    create(&RealFs, path, write_fn)
}

/// Exclusively creates and writes `bytes` to `path` through a custom [`AtomicFs`].
pub fn write_new_custom(fs: &dyn AtomicFs, path: &Path, bytes: &[u8]) -> io::Result<()> {
    create(fs, path, |f| f.write_all(bytes))
}

/// The create path behind every `write_new*` function: stage the content beside the target,
/// then give it the target's name with a hard link, which fails rather than replace a file that
/// appeared in the meantime. Volumes without hard links get a copy into a newly created file.
fn create(fs: &dyn AtomicFs, path: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let target = resolve_links(path)?;
    let (dir, name) = split(&target, path)?;
    if target.exists() {
        return Err(write_error(io::Error::from(io::ErrorKind::AlreadyExists), path, &dir, true));
    }

    let tmp = stage(fs, &dir, &name, path, None, fill)?;

    let placed = match fs.hard_link(&tmp, &target) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => copy_new(fs, &tmp, &target),
        linked => linked,
    };
    let _ = fs.remove(&tmp);
    if let Err(e) = placed {
        // A file that appeared in the meantime is someone else's and stays as it is.
        let existed = e.kind() == io::ErrorKind::AlreadyExists;
        return Err(write_error(e, path, &dir, existed));
    }

    // The new file is in place; a failed directory fsync can't be undone and is best effort.
    let _ = fs.sync_dir(&dir);
    Ok(())
}

/// Copy `tmp` into `target`, which must not exist yet, and flush it. A partly written `target` is
/// removed.
fn copy_new(fs: &dyn AtomicFs, tmp: &Path, target: &Path) -> io::Result<()> {
    let mut to = fs.create_new(target)?;
    let copied = File::open(tmp).and_then(|mut from| {
        io::copy(&mut from, &mut to)?;
        // Keep callback-selected modes just as hard-link publication does, but only
        // after copying the payload into a destination that was created privately.
        #[cfg(unix)]
        fs.set_permissions(&to, from.metadata()?.permissions())?;
        fs.sync(&to)
    });
    drop(to);
    if copied.is_err() {
        let _ = fs.remove(target);
    }
    copied
}

/// The longest pause between two rename attempts.
const MAX_RETRY_DELAY_MS: u64 = 200;

/// Retries an operation with exponential backoff while it fails because another process holds
/// the file: `PermissionDenied`, or on Windows a sharing or lock violation.
pub fn retry_rename(retry: RenameRetry, mut rename: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut delay = retry.first_delay_ms.min(MAX_RETRY_DELAY_MS);
    let mut left = retry.attempts;
    loop {
        match rename() {
            Err(e) if is_transient_lock(&e) && left > 0 => {
                left -= 1;
                sleep_ms(delay);
                delay = delay.saturating_mul(2).min(MAX_RETRY_DELAY_MS);
            }
            r => return r,
        }
    }
}

fn is_transient_lock(e: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    e.kind() == io::ErrorKind::PermissionDenied || (cfg!(windows) && matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)))
}

#[cfg(not(target_arch = "wasm32"))]
fn sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
#[cfg(target_arch = "wasm32")]
fn sleep_ms(_ms: u64) {}

/// Symbolic links followed before giving up (a loop, or an absurd chain).
const MAX_LINKS: usize = 40;

/// The file a chain of symbolic links ends at (`path` itself when it is not a link), so the link
/// is kept and the file it names is written. A dangling link resolves to the missing file it
/// names, which is then created.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let mut p = path.to_path_buf();
    for _ in 0..MAX_LINKS {
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                let to = std::fs::read_link(&p).map_err(|e| io::Error::new(e.kind(), format!("cannot save {}: {e}", path.display())))?;
                p = match p.parent() {
                    Some(dir) if to.is_relative() => dir.join(to),
                    _ => to,
                };
            }
            _ => return Ok(p),
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, format!("cannot save {}: too many levels of symbolic links", path.display())))
}

/// Generates a unique, hidden temporary file name beside `file`.
pub fn temp_name(file: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem: String = file.chars().take(64).collect();
    format!(".{stem}.{}-{n}.craft_tmp", std::process::id())
}

/// The end of an error message: whether an original file was left as it was.
fn unchanged(existed: bool) -> &'static str {
    if existed { "; the original file was not changed" } else { "" }
}

fn write_error(e: io::Error, path: &Path, dir: &Path, existed: bool) -> io::Error {
    let kept = unchanged(existed);
    let msg = match e.kind() {
        io::ErrorKind::AlreadyExists => format!("cannot save {}: the file already exists{kept}", path.display()),
        io::ErrorKind::NotFound => format!("cannot save {}: the folder {} does not exist ({e}){kept}", path.display(), dir.display()),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            format!("cannot save {}: the folder {} is not writable ({e}){kept}", path.display(), dir.display())
        }
        io::ErrorKind::StorageFull => format!("cannot save {}: the disk is full ({e}){kept}", path.display()),
        _ => format!("cannot save {}: {e}{kept}", path.display()),
    };
    io::Error::new(e.kind(), msg)
}

fn permissions_error(e: io::Error, path: &Path) -> io::Error {
    let msg = format!("cannot save {}: the new file cannot be given the original's permissions ({e}){}", path.display(), unchanged(true));
    io::Error::new(e.kind(), msg)
}

fn read_only_error(e: io::Error, path: &Path) -> io::Error {
    let kept = unchanged(true);
    let msg = match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            format!("cannot save {}: the file is read-only ({e}){kept}", path.display())
        }
        _ => format!("cannot save {}: {e}{kept}", path.display()),
    };
    io::Error::new(e.kind(), msg)
}

fn rename_error(e: io::Error, path: &Path, existed: bool) -> io::Error {
    let kept = unchanged(existed);
    let msg = if is_cross_device(&e) {
        format!("cannot save {}: the temporary file is on a different volume than the destination ({e}){kept}", path.display())
    } else if e.kind() == io::ErrorKind::PermissionDenied {
        format!("cannot save {}: the file is locked or read-only ({e}){kept}", path.display())
    } else {
        format!("cannot save {}: {e}{kept}", path.display())
    };
    io::Error::new(e.kind(), msg)
}

fn is_cross_device(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::CrossesDevices || (cfg!(unix) && e.raw_os_error() == Some(18)) || (cfg!(windows) && e.raw_os_error() == Some(17))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let d = std::env::temp_dir().join(format!("craft-atomic-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).ok();
            Self(d)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn assert_no_temp(dir: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .ok()
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".craft_tmp")).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
    }

    const ORIGINAL: &[u8] = b"the user's precious original document";

    #[derive(Default)]
    struct Faulty {
        events: std::cell::RefCell<Vec<&'static str>>,
        #[cfg(unix)]
        created_modes: std::cell::RefCell<Vec<u32>>,
        fail_sync: bool,
        /// Syncs fail once this many have succeeded.
        syncs_before_failure: Option<u32>,
        syncs: Cell<u32>,
        fail_rename: Option<io::ErrorKind>,
        locked_renames: Cell<u32>,
        renames: Cell<u32>,
        no_hard_links: bool,
        fail_permissions: bool,
        fail_sync_dir: bool,
        /// Another process creates the target just before the hard link.
        created_meanwhile: Option<&'static [u8]>,
    }

    impl AtomicFs for Faulty {
        fn create_new(&self, path: &Path) -> io::Result<File> {
            self.events.borrow_mut().push("create_new");
            let file = RealFs.create_new(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                self.created_modes.borrow_mut().push(file.metadata()?.permissions().mode() & 0o777);
            }
            Ok(file)
        }
        fn sync(&self, file: &File) -> io::Result<()> {
            self.events.borrow_mut().push("sync_file");
            let done = self.syncs.replace(self.syncs.get() + 1);
            if self.fail_sync || self.syncs_before_failure.is_some_and(|n| done >= n) {
                return Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"));
            }
            RealFs.sync(file)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.events.borrow_mut().push("rename");
            self.renames.set(self.renames.get() + 1);
            if let Some(kind) = self.fail_rename {
                return Err(io::Error::new(kind, "injected rename failure"));
            }
            if self.locked_renames.get() > 0 {
                self.locked_renames.set(self.locked_renames.get() - 1);
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: sharing violation"));
            }
            RealFs.rename(from, to)
        }
        fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.events.borrow_mut().push("hard_link");
            if let Some(bytes) = self.created_meanwhile {
                std::fs::write(to, bytes)?;
            }
            if self.no_hard_links {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: hard links not supported"));
            }
            RealFs.hard_link(from, to)
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            self.events.borrow_mut().push("remove");
            RealFs.remove(path)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            self.events.borrow_mut().push("sync_dir");
            if self.fail_sync_dir {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: directory sync failed"));
            }
            RealFs.sync_dir(dir)
        }
        fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()> {
            self.events.borrow_mut().push("set_permissions");
            if self.fail_permissions {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: operation not permitted"));
            }
            RealFs.set_permissions(file, perms)
        }
    }

    const NO_RETRY: RenameRetry = RenameRetry { attempts: 0, first_delay_ms: 0 };

    #[test]
    fn replaces_and_creates() {
        let d = TempDir::new("ok");
        let p = d.0.join("doc.craft");
        atomic_write(&p, b"first").ok();
        assert_eq!(std::fs::read(&p).ok(), Some(b"first".to_vec()));
        atomic_write(&p, b"second, longer").ok();
        assert_eq!(std::fs::read(&p).ok(), Some(b"second, longer".to_vec()));
        assert_no_temp(&d.0);
    }

    #[test]
    fn a_missing_folder_is_an_error_naming_it() {
        let d = TempDir::new("parent");
        let folder = d.0.join("a");
        let p = folder.join("b/out.png");
        for r in [atomic_write(&p, b"x"), write_new(&p, b"x")] {
            let msg = r.err().map(|e| e.to_string()).unwrap_or_default();
            assert!(msg.contains(&folder.join("b").display().to_string()), "{msg}");
        }
        assert!(!folder.exists(), "no folder is created");
    }

    #[test]
    fn failed_flush_keeps_original() {
        let d = TempDir::new("write");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();
        let fs = Faulty { fail_sync: true, ..Default::default() };
        let e = atomic_write_custom(&fs, NO_RETRY, &p, &[7u8; 4096]);
        assert_eq!(e.map_err(|e| e.kind()), Err(io::ErrorKind::StorageFull));
        assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
        assert_eq!(fs.renames.get(), 0);
        assert_no_temp(&d.0);
    }

    #[test]
    fn streaming_writes_go_through_the_injected_fs() {
        let d = TempDir::new("stream");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();
        let chunks = |f: &mut File| {
            f.write_all(b"chunk1 ")?;
            f.write_all(b"chunk2")
        };
        let fs = Faulty { fail_sync: true, ..Default::default() };
        assert!(replace(&fs, NO_RETRY, &p, chunks).is_err());
        let fs = Faulty { fail_rename: Some(io::ErrorKind::Other), ..Default::default() };
        assert!(replace(&fs, NO_RETRY, &p, chunks).is_err());
        assert_eq!(fs.renames.get(), 1);
        assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
        assert_no_temp(&d.0);
        let fs = Faulty::default();
        assert!(replace(&fs, NO_RETRY, &p, chunks).is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"chunk1 chunk2".to_vec()));
    }

    #[test]
    fn rename_failure_keeps_original() {
        let d = TempDir::new("rename");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();
        let fs = Faulty { fail_rename: Some(io::ErrorKind::Other), ..Default::default() };
        assert!(atomic_write_custom(&fs, NO_RETRY, &p, b"new").is_err());
        assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
        assert_no_temp(&d.0);
    }

    #[test]
    fn locked_target_rename_is_retried() {
        let d = TempDir::new("locked");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();
        let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
        atomic_write_custom(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, b"new").ok();
        assert_eq!(std::fs::read(&p).ok(), Some(b"new".to_vec()));
        assert_eq!(fs.renames.get(), 3);
        assert_no_temp(&d.0);
    }

    #[test]
    fn write_new_never_overwrites_existing() {
        let d = TempDir::new("new");
        let p = d.0.join("file.craft");
        write_new(&p, b"first").ok();
        assert_eq!(std::fs::read(&p).ok(), Some(b"first".to_vec()));
        let res = write_new(&p, b"second");
        assert!(res.is_err());
        assert_eq!(std::fs::read(&p).ok(), Some(b"first".to_vec()));
        assert_no_temp(&d.0);
    }

    #[test]
    fn write_new_copies_where_hard_links_fail_and_never_replaces() {
        let d = TempDir::new("new-copy");
        let p = d.0.join("export.png");
        let fs = Faulty { no_hard_links: true, ..Default::default() };
        assert!(create(&fs, &p, |f| f.write_all(b"copied")).is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"copied".to_vec()));
        assert_no_temp(&d.0);

        // A file that appears after the existence check is kept, whether the copy or the hard
        // link finds it.
        for no_hard_links in [true, false] {
            let p = d.0.join(format!("raced-{no_hard_links}.png"));
            let fs = Faulty { no_hard_links, created_meanwhile: Some(ORIGINAL), ..Default::default() };
            let e = create(&fs, &p, |f| f.write_all(b"mine"));
            assert_eq!(e.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
            assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
            assert_eq!(fs.renames.get(), 0);
        }
        assert_no_temp(&d.0);

        // A failed new file has no original to speak of.
        let p = d.0.join("full.png");
        let fs = Faulty { fail_sync: true, ..Default::default() };
        let msg = create(&fs, &p, |f| f.write_all(b"x")).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("disk is full") && !msg.contains("original"), "{msg}");
        assert!(!p.exists());
        assert_no_temp(&d.0);
    }

    #[test]
    fn a_failed_copy_leaves_no_partial_file() {
        let d = TempDir::new("new-copy-fails");
        let p = d.0.join("export.png");
        // The temporary file flushes; the copy made in its place does not.
        let fs = Faulty { no_hard_links: true, syncs_before_failure: Some(1), ..Default::default() };
        let e = write_new_custom(&fs, &p, b"copied");
        assert_eq!(e.map_err(|e| e.kind()), Err(io::ErrorKind::StorageFull));
        assert_eq!(fs.syncs.get(), 2);
        assert!(!p.exists());
        assert_no_temp(&d.0);
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_copy_starts_private_and_keeps_callback_mode() -> io::Result<()> {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let d = TempDir::new("private-copy");
        for mode in [0o600, 0o640] {
            let p = d.0.join(format!("export-{mode:o}.craft"));
            let fs = Faulty { no_hard_links: true, ..Default::default() };
            create(&fs, &p, |f| {
                f.write_all(b"private export")?;
                f.set_permissions(Permissions::from_mode(mode))
            })?;
            // Check both the staging file and the exclusive-copy destination at creation.
            assert_eq!(*fs.created_modes.borrow(), [0o600, 0o600]);
            assert_eq!(std::fs::metadata(&p)?.permissions().mode() & 0o777, mode);
            assert_eq!(std::fs::read(&p)?, b"private export");
        }
        assert_no_temp(&d.0);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failing_to_keep_permissions_fails_the_save() {
        let d = TempDir::new("perms");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();
        let fs = Faulty { fail_permissions: true, ..Default::default() };
        let msg = atomic_write_custom(&fs, NO_RETRY, &p, b"new").err().map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("permissions"), "{msg}");
        assert_eq!(std::fs::read(&p).ok(), Some(ORIGINAL.to_vec()));
        assert_eq!(fs.renames.get(), 0);
        assert_no_temp(&d.0);
    }

    #[test]
    fn retry_delays_are_capped() {
        let d = TempDir::new("cap");
        let p = d.0.join("doc.craft");
        let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
        let started = std::time::Instant::now();
        assert!(atomic_write_custom(&fs, RenameRetry { attempts: 2, first_delay_ms: 60_000 }, &p, b"new").is_ok());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(fs.renames.get(), 3);
    }

    #[test]
    fn distinguishes_atomicity_and_durability_order() {
        let d = TempDir::new("durability_order");
        let p = d.0.join("doc.craft");
        std::fs::write(&p, ORIGINAL).ok();

        let fs = Faulty::default();
        atomic_write_custom(&fs, NO_RETRY, &p, b"durable payload").expect("successful atomic write");

        let events = fs.events.into_inner();
        let sync_file_idx = events.iter().position(|&e| e == "sync_file").expect("file content must be flushed");
        let rename_idx = events.iter().position(|&e| e == "rename").expect("atomic rename must be performed");

        // 1. Content Durability: staging file must be synced to stable media BEFORE pointer switch (rename)
        assert!(
            sync_file_idx < rename_idx,
            "Durability invariant violated: file content must be flushed to stable storage before rename (sync={sync_file_idx}, rename={rename_idx})"
        );

        // 2. Directory Durability: on Unix, parent directory must be synced AFTER rename to commit directory entry
        #[cfg(unix)]
        {
            let sync_dir_idx = events.iter().position(|&e| e == "sync_dir").expect("directory must be flushed on Unix");
            assert!(
                rename_idx < sync_dir_idx,
                "Durability invariant violated: parent directory must be flushed after rename (rename={rename_idx}, sync_dir={sync_dir_idx})"
            );
        }
    }

    #[test]
    fn sync_dir_failure_is_best_effort() {
        let d = TempDir::new("best_effort_sync_dir");
        let p = d.0.join("doc.craft");
        let fs = Faulty { fail_sync_dir: true, ..Default::default() };
        assert!(atomic_write_custom(&fs, NO_RETRY, &p, b"best effort data").is_ok());
        assert_eq!(std::fs::read(&p).ok(), Some(b"best effort data".to_vec()));
        assert_no_temp(&d.0);
    }
}
