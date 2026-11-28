//! Test folders that are removed with their contents when the test that made them ends.
//!
//! [`TempDir::new`] makes a new, empty folder in the system temporary directory with
//! [`tempfile::Builder`]: the name is `craft-testkit-<tag>-` followed by random characters, and a
//! name that is already taken is skipped. A folder this module did not make is never emptied or
//! reused. On Unix the folder is made with mode 0o700, and before [`TempDir::new`] returns it checks
//! that the folder is a real folder, not a link, with no access for other users, and owned by the
//! user running the test. The access check reads the mode bits only; it does not read POSIX or NFSv4
//! access control lists, so an entry that grants another user access is not detected. The checks apply to the created folder only, so a temporary directory
//! reached through a link (`/var` on macOS links to `/private/var`) works. On Windows the folder
//! gets the inheritable access entries of the temporary directory, which is the user's own
//! `%LOCALAPPDATA%\Temp` by default, and no access check is made. When `TMP` or `TEMP` names a
//! shared folder, other users may be able to open the new folder, depending on that folder's
//! access entries.
//!
//! [`TempDir::path`] returns the canonical path of the folder: `/private/var/...` on macOS, and a
//! `\\?\` path on Windows. Compare it with another canonical path, not with a join of
//! [`std::env::temp_dir`].
//!
//! Dropping a [`TempDir`], also while a test panics, removes the folder with everything in it. A
//! link inside the folder is removed, not followed. Removal on drop is best effort: a file another
//! process holds open can stop it on Windows. [`TempDir::close`] removes the folder and returns the
//! error. A process that is killed or aborts leaves its folders behind, and later calls skip them.
//!
//! The checks read the folder once, after it is made. They do not protect against a process of the
//! same user that replaces the folder after the check.

use std::io;
use std::path::{Path, PathBuf};

/// How many random characters follow the tag in a folder name.
const RANDOM_CHARS: usize = 10;

/// The file [`TempDir::new`] makes, reads and removes in a new folder on Unix to learn which user
/// owns the files this process makes.
#[cfg(unix)]
const OWNER_PROBE: &str = ".craft-testkit-owner";

/// A new, empty folder in the system temporary directory, removed with its contents when dropped,
/// also when the test panics.
///
/// Bind it to a variable that lives as long as the folder is needed. Used as a temporary, as in
/// `TempDir::new("x")?.file("a.png")`, it is dropped, and the folder removed, at the end of that
/// statement.
#[derive(Debug)]
pub struct TempDir {
    /// The canonical path of the folder.
    path: PathBuf,
    /// Removes the folder when dropped.
    dir: tempfile::TempDir,
}

impl TempDir {
    /// Makes a new folder named `craft-testkit-<tag>-<random characters>` in
    /// [`std::env::temp_dir`]. Each call makes a folder of its own, also for calls with the same
    /// tag at the same time.
    ///
    /// # Errors
    ///
    /// `InvalidInput` when `tag` is empty, `.` or `..`, or holds `/`, `\`, `:` or a NUL character.
    /// The error from the file system when the folder cannot be made. An error when the folder that
    /// was made is a link or another kind of entry, is open to other users, or is owned by another
    /// user; such a folder is left in place.
    pub fn new(tag: &str) -> io::Result<Self> {
        Self::new_in(tag, &std::env::temp_dir(), RANDOM_CHARS)
    }

    /// Makes the folder in `root`, with `random` random characters after the tag.
    fn new_in(tag: &str, root: &Path, random: usize) -> io::Result<Self> {
        one_name("tag", tag)?;
        let prefix = format!("craft-testkit-{tag}-");
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix).rand_bytes(random);
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        Self::guard(builder.tempdir_in(root)?)
    }

    /// Checks the folder `dir` made and returns it with its canonical path. A folder that fails the
    /// check is left in place, since it may not be the one `dir` made.
    fn guard(mut dir: tempfile::TempDir) -> io::Result<Self> {
        if let Err(e) = verify(dir.path()) {
            dir.disable_cleanup(true);
            return Err(e);
        }
        let path = std::fs::canonicalize(dir.path())?;
        Ok(Self { path, dir })
    }

    /// The canonical, absolute path of the folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path of `name` in the folder. The file is not made.
    ///
    /// # Errors
    ///
    /// `InvalidInput` when `name` is empty, `.` or `..`, or holds `/`, `\`, `:` or a NUL
    /// character, so the path always names an entry directly inside the folder.
    pub fn file(&self, name: &str) -> io::Result<PathBuf> {
        one_name("file name", name)?;
        Ok(self.path.join(name))
    }

    /// Removes the folder with everything in it.
    ///
    /// # Errors
    ///
    /// The error from the file system when the folder or something in it cannot be removed.
    pub fn close(self) -> io::Result<()> {
        self.dir.close()
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Returns `InvalidInput` unless `name` is one path component on every platform.
fn one_name(what: &str, name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{what} {name:?} is not a single file name")));
    }
    Ok(())
}

