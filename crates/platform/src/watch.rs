//! Open files that change on disk (N10.02): another app or a build rewrites a document that is
//! open, and the app reloads it or says so.
//!
//! Cheap by design (storytold/pdfcraft#431). One worker thread looks at each watched file's metadata,
//! never its contents, every [`IDLE`], and sleeps without waking while nothing is watched. A
//! changed file is read only once it has stopped changing ([`SETTLE`] apart) and, when the app
//! gives a [`ReadPolicy::Whole`] check, once it looks complete (PdfCraft reads the last KiB
//! first and wants `%%EOF`). The app's own saves are recognised by the [`FileStamp`] it records
//! after writing ([`FileWatch::known_now`]), so they never count as a change.
//!
//! What the app does with a [`Changed`] is its rule: PdfCraft reloads a clean document in place
//! and shows [`FILE_CHANGED_UNSAVED`] over one with unsaved changes; PhotoCraft does not watch
//! and asks at Save time instead ([`FileWatch::disk_differs`], `craft_dialogs::save`).
//!
//! No egui: the worker calls the app's [`Wake`] when it finds a change, so the app runs a frame
//! that reads it with [`FileWatch::take_changes`].

use std::collections::HashMap;
use std::fs::{File, Metadata};
use std::hash::Hash;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use craft_host::Wake;

use crate::PlatformText;

/// How often an unchanged file is looked at: 250 ms.
pub const IDLE: Duration = Duration::from_millis(250);
/// How long a changed file must stay the same before it is read: 40 ms (writers pause between
/// chunks).
pub const SETTLE: Duration = Duration::from_millis(40);

/// One version of a file, told apart without reading it: its length and modification time, and
/// what the platform adds (Unix: device, inode and change time in seconds and nanoseconds;
/// Windows: creation and last-write times in 100 ns ticks). The modification time alone is
/// coarse on FAT and some network drives, and a rebuild can keep the length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    extra: [u64; 4],
}

impl FileStamp {
    /// The stamp of a file from its metadata.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:42 Stamp::of
    pub fn of(meta: &Metadata) -> FileStamp {
        #[cfg(unix)]
        let extra = {
            use std::os::unix::fs::MetadataExt;
            [meta.dev(), meta.ino(), meta.ctime().cast_unsigned(), meta.ctime_nsec().cast_unsigned()]
        };
        #[cfg(windows)]
        let extra = {
            use std::os::windows::fs::MetadataExt;
            [meta.creation_time(), meta.last_write_time(), 0, 0]
        };
        #[cfg(not(any(unix, windows)))]
        let extra = [0; 4];
        FileStamp { len: meta.len(), modified: meta.modified().ok(), extra }
    }

    /// The file at `path` now, following a symlink (Save writes through it); `None` when it is
    /// missing, not a file or can't be read.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:72 Stamp::at
    pub fn at(path: &Path) -> Option<FileStamp> {
        std::fs::metadata(path).ok().filter(Metadata::is_file).map(|m| FileStamp::of(&m))
    }

    /// The file's length in bytes when stamped.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the file was empty when stamped.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The modification time; `None` where the file system keeps none.
    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    /// Only the Unix change time differs from `other`: the metadata may have changed without a
    /// content edit (LaunchServices, Finder tags). [`FileWatch::disk_differs`] then compares the
    /// bytes; the watcher itself reports such a change, and the app compares it with the bytes it
    /// saved.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:59 metadata_only_candidate
    pub fn metadata_only_change(self, other: FileStamp) -> bool {
        #[cfg(unix)]
        {
            self.len == other.len && self.modified == other.modified && self.extra.get(..2) == other.extra.get(..2)
        }
        #[cfg(not(unix))]
        {
            let _ = other;
            false
        }
    }
}

/// What the watcher reads of a changed file.
#[derive(Clone)]
pub enum ReadPolicy {
    /// Nothing: the change is reported with its stamp and no bytes. For apps that ask before
    /// reloading anything.
    Stamp,
    /// The whole file, into a buffer of exactly its size, once `complete` says the last `tail`
    /// bytes (read first) end a whole file. A file still being written then costs one small
    /// read. PdfCraft: `tail` 1024, `complete` finds `%%EOF`.
    Whole {
        /// How many bytes at the end of the file to read first, for `complete`.
        tail: usize,
        /// Tells whether those last bytes end a whole file.
        complete: Complete,
    },
}

