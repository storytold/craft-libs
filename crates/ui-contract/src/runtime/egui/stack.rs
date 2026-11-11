//! `UiStack` walks for `Env::of`.

use egui::{Id, Ui};

/// The unique id of `ui`'s stack node: the key env entries are installed under.
pub(crate) fn node_id(ui: &Ui) -> Id {
    ui.stack().unique_id
}

/// The first value `lookup` finds walking from `ui` outwards, by the stack nodes' unique ids.
pub(crate) fn nearest<T>(ui: &Ui, lookup: impl Fn(Id) -> Option<T>) -> Option<T> {
    ui.stack().iter().find_map(|node| lookup(node.unique_id))
}
