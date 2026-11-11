//! Crate-private logic the trials check without a frame: the geometry of a row, the order of footer buttons, the
//! reorder arithmetic and the submenu pointer triangle.

pub use crate::contract::reorder::Gap;
use crate::list::ListRow;
pub use crate::list::row::RowPlacement;
use crate::sections::DialogRole;
use craft_behavior::Axis;
use egui::{Pos2, Rect};

/// Where the parts of `row` lie in a rect of `width`, under fixed sizes.
pub fn place_row(row: &ListRow<'_>, width: f32, rtl: bool) -> RowPlacement {
    crate::list::row::placement_under_fixed_sizes(row, width, rtl)
}

/// The indent steps of `row`.
pub fn row_indent_steps(row: &ListRow<'_>) -> usize {
    crate::list::row::indent_steps(row)
}

/// The indexes of `roles` in display order.
pub fn footer_order(roles: &[DialogRole], windows: bool) -> Vec<usize> {
    crate::sections::footer::display_order(roles, windows)
}

/// The index an item ends at when it moves from `from` to the gap before item `insert`.
pub fn reorder_move_to(from: usize, insert: usize) -> usize {
    crate::contract::reorder::move_to(from, insert)
}

/// The index the first of `places` ends at when the group moves to the gap before item `insert`.
pub fn reorder_move_all_to(places: &[usize], insert: usize) -> usize {
    crate::contract::reorder::move_all_to(places, insert)
}

/// The gap nearest `at` among `rects` that lie in order along `axis`.
pub fn reorder_gap_at(rects: &[Rect], axis: Axis, at: Pos2) -> Option<Gap> {
    crate::contract::reorder::gap_at(rects, axis, at)
}

/// Whether the pointer at `now`, having come from `from`, heads for `submenu`.
pub fn pointer_heads_toward(from: Pos2, now: Pos2, submenu: Rect) -> bool {
    crate::menu::level::heads_toward(from, now, submenu)
}
