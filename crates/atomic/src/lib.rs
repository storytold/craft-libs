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
//!   [`atomic_write`] guarantees that readers of the destination never observe a partial write. Either the target
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
//! ### Permissions
//! New documents and exports use normal OS creation permissions, including while staging.
//! Use [`WriteOptions`] with [`NewFilePermissions::Private`] for settings or sensitive payloads.
//! Existing destinations always stage privately and retain their original Unix mode. Restoring
//! an already-matching mode needs no chmod, including on fixed-mode network filesystems.
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
    /// Exclusively create a payload file with the requested permission policy. On Unix,
    /// request mode `0600` for `Private` or `0666` for `PlatformDefault`; the OS applies its
    /// umask/default ACL. Other platforms keep their native creation behavior.
    fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File>;
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
    /// Give the open temporary file its final permissions.
    fn set_permissions(&self, file: &File, perms: std::fs::Permissions) -> io::Result<()>;
}

/// The production file system implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl AtomicFs for RealFs {
    fn create_new(&self, path: &Path, permissions: NewFilePermissions) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Apply the policy at creation: chmod afterward cannot revoke an already-open
            // handle to private payload, and ordinary new documents need no chmod at all.
            options.mode(match permissions {
                NewFilePermissions::Private => 0o600,
                NewFilePermissions::PlatformDefault => 0o666,
            });
        }
        #[cfg(not(unix))]
        let _ = permissions;
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

/// Creation permissions for a new file on Unix. Existing destination modes are preserved.
/// Other platforms keep their existing creation behavior. Streaming callbacks may change a
/// new file's mode; choose `Private` up front when payload must not start with broader access.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NewFilePermissions {
    /// Keep the private staging mode (`0600`, subject to umask/default ACL), unless the streaming
    /// callback selects another mode. Use this for settings, credentials and recovery data.
    Private,
    /// Use the permissions normal creation with mode `0666` would receive in the destination
    /// directory. The OS applies its umask/default ACL; the library never changes process umask.
    /// The stage has these permissions while being written, just like ordinary file creation.
    #[default]
    PlatformDefault,
}

/// Permission policy for document saves and exports. Defaults to normal new-file creation,
/// matching [`atomic_write`] and [`write_new`]. Select [`NewFilePermissions::Private`] explicitly
/// for settings and other private data. Existing files always stage privately before replacement.
///
/// ```
/// use craft_atomic::{NewFilePermissions, WriteOptions};
/// let private = WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private);
/// # let _ = private;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Final permissions for new files; ignored when replacing an existing Unix file.
    pub new_file_permissions: NewFilePermissions,
}

impl WriteOptions {
    /// Select creation permissions without depending on the complete set of options.
    pub const fn with_new_file_permissions(mut self, permissions: NewFilePermissions) -> Self {
        self.new_file_permissions = permissions;
        self
    }

    /// [`atomic_write`] with this new-file permission policy.
    pub fn atomic_write(self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.atomic_write_custom(&RealFs, RenameRetry::platform(), path, bytes)
    }

    /// [`atomic_write_with`] with this policy. Existing modes are restored after the callback;
    /// for new files, callback-selected modes are kept.
    pub fn atomic_write_with(self, path: &Path, write_fn: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
        self.atomic_write_with_custom(&RealFs, RenameRetry::platform(), path, write_fn)
    }

    /// [`atomic_write_custom`] with this policy, an injected filesystem and caller-selected retries.
    pub fn atomic_write_custom(self, fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.atomic_write_with_custom(fs, retry, path, |f| f.write_all(bytes))
    }

    /// Stream an atomic replacement through an injected filesystem with this policy.
    pub fn atomic_write_with_custom(
        self,
        fs: &dyn AtomicFs,
        retry: RenameRetry,
        path: &Path,
        write_fn: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<()> {
        replace_with_options(fs, retry, path, self.new_file_permissions, write_fn)
    }

    /// [`write_new`] with this new-file permission policy; never replaces an existing file.
    pub fn write_new(self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_new_custom(&RealFs, path, bytes)
    }

    /// [`write_new_with`] with this policy. Callback-selected modes are kept.
    pub fn write_new_with(self, path: &Path, write_fn: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
        self.write_new_with_custom(&RealFs, path, write_fn)
    }

    /// [`write_new_custom`] with this policy and an injected filesystem.
    pub fn write_new_custom(self, fs: &dyn AtomicFs, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_new_with_custom(fs, path, |f| f.write_all(bytes))
    }

    /// Stream exclusive creation through an injected filesystem with this policy.
    pub fn write_new_with_custom(self, fs: &dyn AtomicFs, path: &Path, write_fn: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
        create_with_options(fs, path, self.new_file_permissions, write_fn)
    }
}

/// Atomically writes `bytes` to `path`, replacing any existing content.
///
/// New files use normal OS creation permissions. Use [`WriteOptions`] for private data.
/// If write or rename fails, the target file is untouched and any temporary staging file is removed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_custom(&RealFs, RenameRetry::platform(), path, bytes)
}

