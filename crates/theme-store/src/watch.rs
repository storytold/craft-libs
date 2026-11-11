//! [`SuiteWatch`]: the suite folder watched from a worker thread and published as snapshots that
//! a UI checks with one atomic load per frame.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use craft_theme::{SuiteChoice, ThemeDef, ThemeId, ThemePair};

use crate::{
    Accepted, FileStamp, LibraryFile, Problem, SelectionChange, Store, StoreError, StoreOptions, SuiteDir, ThemeEntry, ThemeRecord,
    store::{builtin, is_other_schema},
};

/// How often the worker checks `selection.json`'s stamp (length, modification time, file
/// identity) and the stamps of the themes in use.
pub const STAT_INTERVAL: Duration = Duration::from_millis(500);
/// How often the worker re-reads `selection.json` and compares its bytes, which catches a
/// replacement that kept length and modification time.
pub const VERIFY_INTERVAL: Duration = Duration::from_secs(10);
/// How many operation results a snapshot keeps.
pub const RESULTS_KEPT: usize = 16;
/// How long dropping a [`SuiteWatch`] waits for its worker.
#[cfg(not(target_arch = "wasm32"))]
const DROP_WAIT: Duration = Duration::from_millis(100);

/// Work for the worker; [`SuiteWatch::request`] returns its id and the result appears in
/// [`Snapshot::results`].
#[derive(Clone, Debug)]
pub enum Op {
    /// Writes the fields the user changed ([`Store::update`]). `base` is the accepted state the
    /// UI showed when the user made the change.
    Update {
        /// The accepted state the change was made against.
        base: Accepted,
        /// The fields to write.
        change: SelectionChange,
    },
    /// Adds a theme file to the library ([`Store::import`]); the result is
    /// [`OpOutput::Imported`].
    Import {
        /// The theme file's contents.
        bytes: Arc<[u8]>,
        /// Give the theme a new id when its id is already in the library, instead of failing.
        as_copy: bool,
    },
    /// Removes a library theme ([`Store::delete`]).
    Delete(ThemeId),
    /// A theme file for sharing ([`Store::export`]); the result is [`OpOutput::Exported`].
    Export(ThemeId),
    /// Re-lists the library now, for a theme picker that opens. Unchanged files are not read.
    /// No result: a new snapshot follows only if something changed.
    Rescan,
    /// The themes the app uses for its own choice ("Use a different theme in this app"); their
    /// library ones are kept parsed in [`Snapshot::app_theme`]. `None` when it uses the suite's.
    /// Send it when that choice changes. No result: the same pair again does nothing.
    AppTheme(Option<ThemePair>),
}

/// What a successful [`Op`] returns.
#[derive(Clone, Debug)]
pub enum OpOutput {
    /// The operation had no output.
    Done,
    /// The theme file's bytes.
    Exported(Arc<[u8]>),
    /// The stored id; for an import as copy, the new one.
    Imported(ThemeId),
}

/// The state of the suite folder at one generation. Snapshots never change; a new one replaces
/// the old.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The selection as last read or written. A damaged, too new or unreadable `selection.json`
    /// leaves it in place and shows up in `problems`.
    pub accepted: Accepted,
    /// The library themes, by file name.
    pub index: Arc<[ThemeEntry]>,
    /// The suite selection's library themes, light slot first and each once: the last version
    /// that parsed, kept while its file is missing, damaged or unreadable.
    pub active: Vec<ThemeRecord>,
    /// The library themes named by [`Op::AppTheme`], light slot first and each once: the last
    /// version that parsed, gone once its file is deleted, so the app falls back to its default.
    pub app_theme: Vec<ThemeRecord>,
    /// Files and folders the store could not use.
    pub problems: Vec<Problem>,
    /// The latest operation results by request id, oldest first, at most [`RESULTS_KEPT`].
    pub results: Arc<[(u64, Result<OpOutput, StoreError>)]>,
    /// Counts snapshots; each new one is higher.
    pub generation: u64,
}

