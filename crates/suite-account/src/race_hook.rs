//! A synchronization point inside [`crate::Store::update_token`], between its first, unlocked
//! generation check and the lock it takes for its second one. Test only, compiled for nothing
//! but `cargo test`.
//!
//! Added for a review note on this crate's own race tests: a thread barrier only lines the
//! refresh and a competing sign-in or discard-and-re-add up at their *start*. From there either
//! one can finish first. A build that still has the bug this crate's tests guard against can
//! then pass for the wrong reason, by rejecting at the first, unlocked check because the
//! competing write happened to land first, never reaching the second, locked check the test is
//! actually named for. `arm` and `pause` below pin that window instead: a test calls `arm`
//! before starting the refresh, waits on the returned handle's `arrived` until the refresh has
//! done its first check, runs the competing operation to completion, then sends on `go` to let
//! the refresh continue into its second check.
//!
//! Outside a test that calls `arm`, [`pause`] does nothing: every other `update_token` call,
//! including every one outside this crate's own test suite, runs straight through.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, OnceLock};

struct Gate {
    arrived: SyncSender<()>,
    go: Receiver<()>,
}

fn slot() -> &'static Mutex<Option<Gate>> {
    static SLOT: OnceLock<Mutex<Option<Gate>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// The test's half of the gate. `arrived.recv()` blocks until the refresh reaches the pause
/// point; sending on `go` releases it to continue past it.
pub(crate) struct Handle {
    pub(crate) arrived: Receiver<()>,
    pub(crate) go: SyncSender<()>,
}

/// Arms the gate for the next call into [`crate::Store::update_token`] in this process. This
/// crate's tests run single threaded (its established pattern; see the gate instructions in the
/// workspace's house rules), so one armed gate at a time is always enough.
pub(crate) fn arm() -> Handle {
    let (arrived_tx, arrived_rx) = sync_channel::<()>(0);
    let (go_tx, go_rx) = sync_channel::<()>(0);
    let mut guard = slot().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Gate { arrived: arrived_tx, go: go_rx });
    Handle { arrived: arrived_rx, go: go_tx }
}

/// `update_token`'s half. A no-op when nothing is armed, which is every call outside a test that
/// called `arm`. When armed, tells the test it has just finished its first, unlocked check, then
/// blocks until the test releases it.
pub(crate) fn pause() {
    let gate = slot().lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(gate) = gate {
        let _ = gate.arrived.send(());
        let _ = gate.go.recv();
    }
}
