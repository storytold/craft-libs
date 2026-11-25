//! macOS open-documents and quit Apple events, fed into a [`FileIntake`](super::FileIntake).
//!
//! Finder double-clicks, Open With, drops on the Dock icon and `open -a App file` don't pass paths
//! on the command line: LaunchServices sends the running (or just-launched) app a
//! `kAEOpenDocuments` ('odoc') Apple event. winit doesn't handle it and owns the
//! `NSApplicationDelegate`, so without a handler AppKit answers that the app cannot open files of
//! that format. Handling it needs Objective-C class declarations, i.e. `unsafe`, which the
//! workspace forbids; `fmv-macos-events` wraps exactly that (an `NSAppleEventManager` handler
//! registered before Finder's launch event, leaving winit's delegate alone) behind a safe
//! main-thread API.

use fmv_macos_events::{Event, Inbox, Registration};

use super::{DrainRegistration, IntakeSender};

/// Keeps the Apple-event handlers registered; hold it until the event loop returns.
pub struct AppleEvents {
    _registration: Registration,
    // The inbox is not `Send`, so it stays here and in the drain registered on this thread.
    _inbox: Inbox,
    _drain: DrainRegistration,
}

/// Registers the open-documents and quit handlers and routes their events into `sender`: an open
/// event becomes [`IntakeSender::open`], a quit event [`IntakeSender::quit`], when the app next
/// takes its intake.
///
/// The inbox calls its wake while it holds its own borrow, so the wake only asks the UI to
/// repaint; the events are drained on the main thread by `FileIntake::take`.
///
/// Call on the main thread before the event loop starts, so the event that launched the app (a
/// Finder double-click) is caught too.
pub fn install(sender: IntakeSender) -> AppleEvents {
    let (registration, inbox) = Registration::install();
    let woken = sender.clone();
    inbox.set_wake(move || woken.wake_ui());
    let source = inbox.clone();
    let drain = sender.register_drain(move |sender| forward(&source, sender));
    // An event queued before the wake was set has no wake to run it.
    sender.wake_ui();
    AppleEvents { _registration: registration, _inbox: inbox, _drain: drain }
}

fn forward(inbox: &Inbox, sender: &IntakeSender) {
    for event in inbox.drain() {
        match event {
            Event::Open(paths) => sender.open(paths.into_iter().collect()),
            Event::Quit => sender.quit(),
        }
    }
}
