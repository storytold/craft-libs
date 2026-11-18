//! AccessKit node building, `register_accesskit_parent` and `set_accesskit_focus`: the one place that names
//! AccessKit's node API. `node` writes a decided node, `patch` applies the label scope and the status node, `tree`
//! reads a finished tree update, which automation keeps per viewport (`keep_trees`). What a node says is decided in
//! `runtime::access`.

mod node;
mod patch;
mod roles;
mod text;
mod tree;

use crate::runtime::pass_state;
use egui::{Context, FullOutput, Id, IdMap, IdSet, ViewportId};
use std::collections::HashMap;

use super::pass;

pub(crate) use node::{NodeAction, NodeSpec, Toggle, write_node};
pub(crate) use patch::{write_label_patch, write_merge_patch, write_status};
pub(crate) use text::{
    NodeEnd, RunLink, RunNode, TextNode, fold_widget, is_text_request, requested_selection, requested_value, write_run, write_selection, write_text,
};
pub use tree::{AccessTree, Declared, NodeKey};

/// The rect `id`'s response had in the last finished pass, converted as the node's bounds are; `None` when absent,
/// without area, or behind the open modal. The ids it answers are component and item widget ids; parts have none.
/// It answers for the context's current viewport (the root between frames); a child window's rects are read inside
/// that window's pass.
pub fn bounds(ctx: &Context, id: Id) -> Option<egui::Rect> {
    let widget = ctx.widget_rect_last_pass(id)?;
    if behind_modal(ctx, ctx.viewport_id(), widget.layer_id) {
        return None;
    }
    let rect = widget.rect;
    (rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

/// Whether a widget on `layer` sits behind the open modal of `viewport`: a modal layer is open there and `layer` is
/// not above it. The access tree and [`bounds`] both ask this, so automation never gets a rect the tree hides.
fn behind_modal(ctx: &Context, viewport: ViewportId, layer: egui::LayerId) -> bool {
    crate::runtime::commands::modal_root(ctx, viewport).is_some() && !ctx.memory(|memory| memory.is_above_modal_layer(layer))
}

/// The AccessKit id of a node, as a host test holds it.
#[cfg(feature = "testing")]
pub type NodeId = accesskit::NodeId;

/// The AccessKit id of the node of `id`.
#[cfg(feature = "testing")]
pub(crate) fn node_id(id: Id) -> NodeId {
    id.accesskit_id()
}

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
    emit(ctx, id);
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
    written: IdSet,
    /// Text widgets whose node folds into their owner's node, each with its owner.
    folded: IdMap<Id>,
    /// Whether the runtime reported a focused node this pass.
    focus_set: bool,
    /// Components that offer a context menu this pass (a `Trigger::Context` opener on them).
    context: IdSet,
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
pub(crate) struct Reported {
    emitted: HashMap<ViewportId, Emitted>,
    /// The viewport whose pass ended last. egui leaves a viewport before it runs the output hook of its pass, and a
    /// nested viewport's output hook runs before its parent's pass ends, so this names the pass an output came from.
    ended: Option<ViewportId>,
}

fn with_emitted<R>(ctx: &Context, f: impl FnOnce(&mut Emitted) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    pass_state::with::<Reported, _>(ctx, |reported| f(reported.emitted.entry(viewport).or_default()))
}

/// Makes `parent` the node `id` attaches to. Call it before the node of `id` is created.
pub(crate) fn register_parent(ctx: &Context, id: Id, parent: Id) {
    ctx.register_accesskit_parent(id, parent);
    emit(ctx, id);
}

/// Notes that the runtime emitted the node of `id`.
fn emit(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| emitted.emit(id));
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| emit(ctx, id));
}

/// Notes that the node of the text widget `widget` folds into the node of `owner`.
fn note_folded(ctx: &Context, widget: Id, owner: Id) {
    with_emitted(ctx, |emitted| {
        emitted.folded.insert(widget, owner);
    });
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| note_folded(ctx, widget, owner));
}

/// Notes that the runtime wrote the node of `id`.
pub(crate) fn note_written(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| {
        emitted.written.insert(id);
    });
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| note_written(ctx, id));
}

