//! The title bar's window side: which window the app's bar lets the OS move, and on macOS the `NSWindow.movable`
//! flag that follows it.

use crate::runtime::egui::viewports;
use egui::Context;

/// Whether the OS may move the running viewport's window from where the pointer is in this pass: what the move bands
/// of the pass wrote (the body step writes it for every move band of an OS window, `true` only where a press would go
/// to the band), else what the app wrote with [`set_window_movable`], else true.
pub fn window_movable(ctx: &Context) -> bool {
    match viewports::movable_record(ctx) {
        Some((pass, movable)) if pass == viewports::pass_number(ctx) => movable,
        _ => true,
    }
}

/// Records that the OS may move the running viewport's window in this pass (`true`) or not. `AppShell` writes `false`
/// for a pass that draws no title bar, or AppKit drags the window from the transparent strip; an app that draws its
/// window without `AppShell` does the same. A move band shown later in the pass still allows the move where a press
/// would go to it.
pub fn set_window_movable(ctx: &Context, movable: bool) {
    viewports::write_movable(ctx, movable);
}

/// Sets the `movable` flag of the context's window from this pass's [`window_movable`]: the one place AppKit's flag is
/// set. Call it once per frame per window, after that window's UI has run; `applied` holds the value last set for the window, so AppKit is only
/// called when it changes.
///
/// A window is told apart by its viewport: the key window is the one whose viewport has focus, so an unfocused
/// viewport is left as it is. Only a window with a full-size content view and visible window buttons is touched,
/// because floating panels in their own windows keep their own `movable`, which AppKit also needs to resize them.
/// Only a macOS window has a flag to set; elsewhere the call does nothing. Apple builds do not run on the dev box,
/// so the macOS side compiles by inspection until the Mac builds it.
pub fn sync_movable(ctx: &Context, applied: &mut Option<bool>) {
    if !viewports::current(ctx).focus_reported {
        return;
    }
    let movable = window_movable(ctx);
    if *applied == Some(movable) {
        return;
    }
    viewports::set_native_movable(movable, applied);
}

// The bar, its title and its two ends.
mod bar;
mod measure;
mod style;
mod title;

pub use bar::{TitleBar, TitleBarOutput};
pub use style::TitleBarStyle;
pub use title::Title;