/// [`ReadPolicy::Whole`]'s check of a file's last bytes.
pub type Complete = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

impl std::fmt::Debug for ReadPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadPolicy::Stamp => f.write_str("Stamp"),
            ReadPolicy::Whole { tail, .. } => f.debug_struct("Whole").field("tail", tail).finish_non_exhaustive(),
        }
    }
}

/// A new version of a watched file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed<K> {
    /// The app's key for the document (its id).
    pub key: K,
    /// The file's path.
    pub path: PathBuf,
    /// The file's stamp now.
    pub stamp: FileStamp,
    /// The bytes with [`ReadPolicy::Whole`]; `None` with [`ReadPolicy::Stamp`].
    pub bytes: Option<Arc<Vec<u8>>>,
    /// Which of the app's readings or writings of the file this was found for. A change found
    /// before the latest one ([`FileWatch::known_now`] bumps it) is older than the document in
    /// memory and [`FileWatch::take_changes`] drops it (storytold/pdfcraft#1802).
    pub epoch: u64,
}

/// What the worker is told about one file: the key, the path, the version the app knows and the
/// epoch.
type WatchEntry<K> = (K, PathBuf, Option<FileStamp>, u64);

/// The app's side of the watcher, on the UI thread.
///
/// ```ignore
/// let mut watch = FileWatch::new(Wake::new(move || ctx.request_repaint()))
///     .read(ReadPolicy::Whole { tail: 1024, complete: Arc::new(|tail| tail.windows(5).any(|w| w == b"%%EOF")) });
/// // Each frame: the open documents that have a file (while the preference is on).
/// watch.set(docs.iter().filter_map(|d| Some((d.id, d.path.clone()?))));
/// for change in watch.take_changes() {
///     if doc(change.key).is_dirty() { show(FILE_CHANGED_UNSAVED) } else { reload(change.key, change.bytes) }
/// }
/// // After opening or saving a document:
/// watch.known_now(doc.id, &path);
/// ```
pub struct FileWatch<K> {
    wake: Wake,
    policy: ReadPolicy,
    /// `None` until a file is watched; dropped to stop the worker.
    inbox: Option<Sender<Vec<WatchEntry<K>>>>,
    found: Arc<Mutex<Vec<Changed<K>>>>,
    /// The version of each document's file the app last read or wrote.
    known: HashMap<K, (PathBuf, FileStamp)>,
    /// How many times the app has read or written each document's file.
    epochs: HashMap<K, u64>,
    /// What the worker was last told, so an unchanged list sends nothing.
    sent: Vec<(K, PathBuf)>,
    /// The version each document came from (its open, reload or last save). Unlike `known`, a
    /// version found on disk and handed to the app is not recorded here, so
    /// [`FileWatch::disk_differs`] still sees that the file is not the one the document came from.
    basis: HashMap<K, (PathBuf, FileStamp)>,
    /// `known` or `epochs` changed since the worker was last told.
    dirty: bool,
    /// Tests: no thread; [`FileWatch::tick`] looks once.
    inline: bool,
    local: Watcher<K>,
}

