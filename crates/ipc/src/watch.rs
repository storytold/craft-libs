//! Scheduled polling of linked files (`PROTOCOL.md` section 10, rule 4).
//!
//! [`crate::refresh::Tracker`] looks at the disk once per call. A [`Watcher`]
//! owns trackers and calls them on a timer: a shallow poll every
//! [`SHALLOW_INTERVAL`] (1 s) and a deep poll every [`DEEP_INTERVAL`] (30 s).
//! A deep poll replaces the shallow poll that falls on the same tick. Nothing
//! here needs the producer, so it works while the producer is offline.
//!
//! # Time
//!
//! The watcher never reads a clock. A [`Ticker`] blocks until the next tick and
//! returns the time elapsed since it started. Two tickers exist:
//!
//! - [`IntervalTicker`] waits on a condition variable with a timeout. It is the
//!   native ticker. It uses no CPU between ticks and wakes at once on close.
//! - [`ManualTicker`] moves only when [`ManualTicker::advance`] is called. Tests
//!   use it, so they never sleep.
//!
//! # Threads and shutdown
//!
//! [`Watcher::new`] starts one thread. Dropping the watcher closes the ticker
//! and joins the thread. A poll that is running finishes first. A panic in the
//! notice callback ends the thread, and the panic message goes to stderr.
//!
//! # Notices
//!
//! The callback runs on the watcher thread with no lock held, so it may call
//! back into the watcher. It receives [`Notice::Changed`] when a poll bumped the
//! epoch or moved the state (`current`, `modified`, `missing`), and
//! [`Notice::PollFailed`] when a poll hit a read error (rule 6). A repeated
//! identical error is reported once until a poll succeeds. An app turns
//! `Changed` into a `link.changed` event with the new epoch.
//!
//! `std` only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::LinkError;
use crate::refresh::{Change, FileState, Tracker};

/// How often a shallow poll runs (size and mtime, hash only on a difference).
pub const SHALLOW_INTERVAL: Duration = Duration::from_secs(1);
/// How often a deep poll runs (always hash). Covers coarse file system mtime.
pub const DEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Poll periods. [`Intervals::default`] is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intervals {
    pub shallow: Duration,
    pub deep: Duration,
}

impl Default for Intervals {
    fn default() -> Self {
        Self { shallow: SHALLOW_INTERVAL, deep: DEEP_INTERVAL }
    }
}

impl Intervals {
    fn validate(self) -> Result<(), LinkError> {
        if self.shallow.is_zero() {
            return Err(LinkError::Invalid("the shallow interval must be above zero".into()));
        }
        if self.deep < self.shallow {
            return Err(LinkError::Invalid("the deep interval must not be shorter than the shallow interval".into()));
        }
        Ok(())
    }
}

/// A source of ticks for a [`Watcher`].
pub trait Ticker: Send + Sync {
    /// Block until the next tick, then return the time since the ticker started.
    /// The value never decreases. Return `None` after [`Ticker::close`].
    fn wait(&self) -> Option<Duration>;
    /// Wake a blocked [`Ticker::wait`] and make every later call return `None`.
    fn close(&self);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The native ticker: one tick per period on the wall clock.
///
/// A tick that comes late skips the ticks it missed. It does not fire them in a burst.
#[derive(Debug)]
pub struct IntervalTicker {
    start: Instant,
    period: Duration,
    state: Mutex<IntervalState>,
    wake: Condvar,
}

#[derive(Debug)]
struct IntervalState {
    /// The number of the next tick, counted from 1.
    next: u32,
    closed: bool,
}

impl IntervalTicker {
    /// `period` must be above zero.
    pub fn new(period: Duration) -> Result<Self, LinkError> {
        if period.is_zero() {
            return Err(LinkError::Invalid("the tick period must be above zero".into()));
        }
        Ok(Self { start: Instant::now(), period, state: Mutex::new(IntervalState { next: 1, closed: false }), wake: Condvar::new() })
    }
}

impl Ticker for IntervalTicker {
    fn wait(&self) -> Option<Duration> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return None;
            }
            let deadline = self.start + self.period.saturating_mul(state.next);
            let now = Instant::now();
            if now >= deadline {
                let elapsed = now.duration_since(self.start);
                let passed = elapsed.as_nanos() / self.period.as_nanos().max(1);
                state.next = u32::try_from(passed.saturating_add(1)).unwrap_or(u32::MAX);
                return Some(elapsed);
            }
            state = self.wake.wait_timeout(state, deadline - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    fn close(&self) {
        lock(&self.state).closed = true;
        self.wake.notify_all();
    }
}

/// A ticker that moves only when told. For tests and for hosts with their own clock.
#[derive(Debug, Default)]
pub struct ManualTicker {
    state: Mutex<ManualState>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct ManualState {
    now: Duration,
    delivered: Duration,
    /// The watcher thread is blocked in `wait` and has handled every earlier tick.
    idle: bool,
    closed: bool,
}

impl ManualTicker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Move time forward by `by` and wake the watcher. The watcher sees one tick
    /// at the new time, however large `by` is.
    pub fn advance(&self, by: Duration) {
        let mut state = lock(&self.state);
        let moved = state.now.saturating_add(by);
        if moved > state.now {
            state.now = moved;
            state.idle = false;
        }
        drop(state);
        self.wake.notify_all();
    }

