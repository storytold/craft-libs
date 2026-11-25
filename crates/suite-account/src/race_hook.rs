//! A synchronization point inside [`crate::Store::update_token`], between its first, unlocked
//! generation check and the lock it takes for its second one. Test only, compiled for nothing
//! but `cargo test`.
//!
//! Added for a review note on this crate's own race tests: a thread barrier only lines the
//! refresh and a competing sign-in or discard-and-re-add up at their *start*. From there either
//! one can finish first. A build that still has the bug this crate's tests guard against can
//! then pass for the wrong reason, by rejecting at the first, unlocked check because the
//! competing write happened to land first, never reaching the second, locked check the test is
//! actually named for. [`Hook::arm`] and [`Hook::pause`] below pin that window instead: a test
//! arms its own `Store`'s hook before starting the refresh on that same store, waits on the
//! returned handle's `arrived` until the refresh has done its first check, runs the competing
//! operation to completion, then sends on `go` to let the refresh continue into its second
//! check.
//!
//! The hook lives on the `Store` instance, one [`Hook`] per store, rather than behind one shared
//! slot for the whole process. The workspace's own CI runs this crate's suite with the default,
//! parallel test runner, not the single-threaded invocation this crate's local gate had been
//! using; a process-wide hook armed by two of this crate's race tests running at the same time
//! answered each other's `pause`, so one test's release freed the other test's refresh early and
//! the pinned ordering both tests rely on collapsed. Scoping the hook to the store a given test
//! actually built means a concurrent test's hook can never be the one `update_token` reaches,
//! because every store keeps its own, so there is nothing left to cross.
//!
//! Outside a test that calls `arm`, [`Hook::pause`] does nothing: every other `update_token`
//! call, including every one outside this crate's own test suite, runs straight through.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

struct Gate {
    arrived: SyncSender<()>,
    go: Receiver<()>,
}

/// One store's own pause point. `Store::at` creates one for every instance; most never arm it,
/// so [`Hook::pause`] stays a no-op for them.
pub(crate) struct Hook(Mutex<Option<Gate>>);

/// The test's half of the gate. `arrived.recv()` blocks until the refresh reaches the pause
/// point on this same store; sending on `go` releases it to continue past it.
pub(crate) struct Handle {
    pub(crate) arrived: Receiver<()>,
    pub(crate) go: SyncSender<()>,
}

impl Hook {
    /// A fresh, disarmed hook, for `Store::at` to give every store it builds.
    pub(crate) fn new() -> Hook {
        Hook(Mutex::new(None))
    }

    /// Arms this store's own pause point for the next call into its [`crate::Store::update_token`].
    /// A store this test never built, and so never raced on, keeps its own hook disarmed the
    /// whole time, whatever else is running in parallel.
    pub(crate) fn arm(&self) -> Handle {
        let (arrived_tx, arrived_rx) = sync_channel::<()>(0);
        let (go_tx, go_rx) = sync_channel::<()>(0);
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Gate { arrived: arrived_tx, go: go_rx });
        Handle { arrived: arrived_rx, go: go_tx }
    }

    /// `update_token`'s half. A no-op when this store's hook was never armed, which is every
    /// call outside a test that called `arm` on this exact store. When armed, tells the test it
    /// has just finished its first, unlocked check, then blocks until the test releases it.
    pub(crate) fn pause(&self) {
        let gate = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(gate) = gate {
            let _ = gate.arrived.send(());
            let _ = gate.go.recv();
        }
    }
}