impl<K: Clone + Eq + Hash + Send + 'static> FileWatch<K> {
    /// A watcher that reports changes by stamp only ([`ReadPolicy::Stamp`]). `wake` runs on the
    /// worker when a change is found. An app that only asks [`disk_differs`](Self::disk_differs)
    /// and never calls [`set`](Self::set) passes [`Wake::none`]: no worker starts.
    pub fn new(wake: Wake) -> Self {
        Self {
            wake,
            policy: ReadPolicy::Stamp,
            inbox: None,
            found: Arc::default(),
            known: HashMap::new(),
            epochs: HashMap::new(),
            sent: Vec::new(),
            basis: HashMap::new(),
            dirty: false,
            inline: false,
            local: Watcher::default(),
        }
    }

    /// Sets what the watcher reads of a changed file.
    pub fn read(mut self, policy: ReadPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// No worker thread: each [`tick`](Self::tick) looks at every file once, on the caller's
    /// thread. For tests and wasm (where nothing is watched: there are no paths). A change is
    /// reported by the second tick that sees the same new version, as the worker reports it after
    /// [`SETTLE`]: one look can't tell a finished write from one in progress, and waiting inside a
    /// tick would block the caller.
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    /// The files to watch from now on, each with the app's key. A file watched already keeps its
    /// pending change; one left out stops being watched but keeps its recorded version, so
    /// [`disk_differs`](Self::disk_differs) still answers for it (the app may watch only while a
    /// preference is on). Records go only through [`forget`](Self::forget). Allocates nothing when the list is the
    /// one sent last. Starts the worker on the first file; a worker that couldn't start is tried
    /// again on the next call.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:343 watch_files, :374 send_to_watcher, :118 Watcher::set
    pub fn set(&mut self, files: impl IntoIterator<Item = (K, PathBuf)>) {
        let files: Vec<(K, PathBuf)> = files.into_iter().collect();
        if !self.dirty && files == self.sent {
            return;
        }
        let list: Vec<WatchEntry<K>> = files
            .iter()
            .map(|(key, path)| {
                let known = self.known.get(key).filter(|(own, _)| own == path).map(|(_, stamp)| *stamp);
                (key.clone(), path.clone(), known, self.epoch_of(key))
            })
            .collect();
        if self.deliver(list) {
            self.sent = files;
            self.dirty = false;
        }
    }

    /// The app just read or wrote `key`'s file at `path`: that version is not a change, and
    /// anything found before it is stale.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:296 known_now
    pub fn known_now(&mut self, key: K, path: &Path) {
        self.known_as(key, path, FileStamp::at(path));
    }

    /// Records `stamp`, taken before the file was read, as the version `key` came from.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:301 known_as
    pub fn known_as(&mut self, key: K, path: &Path, stamp: Option<FileStamp>) {
        if let Some(stamp) = stamp {
            self.known.insert(key.clone(), (path.to_path_buf(), stamp));
            self.basis.insert(key.clone(), (path.to_path_buf(), stamp));
        }
        let epoch = self.epochs.entry(key).or_default();
        *epoch = epoch.wrapping_add(1);
        self.dirty = true;
    }

    /// Stops tracking `key` (its document closed).
    pub fn forget(&mut self, key: &K) {
        self.known.remove(key);
        self.basis.remove(key);
        self.epochs.remove(key);
        self.dirty = true;
    }

    /// Whether the file at `path` is another version than the one `key` came from: something
    /// else wrote it since the document was opened, reloaded or last saved (storytold/pdfcraft#2166,
    /// PhotoCraft `changed_on_disk`). A missing or unreadable file, or a document whose version
    /// was never recorded, is not a difference. When only the change time differs and `saved`
    /// holds the bytes the app last wrote or read, equal bytes are not a difference.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:313 disk_differs
    // pc: crates/ui-egui/src/app/files.rs:539 changed_on_disk
    pub fn disk_differs(&self, key: &K, path: &Path, saved: Option<&[u8]>) -> bool {
        let (Some(now), Some((own, basis))) = (FileStamp::at(path), self.basis.get(key)) else { return false };
        if own != path || now == *basis {
            return false;
        }
        !(now.metadata_only_change(*basis) && saved.is_some_and(|bytes| contents_match(path, now, bytes)))
    }

    /// The changes found since the last call, oldest first, without the stale ones (another
    /// file than the key's now, an older epoch, or the version the app recorded).
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:401 file_changed (the checks before reloading)
    pub fn take_changes(&mut self) -> Vec<Changed<K>> {
        let found = std::mem::take(&mut *self.found.lock().unwrap_or_else(PoisonError::into_inner));
        let mut changes = Vec::new();
        for change in found {
            let same_file = self.sent.iter().any(|(key, path)| *key == change.key && *path == change.path);
            let current = self.epoch_of(&change.key) == change.epoch;
            let recorded = self.known.get(&change.key).is_some_and(|(path, stamp)| *path == change.path && *stamp == change.stamp);
            if !same_file || !current || recorded {
                continue;
            }
            self.known.insert(change.key.clone(), (change.path.clone(), change.stamp));
            changes.push(change);
        }
        changes
    }

    /// With [`inline`](Self::inline): looks at every watched file once. Does nothing otherwise.
    // pdfcraft: crates/ui-egui/src/platform/file_watch.rs:137 Watcher::tick
    pub fn tick(&mut self) {
        if !self.inline {
            return;
        }
        let (found, _) = self.local.tick(&self.policy);
        self.found.lock().unwrap_or_else(PoisonError::into_inner).extend(found);
    }
}