/// The pass-end step: the children of each node the runtime wrote follow the order the runtime emitted them in,
/// which is reading order. Children the runtime never emitted follow them in egui's order. A folded text widget
/// that holds key focus reports its owner's node as focused, unless the runtime reported another node this pass
/// (the active item of a list the field drives).
pub(crate) fn order_children(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let emitted = pass_state::with::<Reported, _>(ctx, |reported| {
        reported.ended = Some(viewport);
        reported.emitted.remove(&viewport)
    })
    .unwrap_or_default();
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
fn hide_behind_modal(ctx: &Context, viewport: ViewportId, written: &IdSet) {
    // Without a modal nothing is behind one; this spares reading every written node's response.
    if crate::runtime::commands::modal_root(ctx, viewport).is_none() {
        return;
    }
    for id in written {
        let behind = ctx.read_response(*id).is_some_and(|response| behind_modal(ctx, viewport, response.layer_id));
        if behind {
            hide(ctx, *id);
        }
    }
}

/// Records that the component `id` has a `Trigger::Context` opener in this pass, so its node declares the context
/// menu action. An opener that runs after the component reported (a context menu hung on its response after `show`)
/// adds the action to the node written already: only the node's own write clears its actions, and that ran.
pub(crate) fn offer_context_menu(ctx: &Context, id: Id) {
    let written = with_emitted(ctx, |emitted| {
        emitted.context.insert(id);
        emitted.written.contains(&id)
    });
    if written {
        ctx.accesskit_node_builder(id, |node| node.add_action(accesskit::Action::ShowContextMenu));
    }
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| offer_context_menu(ctx, id));
}

/// Whether `id` has a `Trigger::Context` opener recorded in this pass.
pub(crate) fn offers_context_menu(ctx: &Context, id: Id) -> bool {
    with_emitted(ctx, |emitted| emitted.context.contains(&id))
}

/// The AccessKit trees automation reads ([`keep_trees`], [`kept_tree`]): an observation of finished output, kept
/// apart from the pass state, which no pass reads.
#[derive(Default)]
struct KeptTrees {
    /// Whether trees are kept: automation asked for them.
    on: bool,
    /// Each viewport's tree update from its last presented pass.
    trees: HashMap<ViewportId, accesskit::TreeUpdate>,
}

fn with_kept<R>(ctx: &Context, f: impl FnOnce(&mut KeptTrees) -> R) -> R {
    super::memory::Service::<KeptTrees>::run(ctx, f)
}

/// Turns AccessKit output on and keeps every viewport's tree from then on. The trees of closed viewports go in the
/// frame loop's prune step.
pub(crate) fn keep_trees(ctx: &Context) {
    let first = with_kept(ctx, |kept| !std::mem::replace(&mut kept.on, true));
    if first {
        crate::runtime::frame::register_prune(ctx, std::any::TypeId::of::<KeptTrees>(), prune_kept);
    }
    ctx.enable_accesskit();
}

fn prune_kept(ctx: &Context, live: &[ViewportId]) {
    with_kept(ctx, |kept| kept.trees.retain(|viewport, _| live.contains(viewport)));
}

/// The output hook's step: keeps a copy of the tree update `output` carries, for the viewport whose pass made it,
/// while [`keep_trees`] is on. The update itself goes on to the platform untouched. A frame that reruns a pass keeps
/// the last pass's tree, the one presented.
pub(crate) fn keep_output(ctx: &Context, output: &FullOutput) {
    // AccessKit off: no update, and nothing to look up.
    let Some(update) = &output.platform_output.accesskit_update else { return };
    let Some(viewport) = pass_state::with::<Reported, _>(ctx, |reported| reported.ended) else { return };
    with_kept(ctx, |kept| {
        if kept.on {
            kept.trees.insert(viewport, update.clone());
        }
    });
}

/// The tree [`keep_trees`] kept for `viewport`; `None` before its first presented pass or while trees are not kept.
pub(crate) fn kept_tree(ctx: &Context, viewport: ViewportId) -> Option<AccessTree> {
    with_kept(ctx, |kept| kept.trees.get(&viewport).cloned()).map(AccessTree::new)
}

/// The tree update of a finished frame.
#[cfg(feature = "testing")]
pub(crate) fn tree_of(output: &mut FullOutput) -> Option<AccessTree> {
    output.platform_output.accesskit_update.take().map(AccessTree::new)
}
