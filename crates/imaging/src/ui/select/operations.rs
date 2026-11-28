//! The four selection operations as entries for a `Segmented`: New, Add, Subtract and Intersect.

use craft_behavior::IconId;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_ui::contract::Entry;
use craft_ui::primitives::IconSpec;

craft_ui::words! {
    /// The selection operations' words, English by default (Photoshop's names).
    pub struct SelectionOpWords {
        new_selection: "selectionOp.newSelection" = "New selection";
        add_to_selection: "selectionOp.addToSelection" = "Add to selection";
        subtract_from_selection: "selectionOp.subtractFromSelection" = "Subtract from selection";
        intersect_with_selection: "selectionOp.intersectWithSelection" = "Intersect with selection";
    }
}

/// The four entries (icons `square`, `plus`, `minus`, `squares_subtract`) for a `Segmented`.
pub fn selection_operations(words: &SelectionOpWords) -> [Entry<SelectionOperation>; 4] {
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    [
        Entry::new(SelectionOperation::New, words.new_selection.clone()).icon(icon("square")),
        Entry::new(SelectionOperation::Add, words.add_to_selection.clone()).icon(icon("plus")),
        Entry::new(SelectionOperation::Subtract, words.subtract_from_selection.clone()).icon(icon("minus")),
        Entry::new(SelectionOperation::Intersect, words.intersect_with_selection.clone()).icon(icon("squares_subtract")),
    ]
}