impl<K: Clone + Eq + Hash + Send + 'static> FileWatch<K> {
    fn epoch_of(&self, key: &K) -> u64 {
        self.epochs.get(key).copied().unwrap_or(0)
    }

    /// Hands `list` to the watcher, starting the worker when needed. `false`: not delivered (the
    /// worker couldn't start or ended); the next [`set`](Self::set) tries again.
    fn deliver(&mut self, list: Vec<WatchEntry<K>>) -> bool {
        if self.inline {
            self.local.set(list);
            return true;
        }
        if self.inbox.is_none() {
            if list.is_empty() {
                return true;
            }
            let (tx, rx) = mpsc::channel();
            let out = Arc::clone(&self.found);
            let (wake, policy) = (self.wake.clone(), self.policy.clone());
            if std::thread::Builder::new().name("craft-file-watch".into()).spawn(move || run(&rx, &out, &wake, &policy)).is_err() {
                return false;
            }
            self.inbox = Some(tx);
        }
        let delivered = self.inbox.as_ref().is_some_and(|tx| tx.send(list).is_ok());
        if !delivered {
            self.inbox = None;
        }
        delivered
    }
}

/// The watcher's record of one file.
struct Entry<K> {
    key: K,
    path: PathBuf,
    epoch: u64,
    known: Option<FileStamp>,
    /// A changed version seen once, waiting to be seen unchanged again.
    pending: Option<FileStamp>,
}

/// Looks at the watched files: on the worker thread, or on the caller's in tests.
struct Watcher<K> {
    files: Vec<Entry<K>>,
}

impl<K> Default for Watcher<K> {
    fn default() -> Self {
        Self { files: Vec::new() }
    }
}

impl<K: Clone + Eq + Hash> Watcher<K> {
    /// Watches `list` from now on. A file watched already keeps its pending change; a version
    /// the app knows replaces the watcher's own.
    fn set(&mut self, list: Vec<WatchEntry<K>>) {
        let old = std::mem::take(&mut self.files);
        self.files = list
            .into_iter()
            .map(|(key, path, known, epoch)| {
                let prev = old.iter().find(|e| e.key == key && e.path == path);
                Entry { known: known.or(prev.and_then(|e| e.known)), pending: prev.and_then(|e| e.pending), key, path, epoch }
            })
            .collect();
    }

    /// Looks at every file once: the new versions found, and whether a file is still changing
    /// (look again after [`SETTLE`] rather than [`IDLE`]).
    fn tick(&mut self, policy: &ReadPolicy) -> (Vec<Changed<K>>, bool) {
        let mut found = Vec::new();
        let mut settling = false;
        for e in &mut self.files {
            // Missing (deleted, or being replaced) or unreadable: the document stays as it is.
            let Some(now) = FileStamp::at(&e.path) else {
                e.pending = None;
                continue;
            };
            let known = *e.known.get_or_insert(now);
            if now == known {
                e.pending = None;
                continue;
            }
            if e.pending != Some(now) {
                e.pending = Some(now);
                settling = true;
                continue;
            }
            e.pending = None;
            let bytes = match policy {
                ReadPolicy::Stamp => None,
                ReadPolicy::Whole { tail, complete } => match read_whole(&e.path, now, *tail, complete) {
                    Version::Ready(bytes) => Some(bytes),
                    // Not a whole file in this version (a build that failed or paused): wait for
                    // the next one.
                    Version::Incomplete => {
                        e.known = Some(now);
                        continue;
                    }
                    // Changed again while being read, or held by its writer: look again soon.
                    Version::Busy => {
                        settling = true;
                        continue;
                    }
                },
            };
            e.known = Some(now);
            found.push(Changed { key: e.key.clone(), path: e.path.clone(), stamp: now, bytes, epoch: e.epoch });
        }
        (found, settling)
    }
}

/// What a changed file turned out to be.
enum Version {
    Ready(Arc<Vec<u8>>),
    Incomplete,
    Busy,
}

