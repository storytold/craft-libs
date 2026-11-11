//! [`JobQueue`]: background work with priorities, one job per key, replacement and cancellation,
//! on top of the host's [`Jobs`].
//!
//! Every library feature whose background work needs more than [`Jobs::spawn`] uses it:
//! thumbnails, file thumbnails, link checks, folder listings, the relink search. No crate spawns
//! threads itself; a queue's lanes are jobs on the app's [`Jobs`], so `InlineJobs`, `ManualJobs`
//! and a test's own `Jobs` need no special case.
//!
//! ```ignore
//! let mut queue = JobQueue::new(host.jobs_shared().clone(), Wake::new(move || ctx.request_repaint()), QueueOptions::default());
//! queue.submit(photo_id, Priority::VISIBLE, move |cancel| decode(&path, cancel));
//! // In a later pass, after the wake:
//! while let Some(Finished { key, result }) = queue.try_recv() { show(key, result) }
//! ```
//!
//! On wasm32 there are no threads: [`QueueOptions::default_lanes`] is 0 and the owner calls
//! [`JobQueue::run_inline`] once per pass.

use std::collections::VecDeque;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::Duration;
use web_time::Instant;

use crate::{Jobs, Wake};

/// How urgent a queued job is: higher runs first. Widgets ask with the three named values; an app
/// may use any other (FilmCraft's frame requests at 50 sit between [`BACKGROUND`](Self::BACKGROUND)
/// and [`NEAR`](Self::NEAR)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(pub u32);

impl Priority {
    /// What is on screen now: a row or tile in view.
    pub const VISIBLE: Priority = Priority(300);
    /// What is about to be: one row or one viewport beyond the visible ones.
    pub const NEAR: Priority = Priority(200);
    /// Everything else (prefetch, a whole folder's thumbnails).
    pub const BACKGROUND: Priority = Priority(100);
}

/// The cancel flag a running job reads. The queue sets it when the job's key is dropped
/// ([`JobQueue::reprioritize`] returning `None`, [`JobQueue::shutdown`], dropping the queue); a
/// long job checks [`is_cancelled`](Cancel::is_cancelled) between steps and returns early. A
/// cancelled job's result is never delivered. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag that is not set (tests call a decoder with it).
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the job should stop.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Sets the flag.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A queue's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueOptions {
    /// How many jobs run at once. 0: none runs until [`JobQueue::run_inline`] runs it (wasm32,
    /// tests).
    pub lanes: usize,
    /// The name each lane's job is spawned with (a thread name with `ThreadJobs`).
    pub name: &'static str,
}

impl QueueOptions {
    /// Lanes for this machine: the number of cores less one, at least 2 and at most 8 (4 when
    /// the core count is unknown); 0 on wasm32.
    // lightcraft: crates/preview/src/pool.rs:104 `default_threads` (cores − 1 clamped to 2..=8;
    // ours is 0 on wasm32 instead of 1, because wasm runs jobs only through `run_inline`).
    pub fn default_lanes() -> usize {
        #[cfg(target_arch = "wasm32")]
        return 0;
        #[cfg(not(target_arch = "wasm32"))]
        thread::available_parallelism().map_or(4, |n| n.get().saturating_sub(1).clamp(2, 8))
    }
}

impl Default for QueueOptions {
    /// [`default_lanes`](QueueOptions::default_lanes) lanes named `"craft-queue"`.
    fn default() -> Self {
        // new: the default every library queue starts from.
        Self { lanes: Self::default_lanes(), name: "craft-queue" }
    }
}

/// A job's result, from [`JobQueue::try_recv`].
#[derive(Debug)]
#[non_exhaustive]
pub struct Finished<K, R> {
    /// The key the job was submitted with.
    pub key: K,
    /// What the job returned.
    pub result: R,
}

/// How much work a queue holds, for tests, stories and automation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueCounts {
    /// Jobs waiting to start.
    pub queued: usize,
    /// Jobs running now.
    pub running: usize,
    /// Results waiting for [`JobQueue::try_recv`].
    pub finished: usize,
}

type QueueJob<R> = Box<dyn FnOnce(&Cancel) -> R + Send>;

/// One waiting job.
struct Queued<K, R> {
    key: K,
    priority: Priority,
    /// Submission order: among equal priorities the newest runs first.
    seq: u64,
    job: QueueJob<R>,
}

/// A job that has started.
struct Running<K> {
    /// The submission number of the job, to find it again when it ends.
    id: u64,
    key: K,
    priority: Priority,
    cancel: Cancel,
    /// The thread running it, so `shutdown` never waits for the caller's own job.
    thread: ThreadId,
}

