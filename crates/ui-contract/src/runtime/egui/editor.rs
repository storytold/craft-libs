//! The egui widget of an editor part: it holds key focus so the focus service, the key filter and the IME owner have
//! an id, and draws nothing.

use egui::{EventFilter, Id, Rect, Ui};

/// Registers the editor's focus widget over `rect` and applies `filter` while it holds focus; returns whether it holds
/// focus. The widget is one of egui's focus candidates only while `focusable` (its component takes focus), so neither
/// Tab nor egui's arrows reach the editor of a disabled component.
pub(crate) fn show(ui: &mut Ui, id: Id, rect: Rect, filter: EventFilter, focusable: bool) -> bool {
    let sense = if focusable { egui::Sense::focusable_noninteractive() } else { egui::Sense::hover() };
    let response = ui.interact(rect, id, sense);
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
    }
    focused
}