/// Reads `path` if it is still the version `stamp` and `complete` accepts its last `tail` bytes
/// (read first). The whole file is then read into a buffer of exactly its size.
fn read_whole(path: &Path, stamp: FileStamp, tail: usize, complete: &Complete) -> Version {
    // Windows refuses to open a file its writer holds exclusively: not ready yet.
    let Ok(mut file) = File::open(path) else { return Version::Busy };
    let Ok(meta) = file.metadata() else { return Version::Busy };
    if FileStamp::of(&meta) != stamp {
        return Version::Busy;
    }
    let Ok(size) = usize::try_from(meta.len()) else { return Version::Incomplete };
    let mut end = vec![0u8; size.min(tail)];
    let Ok(back) = i64::try_from(end.len()) else { return Version::Incomplete };
    if file.seek(SeekFrom::End(-back)).is_err() || file.read_exact(&mut end).is_err() {
        return Version::Busy;
    }
    if !complete(&end) {
        return Version::Incomplete;
    }
    let mut bytes = Vec::new();
    // Too big to hold: the open version stays.
    if bytes.try_reserve_exact(size).is_err() {
        return Version::Incomplete;
    }
    if file.seek(SeekFrom::Start(0)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return Version::Busy;
    }
    // Still that version, under that name: a writer may have replaced the file meanwhile.
    if bytes.len() != size || FileStamp::at(path) != Some(stamp) {
        return Version::Busy;
    }
    Version::Ready(Arc::new(bytes))
}

/// Whether the file at `path` still has `stamp` and holds exactly `expected`. A replacement,
/// changed bytes or any read failure is "no".
fn contents_match(path: &Path, stamp: FileStamp, expected: &[u8]) -> bool {
    let Ok(mut file) = File::open(path) else { return false };
    if !file.metadata().is_ok_and(|m| FileStamp::of(&m) == stamp) {
        return false;
    }
    let mut buffer = [0u8; 8192];
    for chunk in expected.chunks(buffer.len()) {
        let Some(out) = buffer.get_mut(..chunk.len()) else { return false };
        if file.read_exact(out).is_err() || out != chunk {
            return false;
        }
    }
    let mut extra = [0];
    matches!(file.read(&mut extra), Ok(0)) && file.metadata().is_ok_and(|m| FileStamp::of(&m) == stamp) && FileStamp::at(path) == Some(stamp)
}

/// The worker thread: watches the newest list the app sent and hands back new versions, waking
/// the app for them. It ends when the app (the inbox's sender) is gone.
fn run<K: Clone + Eq + Hash>(inbox: &mpsc::Receiver<Vec<WatchEntry<K>>>, out: &Mutex<Vec<Changed<K>>>, wake: &Wake, policy: &ReadPolicy) {
    let mut watcher = Watcher::default();
    // `None`: nothing to watch, so block until there is.
    let mut wait = None;
    loop {
        let message = match wait {
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(wait) => inbox.recv_timeout(wait),
        };
        match message {
            Ok(list) => watcher.set(list),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(list) = inbox.try_recv() {
            watcher.set(list);
        }
        if watcher.files.is_empty() {
            wait = None;
            continue;
        }
        let (found, settling) = watcher.tick(policy);
        if !found.is_empty() {
            out.lock().unwrap_or_else(PoisonError::into_inner).extend(found);
            wake.wake();
        }
        wait = Some(if settling { SETTLE } else { IDLE });
    }
}

/// The notice over a document with unsaved changes whose file changed on disk (PdfCraft's
/// notice bar, with Reload and Dismiss).
pub const FILE_CHANGED_UNSAVED: PlatformText = PlatformText {
    id: "watch.changedUnsaved",
    english: "This file changed on disk. Reload shows the new version and drops your unsaved changes; Save replaces it with this version.",
};
/// The status line after a clean document reloaded; `{name}` is its file name.
pub const FILE_RELOADED: PlatformText = PlatformText { id: "watch.reloaded", english: "Reloaded “{name}”: it changed on disk." };
/// Every [`PlatformText`] of this module, for the missing-translation report ([`crate::texts`]).
pub const TEXTS: &[PlatformText] = &[FILE_CHANGED_UNSAVED, FILE_RELOADED];
