//! The egui widget of an editor part: it holds key focus so the focus service, the key filter and the IME owner have
//! an id, and draws nothing.

use egui::{EventFilter, Id, Rect, Ui};

/// Registers the editor's focus widget over `rect` and applies `filter` while it holds focus; returns whether it holds
/// focus.
pub(crate) fn show(ui: &mut Ui, id: Id, rect: Rect, filter: EventFilter) -> bool {
    ui.interact(rect, id, egui::Sense::focusable_noninteractive());
    // The widget's key focus, whether or not the window has the system's: switching to another app is no focus move
    // (egui's `Response::has_focus` is false while the window is in the background).
    let focused = ui.memory(|memory| memory.has_focus(id));
    if focused {
        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
    }
    focused
}
