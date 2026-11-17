//! A fixed number of gate runs at a time on one machine.
//!
//! A gate run builds the workspace several times with one job per CPU, and several checkouts running
//! it at once slow each other down. [`acquire`] takes one of `slots` lock files in a directory the app
//! chooses (usually a per-user cache directory shared by every checkout) and holds it until the
//! [`Held`] guard is dropped. The operating system releases the lock when the process ends, crashed
//! or not. Runs that use the same directory and slot count share one pool; separate directories are
//! separate pools. Running without a lock is the caller's choice: it passes no [`LockConfig`].
//!
//! Next to each held lock file, a `.owner` file holds the holder's label, which waiting runs print.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::runner::{Cancel, Error};

/// The most slots a [`LockConfig`] can name.
pub const MAX_SLOTS: usize = 64;
/// The longest owner label [`holders`] reads from one owner file.
pub const MAX_OWNER_BYTES: u64 = 4096;
/// How often [`acquire`] tries again by default while every slot is held.
pub const DEFAULT_POLL: Duration = Duration::from_secs(2);
/// The longest [`acquire`] sleeps before it checks for cancellation again.
const CANCEL_CHECK: Duration = Duration::from_millis(50);

/// Where the lock files are and how many runs may hold one at once.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockConfig {
    /// The directory of the lock files. It is created when missing.
    pub directory: PathBuf,
    /// How many runs may hold a slot at once, from 1 to [`MAX_SLOTS`].
    pub slots: usize,
    /// What waiting runs print about this run, for example `pid 123 in /src/app: cargo xtask ci`.
    pub owner_label: String,
    /// How often [`acquire`] tries again while every slot is held.
    pub poll: Duration,
}

impl LockConfig {
    /// `slots` slots in `directory`, labeled `owner_label`, polled every [`DEFAULT_POLL`].
    pub fn new(directory: impl Into<PathBuf>, slots: usize, owner_label: impl Into<String>) -> Self {
        Self { directory: directory.into(), slots, owner_label: owner_label.into(), poll: DEFAULT_POLL }
    }

    /// Sets how often [`acquire`] tries again.
    pub fn with_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    fn check(&self) -> Result<(), Error> {
        if !(1..=MAX_SLOTS).contains(&self.slots) {
            return Err(Error::Config(format!("lock: {} slots: the slot count must be from 1 to {MAX_SLOTS}", self.slots)));
        }
        if self.poll.is_zero() {
            return Err(Error::Config("lock: the poll interval must be longer than zero".to_owned()));
        }
        Ok(())
    }
}

/// A held slot. Dropping it releases the slot and removes its owner file.
#[derive(Debug)]
pub struct Held {
    _file: File,
    owner: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.owner);
    }
}

fn slot_path(dir: &Path, slot: usize) -> PathBuf {
    if slot == 0 { dir.join("ci.lock") } else { dir.join(format!("ci-{}.lock", slot + 1)) }
}

fn owner_path(lock: &Path) -> PathBuf {
    lock.with_extension("lock.owner")
}

/// Takes the first free slot, or returns `None` when every slot is held.
pub fn try_acquire(config: &LockConfig) -> Result<Option<Held>, Error> {
    config.check()?;
    let dir = &config.directory;
    std::fs::create_dir_all(dir).map_err(|e| Error::Lock(format!("{}: {e}", dir.display())))?;
    for slot in 0..config.slots {
        let path = slot_path(dir, slot);
        let file =
            OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| Error::Lock(format!("{}: {e}", path.display())))?;
        match file.try_lock() {
            Ok(()) => return Ok(Some(Held { _file: file, owner: record_owner(&path, &config.owner_label) })),
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(e)) => return Err(Error::Lock(format!("{}: {e}", path.display()))),
        }
    }
    Ok(None)
}

/// Writes the owner file of a slot this run holds. The owner is kept in a separate file because a
/// lock on Windows is mandatory, so other runs could not read the lock file itself.
fn record_owner(lock: &Path, label: &str) -> PathBuf {
    let owner_file = owner_path(lock);
    // A new file: an entry left under the name (a link planted in a shared lock directory) is removed,
    // never written through.
    let removed = std::fs::remove_file(&owner_file);
    let written = OpenOptions::new().write(true).create_new(true).open(&owner_file).and_then(|mut f| f.write_all(label.as_bytes()));
    if let (Err(e), Err(w)) = (&removed, &written)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("lock: could not record the owner in {} ({e}; {w}): waiting runs may show a stale owner", owner_file.display());
    }
    owner_file
}

