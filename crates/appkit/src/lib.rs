//! The AppKit calls that need `unsafe`, behind a safe API: observing when a menu starts and stops tracking, and the
//! Save panel calls in `save_panel`. macOS only; the crate is empty elsewhere.
//!
//! This is one of the crates that may allow `unsafe`; the design-lint rule `contract/unsafe-allow` names it. Every
//! block says why it is sound.
#![warn(missing_docs)]
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::ptr::NonNull;
use std::sync::{Arc, Once};

use block2::RcBlock;
use objc2_app_kit::{NSMenuDidBeginTrackingNotification, NSMenuDidEndTrackingNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};

pub mod save_panel;

/// Calls `on_change(true)` when a menu begins tracking (`NSMenuDidBeginTrackingNotification`) and `on_change(false)`
/// when it ends (`NSMenuDidEndTrackingNotification`), from the thread that posts the notification. Observes once
/// for the process; a later call replaces nothing and does nothing.
pub fn observe_menu_tracking(on_change: Arc<dyn Fn(bool) + Send + Sync>) {
    static OBSERVE: Once = Once::new();
    OBSERVE.call_once(|| {
        // SAFETY: AppKit initializes these immutable notification-name constants before any code of the app runs.
        let names = unsafe { [(NSMenuDidBeginTrackingNotification, true), (NSMenuDidEndTrackingNotification, false)] };
        let center = NSNotificationCenter::defaultCenter();
        for (name, tracking) in names {
            let on_change = on_change.clone();
            let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change(tracking));
            // SAFETY: `name` is one of AppKit's notification names and the block takes the notification by pointer
            // without reading it. The center copies the block and keeps the observer for the process's lifetime;
            // nothing removes it, so the returned token is not needed.
            unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
            }
        }
    });
}
