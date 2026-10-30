//! Pull frames from a peer without blocking the caller past a deadline.
//!
//! [`FrameFetcher`] owns the control connection that holds the `frames.open` ticket and a few
//! bulk connections ([`crate::bulk::BulkClient`]), each on its own worker thread. A caller asks
//! with [`FrameFetcher::fetch`] and a time limit.
//!
//! - A frame in the RAM cache answers at once. The cache keeps the newest epoch of each
//!   [`FrameKey`] inside a byte budget and drops the least recently used frame first.
//! - A miss goes to a worker. Asks for one key share one render.
//! - A caller that runs out of time gets [`Fetched::TimedOut`]. The render goes on, and its frame
//!   lands in the cache for the next ask. The caller never waits for the socket.
//! - A queued ask older than [`FetchOptions::max_age`] is dropped when a worker reaches it,
//!   because the caller has moved on.
//! - A connection error ends that worker. When the last worker ends the fetcher is down and every
//!   ask that misses the cache returns [`Fetched::Down`] at once.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::InstanceRecord;
use crate::bulk::{BulkClient, Frame, FrameFormat, FrameTicket};
use crate::client::Client;
use crate::error::LinkError;

/// Most bulk connections one fetcher opens.
pub const CONNECTIONS_MAX: usize = 8;

/// What identifies a frame for the cache: the caller's frame number and scale (in thousandths).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub frame: i64,
    pub scale_milli: u32,
}

/// Tuning of a [`FrameFetcher`].
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Bulk connections, and so renders in flight. Clamped to `1..=`[`CONNECTIONS_MAX`].
    pub connections: usize,
    /// How long one socket read or write may take before the connection is dropped.
    pub io_timeout: Duration,
    /// Cache size in bytes of pixel data.
    pub cache_bytes: usize,
    /// A queued ask older than this is skipped.
    pub max_age: Duration,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self { connections: 3, io_timeout: Duration::from_secs(5), cache_bytes: 256 * 1024 * 1024, max_age: Duration::from_secs(1) }
    }
}

/// The answer to one [`FrameFetcher::fetch`].
#[derive(Debug, Clone)]
pub enum Fetched {
    /// A frame whose epoch is at least the one asked for.
    Frame(Arc<Frame>),
    /// The time limit passed. The render may still finish and fill the cache.
    TimedOut,
    /// The peer answered this frame with an error. The fetcher still works.
    Failed(String),
    /// The connections are gone and the cache has no answer.
    Down,
}

/// Counters since [`FrameFetcher::connect`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchStats {
    /// Calls to [`FrameFetcher::fetch`].
    pub requests: u64,
    /// Calls answered by the cache on the first look.
    pub hits: u64,
    /// Frames a worker received.
    pub renders: u64,
    pub timeouts: u64,
    pub failures: u64,
    /// Queued asks dropped for age.
    pub skipped: u64,
}

struct Job {
    key: FrameKey,
    time_s: f64,
    epoch: u64,
    queued: Instant,
}

struct Cached {
    frame: Arc<Frame>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    frames: HashMap<FrameKey, Cached>,
    bytes: usize,
    tick: u64,
}

impl Cache {
    fn get(&mut self, key: FrameKey, min_epoch: u64) -> Option<Arc<Frame>> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.frames.get_mut(&key).filter(|entry| entry.frame.epoch >= min_epoch)?;
        entry.used = tick;
        Some(Arc::clone(&entry.frame))
    }

    fn insert(&mut self, key: FrameKey, frame: Arc<Frame>, budget: usize) {
        if self.frames.get(&key).is_some_and(|old| old.frame.epoch > frame.epoch) {
            return;
        }
        self.tick += 1;
        self.bytes += frame.pixels.len();
        if let Some(old) = self.frames.insert(key, Cached { frame, used: self.tick }) {
            self.bytes -= old.frame.pixels.len();
        }
        while self.bytes > budget && self.frames.len() > 1 {
            let Some(oldest) = self.frames.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| *key) else { break };
            if let Some(gone) = self.frames.remove(&oldest) {
                self.bytes -= gone.frame.pixels.len();
            }
        }
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    inflight: HashSet<FrameKey>,
    cache: Cache,
    failed: HashMap<FrameKey, (u64, String)>,
    fail_seq: u64,
    stats: FetchStats,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Workers wait here for a job.
    work: Condvar,
    /// Callers wait here for a frame, a failure or the end.
    done: Condvar,
    alive: AtomicUsize,
    options: FetchOptions,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Frames pulled from one link of a peer. See the module docs.
pub struct FrameFetcher {
    shared: Arc<Shared>,
    ticket: FrameTicket,
    /// Holds the ticket: the peer drops it when this connection closes.
    control: Mutex<Client>,
}

