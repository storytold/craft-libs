//! The title bar's window side: which window the app's bar lets the OS move, and on macOS the `NSWindow.movable`
//! flag that follows it.

use crate::runtime::egui::viewports;
use egui::Context;

/// Whether the OS may move the running viewport's window from where the pointer is in this pass: what the move bands
/// of the pass wrote (the body step writes it for every move band of an OS window, `true` only where a press would go
/// to the band), else what the app wrote with [`set_window_movable`], else true. The runtime sets macOS's
/// `NSWindow.movable` from it at the end of each window's pass.
pub fn window_movable(ctx: &Context) -> bool {
    viewports::movable(ctx)
}

/// Records that the OS may move the running viewport's window in this pass (`true`) or not. `AppShell` writes `false`
/// for a pass that draws no title bar, or AppKit drags the window from the transparent strip; an app that draws its
/// window without `AppShell` does the same. A move band shown later in the pass still allows the move where a press
/// would go to it.
pub fn set_window_movable(ctx: &Context, movable: bool) {
    viewports::write_movable(ctx, movable);
}

// The bar, its title and its two ends.
mod bar;
mod measure;
mod style;
mod title;

pub use bar::{TitleBar, TitleBarOutput};
pub use style::TitleBarStyle;
pub use title::Title;
