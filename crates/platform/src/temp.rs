//! Private temporary files (N10.06): print previews handed to the system viewer, screenshots,
//! files written for another app to open.
//!
//! The shared temporary folder is writable by every local user, who could plant a symlink or a
//! folder under the name the app is about to use. So a [`TempDir`] is created exclusively with
//! owner-only permissions (`0700` on Unix; plain create elsewhere), under a name with the process
//! id and a counter, retried only on `AlreadyExists`; its files are created with `create_new`
//! (`O_EXCL`, which refuses an existing path, a symlink included) and mode `0600`. A name that
//! exists already is never reused: creation fails with `AlreadyExists` and the next name is
//! tried. [`is_private_dir`] is for an app that keeps a folder of its own between runs and
//! checks it before use.
//!
//! [`TempDir`] removes its folder and everything in it on drop (DeckCraft's print previews); a
//! [`TempFiles`] owns several and removes each when asked or on drop.

use std::fs::{File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::PlatformText;

/// How many names [`TempDir::new`] tries before giving up: 128 (DeckCraft's loop).
pub const TEMP_ATTEMPTS: usize = 128;

/// An owner-only folder in the system's temporary folder, removed with its contents on drop.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
    keep: bool,
}

impl TempDir {
    /// A new folder `<prefix>-<pid>-<n>` in [`std::env::temp_dir`] (`photocraft-print-4211-0`).
    pub fn new(prefix: &str) -> std::io::Result<TempDir> {
        Self::new_in(&std::env::temp_dir(), prefix)
    }

    /// [`TempDir::new`] inside `base`. Fails with `AlreadyExists` after [`TEMP_ATTEMPTS`] taken
    /// names, and with any other error at once.
    // deckcraft: crates/ui-egui/src/app/files.rs:40 create_private_print_dir
    // cadcraft: crates/ui-egui/src/control.rs:568 create_private_shot_dir, :584 create_private_dir
    pub fn new_in(base: &Path, prefix: &str) -> std::io::Result<TempDir> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..TEMP_ATTEMPTS {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{prefix}-{}-{n}", std::process::id()));
            match create_private_dir(&path) {
                Ok(()) => return Ok(TempDir { path, keep: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no unused private temporary folder name"))
    }

    /// The folder's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A new file `name` inside, created exclusively with mode `0600`. `name` is a plain file
    /// name (no separators); make one from user text with `craft_docpath::names::safe_file_name`.
    // cadcraft: crates/ui-egui/src/control.rs:598 create_private_file
    pub fn create(&self, name: &str) -> std::io::Result<(PathBuf, File)> {
        check_plain(name)?;
        let path = self.path.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        Ok((path, file))
    }

    /// Leaves the folder on disk when dropped (the system viewer still reads the file). The app
    /// removes it later with [`TempFiles`] or at its next start.
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for TempDir {
    // deckcraft: crates/ui-egui/src/app/files.rs:36 PrintPreviews::drop
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Creates `path` with mode `0700` on Unix (plain create elsewhere), failing on an existing
/// entry instead of following or reusing it.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

/// `name` must be one plain file name: no separators, not `.` or `..`.
fn check_plain(name: &str) -> std::io::Result<()> {
    if Path::new(name).file_name().is_some_and(|plain| plain == name) {
        Ok(())
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a plain file name"))
    }
}

/// Whether `meta` is a folder the app may reuse: a directory, not a symlink, with no group or
/// other permission bits (always true for a directory on platforms without Unix modes). `meta`
/// must come from `std::fs::symlink_metadata`: `std::fs::metadata` follows a symlink, so its
/// `is_symlink()` is never true.
// cadcraft: crates/ui-egui/src/control.rs:609 is_private_shot_dir
pub fn is_private_dir(meta: &Metadata) -> bool {
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Temporary folders the app owns until it is done with them: print previews open in the system
/// viewer stay until the next preview or quit (DeckCraft).
#[derive(Debug, Default)]
pub struct TempFiles {
    dirs: Vec<PathBuf>,
}

impl TempFiles {
    /// A new private folder with `prefix`, owned until [`discard`](Self::discard) or
    /// [`cleanup`](Self::cleanup), and the path of `file_name` inside it (not created).
    // deckcraft: crates/ui-egui/src/app/files.rs:14 PrintPreviews::create_in
    pub fn create(&mut self, prefix: &str, file_name: &str) -> std::io::Result<PathBuf> {
        check_plain(file_name)?;
        let dir = TempDir::new(prefix)?;
        let path = dir.path().join(file_name);
        self.dirs.push(dir.keep());
        Ok(path)
    }

    /// Removes the folder that holds `path`, if this owns it.
    // deckcraft: crates/ui-egui/src/app/files.rs:21 PrintPreviews::discard
    pub fn discard(&mut self, path: &Path) {
        let Some(dir) = path.parent() else { return };
        if let Some(i) = self.dirs.iter().position(|owned| owned == dir) {
            let _ = std::fs::remove_dir_all(self.dirs.swap_remove(i));
        }
    }

    /// Removes every owned folder.
    // deckcraft: crates/ui-egui/src/app/files.rs:28 PrintPreviews::cleanup
    pub fn cleanup(&mut self) {
        for dir in self.dirs.drain(..) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// When no private temporary folder could be made; `{error}`.
pub const TEMP_FAILED: PlatformText = PlatformText { id: "temp.failed", english: "Couldn't make a private temporary folder: {error}" };
/// Every [`PlatformText`] of this module, for the missing-translation report ([`crate::texts`]).
pub const TEXTS: &[PlatformText] = &[TEMP_FAILED];