impl Snapshot {
    /// The result of request `id`, once the worker has run it.
    pub fn result(&self, id: u64) -> Option<&Result<OpOutput, StoreError>> {
        self.results.iter().rev().find(|(r, _)| *r == id).map(|(_, result)| result)
    }

    /// The newest kept result.
    pub fn last_result(&self) -> Option<&(u64, Result<OpOutput, StoreError>)> {
        self.results.last()
    }

    /// A built-in theme, an active theme or one of the app's own library themes, for
    /// [`resolve_selection`](craft_theme::resolve_selection)'s `lookup`. Other library
    /// themes give `None` until the app names them with [`Op::AppTheme`].
    pub fn theme(&self, id: &ThemeId) -> Option<ThemeDef> {
        if let Some(theme) = builtin(id) {
            return Some(theme.clone());
        }
        self.active.iter().chain(&self.app_theme).find(|r| r.theme.id == *id).map(|r| r.theme.clone())
    }

    /// Any theme the suite knows, for a preview card: a built-in, an active or app theme, or a
    /// library theme in [`index`](Self::index), with no file read or [`Op::Export`].
    pub fn preview(&self, id: &ThemeId) -> Option<ThemeDef> {
        self.theme(id).or_else(|| {
            // The index is sorted by file name, and a library file's name is its id.
            let at = self.index.binary_search_by(|e| e.id.as_str().cmp(id.as_str())).ok()?;
            self.index.get(at).map(|e| e.theme.clone())
        })
    }
}

/// Watches the suite folder and runs store operations off the UI thread.
///
/// The worker thread ("craft-theme-watch") checks `selection.json`'s stamp every
/// [`STAT_INTERVAL`] and parses it only when it changed, compares its bytes every
/// [`VERIFY_INTERVAL`], re-lists the library only when `library_revision` changes or on
/// [`Op::Rescan`] (parsing only changed files), and re-reads the themes in use only when their
/// stamps change. Each change publishes a new [`Snapshot`], bumps [`generation`](Self::generation)
/// and calls `wake` once; an unchanged poll does neither.
///
/// Without a folder (and on `wasm32`) there is no thread: operations run inside
/// [`request`](Self::request), with the same snapshots and generations. `wake` is not called
/// then, since the caller is already in its frame. If the worker thread can't start, the same
/// form runs on the folder with a [`Problem::Watcher`] in the snapshot: it reads the folder
/// once and never polls.
///
/// Dropping stops the worker and waits at most 100 ms for it: operations already requested
/// finish first. A worker still blocked on a file or the lock then is detached, and ends when
/// its operation returns. A queued update that hasn't run when the process exits is lost, which
/// is one reason apps also keep the user's choice in their own preferences (`last_suite`).
///
/// `wake` runs on the worker and must not block or call back into the watch.
pub struct SuiteWatch {
    shared: Arc<Shared>,
    /// From [`StoreOptions::app_theme`]: already requested.
    app_theme: Option<ThemePair>,
    /// Whether open read `selection.json`, so generation 0 holds the disk state.
    selection_read: bool,
    next_request: AtomicU64,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
}

impl SuiteWatch {
    /// Watches `dir` with default store options. Returns at once; the first snapshot with the
    /// folder's contents follows from the worker.
    pub fn start(dir: SuiteDir, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::open(&dir, StoreOptions::new(concat!("craft-theme-store ", env!("CARGO_PKG_VERSION"))), wake)
    }

    /// Watches `dir` with the app's store options: `written_by` and the last accepted state from
    /// its preferences, which the first snapshot shows until the folder has been read.
    pub fn open(dir: &SuiteDir, options: StoreOptions, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let store = Store::open(dir, options);
        #[cfg(not(target_arch = "wasm32"))]
        if !dir.is_memory() {
            return Self::spawn(store, Box::new(wake), Intervals::default());
        }
        Self::inline(store, Box::new(wake))
    }

    /// An in-memory suite, for app constructors, tests, web builds and portable installs.
    pub fn memory() -> Self {
        Self::open(&SuiteDir::memory(), StoreOptions::new(concat!("craft-theme-store ", env!("CARGO_PKG_VERSION"))), || {})
    }