impl FrameFetcher {
    /// Open the ticket and the bulk connections. Any failure is returned here, so a caller knows
    /// the peer can serve frames before it relies on them.
    pub fn connect(record: &InstanceRecord, link_id: &str, format: FrameFormat, scale: f64, options: FetchOptions) -> Result<Self, LinkError> {
        let mut control = Client::connect(record)?;
        let ticket = control.frames_open(link_id, format, scale)?;
        let connections = options.connections.clamp(1, CONNECTIONS_MAX);
        let mut clients = Vec::with_capacity(connections);
        for _ in 0..connections {
            let mut client = BulkClient::attach(record, &ticket.ticket)?;
            client.set_io_timeout(options.io_timeout);
            clients.push(client);
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            work: Condvar::new(),
            done: Condvar::new(),
            alive: AtomicUsize::new(clients.len()),
            options,
        });
        for client in clients {
            let worker = Arc::clone(&shared);
            // Workers are not joined: one may sit in a socket read for up to `io_timeout`, and a
            // drop on a UI thread must not wait for that. A worker sees `stop` and leaves.
            if let Err(err) = std::thread::Builder::new().name("craft-ipc-fetch".into()).spawn(move || run_worker(&worker, client)) {
                shared_stop(&shared);
                return Err(err.into());
            }
        }
        Ok(Self { shared, ticket, control: Mutex::new(control) })
    }

    /// Size of the frames this fetcher yields.
    pub fn layout(&self) -> FrameTicket {
        self.ticket.clone()
    }

    /// The cached frame for `key` if its epoch is at least `min_epoch`. Never asks the peer.
    pub fn cached(&self, key: FrameKey, min_epoch: u64) -> Option<Arc<Frame>> {
        self.shared.lock().cache.get(key, min_epoch)
    }

    /// The frame for `key` at an epoch of at least `min_epoch`, waiting at most `wait`.
    /// `time_s` is the comp time the peer renders.
    pub fn fetch(&self, key: FrameKey, time_s: f64, min_epoch: u64, wait: Duration) -> Fetched {
        let deadline = Instant::now() + wait;
        let shared = &self.shared;
        let mut state = shared.lock();
        state.stats.requests += 1;
        let started_at = state.fail_seq;
        let mut first = true;
        loop {
            if let Some(frame) = state.cache.get(key, min_epoch) {
                if first {
                    state.stats.hits += 1;
                }
                return Fetched::Frame(frame);
            }
            first = false;
            if let Some((seq, message)) = state.failed.get(&key)
                && *seq > started_at
            {
                return Fetched::Failed(message.clone());
            }
            if shared.alive.load(Ordering::SeqCst) == 0 || state.stop {
                return Fetched::Down;
            }
            if !state.inflight.contains(&key) {
                state.inflight.insert(key);
                state.queue.push_back(Job { key, time_s, epoch: min_epoch, queued: Instant::now() });
                shared.work.notify_one();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                state.stats.timeouts += 1;
                return Fetched::TimedOut;
            }
            state = shared.done.wait_timeout(state, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// True once every connection has ended.
    pub fn is_down(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst) == 0
    }

    pub fn stats(&self) -> FetchStats {
        self.shared.lock().stats
    }

    /// Ask the peer to close the ticket now instead of at drop.
    pub fn close(&self) {
        shared_stop(&self.shared);
        let mut control = self.control.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = control.call("link.bye", serde_json::json!({}));
    }
}

impl Drop for FrameFetcher {
    fn drop(&mut self) {
        shared_stop(&self.shared);
    }
}

fn shared_stop(shared: &Shared) {
    shared.lock().stop = true;
    shared.work.notify_all();
    shared.done.notify_all();
}

fn run_worker(shared: &Shared, mut client: BulkClient) {
    while let Some(job) = next_job(shared) {
        let reply = client.request_reply(job.time_s, job.epoch, None);
        let mut state = shared.lock();
        state.inflight.remove(&job.key);
        match reply {
            Ok(Ok(frame)) => {
                state.stats.renders += 1;
                state.failed.remove(&job.key);
                let budget = shared.options.cache_bytes;
                state.cache.insert(job.key, Arc::new(frame), budget);
            }
            Ok(Err(message)) => {
                state.stats.failures += 1;
                state.fail_seq += 1;
                let seq = state.fail_seq;
                state.failed.insert(job.key, (seq, message));
            }
            Err(_) => {
                drop(state);
                break;
            }
        }
        drop(state);
        shared.done.notify_all();
    }
    if shared.alive.fetch_sub(1, Ordering::SeqCst) == 1 {
        let mut state = shared.lock();
        state.queue.clear();
        state.inflight.clear();
    }
    shared.done.notify_all();
}

/// The oldest queued job that is still worth rendering. `None` when the fetcher stops.
fn next_job(shared: &Shared) -> Option<Job> {
    let mut state = shared.lock();
    loop {
        if state.stop {
            return None;
        }
        while let Some(job) = state.queue.pop_front() {
            if job.queued.elapsed() > shared.options.max_age {
                state.inflight.remove(&job.key);
                state.stats.skipped += 1;
                shared.done.notify_all();
                continue;
            }
            return Some(job);
        }
        state = shared.work.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}
