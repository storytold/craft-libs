//! Document change feed. One bus, many subscribers. `D` is the snapshot type, usually `Arc<Project>`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::hooks::Origin;
use crate::mutex_guard;

/// Why a revision was published. Plugin command ids are `Arc<str>`, not `'static`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    Command { id: Arc<str>, depth: u32, origin: Origin },
    Undo,
    Redo,
    Job(&'static str),
    Load,
    Untracked,
}

/// One published document change. `snapshot` is the value after the edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Change<D> {
    pub revision: u64,
    pub cause: Cause,
    pub merged: bool,
    pub snapshot: D,
    pub prev: Option<D>,
}

/// Changes one subscriber may hold. A subscriber that does not drain keeps at most this many snapshots alive.
const CAPACITY: usize = 256;

struct Queue<D> {
    changes: VecDeque<Change<D>>,
    /// Changes dropped since the last drain.
    lost: u64,
}

impl<D> Queue<D> {
    fn new() -> Self {
        Self { changes: VecDeque::new(), lost: 0 }
    }
}

struct ChangeState<D> {
    next_id: u64,
    subs: HashMap<u64, Queue<D>>,
}

/// Multi-subscriber queue. `publish` clones the change once per subscriber.
///
/// Each subscriber holds at most 256 changes. When a queue is full, `publish` drops the oldest
/// change and counts it. [`ChangeBus::drain_with_lost`] returns that count, so a subscriber that
/// lost changes knows to re-read the document instead of replaying the feed.
pub struct ChangeBus<D> {
    state: Mutex<ChangeState<D>>,
    /// Live subscriber count. `Hooks` shares one counter across all its buses.
    subscribers: Arc<AtomicUsize>,
}

impl<D> ChangeBus<D> {
    pub fn new() -> Self {
        Self::with_counter(Arc::new(AtomicUsize::new(0)))
    }

    /// A bus that adds its subscribers to `subscribers`.
    pub(crate) fn with_counter(subscribers: Arc<AtomicUsize>) -> Self {
        Self { state: Mutex::new(ChangeState { next_id: 0, subs: HashMap::new() }), subscribers }
    }

    pub fn subscribe(&self) -> u64 {
        let mut state = mutex_guard(&self.state);
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state.subs.insert(id, Queue::new());
        self.subscribers.fetch_add(1, Ordering::Release);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        if mutex_guard(&self.state).subs.remove(&id).is_some() {
            self.subscribers.fetch_sub(1, Ordering::Release);
        }
    }

    /// Take the queued changes of one subscriber, oldest first.
    pub fn drain(&self, id: u64) -> Vec<Change<D>> {
        self.drain_with_lost(id).0
    }

    /// Like [`ChangeBus::drain`], plus how many changes were dropped since the last drain.
    /// The dropped changes are the oldest ones. A non-zero count means the subscriber must resync.
    pub fn drain_with_lost(&self, id: u64) -> (Vec<Change<D>>, u64) {
        let mut state = mutex_guard(&self.state);
        match state.subs.get_mut(&id) {
            Some(queue) => (queue.changes.drain(..).collect(), std::mem::take(&mut queue.lost)),
            None => (Vec::new(), 0),
        }
    }
}

impl<D> Default for ChangeBus<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Clone> ChangeBus<D> {
    pub fn publish(&self, change: Change<D>) {
        let mut state = mutex_guard(&self.state);
        for queue in state.subs.values_mut() {
            if queue.changes.len() >= CAPACITY {
                queue.changes.pop_front();
                queue.lost = queue.lost.saturating_add(1);
            }
            queue.changes.push_back(change.clone());
        }
    }
}
