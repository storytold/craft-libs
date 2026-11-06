#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Command registry, hooks, and the document change feed.

pub mod change;
#[cfg(feature = "testing")]
pub mod conformance;
pub mod hooks;
pub mod json_diff;
pub mod observe;
pub mod overlay;
pub mod registry;
pub mod schema;
pub mod spec;
pub mod tracked;

pub use change::{Cause, Change, ChangeBus};
pub use hooks::{Call, CallInfo, Clock, DisableCause, DisableListener, GateDisabled, Hooks, Interceptor, Interest, Origin, Outcome, Verdict};
pub use observe::{ObserverEvent, ObserverQueue};
pub use overlay::{DynCommand, Enablement, Owner, RegisterError, Visibility};
pub use registry::{CommandEntry, Registry};
pub use spec::{CommandSpec, CommandView, Menu};
pub use tracked::Tracked;

/// A command-layer failure an app can wrap with `#[from]`.
#[derive(Debug)]
pub enum CommandError {
    /// A `before` gate rejected the call. `after` still runs.
    Vetoed { reason: String, by: String },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Vetoed { reason, by } => write!(f, "vetoed by {by}: {reason}"),
        }
    }
}

impl std::error::Error for CommandError {}

pub(crate) fn mutex_guard<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn read_guard<T>(lock: &std::sync::RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn write_guard<T>(lock: &std::sync::RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}
