//! `-[NSApplication terminate:]`, scheduled on the main queue. All safe objc2 calls.

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

/// Send `terminate:` from the main queue rather than from the caller. The caller is inside winit's
/// event handler, and `terminate:` makes winit deliver its exit event, which it can't do while
/// another event is being handled. From the main queue winit is idle: its
/// `applicationWillTerminate:` closes the windows and runs the app's exit handling (eframe saves
/// and calls `on_exit`), and AppKit ends the process.
pub(crate) fn terminate_later() {
    DispatchQueue::main().exec_async(|| {
        // The main queue runs on the main thread, so there is always a marker.
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).terminate(None);
        }
    });
}