/// A job taken from the queue, about to run.
struct Started<R> {
    id: u64,
    cancel: Cancel,
    job: QueueJob<R>,
}

/// State shared with the lanes, behind one lock.
struct State<K, R> {
    queued: Vec<Queued<K, R>>,
    /// Jobs running now: their keys' next jobs wait until they end.
    running: Vec<Running<K>>,
    finished: VecDeque<Finished<K, R>>,
    /// Lanes alive now (at most `lanes`).
    lanes_alive: usize,
    seq: u64,
    stopped: bool,
}

struct Shared<K, R> {
    state: Mutex<State<K, R>>,
    /// Signalled when a running job ends, for [`JobQueue::shutdown`].
    ended: Condvar,
    jobs: Arc<dyn Jobs>,
    wake: Wake,
    options: QueueOptions,
}

impl<K: Clone + Eq, R> Shared<K, R> {
    fn lock(&self) -> MutexGuard<'_, State<K, R>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<K, R> State<K, R> {
    /// Drops the waiting jobs and cancels the running ones.
    fn stop(&mut self) {
        self.stopped = true;
        self.queued.clear();
        for r in &self.running {
            r.cancel.cancel();
        }
    }
}

impl<K: Clone + Eq, R> State<K, R> {
    /// Index of the best waiting job whose key is not running: highest priority, then newest.
    fn best(&self) -> Option<usize> {
        self.queued
            .iter()
            .enumerate()
            .filter(|(_, q)| !self.running.iter().any(|r| r.key == q.key))
            .max_by_key(|(_, q)| (q.priority, q.seq))
            .map(|(i, _)| i)
    }

    /// Moves the best startable job to `running` and returns it.
    fn start_best(&mut self) -> Option<Started<R>> {
        let queued = self.queued.swap_remove(self.best()?);
        let cancel = Cancel::new();
        self.running.push(Running {
            id: queued.seq,
            key: queued.key,
            priority: queued.priority,
            cancel: cancel.clone(),
            thread: thread::current().id(),
        });
        Some(Started { id: queued.seq, cancel, job: queued.job })
    }
}

/// Holds a lane's claim on `lanes_alive` until the lane starts; dropped unrun, it releases the
/// claim.
struct LaneGuard<K, R>(Option<Arc<Shared<K, R>>>);

impl<K, R> Drop for LaneGuard<K, R> {
    fn drop(&mut self) {
        if let Some(shared) = self.0.take() {
            shared.state.lock().unwrap_or_else(|e| e.into_inner()).lanes_alive -= 1;
        }
    }
}

impl<K: Clone + Eq + Send + 'static, R: Send + 'static> Shared<K, R> {
    /// Spawns a lane through [`Jobs::spawn`]; the caller has counted it in `lanes_alive`. When
    /// the host drops the closure unrun (a thread that could not start, a dropped `ManualJobs`),
    /// the guard gives the count back.
    fn spawn_lane(self: &Arc<Self>) {
        let mut guard = LaneGuard(Some(Arc::clone(self)));
        self.jobs.spawn(
            self.options.name,
            Box::new(move || {
                if let Some(shared) = guard.0.take() {
                    shared.lane();
                }
            }),
        );
    }

    /// Spawns a lane when fewer than `lanes` are alive and a job can start.
    fn spawn_lane_if_free(self: &Arc<Self>) {
        let spawn = {
            let mut state = self.lock();
            let spawn = !state.stopped && state.lanes_alive < self.options.lanes && state.best().is_some();
            if spawn {
                state.lanes_alive += 1;
            }
            spawn
        };
        if spawn {
            self.spawn_lane();
        }
    }

    /// One run of a lane: one job, then the next run when work waits.
    fn lane(self: Arc<Self>) {
        let started = {
            let mut state = self.lock();
            let started = if state.stopped { None } else { state.start_best() };
            if started.is_none() {
                state.lanes_alive -= 1;
            }
            started
        };
        let Some(started) = started else { return };
        if self.run(started, true) {
            self.spawn_lane();
        }
    }

    /// Runs a started job on this thread and files its result. With `from_lane`, returns whether
    /// the lane continues; otherwise the lane is given up here.
    fn run(&self, started: Started<R>, from_lane: bool) -> bool {
        let Started { id, cancel, job } = started;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&cancel))).ok();
        let (delivered, again) = {
            let mut state = self.lock();
            let pos = state.running.iter().position(|r| r.id == id);
            let running = pos.map(|i| state.running.swap_remove(i));
            let mut delivered = false;
            if let (Some(running), Some(result)) = (running, result)
                && !running.cancel.is_cancelled()
                && !state.stopped
            {
                state.finished.push_back(Finished { key: running.key, result });
                delivered = true;
            }
            let again = from_lane && !state.stopped && state.best().is_some();
            if from_lane && !again {
                state.lanes_alive -= 1;
            }
            (delivered, again)
        };
        self.ended.notify_all();
        if delivered {
            self.wake.wake();
        }
        again
    }
}