/// What [`verify`] reads about a folder, without following a link.
#[derive(Debug, Clone, Copy)]
struct Entry {
    is_dir: bool,
    is_symlink: bool,
    /// The permission bits, on Unix.
    mode: Option<u32>,
    /// The user id of the owner, on Unix.
    owner: Option<u32>,
}

/// Checks that `path` is a folder this process made, as [`check`] describes.
fn verify(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    let (mode, owner) = (Some(std::os::unix::fs::PermissionsExt::mode(&meta.permissions())), Some(std::os::unix::fs::MetadataExt::uid(&meta)));
    #[cfg(not(unix))]
    let (mode, owner) = (None, None);
    let entry = Entry { is_dir: meta.is_dir(), is_symlink: meta.file_type().is_symlink(), mode, owner };
    check(path, entry, || my_uid(path))
}

/// Returns an error unless `entry` is a real folder, not a link, with no access for group or other
/// users, owned by the user `me` returns. `me` runs only after the other checks pass.
fn check(path: &Path, entry: Entry, me: impl FnOnce() -> io::Result<Option<u32>>) -> io::Result<()> {
    if entry.is_symlink || !entry.is_dir {
        return Err(io::Error::other(format!("{}: a link or another kind of entry is in place of the new folder", path.display())));
    }
    if entry.mode.is_some_and(|mode| mode & 0o077 != 0) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: the new folder is open to other users", path.display())));
    }
    if entry.owner != me()? {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: the new folder is owned by another user", path.display())));
    }
    Ok(())
}

/// The user id that owns a file this process makes, read from a probe file made new in `dir` and
/// removed again.
#[cfg(unix)]
fn my_uid(dir: &Path) -> io::Result<Option<u32>> {
    let probe = dir.join(OWNER_PROBE);
    let meta = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?.metadata();
    std::fs::remove_file(&probe)?;
    Ok(Some(std::os::unix::fs::MetadataExt::uid(&meta?)))
}

