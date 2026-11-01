//! A bounded, coalescing "something happened" signal between a backend that can be told about
//! changes (a push channel) and the [`crate::JobWatch`] that follows a job.
//!
//! A wake carries **no data**: it only means "ask for the status now". A watch that is woken polls
//! like it would have a little later, so a lost, duplicated or forged wake can cost a request or some
//! latency and nothing else. Wakes coalesce into one counter, so a flood cannot grow memory, and the
//! receiver also carries a *health* flag: the backend says whether the push channel is currently
//! trustworthy, and the watch only relaxes its polling while it is.
//!
//! The pair is made with [`wake_channel`]. The [`WakeSender`] holds a weak reference: dropping the
//! [`WakeReceiver`] (the watch ended) disconnects it, and the backend sees that through
//! [`WakeSender::is_connected`] and can release what it set up for the watch.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

#[derive(Debug, Default)]
struct Shared {
    pending: AtomicU32,
    total: AtomicU64,
    healthy: AtomicBool,
}

/// The sending side, held by the backend.
#[derive(Clone, Debug)]
pub struct WakeSender(Weak<Shared>);

/// The receiving side, held by the watch. Not `Clone`: one watch, one receiver.
#[derive(Debug)]
pub struct WakeReceiver(Arc<Shared>);

/// A connected pair. The receiver starts *unhealthy*: the backend must say when the channel is
/// trustworthy ([`WakeSender::set_healthy`]).
pub fn wake_channel() -> (WakeSender, WakeReceiver) {
    let shared = Arc::new(Shared::default());
    (WakeSender(Arc::downgrade(&shared)), WakeReceiver(shared))
}

impl WakeSender {
    /// Wake the receiver. `false` when the receiver is gone.
    pub fn wake(&self) -> bool {
        let Some(shared) = self.0.upgrade() else { return false };
        // Saturating: the count is only "at least this many", never an unbounded queue.
        let mut current = shared.pending.load(Ordering::Acquire);
        while let Err(seen) = shared.pending.compare_exchange_weak(current, current.saturating_add(1), Ordering::AcqRel, Ordering::Acquire) {
            current = seen;
        }
        shared.total.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Say whether the push channel is currently trustworthy.
    pub fn set_healthy(&self, healthy: bool) {
        if let Some(shared) = self.0.upgrade() {
            shared.healthy.store(healthy, Ordering::Release);
        }
    }

    /// Whether the receiver still exists.
    pub fn is_connected(&self) -> bool {
        self.0.strong_count() > 0
    }
}

impl WakeReceiver {
    /// Whether a wake is waiting.
    pub fn is_pending(&self) -> bool {
        self.0.pending.load(Ordering::Acquire) > 0
    }

    /// Take the wakes that arrived since the last call: how many (saturating), zero for none.
    pub fn take(&self) -> u32 {
        self.0.pending.swap(0, Ordering::AcqRel)
    }

    /// Whether the backend currently trusts its push channel.
    pub fn is_healthy(&self) -> bool {
        self.0.healthy.load(Ordering::Acquire)
    }

    /// Every wake ever sent to this receiver (for diagnostics and tests).
    pub fn total(&self) -> u64 {
        self.0.total.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakes_coalesce_into_a_count_and_are_taken_once() {
        let (tx, rx) = wake_channel();
        assert!(!rx.is_pending() && !rx.is_healthy());
        for _ in 0..5 {
            assert!(tx.wake());
        }
        assert!(rx.is_pending());
        assert_eq!(rx.take(), 5);
        assert_eq!(rx.take(), 0);
        assert!(!rx.is_pending());
        assert_eq!(rx.total(), 5);
    }

    #[test]
    fn the_count_saturates_instead_of_growing_without_bound() {
        let (tx, rx) = wake_channel();
        rx.0.pending.store(u32::MAX, Ordering::Release);
        assert!(tx.wake());
        assert_eq!(rx.take(), u32::MAX);
    }

    #[test]
    fn dropping_the_receiver_disconnects_the_sender() {
        let (tx, rx) = wake_channel();
        tx.set_healthy(true);
        assert!(rx.is_healthy() && tx.is_connected());
        drop(rx);
        assert!(!tx.is_connected());
        assert!(!tx.wake());
        tx.set_healthy(false);
    }
}
