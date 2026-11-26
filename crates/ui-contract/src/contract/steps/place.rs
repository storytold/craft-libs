//! Step 6, place.

use egui::{Rect, Ui, Vec2};

/// Step 6, place: in flow one allocation with no widget, at a rect none.
pub(crate) fn place(ui: &mut Ui, at: crate::contract::run::ShowAt, size: Vec2, child: crate::layout::InFlow) -> Rect {
    match at {
        crate::contract::run::ShowAt::Flow => crate::layout::place_in_cell(ui, size, child),
        crate::contract::run::ShowAt::Rect(rect) => rect,
    }
}
