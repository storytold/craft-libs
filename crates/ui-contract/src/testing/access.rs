//! What the accessibility trials read: the AccessKit tree a frame produced, and the label scope a container opens.

use crate::runtime::access::{FieldLabel, scope};
use egui::{Context, Id};

pub use crate::runtime::access::layer_root;
pub use crate::runtime::egui::access::{AccessTree, Declared, NodeKey, is_interactive, name, prune_controls};

/// The AccessKit tree a finished pass of a viewport produced, taken out of its output; empty when it made none.
pub fn tree_of(output: &mut crate::runtime::egui::fixture::FullOutput) -> AccessTree {
    crate::runtime::egui::access::tree_of(output).unwrap_or_else(AccessTree::none)
}

/// Opens the label scope of `container` with `label`, as a field container's body step does; the container's
/// report step closes it.
pub fn begin_label_scope(ctx: &Context, container: Id, label: FieldLabel) {
    scope::begin(ctx, container, container, scope::Kind::Label(label));
}

/// The node AccessKit reports as focused.
pub fn focused_key(tree: &AccessTree) -> NodeKey {
    tree.focused_key()
}

/// The rule violations of a frame: interactive nodes with no name, and every finding the runtime's role rules
/// reported on `ctx`.
pub fn violations(ctx: &Context, tree: &AccessTree) -> Vec<String> {
    let mut found: Vec<String> = tree
        .keys()
        .into_iter()
        .filter(|key| !tree.is_hidden(*key) && is_interactive(tree, *key) && name(tree, *key).is_none())
        .map(|key| format!("an interactive node with no name: {:?}", tree.role_of(key)))
        .collect();
    found.extend(super::contract::findings(ctx).into_iter().map(|finding| format!("{}: {} {}", finding.rule, finding.component, finding.subject)));
    found
}