    /// Whether generation 0 holds the state read from disk at open, rather than only the last
    /// accepted state.
    pub(crate) fn selection_read(&self) -> bool {
        self.selection_read
    }

    /// The app's themes from [`StoreOptions::app_theme`], already requested at open.
    pub(crate) fn initial_app_theme(&self) -> Option<&ThemePair> {
        self.app_theme.as_ref()
    }

    /// Goes up by one with every new snapshot: the only call an app makes each frame.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    /// The current snapshot; take it when [`generation`](Self::generation) changed.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&lock(&self.shared.snapshot))
    }

    /// Queues `op` and returns its id; never blocks on the disk while a worker runs. The result
    /// appears in a later snapshot's [`results`](Snapshot::results).
    pub fn request(&self, op: Op) -> u64 {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(worker) = &self.worker {
            match worker.ops.send((id, op)) {
                Ok(()) => return id,
                // The worker is gone: run the request here rather than lose it.
                Err(std::sync::mpsc::SendError((id, op))) => {
                    let mut engine = lock(&self.shared.engine);
                    engine.watcher_failed(WORKER_STOPPED);
                    engine.apply(id, op);
                    self.shared.publish(&mut engine, false);
                    return id;
                }
            }
        }
        let mut engine = lock(&self.shared.engine);
        engine.apply(id, op);
        self.shared.publish(&mut engine, false);
        id
    }

    fn build(mut store: Store, wake: Wake) -> (Arc<Shared>, Self) {
        let app_theme = store.app_theme.take();
        let mut engine = Engine::new(store);
        let selection_read = engine.preload(app_theme.as_ref());
        let first = engine.snapshot(0);
        // Without the disk state in generation 0, the worker's first read is always published:
        // clients wait for it before sending changes.
        engine.changed = !selection_read;
        let shared = Arc::new(Shared {
            engine: Mutex::new(engine),
            snapshot: Mutex::new(Arc::new(first)),
            generation: AtomicU64::new(0),
            wake,
            stopped: AtomicBool::new(false),
        });
        let watch = SuiteWatch {
            shared: Arc::clone(&shared),
            app_theme: with_library_theme(app_theme),
            selection_read,
            next_request: AtomicU64::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
        };
        (shared, watch)
    }

    /// No thread: the first load and every operation run on the caller's thread.
    pub(crate) fn inline(store: Store, wake: Wake) -> Self {
        let (shared, watch) = Self::build(store, wake);
        let mut engine = lock(&shared.engine);
        engine.poll(true);
        shared.publish(&mut engine, false);
        drop(engine);
        watch
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn spawn(store: Store, wake: Wake, intervals: Intervals) -> Self {
        let (shared, mut watch) = Self::build(store, wake);
        let (ops, queue) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("craft-theme-watch".into()).spawn(move || {
            run(&worker_shared, &queue, intervals);
            // Released before the acknowledgement, so that a drop which sees it knows the
            // shared state (and the app's wake callback) is no longer held here.
            drop(worker_shared);
            let _ = done.send(());
        });
        match spawned {
            Ok(thread) => watch.worker = Some(Worker { ops, thread, finished: Mutex::new(finished) }),
            // No thread to be had: work on the caller's thread rather than not at all.
            Err(_) => {
                let mut engine = lock(&shared.engine);
                engine.watcher_failed(WORKER_NOT_STARTED);
                engine.poll(true);
                shared.publish(&mut engine, false);
            }
        }
        watch
    }

    /// Runs one poll on the caller's thread, as the worker does on its timer.
    #[cfg(test)]
    pub(crate) fn poll_once(&self, verify: bool) {
        let mut engine = lock(&self.shared.engine);
        engine.poll(verify);
        self.shared.publish(&mut engine, true);
    }
}