/// The owner labels of the held slots, as their holders wrote them, each cut to [`MAX_OWNER_BYTES`].
///
/// A slot whose lock file is missing or free is left out, also when a run that ended without dropping
/// its [`Held`] guard left its owner file behind.
pub fn holders(config: &LockConfig) -> Result<Vec<String>, Error> {
    config.check()?;
    Ok((0..config.slots)
        .map(|slot| slot_path(&config.directory, slot))
        .filter(|lock| is_held(lock))
        .filter_map(|lock| read_owner(&owner_path(&lock)))
        .collect())
}

/// Whether a run holds the slot whose lock file is `lock`. The file is opened read-only and never
/// created; a free slot is locked shared for a moment and released when the file closes.
fn is_held(lock: &Path) -> bool {
    match File::open(lock) {
        Ok(file) => matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock)),
        Err(_) => false,
    }
}

fn read_owner(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(MAX_OWNER_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Waits for a slot, printing the holders once while it waits.
///
/// Returns [`Error::Canceled`] when `cancel` is set while waiting, and a configuration or lock error
/// before any wait.
pub fn acquire(config: &LockConfig, cancel: Option<&Cancel>) -> Result<Held, Error> {
    acquire_with(config, cancel, || {
        eprintln!("=== lock: waiting for another run to finish ({} at a time) ===", config.slots);
        for h in holders(config).unwrap_or_default() {
            eprintln!("  running: {h}");
        }
    })
}

fn acquire_with(config: &LockConfig, cancel: Option<&Cancel>, on_wait: impl FnOnce()) -> Result<Held, Error> {
    if let Some(held) = try_acquire(config)? {
        return Ok(held);
    }
    let waiting = Instant::now();
    on_wait();
    loop {
        if !pause(config.poll, cancel) {
            return Err(Error::Canceled { step: None });
        }
        if let Some(held) = try_acquire(config)? {
            eprintln!("=== lock: waited {:.0} s for a slot ===", waiting.elapsed().as_secs_f64());
            return Ok(held);
        }
    }
}

/// Sleeps for `poll`, checking `cancel` every [`CANCEL_CHECK`]. Returns `false` when canceled. A
/// `poll` too long for the clock to represent waits until canceled.
fn pause(poll: Duration, cancel: Option<&Cancel>) -> bool {
    let until = Instant::now().checked_add(poll);
    loop {
        if cancel.is_some_and(Cancel::is_canceled) {
            return false;
        }
        let left = match until {
            Some(until) => until.saturating_duration_since(Instant::now()),
            None => CANCEL_CHECK,
        };
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(CANCEL_CHECK));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{RunOptions, Step, run_steps_to};
    use std::sync::mpsc;

    fn config(dir: &Path, slots: usize, owner: &str) -> LockConfig {
        LockConfig::new(dir, slots, owner).with_poll(Duration::from_millis(1))
    }

    #[test]
    fn one_slot_is_exclusive_until_dropped() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        let first = try_acquire(&config(dir.path(), 1, "first")).unwrap().expect("free slot");
        assert!(try_acquire(&config(dir.path(), 1, "second")).unwrap().is_none());
        assert_eq!(holders(&config(dir.path(), 1, "")).unwrap(), ["first"]);
        drop(first);
        assert!(holders(&config(dir.path(), 1, "")).unwrap().is_empty());
        assert!(try_acquire(&config(dir.path(), 1, "third")).unwrap().is_some());
    }

    #[test]
    fn more_slots_admit_more_runs() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        let a = try_acquire(&config(dir.path(), 2, "a")).unwrap();
        let b = try_acquire(&config(dir.path(), 2, "b")).unwrap();
        assert!(a.is_some() && b.is_some());
        assert!(try_acquire(&config(dir.path(), 2, "c")).unwrap().is_none());
        assert_eq!(holders(&config(dir.path(), 2, "")).unwrap(), ["a", "b"]);
    }

    #[test]
    fn slot_count_is_bounded() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        for slots in [0, MAX_SLOTS + 1, usize::MAX] {
            let c = config(dir.path(), slots, "x");
            assert!(matches!(try_acquire(&c), Err(Error::Config(_))), "{slots}");
            assert!(matches!(holders(&c), Err(Error::Config(_))), "{slots}");
            assert!(matches!(acquire(&c, None), Err(Error::Config(_))), "{slots}");
        }
        assert!(matches!(try_acquire(&config(dir.path(), 1, "x").with_poll(Duration::ZERO)), Err(Error::Config(_))));
        // Nothing was created for the rejected configurations.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

        let one = try_acquire(&config(dir.path(), 1, "one")).unwrap();
        assert!(one.is_some());
        let all: Vec<Held> = (0..MAX_SLOTS - 1).filter_map(|_| try_acquire(&config(dir.path(), MAX_SLOTS, "n")).unwrap()).collect();
        assert_eq!(all.len(), MAX_SLOTS - 1);
        assert!(try_acquire(&config(dir.path(), MAX_SLOTS, "full")).unwrap().is_none());
    }

    #[test]
    fn owner_labels_are_read_within_the_limit() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        let c = config(dir.path(), 1, "");
        let _held = try_acquire(&c).unwrap().expect("free slot");
        std::fs::write(owner_path(&slot_path(dir.path(), 0)), vec![b'x'; 3 * MAX_OWNER_BYTES as usize]).unwrap();
        assert_eq!(holders(&c).unwrap(), ["x".repeat(MAX_OWNER_BYTES as usize)]);
    }

    #[test]
    fn owner_files_of_free_slots_are_not_holders() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        let c = config(dir.path(), 2, "");
        // The first slot: a run that ended without dropping its guard left its lock and owner files.
        std::fs::write(slot_path(dir.path(), 0), b"").unwrap();
        std::fs::write(owner_path(&slot_path(dir.path(), 0)), b"crashed").unwrap();
        // The second slot: an owner file with no lock file.
        std::fs::write(owner_path(&slot_path(dir.path(), 1)), b"orphan").unwrap();
        assert!(holders(&c).unwrap().is_empty());

        // A live holder of the first slot is listed, and the free second slot still is not.
        let _held = try_acquire(&config(dir.path(), 2, "live")).unwrap().expect("free slot");
        assert_eq!(holders(&c).unwrap(), ["live"]);
    }

    #[test]
    fn lock_wait_cancels_and_releases() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().expect("temporary directory");
        let holder = try_acquire(&config(dir.path(), 1, "holder")).unwrap().expect("free slot");

        // A waiter stops waiting when canceled, also with a poll interval the clock cannot add.
        for poll in [Duration::from_millis(1), Duration::MAX] {
            let cancel = Cancel::new();
            let (waiting, waited) = mpsc::channel();
            let waiter = {
                let (c, cancel) = (config(dir.path(), 1, "waiter").with_poll(poll), cancel.clone());
                std::thread::spawn(move || acquire_with(&c, Some(&cancel), || waiting.send(()).unwrap()).map(drop))
            };
            waited.recv().unwrap();
            cancel.cancel();
            assert_eq!(waiter.join().unwrap(), Err(Error::Canceled { step: None }), "{poll:?}");
        }

        // A waiter gets the slot once the holder drops it.
        let (waiting, waited) = mpsc::channel();
        let waiter = {
            let c = config(dir.path(), 1, "next");
            std::thread::spawn(move || acquire_with(&c, None, || waiting.send(()).unwrap()).map(drop))
        };
        waited.recv().unwrap();
        drop(holder);
        assert_eq!(waiter.join().unwrap(), Ok(()));

        // A run that fails or is canceled releases its slot when it returns.
        let c = config(dir.path(), 1, "run");
        let mut failing = vec![Step::new("test", |_| Err("failed".to_owned()))];
        let result = run_steps_to(&mut failing, &RunOptions::default().with_lock(c.clone()), &mut Vec::new(), &mut Vec::new());
        assert!(matches!(result, Err(Error::Step { .. })));
        assert!(try_acquire(&c).unwrap().is_some());

        let cancel = Cancel::new();
        cancel.cancel();
        let mut canceled = vec![Step::new("test", |_| Ok(()))];
        let options = RunOptions::default().with_lock(c.clone()).with_cancel(cancel);
        let result = run_steps_to(&mut canceled, &options, &mut Vec::new(), &mut Vec::new());
        assert!(matches!(result, Err(Error::Canceled { .. })));
        assert!(try_acquire(&c).unwrap().is_some());

        // A run waiting for a held slot ends when canceled, without running a step.
        let held = try_acquire(&c).unwrap().expect("free slot");
        let cancel = Cancel::new();
        cancel.cancel();
        let mut never = vec![Step::new("test", |_| Err("ran".to_owned()))];
        let options = RunOptions::default().with_lock(c.clone()).with_cancel(cancel);
        let result = run_steps_to(&mut never, &options, &mut Vec::new(), &mut Vec::new());
        assert_eq!(result, Err(Error::Canceled { step: None }));
        drop(held);
    }
}
