//! Drags that cross windows. The drag session maps the desktop pointer into every window, decides which window a
//! drop lands in (never the window a drag moves) and each window's dock resolves the drop from its own spots, so a
//! panel, group or window dropped in another window docks there with no second path. What is left for the dock is
//! the focus the receiving window takes.

use crate::runtime::drag;
use crate::runtime::windows::{WindowAction, WindowHost};
use egui::Context;

/// Raises the running window when the drop it takes comes from a drag that started in another window, so the user
/// continues in the window the panel landed in. The dock calls it where it takes the drop. False for a drop within the
/// window the drag started in.
#[doc(hidden)]
pub fn accept_cross_window(ctx: &Context) -> bool {
    let host = WindowHost::of(ctx);
    let viewport = host.current();
    drag::cross_window_pointer(ctx, viewport).is_some() && host.act(viewport, WindowAction::Focus)
}
