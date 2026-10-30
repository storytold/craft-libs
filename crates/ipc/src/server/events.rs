//! Per-connection event queue. `emit` only enqueues.
//!
//! The writer sleeps a short grace before it drains so a burst fills the
//! queue of [`crate::wire::EVENT_QUEUE`] instead of racing one write at a time
//! into the socket buffer. Overflow occupies one of those slots.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Hub, lock};
use crate::wire::{self, EVENT_QUEUE};

/// How long a burst may land in the queue before the writer drains it.
const BATCH_GRACE: Duration = Duration::from_millis(30);
const WAIT_SLICE: Duration = Duration::from_millis(200);

pub(crate) struct Subscription {
    pub id: u64,
    filter: Mutex<Filter>,
    outbox: Mutex<Outbox>,
    cv: Condvar,
    closed: AtomicBool,
}

struct Filter {
    names: Option<HashSet<String>>,
}

struct Outbox {
    items: VecDeque<Queued>,
    latched: bool,
    next_seq: u64,
}

struct Queued {
    line: Value,
    overflow: bool,
}

impl Subscription {
    pub(super) fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            id,
            filter: Mutex::new(Filter { names: None }),
            outbox: Mutex::new(Outbox { items: VecDeque::new(), latched: false, next_seq: 1 }),
            cv: Condvar::new(),
            closed: AtomicBool::new(false),
        })
    }
}

pub(super) fn subscribe(sub: &Subscription, names: Vec<String>) {
    let mut filter = lock(&sub.filter);
    filter.names = Some(names.into_iter().collect());
}

pub(super) fn is_subscribed(sub: &Subscription) -> bool {
    lock(&sub.filter).names.is_some()
}

pub(super) fn unsubscribe(sub: &Subscription) {
    let mut filter = lock(&sub.filter);
    filter.names = None;
}

pub(super) fn enqueue(sub: &Subscription, event: &str, data: &Value) {
    if sub.closed.load(Ordering::SeqCst) {
        return;
    }
    let interested = {
        let filter = lock(&sub.filter);
        filter.names.as_ref().is_some_and(|names| names.contains(event))
    };
    if !interested {
        return;
    }
    {
        let mut outbox = lock(&sub.outbox);
        if sub.closed.load(Ordering::SeqCst) || outbox.latched {
            return;
        }
        if outbox.items.len() >= EVENT_QUEUE {
            let since = outbox.items.pop_back().map(|item| item_seq(&item)).unwrap_or(outbox.next_seq);
            let seq = outbox.next_seq;
            outbox.next_seq = outbox.next_seq.saturating_add(1);
            outbox.items.push_back(Queued { overflow: true, line: wire::push_event("events.overflow", seq, json!({"since": since})) });
            outbox.latched = true;
        } else {
            let seq = outbox.next_seq;
            outbox.next_seq = outbox.next_seq.saturating_add(1);
            outbox.items.push_back(Queued { overflow: false, line: wire::push_event(event, seq, data.clone()) });
        }
    }
    sub.cv.notify_one();
}

fn item_seq(item: &Queued) -> u64 {
    item.line.get("seq").and_then(Value::as_u64).unwrap_or(0)
}

pub(super) fn close_sub(sub: &Subscription) {
    sub.closed.store(true, Ordering::SeqCst);
    // The outbox lock pairs with the check in `write_loop`, so the notify
    // cannot land before the writer has entered `wait`.
    drop(lock(&sub.outbox));
    sub.cv.notify_one();
}

pub(super) fn wake_all(subs: &Mutex<Vec<Arc<Subscription>>>) {
    let guard = lock(subs);
    for sub in guard.iter() {
        drop(lock(&sub.outbox));
        sub.cv.notify_one();
    }
}

pub(super) fn remove_sub(hub: &Hub, id: u64) {
    lock(&hub.subs).retain(|sub| sub.id != id);
}

pub(super) fn write_loop(sub: Arc<Subscription>, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>, stop: &AtomicBool) {
    loop {
        {
            let guard = lock(&sub.outbox);
            if sub.closed.load(Ordering::SeqCst) || stop.load(Ordering::SeqCst) {
                return;
            }
            if guard.items.is_empty() {
                let waited = sub.cv.wait_timeout(guard, WAIT_SLICE);
                let (guard, _) = match waited {
                    Ok(pair) => pair,
                    Err(err) => err.into_inner(),
                };
                drop(guard);
                continue;
            }
        }
        std::thread::sleep(BATCH_GRACE);
        if sub.closed.load(Ordering::SeqCst) || stop.load(Ordering::SeqCst) {
            return;
        }
        let batch = {
            let mut guard = lock(&sub.outbox);
            std::mem::take(&mut guard.items)
        };
        for item in batch {
            let wrote = {
                let mut guard = lock(&writer);
                wire::write_line(&mut *guard, &item.line).is_ok()
            };
            if item.overflow {
                lock(&sub.outbox).latched = false;
            }
            if !wrote {
                sub.closed.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn overflow_occupies_one_queue_slot() {
        let sub = Subscription::new(7);
        subscribe(&sub, vec!["doc.changed".into()]);
        enqueue(&sub, "other", &json!({"n": 0}));
        assert!(lock(&sub.outbox).items.is_empty());
        for n in 0..EVENT_QUEUE {
            enqueue(&sub, "doc.changed", &json!({"n": n}));
        }
        enqueue(&sub, "doc.changed", &json!({"n": EVENT_QUEUE}));
        enqueue(&sub, "doc.changed", &json!({"n": EVENT_QUEUE + 1}));
        let outbox = lock(&sub.outbox);
        assert!(outbox.latched);
        assert_eq!(outbox.items.len(), EVENT_QUEUE);
        let overflow: Vec<_> = outbox.items.iter().filter(|item| item.overflow).collect();
        assert_eq!(overflow.len(), 1);
        assert_eq!(outbox.items.iter().filter(|item| !item.overflow).count(), EVENT_QUEUE - 1);
        let last = overflow[0];
        assert_eq!(last.line["event"], json!("events.overflow"));
        assert!(last.line["data"]["since"].is_number());
        assert_eq!(outbox.items.front().expect("queued").line["seq"], json!(1));
    }
}
