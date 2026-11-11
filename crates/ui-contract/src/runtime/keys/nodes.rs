//! The AccessKit node registry: which component takes the requests that target a node.

use super::{Node, current};
use crate::runtime::egui::access::node_key;
use craft_behavior::PartRef;
use egui::{Context, Id};

/// Makes `owner` the receiver of the requests that target the node of `node` (registered by `register_node`) and names
/// `part` as the target: the row of a collection is a node of the row's component, and the collection acts on it.
pub(crate) fn redirect_node(ctx: &Context, node: Id, owner: Id, part: PartRef) {
    current(ctx, |stage| {
        if let Some(entry) = stage.nodes.get_mut(&node_key(node)) {
            entry.owner = owner;
            entry.part = Some(part);
        }
    });
}

/// Records the expanded state of the node registered by `register_node`: an `Expand` or `Collapse` request on a node
/// with the state asks to change it, and acts as the node's activation.
pub(crate) fn register_expanded(ctx: &Context, node: Id, expanded: Option<bool>) {
    current(ctx, |stage| {
        if let Some(entry) = stage.nodes.get_mut(&node_key(node)) {
            entry.expanded = expanded;
        }
    });
}

/// Registers an AccessKit node so a request that targets it reaches `owner`.
pub(crate) fn register_node(ctx: &Context, node: Id, owner: Id, part: Option<PartRef>, focusable: bool) {
    current(ctx, |stage| {
        stage.nodes.insert(node_key(node), Node { owner, part, focusable, expanded: None });
    });
}
