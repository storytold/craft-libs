//! Reading text parts in trials: the widget id, the caret and the text a frame painted.

use crate::primitives::text_adapter;
use craft_behavior::PartKey;
use egui::{Context, Id, Shape};

pub use crate::runtime::egui::fixture::{ImeSeen, ime_preedit, set_value_request, text_selection_request};

/// The run node and character index a position maps to among runs given as `(line, bytes, text)`.
pub fn run_end(owner: Id, runs: &[(usize, std::ops::Range<usize>, &str)], pos: craft_behavior::TextPos) -> Option<(Id, usize)> {
    crate::runtime::access::text::end_of_runs(owner, runs, pos)
}

/// The id of the run node `run` of line `line` below the text component `owner`.
pub fn run_node(owner: Id, line: usize, run: usize) -> Id {
    crate::runtime::access::text::run_id(owner, line, run)
}

/// The egui widget id of the text part `key` of component `id`.
pub fn widget_id(id: Id, key: PartKey) -> Id {
    text_adapter::text_id(id, key)
}

/// The egui widget id of the editor part `key` of component `id`.
pub fn editor_widget_id(id: Id, key: PartKey) -> Id {
    crate::primitives::ImeSurface::of(id, key).widget()
}

/// The caret of a text widget as a character index.
pub fn caret(ctx: &Context, widget: Id) -> Option<usize> {
    crate::runtime::egui::text_edit::selection(ctx, widget).map(|selection| selection.0)
}

/// The texts of the text shapes among `shapes`.
pub fn painted(shapes: &[Shape]) -> Vec<String> {
    shapes.iter().filter_map(|shape| if let Shape::Text(text) = shape { Some(text.galley.text().to_owned()) } else { None }).collect()
}