/// A priority queue of background jobs, after LightCraft's `JobPool`.
///
/// - **Keys.** Each job has a key (a thumbnail's id, a folder's path). [`submit`](Self::submit)
///   replaces a queued job of the same key, so scrolling or a slider drag never builds a backlog.
///   A key runs one job at a time: a key's next job waits until its running one ends (overlapping
///   renders of one picture are mostly thrown away).
/// - **Order.** Highest [`Priority`] first; among equal priorities the newest first.
///   [`reprioritize`](Self::reprioritize) re-ranks the waiting jobs or drops them; dropping a key
///   whose job runs sets its [`Cancel`] and its result is never delivered.
/// - **Lanes.** At most [`QueueOptions::lanes`] jobs run at once, each started through the
///   host's [`Jobs::spawn`]. A lane runs one job per spawn and, when it ends and work is waiting,
///   spawns the next run; it ends when the queue is empty. So with `ThreadJobs` each job gets a
///   thread, nothing idles, and a `ManualJobs` run finishes exactly one job.
/// - **Results.** A finished job's result waits for [`try_recv`](Self::try_recv), in the order
///   jobs finished, and the queue calls its [`Wake`] once per finished job; the owner reads the
///   results in its next pass. Nothing polls on a timer.
/// - **Without lanes** (`lanes == 0`, wasm32 and tests) no job runs until
///   [`run_inline`](Self::run_inline) runs some on the calling thread.
/// - **End.** [`shutdown`](Self::shutdown) drops the waiting jobs, cancels the running ones and
///   waits, bounded, for them to end (a render still inside a GPU driver while the process exits
///   crashes there: storytold/lightcraft#620). Dropping the queue drops and cancels the same way without
///   waiting.
///
/// `JobQueue<K, R>` is `Send + Sync` when `K` and `R` are `Send`, so it can live in egui memory or
/// an `Arc` shared by several widgets.
// lightcraft: crates/preview/src/pool.rs:1-140 (`JobPool`: slot replace, one job per slot, priority
// then newest first, `reprioritize`, `run_inline`, bounded `shutdown`).
pub struct JobQueue<K, R> {
    shared: Arc<Shared<K, R>>,
}

const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    send_sync::<JobQueue<u64, Vec<u8>>>();
    send_sync::<Cancel>();
};

impl<K: Clone + Eq + Hash + Send + 'static, R: Send + 'static> JobQueue<K, R> {
    /// A queue whose lanes run on `jobs` (the host's: `Host::jobs_shared`) and that calls `wake`
    /// after each finished job. No lane starts before the first [`submit`](Self::submit).
    pub fn new(jobs: Arc<dyn Jobs>, wake: Wake, options: QueueOptions) -> Self {
        // new: lanes are spawned on demand by `submit` and by a lane that ends with work waiting.
        let state = State { queued: Vec::new(), running: Vec::new(), finished: VecDeque::new(), lanes_alive: 0, seq: 0, stopped: false };
        Self { shared: Arc::new(Shared { state: Mutex::new(state), ended: Condvar::new(), jobs, wake, options }) }
    }

    /// Queues `job` for `key` at `priority`, replacing a waiting job of the same key (the replaced
    /// one is dropped unrun). When fewer than `lanes` lanes are alive, a lane is spawned through
    /// [`Jobs::spawn`] with [`QueueOptions::name`] (a spawn that drops the lane unrun gives its
    /// place back, so the next submit tries again). A job for a key that is running waits until
    /// that one ends. After [`shutdown`](Self::shutdown) the job is dropped.
    // lightcraft: crates/preview/src/pool.rs:182 `submit`.
    pub fn submit(&self, key: K, priority: Priority, job: impl FnOnce(&Cancel) -> R + Send + 'static) {
        {
            let mut state = self.shared.lock();
            if state.stopped {
                return;
            }
            state.queued.retain(|q| q.key != key);
            state.seq += 1;
            let seq = state.seq;
            state.queued.push(Queued { key, priority, seq, job: Box::new(job) });
        }
        self.shared.spawn_lane_if_free();
    }

