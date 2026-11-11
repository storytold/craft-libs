//! A cross-process lock beside the accounts file.
//!
//! Every mutation of `accounts.json`, a sign-in, a token refresh, a removal, an active-account
//! change, a touch, goes through [`with_lock`]. Without it every mutation is an unlocked
//! read-modify-write: two processes each completing 100 sign-ins into one folder left 101 rows on
//! file instead of 200, because the second writer's read missed the first writer's row. A pure
//! read (`Store::accounts`, `Store::shared`) does not need the lock. [`crate::accounts::write`]
//! already writes through a temporary file and a rename, so a reader never sees a torn file; it
//! can only see a list that is one write older than the latest.
//!
//! This matches the mechanism `craft-ai-integrations`'s own secret store already uses for the same
//! problem: a sibling `.lock` file, taken with `std::fs::File::try_lock` and a bounded retry,
//! never a new dependency. RFC-0025's convergence note names that store's "locked read-change-
//! write" as a requirement the shared crate adopts; this is that adoption, ahead of the
//! convergence PR itself.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::Error;

/// How long to wait for another process holding the lock before giving up. Long enough that a
/// normal read-change-write of a few-hundred-byte file never times out under load; short enough
/// that a process holding the lock across a crash does not wedge every other app indefinitely.
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

fn lock_path(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("accounts.json");
    path.with_file_name(format!("{name}.lock"))
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(path))
}

/// Runs `f` while holding an exclusive, OS-level lock on `path`'s sibling `.lock` file.
///
/// `f` reads, changes and writes the accounts file; `with_lock` only ever wraps that, never a
/// keychain call. A keychain is its own atomic resource per entry; serialising it behind this file
/// lock would mean the two apps this guards could never run a keychain operation while either held
/// the lock, which is exactly the deadlock a same-process caller (two `Store`s sharing one folder,
/// as two applications in one process would be) could hit. Acquiring the lock itself fails only as
/// [`Error::Io`]; `f` may return any [`Error`].
pub(crate) fn with_lock<T>(path: &Path, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let lock = open_lock_file(path).map_err(|e| Error::Io(format!("couldn't open the lock beside {} ({e})", path.display())))?;
    let start = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) => {
                if start.elapsed() > LOCK_TIMEOUT {
                    return Err(Error::Io(format!("timed out waiting for the accounts lock on {}", path.display())));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(Error::Io(format!("couldn't lock {} ({e})", path.display())));
            }
        }
    }
    let result = f();
    let _ = lock.unlock();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_exclusive_while_held_and_released_after() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let blocked = with_lock(&path, || {
            let other = open_lock_file(&path).unwrap();
            Ok(matches!(other.try_lock(), Err(fs::TryLockError::WouldBlock)))
        })
        .unwrap();
        assert!(blocked, "a second lock attempt should see the first one held");
        assert!(with_lock(&path, || Ok(true)).unwrap(), "the lock is free again once `with_lock` returns");
    }

    #[test]
    fn an_error_from_the_closure_still_releases_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let _: Result<(), Error> = with_lock(&path, || Err(Error::Unknown("x".to_owned())));
        assert!(with_lock(&path, || Ok(true)).unwrap(), "a failed closure must not leave the lock held");
    }
}