/// Atomically writes to `path` using a streaming writer callback `F`.
///
/// On Unix, a new destination is staged with mode `0666`, subject to umask/default ACL.
/// Replacements stage privately (`0600`) and restore the existing mode after the callback.
/// New files keep callback-selected modes. Use [`WriteOptions`] for private new files.
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
    replace_with_options(fs, retry, path, NewFilePermissions::default(), fill)
}

fn replace_with_options(
    fs: &dyn AtomicFs,
    retry: RenameRetry,
    path: &Path,
    new_permissions: NewFilePermissions,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_links(path)?;
    let (dir, name) = split(&target, path)?;
    let existing = std::fs::metadata(&target).ok();
    let existed = existing.is_some();
    if existing.as_ref().is_some_and(|m| m.is_file()) {
        check_writable(&target).map_err(|e| read_only_error(e, path))?;
    }

    let tmp = stage(fs, &dir, &name, path, existing.as_ref(), new_permissions, fill)?;

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
/// replaces (`replaced`, Unix) and flush it. Permission restoration is skipped when the mode
/// already matches after `fill`. A leftover file with the same name is skipped. When any step
/// fails, the temporary file is removed.
fn stage(
    fs: &dyn AtomicFs,
    dir: &Path,
    name: &str,
    path: &Path,
    replaced: Option<&std::fs::Metadata>,
    new_permissions: NewFilePermissions,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let existed = replaced.is_some();
    // Windows files carry only a read-only flag, and a read-only file can't be replaced.
    let perms = replaced.filter(|_| cfg!(unix)).map(|m| m.permissions());
    // New documents inherit the directory's policy directly: no probe or chmod is needed.
    // Existing destinations still stage privately, including when their final mode is shared.
    let creation = if existed { NewFilePermissions::Private } else { new_permissions };
    for _ in 0..8 {
        let tmp = dir.join(temp_name(name));
        let mut file = match fs.create_new(&tmp, creation) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(write_error(e, path, dir, existed)),
        };
        let written = fill(&mut file)
            .map_err(|e| write_error(e, path, dir, existed))
            .and_then(|()| {
                perms
                    .map_or(Ok(()), |p| {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            // GVfs/SMB and document portals can expose a fixed mode but reject
                            // chmod, even when the requested mode is already in effect. Check
                            // after fill: a streaming callback may have changed the stage's mode.
                            // Compare permission and special bits, not file-type bits.
                            if (file.metadata()?.permissions().mode() & 0o7777) == (p.mode() & 0o7777) {
                                return Ok(());
                            }
                        }
                        fs.set_permissions(&file, p)
                    })
                    .map_err(|e| permissions_error(e, path))
            })
            .and_then(|()| fs.sync(&file).map_err(|e| write_error(e, path, dir, existed)));
        drop(file);
        if written.is_err() {
            let _ = fs.remove(&tmp);
        }
        return written.map(|()| tmp);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{}: could not create a temporary file", dir.display())))
}

/// Exclusively creates and writes `path` with normal OS permissions, without replacing existing files.
/// Use [`WriteOptions`] for private data.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_new_with(path, |f| f.write_all(bytes))
}

/// Exclusively creates and writes `path` with a streaming writer callback `F`.
/// On Unix, new files start with normal creation permissions (`0666`, subject to umask/default
/// ACL); the callback may change the mode. Use [`WriteOptions`] for private new files.
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
    create_with_options(fs, path, NewFilePermissions::default(), fill)
}

fn create_with_options(
    fs: &dyn AtomicFs,
    path: &Path,
    new_permissions: NewFilePermissions,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_links(path)?;
    let (dir, name) = split(&target, path)?;
    if target.exists() {
        return Err(write_error(io::Error::from(io::ErrorKind::AlreadyExists), path, &dir, true));
    }

    let tmp = stage(fs, &dir, &name, path, None, new_permissions, fill)?;

    let placed = match fs.hard_link(&tmp, &target) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => copy_new(fs, &tmp, &target, new_permissions),
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
fn copy_new(fs: &dyn AtomicFs, tmp: &Path, target: &Path, permissions: NewFilePermissions) -> io::Result<()> {
    let mut to = fs.create_new(target, permissions)?;
    let copied = File::open(tmp).and_then(|mut from| {
        #[cfg(unix)]
        let final_permissions = {
            use std::os::unix::fs::PermissionsExt;
            let source = from.metadata()?.permissions();
            let current = to.metadata()?.permissions();
            (source.mode() != current.mode()).then_some(source)
        };
        // Normal creation already inherits the correct mode/ACL. Avoid a redundant chmod on
        // volumes that reject it. If the callback narrowed the stage, narrow the destination
        // before copying any payload, so it is never exposed with the default broader mode.
        #[cfg(unix)]
        if permissions == NewFilePermissions::PlatformDefault
            && let Some(ref mode) = final_permissions
        {
            fs.set_permissions(&to, mode.clone())?;
        }
        io::copy(&mut from, &mut to)?;
        // Explicit private writes keep their copy destination private until writing finishes,
        // then preserve any mode the callback deliberately selected, as the hard link does.
        #[cfg(unix)]
        if permissions == NewFilePermissions::Private
            && let Some(mode) = final_permissions
        {
            fs.set_permissions(&to, mode)?;
        }
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
mod tests;

#[cfg(test)]
extern crate self as craft_atomic;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