    /// Block until the watcher has handled every tick so far and waits again.
    /// Returns `false` when `limit` passes first or the ticker is closed. The
    /// limit only guards a test from hanging. A healthy watcher settles at once.
    pub fn settle(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return false;
            }
            if state.idle && state.delivered == state.now {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self.wake.wait_timeout(state, deadline - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }
}

impl Ticker for ManualTicker {
    fn wait(&self) -> Option<Duration> {
        let mut state = lock(&self.state);
        state.idle = state.delivered == state.now;
        self.wake.notify_all();
        loop {
            if state.closed {
                return None;
            }
            if state.now > state.delivered {
                state.delivered = state.now;
                state.idle = false;
                return Some(state.now);
            }
            state = self.wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn close(&self) {
        lock(&self.state).closed = true;
        self.wake.notify_all();
    }
}

/// What a [`Watcher`] tells its callback.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// A poll bumped the epoch or moved the state of `link_id`.
    Changed { link_id: String, change: Change, deep: bool },
    /// A poll could not read the file. The state did not change.
    PollFailed { link_id: String, error: LinkError },
}

/// How many polls the watcher thread has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollCounts {
    pub shallow: u64,
    pub deep: u64,
}

type Callback = Box<dyn Fn(Notice) + Send + Sync>;

struct Entry {
    id: String,
    state: Mutex<EntryState>,
}

struct EntryState {
    /// `None` after `unwatch`, for a poll that was already under way.
    tracker: Option<Tracker>,
    /// The state the callback last heard about.
    reported: FileState,
    last_error: Option<LinkError>,
}

struct Shared {
    entries: Mutex<Vec<Arc<Entry>>>,
    notify: Callback,
    shallow_polls: AtomicU64,
    deep_polls: AtomicU64,
}

/// Polls a set of trackers on a schedule. See the module docs.
pub struct Watcher {
    shared: Arc<Shared>,
    ticker: Arc<dyn Ticker>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Watch on the wall clock with the contract intervals.
    pub fn start(notify: impl Fn(Notice) + Send + Sync + 'static) -> Result<Self, LinkError> {
        let ticker = Arc::new(IntervalTicker::new(SHALLOW_INTERVAL)?);
        Self::new(ticker, Intervals::default(), notify)
    }

