#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Quit a Craft app the platform's own way.
//!
//! On macOS an eframe app must not leave by closing its window. eframe drops the window while
//! AppKit's run loop is still running, and in the next display cycle AppKit's Touch Bar finder
//! tries to remove a KVO observer from the view that is gone
//! (`-[_NSTouchBarFinderObservation invalidate]`). That is an uncaught Objective-C exception and
//! the app crashes (seen on macOS 27 on a Touch Bar Mac).
//! `-[NSApplication terminate:]`, what ⌘Q does in winit's default menu, ends the process before
//! another display cycle runs.
//!
//! [`quit`] is that quit. With the `ui` feature, [`on_menu_event`] turns a native Quit menu item
//! into a close request of the root window, and [`intercept_close`] holds a close while documents
//! are unsaved.

mod close;
#[cfg(target_os = "macos")]
mod macos;

pub use close::{CloseAnswer, CloseGuard, CloseScope, GuardStep, QUIT_MENU_ID};
#[cfg(feature = "ui")]
pub use close::{intercept_close, on_menu_event};

/// Quit through the platform's own path, where leaving by closing the window is not safe.
///
/// macOS: schedules `-[NSApplication terminate:]` and returns `true`; the caller keeps its window
/// open and the process ends shortly after, once the current event has been handled. Must be
/// called while the app's event loop is running (any eframe callback is). Return to the event loop
/// before blocking: a blocking native dialog opened in the same callback would run the quit while
/// winit is still handling the event, which panics.
///
/// Other platforms: does nothing and returns `false`; the caller closes its window as usual.
pub fn quit() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::terminate_later();
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
