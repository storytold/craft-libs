//! The AccessKit node registry: which component takes the requests that target a node.

use super::{Node, current};
use crate::runtime::egui::access::node_key;
use craft_behavior::PartRef;
use egui::{Context, Id};

/// Makes `owner` the receiver of the requests that target the node of `node` (registered by `register_node`) and names
/// `part` as the target: the row of a collection is a node of the row's component, and the collection acts on it.
pub(crate) fn redirect_node(ctx: &Context, node: Id, owner: Id, part: PartRef) {
    // Journaled after the `register_node` it amends, so a settled view's replay leaves the row's node with the
    // collection, as a run does.
    crate::runtime::replay::journal(ctx, || (node, owner, part), |ctx, &(node, owner, part)| redirect_node(ctx, node, owner, part));
    crate::runtime::replay::note_node_owner(ctx, owner);
    current(ctx, |stage| {
        if let Some(entry) = stage.nodes.get_mut(&node_key(node)) {
            entry.owner = owner;
            entry.part = Some(part);
        }
    });
}

/// Records the expanded state of the node registered by `register_node`: an `Expand` or `Collapse` request on a node
/// with the state asks to change it, and acts as the node's activation. With `toggle`, the part of the owner that opens
/// the popup and has no node of its own, the request goes to that part instead.
pub(crate) fn register_expanded(ctx: &Context, node: Id, expanded: Option<bool>, toggle: Option<PartRef>) {
    crate::runtime::replay::journal(
        ctx,
        || (node, expanded, toggle),
        |ctx, &(node, expanded, toggle)| register_expanded(ctx, node, expanded, toggle),
    );
    current(ctx, |stage| {
        if let Some(entry) = stage.nodes.get_mut(&node_key(node)) {
            entry.expanded = expanded;
            entry.toggle = toggle;
        }
    });
}

/// Registers an AccessKit node so a request that targets it reaches `owner`.
pub(crate) fn register_node(ctx: &Context, node: Id, owner: Id, part: Option<PartRef>, focusable: bool) {
    crate::runtime::replay::journal(
        ctx,
        || (node, owner, part, focusable),
        |ctx, &(node, owner, part, focusable)| register_node(ctx, node, owner, part, focusable),
    );
    crate::runtime::replay::note_node_owner(ctx, owner);
    current(ctx, |stage| {
        stage.nodes.insert(node_key(node), Node { owner, part, focusable, expanded: None, toggle: None });
    });
}