    /// Watch with a chosen ticker. The ticker should tick at least as often as
    /// `intervals.shallow`, or polls run at the ticker rate.
    pub fn new(ticker: Arc<dyn Ticker>, intervals: Intervals, notify: impl Fn(Notice) + Send + Sync + 'static) -> Result<Self, LinkError> {
        intervals.validate()?;
        let shared = Arc::new(Shared {
            entries: Mutex::new(Vec::new()),
            notify: Box::new(notify),
            shallow_polls: AtomicU64::new(0),
            deep_polls: AtomicU64::new(0),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            let ticker = Arc::clone(&ticker);
            std::thread::Builder::new().name("craft-ipc-watch".into()).spawn(move || run(&shared, ticker.as_ref(), intervals))?
        };
        Ok(Self { shared, ticker, thread: Some(thread) })
    }

    /// Start polling `tracker` under `link_id`. A second tracker for the same id is an error.
    pub fn watch(&self, link_id: impl Into<String>, tracker: Tracker) -> Result<(), LinkError> {
        let id = link_id.into();
        let mut entries = lock(&self.shared.entries);
        if entries.iter().any(|entry| entry.id == id) {
            return Err(LinkError::Invalid(format!("{id} is already watched")));
        }
        let reported = tracker.state();
        entries.push(Arc::new(Entry { id, state: Mutex::new(EntryState { tracker: Some(tracker), reported, last_error: None }) }));
        Ok(())
    }

    /// Stop polling `link_id` and hand its tracker back.
    pub fn unwatch(&self, link_id: &str) -> Option<Tracker> {
        let entry = {
            let mut entries = lock(&self.shared.entries);
            let at = entries.iter().position(|entry| entry.id == link_id)?;
            entries.remove(at)
        };
        // The thread may hold a clone of the entry for one more poll, which skips it now.
        lock(&entry.state).tracker.take()
    }

    /// Run `act` on the tracker of `link_id`: `accept`, `relink`, `adopt_epoch`
    /// or a read. `None` when the id is not watched. The callback does not hear
    /// about the state that `act` leaves behind.
    pub fn with_tracker<R>(&self, link_id: &str, act: impl FnOnce(&mut Tracker) -> R) -> Option<R> {
        let entry = lock(&self.shared.entries).iter().find(|entry| entry.id == link_id).cloned()?;
        let mut state = lock(&entry.state);
        let tracker = state.tracker.as_mut()?;
        let out = act(tracker);
        let now = tracker.state();
        state.reported = now;
        Some(out)
    }

    pub fn poll_counts(&self) -> PollCounts {
        PollCounts { shallow: self.shared.shallow_polls.load(Ordering::SeqCst), deep: self.shared.deep_polls.load(Ordering::SeqCst) }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.ticker.close();
        if let Some(thread) = self.thread.take() {
            // An Err means the callback panicked. The panic message is already on stderr.
            let _ = thread.join();
        }
    }
}

/// Next due time on the grid `first, first + step, ...` that is later than `now`.
fn next_after(due: u128, step: u128, now: u128) -> u128 {
    if due > now { due } else { due + ((now - due) / step + 1) * step }
}

fn run(shared: &Shared, ticker: &dyn Ticker, intervals: Intervals) {
    let shallow = intervals.shallow.as_nanos();
    let deep = intervals.deep.as_nanos();
    let mut due_shallow = shallow;
    let mut due_deep = deep;
    while let Some(elapsed) = ticker.wait() {
        let now = elapsed.as_nanos();
        if now >= due_deep {
            due_deep = next_after(due_deep, deep, now);
            due_shallow = next_after(due_shallow, shallow, now);
            shared.deep_polls.fetch_add(1, Ordering::SeqCst);
            poll_all(shared, true);
        } else if now >= due_shallow {
            due_shallow = next_after(due_shallow, shallow, now);
            shared.shallow_polls.fetch_add(1, Ordering::SeqCst);
            poll_all(shared, false);
        }
    }
}

fn poll_all(shared: &Shared, deep: bool) {
    let entries: Vec<Arc<Entry>> = lock(&shared.entries).clone();
    for entry in entries {
        let notice = poll_entry(&entry, deep);
        if let Some(notice) = notice {
            (shared.notify)(notice);
        }
    }
}

fn poll_entry(entry: &Entry, deep: bool) -> Option<Notice> {
    let mut state = lock(&entry.state);
    let polled = state.tracker.as_mut()?.poll(deep);
    match polled {
        Ok(change) => {
            state.last_error = None;
            if change.bumped || change.state != state.reported {
                state.reported = change.state;
                Some(Notice::Changed { link_id: entry.id.clone(), change, deep })
            } else {
                None
            }
        }
        Err(error) => {
            if state.last_error.as_ref() == Some(&error) {
                return None;
            }
            state.last_error = Some(error.clone());
            Some(Notice::PollFailed { link_id: entry.id.clone(), error })
        }
    }
}
