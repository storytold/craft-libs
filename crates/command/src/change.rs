//! Document change feed. One bus, many subscribers. `D` is the snapshot type, usually `Arc<Project>`.

use std::collections::HashMap;
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

struct ChangeState<D> {
    next_id: u64,
    subs: HashMap<u64, Vec<Change<D>>>,
}

/// Multi-subscriber queue. `publish` clones the change once per subscriber.
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
        state.subs.insert(id, Vec::new());
        self.subscribers.fetch_add(1, Ordering::Release);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        if mutex_guard(&self.state).subs.remove(&id).is_some() {
            self.subscribers.fetch_sub(1, Ordering::Release);
        }
    }

    pub fn drain(&self, id: u64) -> Vec<Change<D>> {
        let mut state = mutex_guard(&self.state);
        if let Some(queue) = state.subs.get_mut(&id) { std::mem::take(queue) } else { Vec::new() }
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
            queue.push(change.clone());
        }
    }
}