impl Drop for SuiteWatch {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(worker) = self.worker.take() {
            let Worker { ops, thread, finished } = worker;
            // Closing the queue ends the worker once the requests already in it are done.
            drop(ops);
            let finished = finished.into_inner().unwrap_or_else(PoisonError::into_inner);
            match finished.recv_timeout(DROP_WAIT) {
                // Done, or unwound past the acknowledgement: the thread is ending.
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = thread.join();
                }
                // Still blocked on a file or the lock: detached, it ends when that returns.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

pub(crate) type Wake = Box<dyn Fn() + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
const WORKER_NOT_STARTED: &str = "the theme watcher couldn't start; the theme folder was read once and isn't watched for changes";
#[cfg(not(target_arch = "wasm32"))]
const WORKER_STOPPED: &str = "the theme watcher stopped; changes are applied directly and the theme folder isn't watched";
#[cfg(not(target_arch = "wasm32"))]
const CHECK_FAILED: &str = "the theme watcher failed while checking the theme folder";
#[cfg(not(target_arch = "wasm32"))]
const REQUEST_FAILED: &str = "the theme watcher failed while running this request";

#[cfg(not(target_arch = "wasm32"))]
struct Worker {
    ops: std::sync::mpsc::Sender<(u64, Op)>,
    thread: std::thread::JoinHandle<()>,
    // In a mutex only so that `SuiteWatch` is `Sync`.
    finished: Mutex<std::sync::mpsc::Receiver<()>>,
}

/// The worker's timers; tests shorten them.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Intervals {
    pub stat: Duration,
    pub verify: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Intervals {
    fn default() -> Self {
        Intervals { stat: STAT_INTERVAL, verify: VERIFY_INTERVAL }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run(shared: &Shared, queue: &std::sync::mpsc::Receiver<(u64, Op)>, intervals: Intervals) {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::mpsc::RecvTimeoutError,
        time::Instant,
    };

    // A bug in one check or request must not end the watching; each is reported instead.
    let poll = |verify: bool| {
        let mut engine = lock(&shared.engine);
        let failed = catch_unwind(AssertUnwindSafe(|| engine.poll(verify))).is_err();
        engine.set_check_failed(failed);
        shared.publish(&mut engine, true);
    };
    poll(true);
    let start = Instant::now();
    let mut next_stat = start + intervals.stat;
    let mut next_verify = start + intervals.verify;
    loop {
        let wait = next_stat.saturating_duration_since(Instant::now());
        match queue.recv_timeout(wait) {
            Ok((id, op)) => {
                let mut engine = lock(&shared.engine);
                if catch_unwind(AssertUnwindSafe(|| engine.apply(id, op))).is_err() {
                    engine.push_result(id, Err(StoreError::Io(Arc::new(std::io::Error::other(REQUEST_FAILED)))));
                }
                shared.publish(&mut engine, true);
            }
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let verify = now >= next_verify;
                if verify {
                    next_verify = now + intervals.verify;
                }
                next_stat = now + intervals.stat;
                poll(verify);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct Shared {
    engine: Mutex<Engine>,
    snapshot: Mutex<Arc<Snapshot>>,
    generation: AtomicU64,
    wake: Wake,
    stopped: AtomicBool,
}

impl Shared {
    /// Publishes a new snapshot when something changed. Called with the engine locked, so
    /// generations never race. `wake` only from the worker.
    fn publish(&self, engine: &mut Engine, wake: bool) {
        if !std::mem::take(&mut engine.changed) {
            return;
        }
        let generation = self.generation.load(Ordering::Acquire).saturating_add(1);
        *lock(&self.snapshot) = Arc::new(engine.snapshot(generation));
        // After the snapshot, so a reader that sees the new generation gets at least its snapshot.
        self.generation.store(generation, Ordering::Release);
        if wake && !self.stopped.load(Ordering::Acquire) {
            // A panic here would end the worker and drop the requests still queued, which then
            // never get a result; the snapshot is out already, so only the repaint is lost.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)()));
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A library file as last read: its stamp and what it gave; `Ok(None)` for a file in another
/// theme format, which the index skips.
struct Cached {
    stamp: FileStamp,
    entry: Result<Option<ThemeEntry>, Problem>,
}

impl Cached {
    /// Read failures other than a file that can't be used as it is are retried: another
    /// process may hold the file for a moment.
    fn retry(&self) -> bool {
        matches!(&self.entry, Err(Problem::Theme { error, .. }) if !is_defect(error))
    }
}

/// A file that won't read differently until it changes: it doesn't parse or is misnamed.
fn is_defect(error: &StoreError) -> bool {
    matches!(error, StoreError::Format(_) | StoreError::FileName { .. })
}

/// `themes` when it names a library theme; built-ins need no file.
pub(crate) fn with_library_theme(themes: Option<ThemePair>) -> Option<ThemePair> {
    themes.filter(|p| !p.light.is_builtin() || !p.dark.is_builtin())
}

fn suite_slots(suite: &SuiteChoice) -> [Option<&ThemeId>; 2] {
    [suite.theme_light.as_ref(), suite.theme_dark.as_ref()]
}

fn pair_slots(pair: Option<&ThemePair>) -> [Option<&ThemeId>; 2] {
    [pair.map(|p| &p.light), pair.map(|p| &p.dark)]
}

/// The library themes in use for one purpose, the suite's or the app's own: one [`Tracked`]
/// per distinct id.
struct Followed {
    themes: Vec<Tracked>,
    /// Keep the last record that parsed when the file goes missing: the suite's themes. The
    /// app's own themes clear instead, so the app falls back to its default.
    keep_missing: bool,
}

impl Followed {
    fn new(keep_missing: bool) -> Self {
        Followed { themes: Vec::new(), keep_missing }
    }

    /// Follows the library themes among `slots`, each once; a theme still in use keeps its
    /// state. Built-ins need no file. Returns whether the ids changed.
    fn follow(&mut self, slots: [Option<&ThemeId>; 2]) -> bool {
        let mut wanted: Vec<&ThemeId> = Vec::new();
        for id in slots.into_iter().flatten().filter(|id| !id.is_builtin()) {
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
        if self.themes.iter().map(|t| &t.id).eq(wanted.iter().copied()) {
            return false;
        }
        let mut old = std::mem::take(&mut self.themes);
        self.themes = wanted
            .into_iter()
            .map(|id| match old.iter().position(|t| t.id == *id) {
                Some(at) => old.swap_remove(at),
                None => Tracked::new(id.clone(), self.keep_missing),
            })
            .collect();
        true
    }

    fn preload(&mut self, store: &mut Store) {
        for tracked in &mut self.themes {
            tracked.preload(store);
        }
    }

    fn offer(&mut self, record: &ThemeRecord, stamp: FileStamp) -> bool {
        self.themes.iter_mut().fold(false, |changed, t| t.offer(record, stamp) | changed)
    }

    fn refresh(&mut self, store: &mut Store) -> bool {
        self.themes.iter_mut().fold(false, |changed, t| t.refresh(store) | changed)
    }

    fn records(&self) -> Vec<ThemeRecord> {
        self.themes.iter().filter_map(|t| t.record.clone()).collect()
    }

    fn problems(&self) -> impl Iterator<Item = &Problem> {
        self.themes.iter().filter_map(|t| t.problem.as_ref())
    }
}

/// A library theme in use, re-read only when its stamp changes. I/O errors are retried on the
/// next check; a file that doesn't parse is read again only once it changes.
struct Tracked {
    id: ThemeId,
    /// The stamp of the last read; `None` with `read` set means the file was missing.
    stamp: Option<FileStamp>,
    read: bool,
    record: Option<ThemeRecord>,
    problem: Option<Problem>,
    /// See [`Followed::keep_missing`].
    keep_missing: bool,
}

impl Tracked {
    fn new(id: ThemeId, keep_missing: bool) -> Self {
        Tracked { id, stamp: None, read: false, record: None, problem: None, keep_missing }
    }

    /// One read at open, before the first snapshot. A failure leaves the slot unread, for the
    /// worker to retry and report.
    fn preload(&mut self, store: &mut Store) {
        self.refresh(store);
        if self.record.is_none() {
            self.read = false;
            self.problem = None;
        }
    }

    /// A record read by the library listing, so the same file isn't parsed twice.
    fn offer(&mut self, record: &ThemeRecord, stamp: FileStamp) -> bool {
        if self.id != record.theme.id || (self.read && self.stamp == Some(stamp)) {
            return false;
        }
        self.stamp = Some(stamp);
        self.read = true;
        self.record = Some(record.clone());
        self.problem = None;
        true
    }

    /// Returns whether anything changed.
    fn refresh(&mut self, store: &mut Store) -> bool {
        let id = self.id.clone();
        let problem = |error: StoreError| Some(Problem::Theme { stem: id.as_str().to_owned(), error });
        let stamp = match store.theme_stamp(&id) {
            Ok(stamp) => stamp,
            Err(e) => {
                self.read = false;
                return set_problem(&mut self.problem, problem(e.into()));
            }
        };
        if self.read && self.stamp == stamp {
            return false;
        }
        if stamp.is_none() {
            self.stamp = None;
            self.read = true;
            let cleared = !self.keep_missing && self.record.take().is_some();
            let reported = set_problem(&mut self.problem, problem(StoreError::NotFound(id.clone())));
            return cleared || reported;
        }
        match store.load_theme(&id) {
            Ok(record) => {
                self.stamp = stamp;
                self.read = true;
                self.record = Some(record);
                self.problem = None;
                true
            }
            Err(e) if is_defect(&e) => {
                self.stamp = stamp;
                self.read = true;
                set_problem(&mut self.problem, problem(e));
                // The file changed even if the problem reads the same.
                true
            }
            Err(e) => {
                self.read = false;
                set_problem(&mut self.problem, problem(e))
            }
        }
    }
}

/// Replaces `slot` and returns whether it changed. Problems carry I/O errors, which can't be
/// compared, so they compare by their text.
fn set_problem(slot: &mut Option<Problem>, next: Option<Problem>) -> bool {
    let same = slot.as_ref().map(ToString::to_string) == next.as_ref().map(ToString::to_string);
    *slot = next;
    !same
}

/// The worker's state. Every method reports a change by setting `changed`.
struct Engine {
    store: Store,
    accepted: Accepted,
    selection_stamp: Option<Option<FileStamp>>,
    selection_problem: Option<Problem>,
    /// Consecutive I/O errors reading `selection.json`.
    selection_failures: u32,
    listed: Option<u64>,
    files: HashMap<String, Cached>,
    index: Arc<[ThemeEntry]>,
    file_problems: Vec<Problem>,
    library_problem: Option<Problem>,
    suite_theme: Followed,
    app_theme: Followed,
    watcher_problem: Option<Problem>,
    results: VecDeque<(u64, Result<OpOutput, StoreError>)>,
    changed: bool,
}

impl Engine {
    fn new(store: Store) -> Self {
        Engine {
            accepted: store.accepted().clone(),
            store,
            selection_stamp: None,
            selection_problem: None,
            selection_failures: 0,
            listed: None,
            files: HashMap::new(),
            index: Arc::from([]),
            file_problems: Vec::new(),
            library_problem: None,
            suite_theme: Followed::new(true),
            app_theme: Followed::new(false),
            watcher_problem: None,
            results: VecDeque::new(),
            changed: false,
        }
    }

    /// Reads `selection.json`, the app's own themes and the suite's themes (the library ones),
    /// so the first snapshot has the real accepted state and changes made in the first
    /// frame merge against it. No library listing: open stays cheap. A failed selection read
    /// keeps the last accepted state; the worker retries it. Returns whether the selection was
    /// read (a missing file counts as read).
    fn preload(&mut self, app_theme: Option<&ThemePair>) -> bool {
        self.check_selection(false);
        let read = self.selection_stamp.is_some();
        self.accepted = self.store.accepted().clone();
        self.app_theme.follow(pair_slots(app_theme));
        self.app_theme.preload(&mut self.store);
        self.suite_theme.follow(suite_slots(&self.accepted.suite));
        self.suite_theme.preload(&mut self.store);
        read
    }

    /// One timer tick: `selection.json` by stamp (by bytes with `verify`), then the library and
    /// the themes in use.
    fn poll(&mut self, verify: bool) {
        self.check_selection(verify);
        self.settle(false);
    }

    fn apply(&mut self, id: u64, op: Op) {
        let result = match op {
            Op::Update { base, change } => self.store.update(&base, change).map(|_| OpOutput::Done),
            Op::Import { bytes, as_copy } => self.store.import(&bytes, as_copy).map(|i| OpOutput::Imported(i.record.theme.id)),
            Op::Delete(theme) => self.store.delete(&theme).map(|_| OpOutput::Done),
            Op::Export(theme) => self.store.export(&theme).map(OpOutput::Exported),
            Op::Rescan => {
                self.check_selection(false);
                self.settle(true);
                return;
            }
            Op::AppTheme(themes) => {
                self.changed |= self.app_theme.follow(pair_slots(themes.as_ref()));
                self.settle(false);
                return;
            }
        };
        self.push_result(id, result);
        // A write leaves the store with its new state; the stamp check picks up the file
        // without parsing it again, and clears a problem the write replaced.
        self.check_selection(false);
        self.settle(false);
    }

    fn push_result(&mut self, id: u64, result: Result<OpOutput, StoreError>) {
        if self.results.len() >= RESULTS_KEPT {
            self.results.pop_front();
        }
        self.results.push_back((id, result));
        self.changed = true;
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn watcher_failed(&mut self, what: &'static str) {
        self.changed |= set_problem(&mut self.watcher_problem, Some(Problem::Watcher(what)));
    }

    /// Reports a check that failed, and clears that report once a check succeeds.
    #[cfg(not(target_arch = "wasm32"))]
    fn set_check_failed(&mut self, failed: bool) {
        if failed {
            self.watcher_failed(CHECK_FAILED);
        } else if matches!(self.watcher_problem, Some(Problem::Watcher(what)) if what == CHECK_FAILED) {
            self.watcher_problem = None;
            self.changed = true;
        }
    }

    fn check_selection(&mut self, verify: bool) {
        let stamp = match self.store.selection_stamp() {
            Ok(stamp) => stamp,
            Err(e) => return self.selection_unreadable(Problem::SelectionUnreadable(Arc::new(e))),
        };
        if !verify && self.selection_stamp == Some(stamp) {
            return;
        }
        self.selection_stamp = Some(stamp);
        match self.store.load_selection() {
            Err(problem @ Problem::SelectionUnreadable(_)) => self.selection_unreadable(problem),
            loaded => {
                self.selection_failures = 0;
                self.changed |= set_problem(&mut self.selection_problem, loaded.err());
            }
        }
    }

    /// An I/O error on `selection.json`: retried on the next check and reported only when that
    /// fails too, since another process may hold the file for a moment.
    fn selection_unreadable(&mut self, problem: Problem) {
        self.selection_stamp = None;
        self.selection_failures = self.selection_failures.saturating_add(1);
        if self.selection_failures > 1 {
            self.changed |= set_problem(&mut self.selection_problem, Some(problem));
        }
    }

    /// Follows the accepted state: the library when its revision moved (or `rescan`), and the
    /// themes in use.
    fn settle(&mut self, rescan: bool) {
        if *self.store.accepted() != self.accepted {
            self.accepted = self.store.accepted().clone();
            self.changed = true;
        }
        self.changed |= self.suite_theme.follow(suite_slots(&self.accepted.suite));
        if rescan || self.listed != Some(self.accepted.library_revision) {
            self.list_library();
        } else {
            self.retry_unread();
        }
        self.changed |= self.suite_theme.refresh(&mut self.store);
        self.changed |= self.app_theme.refresh(&mut self.store);
    }

    fn list_library(&mut self) {
        let files = match self.store.library_files() {
            Ok(files) => files,
            Err(e) => {
                self.changed |= set_problem(&mut self.library_problem, Some(Problem::LibraryUnreadable(Arc::new(e))));
                return;
            }
        };
        self.listed = Some(self.accepted.library_revision);
        self.changed |= set_problem(&mut self.library_problem, None);
        let mut previous = std::mem::take(&mut self.files);
        let mut changed = files.len() != previous.len();
        for file in files {
            let cached = match previous.remove(&file.stem) {
                Some(cached) if cached.stamp == file.stamp && !cached.retry() => cached,
                _ => {
                    changed = true;
                    match self.read_library_file(&file) {
                        Some(cached) => cached,
                        // Removed since the listing.
                        None => continue,
                    }
                }
            };
            self.files.insert(file.stem, cached);
        }
        if changed || !previous.is_empty() {
            self.rebuild_index();
        }
    }

    /// Reads the library files that last failed with an I/O error again.
    fn retry_unread(&mut self) {
        let unread: Vec<LibraryFile> =
            self.files.iter().filter(|(_, c)| c.retry()).map(|(stem, c)| LibraryFile { stem: stem.clone(), stamp: c.stamp }).collect();
        let mut changed = false;
        for file in unread {
            let before = self.files.remove(&file.stem).and_then(|c| c.entry.err()).map(|p| p.to_string());
            match self.read_library_file(&file) {
                Some(cached) => {
                    changed |= cached.entry.as_ref().err().map(ToString::to_string) != before;
                    self.files.insert(file.stem, cached);
                }
                None => changed = true,
            }
        }
        if changed {
            self.rebuild_index();
        }
    }

    fn read_library_file(&mut self, file: &LibraryFile) -> Option<Cached> {
        // A theme in use, read already (at open, or since): not parsed twice.
        for tracked in self.suite_theme.themes.iter().chain(&self.app_theme.themes) {
            if let Some(record) = &tracked.record
                && tracked.read
                && tracked.stamp == Some(file.stamp)
                && record.theme.id.as_str() == file.stem
            {
                return Some(Cached { stamp: file.stamp, entry: Ok(Some(ThemeEntry::new(record, file.stamp))) });
            }
        }
        let entry = match self.store.load_stem(&file.stem) {
            Ok(None) => return None,
            Ok(Some(record)) => {
                self.changed |= self.suite_theme.offer(&record, file.stamp);
                self.changed |= self.app_theme.offer(&record, file.stamp);
                Ok(Some(ThemeEntry::new(&record, file.stamp)))
            }
            Err(error) if is_other_schema(&error) => Ok(None),
            Err(error) => Err(Problem::Theme { stem: file.stem.clone(), error }),
        };
        Some(Cached { stamp: file.stamp, entry })
    }

    fn rebuild_index(&mut self) {
        let mut stems: Vec<&String> = self.files.keys().collect();
        stems.sort();
        let mut entries = Vec::with_capacity(stems.len());
        let mut problems = Vec::new();
        for stem in stems {
            match self.files.get(stem).map(|c| &c.entry) {
                Some(Ok(Some(entry))) => entries.push(entry.clone()),
                Some(Err(problem)) => problems.push(problem.clone()),
                Some(Ok(None)) | None => {}
            }
        }
        self.index = Arc::from(entries);
        self.file_problems = problems;
        self.changed = true;
    }

    fn snapshot(&self, generation: u64) -> Snapshot {
        let mut problems: Vec<Problem> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // A broken theme in use is also a broken library file; list it once.
        let all = [&self.watcher_problem, &self.selection_problem, &self.library_problem]
            .into_iter()
            .flatten()
            .chain(self.suite_theme.problems())
            .chain(self.app_theme.problems())
            .chain(&self.file_problems);
        for problem in all {
            if seen.insert(problem.to_string()) {
                problems.push(problem.clone());
            }
        }
        Snapshot {
            accepted: self.accepted.clone(),
            index: Arc::clone(&self.index),
            active: self.suite_theme.records(),
            app_theme: self.app_theme.records(),
            problems,
            results: self.results.iter().cloned().collect(),
            generation,
        }
    }
}

#[cfg(test)]
mod tests;
