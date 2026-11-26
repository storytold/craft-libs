//! `UiStack` walks for `Env::of`.

use egui::{Id, Ui, UiKind};

/// The unique id of `ui`'s stack node: the key env entries are installed under.
pub(crate) fn node_id(ui: &Ui) -> Id {
    ui.stack().unique_id
}

/// What `nearest` found walking from a `Ui` outwards.
pub(crate) struct Found<T> {
    /// The first value found, and whether it was found for the `Ui`'s own node.
    pub value: Option<(T, bool)>,
    /// The walk passed the `Ui` of an `egui::Grid` (`UiKind::Grid`) before the node the value was found for, or
    /// anywhere on the stack when none was found.
    pub grid: bool,
}

/// The first value `lookup` finds walking from `ui` outwards, by the stack nodes' unique ids.
pub(crate) fn nearest<T>(ui: &Ui, lookup: impl Fn(Id) -> Option<T>) -> Found<T> {
    let mut grid = false;
    for (depth, node) in ui.stack().iter().enumerate() {
        if let Some(value) = lookup(node.unique_id) {
            return Found { value: Some((value, depth == 0)), grid };
        }
        grid |= node.kind() == Some(UiKind::Grid);
    }
    Found { value: None, grid }
}
