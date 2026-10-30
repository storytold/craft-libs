//! Bounded per-subscriber observer queue. Capacity is 1024 call events.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::hooks::{Clock, Origin};
use crate::mutex_guard;

const CAPACITY: usize = 1024;

/// One drained observer item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverEvent {
    Call {
        id: Arc<str>,
        origin: Origin,
        at_ms: u64,
    },
    /// Dropped-call count. One event, updated in place until it is drained.
    Lost {
        n: u64,
    },
}

impl ObserverEvent {
    /// `{"events.lost": n}` for [`ObserverEvent::Lost`]. Call events return `None`.
    pub fn lost_json(&self) -> Option<Value> {
        match self {
            ObserverEvent::Lost { n } => Some(serde_json::json!({ "events.lost": n })),
            ObserverEvent::Call { .. } => None,
        }
    }
}

struct ObsCall {
    id: Arc<str>,
    origin: Origin,
    at_ms: u64,
}

struct Subscriber {
    coalesce_ms: Option<u64>,
    calls: VecDeque<ObsCall>,
    /// Lost count kept beside the call deque and updated in place.
    lost: u64,
}

struct QueueState {
    next_id: u64,
    subs: HashMap<u64, Subscriber>,
}

/// Drop-oldest queue. Each subscriber has its own buffer and its own coalesce window.
pub struct ObserverQueue {
    state: Mutex<QueueState>,
    subscribers: AtomicUsize,
}

impl ObserverQueue {
    pub fn new() -> Self {
        Self { state: Mutex::new(QueueState { next_id: 0, subs: HashMap::new() }), subscribers: AtomicUsize::new(0) }
    }

    pub fn subscribe(&self, coalesce_ms: Option<u64>) -> u64 {
        let mut state = mutex_guard(&self.state);
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state.subs.insert(id, Subscriber { coalesce_ms, calls: VecDeque::new(), lost: 0 });
        self.subscribers.fetch_add(1, Ordering::Release);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        if mutex_guard(&self.state).subs.remove(&id).is_some() {
            self.subscribers.fetch_sub(1, Ordering::Release);
        }
    }

    /// One atomic load. True while at least one subscriber exists.
    pub fn has_subscribers(&self) -> bool {
        self.subscribers.load(Ordering::Acquire) != 0
    }

    /// Record a command observation.
    ///
    /// A second push of the same id inside `coalesce_ms` does not add a call item.
    /// When the call buffer is full, the oldest call is dropped and `lost` grows by one.
    pub fn push(&self, id: &str, origin: Origin, clock: &dyn Clock) {
        let now = clock.now_ms();
        let mut state = mutex_guard(&self.state);
        for sub in state.subs.values_mut() {
            if let Some(window) = sub.coalesce_ms
                && let Some(prev) = sub.calls.iter().rev().find(|call| &*call.id == id)
                && now.saturating_sub(prev.at_ms) < window
            {
                continue;
            }
            if sub.calls.len() == CAPACITY {
                sub.calls.pop_front();
                sub.lost = sub.lost.saturating_add(1);
            }
            sub.calls.push_back(ObsCall { id: Arc::from(id), origin, at_ms: now });
        }
    }

    /// Drain one subscriber. A non-zero lost count is a single leading [`ObserverEvent::Lost`].
    pub fn drain(&self, id: u64) -> Vec<ObserverEvent> {
        let mut state = mutex_guard(&self.state);
        let Some(sub) = state.subs.get_mut(&id) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(sub.calls.len() + usize::from(sub.lost > 0));
        if sub.lost > 0 {
            out.push(ObserverEvent::Lost { n: sub.lost });
            sub.lost = 0;
        }
        out.extend(sub.calls.drain(..).map(|call| ObserverEvent::Call { id: call.id, origin: call.origin, at_ms: call.at_ms }));
        out
    }
}

impl Default for ObserverQueue {
    fn default() -> Self {
        Self::new()
    }
}
