//! The AppKit calls that need `unsafe`, behind a safe API: observing when a menu starts and stops tracking. macOS
//! only; the crate is empty elsewhere.
//!
//! This is one of the crates that may allow `unsafe`; the design-lint rule `contract/unsafe-allow` names it.
#![warn(missing_docs)]
#![cfg(target_os = "macos")]

use std::sync::Arc;

/// Calls `on_change(true)` when a menu begins tracking (`NSMenuDidBeginTrackingNotification`) and `on_change(false)`
/// when it ends (`NSMenuDidEndTrackingNotification`), from the thread that posts the notification. Observes once
/// for the process; a later call replaces nothing and does nothing.
pub fn observe_menu_tracking(on_change: Arc<dyn Fn(bool) + Send + Sync>) {
    let _ = on_change;
    todo!("mn.M12")
}
