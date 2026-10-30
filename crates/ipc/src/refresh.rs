//! Watch and refresh rules for linked files (`PROTOCOL.md` section 10).
//!
//! A producer writes `result_path` with [`write_atomic`]. A consumer keeps one
//! [`Tracker`] per linked file and calls [`Tracker::poll`] on a timer. The
//! tracker needs no connection to the producer, so it also works while the
//! producer is offline. `std` only.
//!
//! | State | Meaning |
//! |---|---|
//! | `Current` | the disk has the content the consumer loaded |
//! | `Modified` | the disk differs from what the consumer loaded; the epoch went up |
//! | `Missing` | the file does not exist |

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::error::LinkError;

/// Size, modification time and content hash of a file at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_ms: u64,
    /// `sha256:` followed by 64 lowercase hex characters.
    pub hash: String,
}

/// What a consumer shows in its Links panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Current,
    Modified,
    Missing,
}

impl FileState {
    /// The wire word used by `link.refresh`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Modified => "modified",
            Self::Missing => "missing",
        }
    }
}

/// The result of one [`Tracker::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub state: FileState,
    pub epoch: u64,
    /// True when this poll added 1 to the epoch.
    pub bumped: bool,
}

/// Watches one file for a consumer.
#[derive(Debug)]
pub struct Tracker {
    path: PathBuf,
    epoch: u64,
    /// What the consumer has loaded.
    loaded: Option<Fingerprint>,
    /// What the last poll saw on disk.
    seen: Option<Fingerprint>,
    state: FileState,
}

impl Tracker {
    /// Start watching `path` at `epoch`. A file that exists now counts as loaded.
    pub fn new(path: impl Into<PathBuf>, epoch: u64) -> Result<Self, LinkError> {
        let path = path.into();
        let seen = fingerprint_if_present(&path)?;
        let state = if seen.is_some() { FileState::Current } else { FileState::Missing };
        Ok(Self { path, epoch, loaded: seen.clone(), seen, state })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn state(&self) -> FileState {
        self.state
    }

    /// The fingerprint from the last poll, or `None` while the file is missing.
    pub fn seen(&self) -> Option<&Fingerprint> {
        self.seen.as_ref()
    }

    /// Raise the epoch to `epoch` when it is larger. Used when the producer is
    /// reachable again and reports its own epoch. An epoch never decreases.
    pub fn adopt_epoch(&mut self, epoch: u64) {
        self.epoch = self.epoch.max(epoch);
    }

    /// Look at the disk once.
    ///
    /// A shallow poll (`deep` false) reads size and mtime and hashes the file
    /// only when either differs from the last poll. A deep poll always hashes.
    /// An edit that keeps size and mtime is invisible to a shallow poll.
    ///
    /// A missing file gives `Missing` and no epoch bump. Any other read error
    /// is returned and leaves the state as it was.
    pub fn poll(&mut self, deep: bool) -> Result<Change, LinkError> {
        let Some(mut file) = open_if_present(&self.path)? else {
            self.seen = None;
            self.state = FileState::Missing;
            return Ok(self.change(false));
        };
        let (size, mtime_ms) = stat(&file, &self.path)?;
        if !deep
            && let Some(prev) = &self.seen
            && prev.size == size
            && prev.mtime_ms == mtime_ms
        {
            return Ok(self.change(false));
        }
        let now = Fingerprint { size, mtime_ms, hash: hash_reader(&mut file)? };
        let changed = self.seen.as_ref().is_none_or(|prev| prev.hash != now.hash);
        let bumped = changed && self.differs_from_loaded_or_was_seen(&now);
        self.apply(now, bumped);
        Ok(self.change(bumped))
    }

    /// The consumer loaded the content it last saw. `Modified` becomes `Current`.
    pub fn accept(&mut self) {
        self.loaded = self.seen.clone();
        self.state = if self.seen.is_some() { FileState::Current } else { FileState::Missing };
    }

    /// Point the link at `new_path`. The new file is read at once. Its hash
    /// equal to what the consumer loaded gives `Current` with no bump. A
    /// different hash gives `Modified` and one bump. A missing `new_path` is an
    /// error and the old path stays.
    pub fn relink(&mut self, new_path: impl Into<PathBuf>) -> Result<Change, LinkError> {
        let new_path = new_path.into();
        let Some(now) = fingerprint_if_present(&new_path)? else {
            return Err(LinkError::Unknown(format!("{} does not exist", new_path.display())));
        };
        let same = self.loaded.as_ref().is_some_and(|loaded| loaded.hash == now.hash);
        self.path = new_path;
        self.apply(now, !same);
        Ok(self.change(!same))
    }

    /// Whether a changed hash counts as a new version. The first sighting after
    /// `Missing` that equals the loaded content is not a new version.
    fn differs_from_loaded_or_was_seen(&self, now: &Fingerprint) -> bool {
        match (&self.seen, &self.loaded) {
            (Some(_), _) => true,
            (None, Some(loaded)) => loaded.hash != now.hash,
            (None, None) => true,
        }
    }

    fn apply(&mut self, now: Fingerprint, bumped: bool) {
        if bumped {
            self.epoch = self.epoch.saturating_add(1);
        }
        let current = self.loaded.as_ref().is_some_and(|loaded| loaded.hash == now.hash);
        self.seen = Some(now);
        self.state = if current { FileState::Current } else { FileState::Modified };
    }

    fn change(&self, bumped: bool) -> Change {
        Change { state: self.state, epoch: self.epoch, bumped }
    }
}

/// Fingerprint `path`. A missing file is an error.
pub fn fingerprint(path: &Path) -> Result<Fingerprint, LinkError> {
    let mut file = File::open(path)?;
    let (size, mtime_ms) = stat(&file, path)?;
    Ok(Fingerprint { size, mtime_ms, hash: hash_reader(&mut file)? })
}

/// `None` when the file does not exist.
pub fn fingerprint_if_present(path: &Path) -> Result<Option<Fingerprint>, LinkError> {
    match open_if_present(path)? {
        None => Ok(None),
        Some(mut file) => {
            let (size, mtime_ms) = stat(&file, path)?;
            Ok(Some(Fingerprint { size, mtime_ms, hash: hash_reader(&mut file)? }))
        }
    }
}

fn open_if_present(path: &Path) -> Result<Option<File>, LinkError> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(LinkError::Io(format!("{}: {err}", path.display()))),
    }
}

