//! What the accessibility trials read: the AccessKit tree a frame produced, and the label scope a container opens.

use crate::runtime::access::{FieldLabel, scope};
use egui::{Context, Id};

pub use crate::runtime::egui::access::{AccessTree, Declared, NodeKey};

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

/// Whether the node declares an action a user can take: focus, click or a value change.
pub fn is_interactive(tree: &AccessTree, key: NodeKey) -> bool {
    [Declared::Focus, Declared::Click, Declared::Increment, Declared::Decrement, Declared::SetValue, Declared::Expand, Declared::Collapse]
        .into_iter()
        .any(|action| tree.declares_key(key, action))
}

/// The accessible name of the node, when it has a non-empty one: its own, else the first non-empty name of the nodes
/// that label it (`labelled_by`).
pub fn name(tree: &AccessTree, key: NodeKey) -> Option<&str> {
    let named = |key: NodeKey| tree.name_of(key).filter(|name| !name.trim().is_empty());
    named(key).or_else(|| tree.labelled_by_key(key).into_iter().find_map(named))
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
