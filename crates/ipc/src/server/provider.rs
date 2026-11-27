//! Calls into app code that must not hold up shutdown.
//!
//! `Assets` and `Activate` are control plane calls, rare and slow enough to pay for a thread. Each
//! call runs on a helper thread while the connection thread waits for the answer, for at most
//! [`super::Limits::provider_deadline`] and not past the stop flag. A call that does not finish in
//! time gets a timeout reply, and the helper thread is detached: nobody joins it, so
//! [`super::Server::shutdown`] and drop do not wait for it. At most [`MAX_HELPERS`] helper
//! threads are alive at once. A new call past that gets a `busy` error until a stuck one returns.
//!
//! `Frames` is not run here. It is the hot path with one call per frame, so it stays on the
//! connection thread (see [`super::Frames`]).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::Hub;
use crate::error::LinkError;
use crate::wire::MAX_CONNECTIONS;

/// Most provider helper threads alive at once, running or stuck. Every connection can have one
/// call in flight, so this leaves room for the same number of stuck ones.
pub(super) const MAX_HELPERS: usize = 2 * MAX_CONNECTIONS;

/// How often a waiting connection thread looks at the stop flag.
const STOP_POLL: Duration = Duration::from_millis(100);

/// Counts one live helper thread. Dropping it, on any path, frees the slot.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Run `run` on a helper thread and wait for its result.
///
/// `Err(Timeout)` when it takes longer than the deadline, `Err(Closed)` when the server stops
/// first, `Err(Busy)` when [`MAX_HELPERS`] helpers are already alive, and `Err(Io)` when the
/// helper could not start or panicked.
pub(super) fn call<T: Send + 'static>(hub: &Hub, what: &str, run: impl FnOnce() -> T + Send + 'static) -> Result<T, LinkError> {
    if hub.providers.fetch_add(1, Ordering::SeqCst) >= MAX_HELPERS {
        hub.providers.fetch_sub(1, Ordering::SeqCst);
        return Err(LinkError::Busy(format!("{what}: too many provider calls are stuck")));
    }
    let slot = Slot(Arc::clone(&hub.providers));
    let (tx, rx) = mpsc::channel();
    let started = std::thread::Builder::new().name("craft-ipc-provider".into()).spawn(move || {
        let _slot = slot;
        let _ = tx.send(run());
    });
    if let Err(err) = started {
        return Err(LinkError::Io(format!("{what}: cannot start a helper thread: {err}")));
    }
    let deadline = Instant::now() + hub.limits.provider_deadline;
    loop {
        if hub.stop.load(Ordering::SeqCst) {
            return Err(LinkError::Closed);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(LinkError::Timeout);
        }
        match rx.recv_timeout(left.min(STOP_POLL)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(LinkError::Io(format!("{what}: the call ended without an answer"))),
        }
    }
}