/// Files have no Unix owner here.
#[cfg(not(unix))]
fn my_uid(_dir: &Path) -> io::Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    #[test]
    fn a_test_dir_is_removed_with_its_contents_when_dropped() {
        let dir = TempDir::new("dropped").unwrap();
        let path = dir.path().to_path_buf();
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("nested").join("a.txt"), b"a").unwrap();
        drop(dir);
        assert!(!path.exists(), "{} is left", path.display());
    }

    #[test]
    fn a_test_dir_is_removed_when_its_test_panics() {
        let mut path = None;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dir = TempDir::new("panicked").unwrap();
            std::fs::write(dir.file("a.txt").unwrap(), b"a").unwrap();
            path = Some(dir.path().to_path_buf());
            panic!("the test fails");
        }));
        assert_eq!(r.err().and_then(|p| p.downcast_ref::<&str>().copied()), Some("the test fails"));
        let path = path.unwrap();
        assert!(!path.exists(), "{} is left", path.display());
    }

    #[test]
    fn test_dirs_with_one_tag_are_separate_and_private() {
        let (a, b) = (TempDir::new("shared-tag").unwrap(), TempDir::new("shared-tag").unwrap());
        assert_ne!(a.path(), b.path());
        std::fs::write(a.file("a.txt").unwrap(), b"a").unwrap();
        assert_eq!(entries(b.path()), 0, "one test's files are not in another's directory");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(a.path()).unwrap().permissions()) & 0o777, 0o700);
        drop(a);
        assert!(b.path().is_dir(), "dropping one leaves the other");
    }

    #[test]
    fn a_scratch_dir_is_removed_with_its_contents_when_dropped() {
        let dir = TempDir::new("testkit-scratch").unwrap();
        let path = dir.path().to_path_buf();
        assert!(path.is_dir(), "{path:?} exists while the test holds it");
        assert_eq!(entries(&path), 0, "and starts empty");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("nested").join("a.pdf"), b"%PDF").unwrap();
        drop(dir);
        assert!(!path.exists(), "{path:?} is gone, contents and all");
    }

    #[test]
    fn scratch_dirs_with_the_same_name_are_separate_folders() {
        let (a, b) = (TempDir::new("testkit-same").unwrap(), TempDir::new("testkit-same").unwrap());
        assert_ne!(a.path(), b.path());
        std::fs::write(a.file("x").unwrap(), b"a").unwrap();
        drop(b);
        assert_eq!(std::fs::read(a.file("x").unwrap()).unwrap(), b"a", "dropping one leaves the other's files");
    }

    /// With one random character there are 62 names, and every one but `7` is taken: the new folder
    /// is `7`, and the folders that were there keep their files.
    #[test]
    fn a_taken_name_is_skipped_and_its_folder_left_alone() {
        let root = TempDir::new("taken-root").unwrap();
        let free = '7';
        let mut planted = Vec::new();
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9').filter(|&c| c != free) {
            let folder = root.path().join(format!("craft-testkit-taken-{c}"));
            // On a file system that ignores case, `A` is the folder `a` made.
            match std::fs::create_dir(&folder) {
                Ok(()) => std::fs::write(folder.join("theirs.pdf"), b"%PDF").unwrap(),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{}: {e}", folder.display()),
            }
            planted.push(folder);
        }
        let dir = TempDir::new_in("taken", root.path(), 1).unwrap();
        assert_eq!(dir.path().file_name().and_then(|n| n.to_str()), Some("craft-testkit-taken-7"));
        assert_eq!(entries(dir.path()), 0, "and starts empty");
        drop(dir);
        for folder in &planted {
            assert_eq!(std::fs::read(folder.join("theirs.pdf")).unwrap(), b"%PDF", "{folder:?} kept its file");
        }
    }

    #[test]
    fn existing_folder_is_never_reused() {
        let root = TempDir::new("reuse-root").unwrap();
        let theirs = root.path().join("craft-testkit-reuse-");
        std::fs::create_dir(&theirs).unwrap();
        std::fs::write(theirs.join("theirs.txt"), b"theirs").unwrap();
        // With no random characters the one name tried is the folder above.
        let err = TempDir::new_in("reuse", root.path(), 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(theirs.join("theirs.txt")).unwrap(), b"theirs");
        assert_eq!(entries(&theirs), 1);
    }

    #[test]
    fn creation_failure_returns_error() {
        let root = TempDir::new("failure-root").unwrap();
        let file = root.file("a-file").unwrap();
        std::fs::write(&file, b"not a folder").unwrap();
        assert!(TempDir::new_in("x", &file, RANDOM_CHARS).is_err(), "a file as the root");
        assert!(TempDir::new_in("x", &root.path().join("missing"), RANDOM_CHARS).is_err(), "a missing root");
    }

    #[test]
    fn tag_rejects_empty_and_separators() {
        for tag in ["", ".", "..", "a/b", "a\\b", "../x", "..\\x", "/x", "C:\\x", "C:x", "a\0b"] {
            let err = TempDir::new(tag).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{tag:?}");
        }
    }

    #[test]
    fn file_rejects_absolute_parent_and_separators() {
        let root = TempDir::new("file-root").unwrap();
        let dir = TempDir::new_in("file", root.path(), RANDOM_CHARS).unwrap();
        for name in ["", ".", "..", "a/b", "a\\b", "../outside.txt", "..\\outside.txt", "/x", "\\x", "C:\\x", "C:x", "a:stream", "a\0b"] {
            let err = dir.file(name).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        let a = dir.file("a.txt").unwrap();
        assert_eq!(a, dir.path().join("a.txt"));
        assert!(!a.exists(), "file does not make the file");
        drop(dir);
        assert_eq!(entries(root.path()), 0, "nothing was written next to the folder");
    }

    #[test]
    fn path_is_canonical() {
        let dir = TempDir::new("canonical").unwrap();
        assert!(dir.path().is_absolute());
        assert_eq!(dir.path(), std::fs::canonicalize(dir.path()).unwrap());
        assert_eq!(dir.path().parent().map(Path::to_path_buf), Some(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_is_verbatim() {
        let dir = TempDir::new("verbatim").unwrap();
        assert!(dir.path().to_string_lossy().starts_with(r"\\?\"), "{:?}", dir.path());
    }

    #[test]
    fn same_tag_concurrent_creation_is_unique() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    TempDir::new("concurrent")
                })
            })
            .collect();
        let dirs: Vec<TempDir> = threads.into_iter().map(|t| t.join().unwrap().unwrap()).collect();
        let paths: std::collections::BTreeSet<&Path> = dirs.iter().map(TempDir::path).collect();
        assert_eq!(paths.len(), 16);
        assert!(dirs.iter().all(|d| entries(d.path()) == 0), "each starts empty");
    }

    #[test]
    fn close_removes_the_folder() {
        let dir = TempDir::new("close").unwrap();
        let path = dir.path().to_path_buf();
        std::fs::write(dir.file("a.txt").unwrap(), b"a").unwrap();
        dir.close().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn close_reports_cleanup_failure() {
        let dir = TempDir::new("close-failure").unwrap();
        std::fs::remove_dir(dir.path()).unwrap();
        assert!(dir.close().is_err(), "a folder that is already gone cannot be removed");
    }

    #[test]
    fn wrong_owner_or_public_mode_is_rejected() {
        let path = Path::new("folder");
        let private = Entry { is_dir: true, is_symlink: false, mode: Some(0o40700), owner: Some(1000) };
        assert!(check(path, private, || Ok(Some(1000))).is_ok());
        for mode in [0o40750, 0o40705, 0o40777] {
            let err = check(path, Entry { mode: Some(mode), ..private }, || Ok(Some(1000))).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{mode:o}");
        }
        let err = check(path, private, || Ok(Some(1001))).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(check(path, Entry { is_symlink: true, ..private }, || panic!("no probe through a link")).is_err());
        assert!(check(path, Entry { is_dir: false, ..private }, || panic!("no probe in a file")).is_err());
        let open = Entry { mode: Some(0o40777), ..private };
        assert!(check(path, open, || panic!("no probe in an open folder")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn new_dir_is_owner_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = TempDir::new("private").unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let mine = dir.file("mine.txt").unwrap();
        std::fs::write(&mine, b"a").unwrap();
        assert_eq!(meta.uid(), std::fs::metadata(&mine).unwrap().uid());
        std::fs::remove_file(&mine).unwrap();
        assert_eq!(entries(dir.path()), 0, "the owner probe is removed");
    }

    #[cfg(unix)]
    #[test]
    fn created_folder_symlink_is_rejected() {
        let root = TempDir::new("link-root").unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("theirs.txt"), b"theirs").unwrap();
        let made = tempfile::Builder::new().prefix("craft-testkit-link-").tempdir_in(root.path()).unwrap();
        let link = made.path().to_path_buf();
        std::fs::remove_dir(&link).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(TempDir::guard(made).is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link is left in place");
        assert_eq!(std::fs::read(real.join("theirs.txt")).unwrap(), b"theirs");
        assert_eq!(entries(&real), 1, "no probe was made through the link");
    }

    #[cfg(unix)]
    #[test]
    fn temp_root_reached_through_symlink_works() {
        let root = TempDir::new("symlink-root").unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let dir = TempDir::new_in("through-link", &link, RANDOM_CHARS).unwrap();
        assert_eq!(dir.path().parent(), Some(real.as_path()), "the path is canonical");
        assert!(dir.path().is_dir());
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
        assert!(real.is_dir() && link.is_symlink(), "the root and the link stay");
    }

    #[cfg(unix)]
    #[test]
    fn drop_does_not_follow_inner_symlink() {
        let root = TempDir::new("inner-link-root").unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let dir = TempDir::new_in("inner-link", root.path(), RANDOM_CHARS).unwrap();
        std::os::unix::fs::symlink(&outside, dir.file("link").unwrap()).unwrap();
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
        assert_eq!(std::fs::read(outside.join("keep.txt")).unwrap(), b"keep");
    }
}
