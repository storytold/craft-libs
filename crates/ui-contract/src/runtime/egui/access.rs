//! AccessKit node building, `register_accesskit_parent` and `set_accesskit_focus`: the one place that names
//! AccessKit's node API. `node` writes a decided node, `patch` applies the label scope and the status node, `tree`
//! reads a finished tree update. What a node says is decided in `runtime::access`.

mod node;
mod patch;
mod roles;
mod text;
#[cfg(feature = "testing")]
mod tree;

#[cfg(feature = "testing")]
use egui::FullOutput;
use egui::{Context, Id, ViewportId};
use std::collections::{HashMap, HashSet};

use super::memory::Service;
use super::pass;

pub(crate) use node::{NodeAction, NodeSpec, Toggle, write_node};
pub(crate) use patch::{write_label_patch, write_merge_patch, write_status};
pub(crate) use text::{
    NodeEnd, RunLink, RunNode, TextNode, fold_widget, is_text_request, requested_selection, requested_value, write_run, write_selection, write_text,
};
#[cfg(feature = "testing")]
pub use tree::{AccessTree, Declared, NodeKey};

/// The AccessKit role of a node.
pub(crate) type NodeRole = accesskit::Role;

/// The AccessKit vocabulary the testing kit reads a tree in.
#[cfg(feature = "testing")]
pub(crate) use accesskit::{Role, Toggled};

/// The AccessKit node number of `id`, the key an action request's target is matched by.
pub(crate) fn node_key(id: Id) -> u64 {
    id.accesskit_id().0
}

/// `builder` with `parent` as the node its `Ui`'s AccessKit nodes attach to.
pub(crate) fn with_parent(builder: egui::UiBuilder, parent: Id) -> egui::UiBuilder {
    builder.accessibility_parent(parent)
}

/// Creates the node of `id` empty if it has none. False while AccessKit is off or the widget is invisible (a
/// popup in its first-show sizing pass).
pub(crate) fn create_node(ctx: &Context, id: Id) -> bool {
    with_emitted(ctx, |emitted| emitted.emit(id));
    ctx.accesskit_node_builder(id, |_| ()).is_some()
}

/// Whether AccessKit is running and the widget `id` is visible: nodes are built only then. Creates the node of `id`.
pub(crate) fn enabled(ctx: &Context, id: Id) -> bool {
    create_node(ctx, id)
}

/// Hides the node of `id` from assistive technology and strips its actions: a hit-test widget the runtime made for
/// a part that has its own sub-node, or to stand in for a root on another layer. The sub-node or the root's own
/// node carries the actions.
pub(crate) fn hide(ctx: &Context, id: Id) {
    crate::runtime::egui::widgets::note_node(ctx, id);
    ctx.accesskit_node_builder(id, |node| {
        node.set_hidden();
        node.clear_actions();
    });
}

/// Reports `id` as the focused node of this pass.
pub(crate) fn set_focus(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| emitted.focus_set = true);
    ctx.set_accesskit_focus(id);
}

/// The runtime's nodes of a pass in the order it emitted them: the first time each node was reported as a child or
/// created, and the ids of the nodes written. egui lists a node's children in the order their nodes were first
/// created, which is not always the order the component emitted them in.
#[derive(Default)]
struct Emitted {
    order: HashMap<u64, usize>,
    written: HashSet<Id>,
    /// Text widgets whose node folds into their owner's node, each with its owner.
    folded: HashMap<Id, Id>,
    /// Whether the runtime reported a focused node this pass.
    focus_set: bool,
}

impl Emitted {
    fn emit(&mut self, id: Id) {
        let next = self.order.len();
        self.order.entry(node_key(id)).or_insert(next);
    }
}

/// One store per viewport: every viewport's pass ends with its own `order_children`, and a nested viewport's pass
/// runs inside its parent's.
#[derive(Default)]
struct Reported(HashMap<ViewportId, Emitted>);

fn with_emitted<R>(ctx: &Context, f: impl FnOnce(&mut Emitted) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    Service::<Reported>::run(ctx, |reported| f(reported.0.entry(viewport).or_default()))
}

/// Makes `parent` the node `id` attaches to. Call it before the node of `id` is created.
pub(crate) fn register_parent(ctx: &Context, id: Id, parent: Id) {
    ctx.register_accesskit_parent(id, parent);
    with_emitted(ctx, |emitted| emitted.emit(id));
}

/// Notes that the node of the text widget `widget` folds into the node of `owner`.
fn note_folded(ctx: &Context, widget: Id, owner: Id) {
    with_emitted(ctx, |emitted| {
        emitted.folded.insert(widget, owner);
    });
}

/// Notes that the runtime wrote the node of `id`.
pub(crate) fn note_written(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| {
        emitted.written.insert(id);
    });
}

/// The pass-end step: the children of each node the runtime wrote follow the order the runtime emitted them in,
/// which is reading order. Children the runtime never emitted follow them in egui's order. A folded text widget
/// that holds key focus reports its owner's node as focused, unless the runtime reported another node this pass
/// (the active item of a list the field drives).
pub(crate) fn order_children(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let emitted = Service::<Reported>::run(ctx, |reported| reported.0.remove(&viewport)).unwrap_or_default();
    if !emitted.focus_set
        && let Some(owner) = ctx.memory(|memory| memory.focused()).and_then(|widget| emitted.folded.get(&widget))
    {
        ctx.set_accesskit_focus(*owner);
    }
    hide_behind_modal(ctx, viewport, &emitted.written);
    for parent in emitted.written {
        ctx.accesskit_node_builder(parent, |node| {
            let mut children = node.children().to_vec();
            children.sort_by_key(|id| emitted.order.get(&id.0).copied().unwrap_or(usize::MAX));
            node.set_children(children);
        });
    }
}

/// While a modal layer is open in `viewport`, hides the nodes the runtime wrote on layers below it: what sits behind
/// a modal is inert for assistive technology, as it is for the pointer and the keys.
fn hide_behind_modal(ctx: &Context, viewport: ViewportId, written: &HashSet<Id>) {
    if crate::runtime::commands::modal_root(ctx, viewport).is_none() {
        return;
    }
    for id in written {
        let behind = ctx.read_response(*id).is_some_and(|response| !ctx.memory(|memory| memory.is_above_modal_layer(response.layer_id)));
        if behind {
            hide(ctx, *id);
        }
    }
}

/// The tree update of a finished frame.
#[cfg(feature = "testing")]
pub(crate) fn tree_of(output: &mut FullOutput) -> Option<AccessTree> {
    output.platform_output.accesskit_update.take().map(AccessTree::new)
}