fn stat(file: &File, path: &Path) -> Result<(u64, u64), LinkError> {
    let meta = file.metadata().map_err(|err| LinkError::Io(format!("{}: {err}", path.display())))?;
    if !meta.is_file() {
        return Err(LinkError::Invalid(format!("{} is not a regular file", path.display())));
    }
    let modified = meta.modified().map_err(|err| LinkError::Io(format!("{}: {err}", path.display())))?;
    let since = modified.duration_since(UNIX_EPOCH).map_err(|_| LinkError::Io(format!("{}: mtime is before 1970", path.display())))?;
    let ms = u64::try_from(since.as_millis()).map_err(|_| LinkError::Io(format!("{}: mtime is out of range", path.display())))?;
    Ok((meta.len(), ms))
}

fn hash_reader(reader: &mut impl Read) -> Result<String, LinkError> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or_default());
    }
    Ok(format!("sha256:{}", crate::auth::encode_hex(&hasher.finalize())))
}

/// True for the temporary names [`write_atomic`] uses. Readers skip them.
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` so a reader sees the old file or the new file.
///
/// The bytes go to `.<name>.<pid>.<n>.tmp` in the same directory, are flushed,
/// and are renamed over `path`. The directory is flushed on Unix. A failure
/// removes the temporary file and leaves `path` as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<Fingerprint, LinkError> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| LinkError::Invalid(format!("{} has no file name", path.display())))?;
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let written = write_and_rename(&tmp, path, bytes);
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written?;
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    fingerprint(path)
}

fn write_and_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> Result<(), LinkError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("cl-refresh-{label}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sha_of(bytes: &[u8]) -> String {
        format!("sha256:{}", crate::auth::encode_hex(&Sha256::digest(bytes)))
    }

    #[test]
    fn hash_is_sha256_of_the_content() {
        let tmp = Tmp::new("hash");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"abc").unwrap();
        let fp = fingerprint(&path).unwrap();
        assert_eq!(fp.hash, "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(fp.size, 3);
        assert!(fp.mtime_ms > 0);
    }

    #[test]
    fn unchanged_file_stays_current_with_no_bump() {
        let tmp = Tmp::new("same");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let mut tracker = Tracker::new(&path, 4).unwrap();
        assert_eq!(tracker.state(), FileState::Current);
        for deep in [false, true] {
            let change = tracker.poll(deep).unwrap();
            assert_eq!(change, Change { state: FileState::Current, epoch: 4, bumped: false });
        }
    }

    #[test]
    fn changed_content_is_modified_and_bumps_once() {
        let tmp = Tmp::new("mod");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let mut tracker = Tracker::new(&path, 4).unwrap();
        write_atomic(&path, b"two!").unwrap();
        let change = tracker.poll(false).unwrap();
        assert_eq!(change, Change { state: FileState::Modified, epoch: 5, bumped: true });
        // Polling again with no new edit does not bump again.
        let again = tracker.poll(true).unwrap();
        assert_eq!(again, Change { state: FileState::Modified, epoch: 5, bumped: false });
        // A second edit bumps again.
        write_atomic(&path, b"three").unwrap();
        assert_eq!(tracker.poll(false).unwrap().epoch, 6);
        // Loading the new content clears the state.
        tracker.accept();
        assert_eq!(tracker.state(), FileState::Current);
        assert_eq!(tracker.poll(true).unwrap(), Change { state: FileState::Current, epoch: 6, bumped: false });
    }

    #[test]
    fn touch_without_content_change_does_not_bump() {
        let tmp = Tmp::new("touch");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let mut tracker = Tracker::new(&path, 0).unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(60)).unwrap();
        drop(file);
        let change = tracker.poll(false).unwrap();
        assert_eq!(change, Change { state: FileState::Current, epoch: 0, bumped: false });
        assert!(tracker.seen().unwrap().mtime_ms > 0);
    }

    #[test]
    fn shallow_poll_catches_a_same_size_edit_when_mtime_moves() {
        let tmp = Tmp::new("mtime");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"aaaa").unwrap();
        let mut tracker = Tracker::new(&path, 0).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, b"bbbb").unwrap();
        OpenOptions::new().write(true).open(&path).unwrap().set_modified(before + Duration::from_secs(10)).unwrap();
        assert_eq!(tracker.poll(false).unwrap(), Change { state: FileState::Modified, epoch: 1, bumped: true });
    }

    #[test]
    fn shallow_poll_misses_an_edit_that_keeps_size_and_mtime_and_deep_poll_finds_it() {
        let tmp = Tmp::new("deep");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"aaaa").unwrap();
        let mut tracker = Tracker::new(&path, 0).unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, b"bbbb").unwrap();
        OpenOptions::new().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
        assert_eq!(tracker.poll(false).unwrap().state, FileState::Current, "documented limit of a shallow poll");
        let change = tracker.poll(true).unwrap();
        assert_eq!(change, Change { state: FileState::Modified, epoch: 1, bumped: true });
    }

    #[test]
    fn missing_then_same_content_returns_to_current_without_bump() {
        let tmp = Tmp::new("missing");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let mut tracker = Tracker::new(&path, 2).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(tracker.poll(false).unwrap(), Change { state: FileState::Missing, epoch: 2, bumped: false });
        assert!(tracker.seen().is_none());
        std::fs::write(&path, b"one").unwrap();
        assert_eq!(tracker.poll(false).unwrap(), Change { state: FileState::Current, epoch: 2, bumped: false });
    }

    #[test]
    fn missing_then_new_content_is_modified_with_a_bump() {
        let tmp = Tmp::new("missing2");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let mut tracker = Tracker::new(&path, 2).unwrap();
        std::fs::remove_file(&path).unwrap();
        tracker.poll(false).unwrap();
        std::fs::write(&path, b"different").unwrap();
        assert_eq!(tracker.poll(false).unwrap(), Change { state: FileState::Modified, epoch: 3, bumped: true });
    }

    #[test]
    fn tracker_for_a_file_that_does_not_exist_yet() {
        let tmp = Tmp::new("pending");
        let path = tmp.0.join("later.png");
        let mut tracker = Tracker::new(&path, 0).unwrap();
        assert_eq!(tracker.state(), FileState::Missing);
        write_atomic(&path, b"first").unwrap();
        assert_eq!(tracker.poll(false).unwrap(), Change { state: FileState::Modified, epoch: 1, bumped: true });
    }

    #[test]
    fn unreadable_path_is_an_error_not_missing() {
        let tmp = Tmp::new("dir");
        let path = tmp.0.join("a.png");
        std::fs::create_dir(&path).unwrap();
        assert!(Tracker::new(&path, 0).is_err(), "a directory is not a result file");
    }

    #[test]
    fn relink_compares_hashes_and_rejects_a_missing_target() {
        let tmp = Tmp::new("relink");
        let old = tmp.0.join("old.png");
        let same = tmp.0.join("same.png");
        let other = tmp.0.join("other.png");
        std::fs::write(&old, b"one").unwrap();
        std::fs::write(&same, b"one").unwrap();
        std::fs::write(&other, b"two").unwrap();
        let mut tracker = Tracker::new(&old, 7).unwrap();

        let err = tracker.relink(tmp.0.join("nope.png")).unwrap_err();
        assert!(matches!(err, LinkError::Unknown(_)), "{err:?}");
        assert_eq!(tracker.path(), old.as_path(), "old path stays");

        assert_eq!(tracker.relink(&same).unwrap(), Change { state: FileState::Current, epoch: 7, bumped: false });
        assert_eq!(tracker.path(), same.as_path());
        assert_eq!(tracker.relink(&other).unwrap(), Change { state: FileState::Modified, epoch: 8, bumped: true });
    }

    #[test]
    fn adopt_epoch_never_decreases() {
        let tmp = Tmp::new("adopt");
        let path = tmp.0.join("a.png");
        std::fs::write(&path, b"x").unwrap();
        let mut tracker = Tracker::new(&path, 5).unwrap();
        tracker.adopt_epoch(3);
        assert_eq!(tracker.epoch(), 5);
        tracker.adopt_epoch(9);
        assert_eq!(tracker.epoch(), 9);
    }

    #[test]
    fn write_atomic_replaces_whole_and_leaves_no_temp() {
        let tmp = Tmp::new("atomic");
        let path = tmp.0.join("out.png");
        let first = write_atomic(&path, b"first").unwrap();
        assert_eq!(first.hash, sha_of(b"first"));
        let second = write_atomic(&path, b"second version").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second version");
        assert_eq!(second.hash, sha_of(b"second version"));
        let names: Vec<String> = std::fs::read_dir(&tmp.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, vec!["out.png".to_string()], "no temp file left");
    }

    #[test]
    fn write_atomic_failure_keeps_the_old_file_and_removes_the_temp() {
        let tmp = Tmp::new("atomic-fail");
        let path = tmp.0.join("out.png");
        write_atomic(&path, b"keep").unwrap();
        // A directory at the destination makes rename fail after the temp file is written.
        let blocked = tmp.0.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("child"), b"x").unwrap();
        assert!(write_atomic(&blocked, b"nope").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        let temps = std::fs::read_dir(&tmp.0).unwrap().filter(|e| is_temp_name(e.as_ref().unwrap().file_name().to_str().unwrap())).count();
        assert_eq!(temps, 0);
    }

    #[test]
    fn reader_never_sees_a_half_file_while_a_writer_replaces_it() {
        let tmp = Tmp::new("race");
        let path = tmp.0.join("out.bin");
        let a = vec![b'a'; 512 * 1024];
        let b = vec![b'b'; 768 * 1024];
        write_atomic(&path, &a).unwrap();
        let writer_path = path.clone();
        let (wa, wb) = (a.clone(), b.clone());
        let writer = std::thread::spawn(move || {
            for i in 0..60 {
                write_atomic(&writer_path, if i % 2 == 0 { &wb } else { &wa }).unwrap();
            }
        });
        while !writer.is_finished() {
            let got = std::fs::read(&path).unwrap();
            assert!(got == a || got == b, "saw a torn file of {} bytes", got.len());
        }
        writer.join().unwrap();
    }

    #[test]
    fn temp_names() {
        assert!(is_temp_name(".out.png.12.0.tmp"));
        assert!(!is_temp_name("out.png"));
        assert!(!is_temp_name("notes.tmp"));
    }
}