    /// Re-ranks the waiting jobs and the running ones: `f(key, priority)` returns the new priority,
    /// or `None` to drop the key. A dropped waiting job never runs; a dropped running job gets its
    /// [`Cancel`] set and its result is discarded. Returns the dropped keys, waiting ones first.
    /// Running jobs keep running at any priority they are given (the priority only orders the
    /// waiting ones).
    // lightcraft: crates/preview/src/pool.rs:197 `reprioritize` (waiting jobs only; ours also
    // cancels running ones, so a thumbnail scrolled away stops decoding).
    pub fn reprioritize(&self, mut f: impl FnMut(&K, Priority) -> Option<Priority>) -> Vec<K> {
        let mut state = self.shared.lock();
        let mut dropped = Vec::new();
        let mut kept = Vec::with_capacity(state.queued.len());
        for mut q in std::mem::take(&mut state.queued) {
            match f(&q.key, q.priority) {
                Some(priority) => {
                    q.priority = priority;
                    kept.push(q);
                }
                None => dropped.push(q.key),
            }
        }
        state.queued = kept;
        for r in &mut state.running {
            if r.cancel.is_cancelled() {
                continue;
            }
            match f(&r.key, r.priority) {
                Some(priority) => r.priority = priority,
                None => {
                    r.cancel.cancel();
                    dropped.push(r.key.clone());
                }
            }
        }
        dropped
    }

    /// The oldest result not yet taken, if any. Results of cancelled jobs are never here.
    // lightcraft: crates/preview/src/pool.rs:224 `try_recv`.
    pub fn try_recv(&self) -> Option<Finished<K, R>> {
        self.shared.lock().finished.pop_front()
    }

    /// Runs up to `max_jobs` waiting jobs, best first, on the calling thread, and returns how
    /// many ran. Each result waits for [`try_recv`](Self::try_recv) and calls the wake, as from a
    /// lane. Used when `lanes == 0` (wasm32: the owner runs one per pass); with lanes it runs jobs
    /// beside them, still one per key.
    // lightcraft: crates/preview/src/pool.rs:237 `run_inline`.
    pub fn run_inline(&self, max_jobs: usize) -> usize {
        let mut ran = 0;
        while ran < max_jobs {
            let started = {
                let mut state = self.shared.lock();
                if state.stopped {
                    break;
                }
                state.start_best()
            };
            let Some(started) = started else { break };
            self.shared.run(started, false);
            ran += 1;
        }
        // The last inline job may have been the only thing keeping a waiting job of its key from
        // starting: give that job a lane.
        self.shared.spawn_lane_if_free();
        ran
    }

    /// Gives waiting work a lane when none is alive: a lane the host dropped unrun (a refused spawn) left jobs
    /// waiting that no later [`submit`](Self::submit) may come to start. An owner that keeps polling a
    /// waiting job calls it each pass; it does nothing while a lane is alive or nothing waits.
    pub fn resume(&self) {
        let spawn = {
            let mut state = self.shared.lock();
            let spawn = !state.stopped && state.lanes_alive == 0 && self.shared.options.lanes > 0 && state.best().is_some();
            if spawn {
                state.lanes_alive += 1;
            }
            spawn
        };
        if spawn {
            self.shared.spawn_lane();
        }
    }

    /// Whether a job for `key` is waiting or running.
    pub fn contains(&self, key: &K) -> bool {
        let state = self.shared.lock();
        state.queued.iter().any(|q| q.key == *key) || state.running.iter().any(|r| r.key == *key)
    }

    /// Jobs waiting, running and finished.
    pub fn counts(&self) -> QueueCounts {
        let state = self.shared.lock();
        QueueCounts { queued: state.queued.len(), running: state.running.len(), finished: state.finished.len() }
    }

    /// Stops the queue: waiting jobs are dropped, running ones are cancelled, no new job starts,
    /// and the call waits up to `timeout` for the running ones to end. Returns whether every one
    /// has; one that hasn't is left to end on its own and its result is discarded. For owners about
    /// to end the process (`std::process::exit` never drops the queue). Never waits for a job
    /// running on the calling thread (a queue shut down by one of its own jobs).
    // lightcraft: crates/preview/src/pool.rs:155 `shutdown` (storytold/lightcraft#620).
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let me = thread::current().id();
        let deadline = Instant::now().checked_add(timeout);
        let mut state = self.shared.lock();
        state.stop();
        loop {
            if !state.running.iter().any(|r| r.thread != me) {
                return true;
            }
            let left = match deadline {
                Some(deadline) => deadline.saturating_duration_since(Instant::now()),
                None => timeout,
            };
            if left.is_zero() {
                return false;
            }
            state = self.shared.ended.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl<K, R> Drop for JobQueue<K, R> {
    /// Drops the waiting jobs and cancels the running ones, without waiting.
    fn drop(&mut self) {
        // new: dropping never blocks the UI thread; `shutdown` is the waiting form.
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).stop();
    }
}
