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

use super::{memory, pass};

pub use accesskit::TreeUpdate;
pub(crate) use node::{NodeAction, NodeSpec, Toggle, write_node};
pub(crate) use patch::{write_label_patch, write_merge_patch, write_set_position, write_status};
pub(crate) use text::{
    NodeEnd, RunNode, TextNode, fold_widget, is_text_request, requested_selection, requested_value, write_run, write_selection, write_text,
};
pub use tree::{AccessTree, Declared, NodeKey, is_interactive, name};

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

/// Settles the AccessKit update's focus: when it is not among the update's nodes, focus goes to the tree root, or
/// to egui's root node when the update carries no tree info but holds that node. A platform adapter rejects an
/// update whose focus names no node, so no pass may hand one out. The runtime plugin's output hook calls it once per
/// pass.
pub(crate) fn settle_focus(output: &mut egui::FullOutput) {
    let Some(update) = output.platform_output.accesskit_update.as_mut() else { return };
    if update.nodes.iter().any(|(id, _)| *id == update.focus) {
        return;
    }
    let egui_root = egui::accesskit_root_id().accesskit_id();
    let root = update.tree.as_ref().map(|tree| tree.root).or_else(|| update.nodes.iter().any(|(id, _)| *id == egui_root).then_some(egui_root));
    if let Some(root) = root {
        update.focus = root;
    }
}

/// The settled update of each viewport's last pass, the host fact `WindowHost::access_tree` reads.
#[derive(Default)]
struct Settled {
    /// The viewports whose pass is ending, the innermost last. egui calls the output hook after it has left the
    /// pass, when the running viewport is the parent's again, so the hook takes the viewport the pass-end step
    /// recorded. A nested viewport's pass ends, hook included, inside its parent's, which keeps the order a stack.
    ending: Vec<ViewportId>,
    /// One update per viewport, replaced by each of its passes.
    trees: HashMap<ViewportId, accesskit::TreeUpdate>,
}

/// Records that the running viewport's pass is ending, for the output hook that follows it ([`settle_output`]).
pub(crate) fn note_pass_end(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    memory::with::<Settled, _>(ctx, |settled| settled.ending.push(viewport));
}

/// The output step of a pass: settles the update's focus ([`settle_focus`]) and records the settled update as the
/// ending viewport's, or forgets that viewport's update while AccessKit is off.
pub(crate) fn settle_output(ctx: &Context, output: &mut egui::FullOutput) {
    settle_focus(output);
    // A closed window never runs again to replace or drop its entry, so the begin hook prunes the store to the viewports
    // that are open as a frame begins.
    crate::runtime::frame::register_prune(ctx, std::any::TypeId::of::<Settled>(), prune_settled);
    let update = output.platform_output.accesskit_update.clone();
    memory::with::<Settled, _>(ctx, |settled| {
        let Some(viewport) = settled.ending.pop() else { return };
        match update {
            Some(update) => {
                settled.trees.insert(viewport, update);
            }
            None => {
                settled.trees.remove(&viewport);
            }
        }
    });
}

/// Drops the settled updates of the viewports that are no longer open.
fn prune_settled(ctx: &Context, live: &[ViewportId]) {
    memory::with::<Settled, _>(ctx, |settled| settled.trees.retain(|viewport, _| live.contains(viewport)));
}

/// The settled update of `viewport`'s last pass; `None` while AccessKit is off or before the viewport has run.
pub(crate) fn settled_tree(ctx: &Context, viewport: ViewportId) -> Option<accesskit::TreeUpdate> {
    memory::with::<Settled, _>(ctx, |settled| settled.trees.get(&viewport).cloned())
}

/// The AccessKit id of the node of `id`.
#[cfg(feature = "testing")]
pub(crate) fn node_id(id: Id) -> NodeId {
    id.accesskit_id()
}

/// The AccessKit role of a node.
pub(crate) type NodeRole = accesskit::Role;

/// The AccessKit vocabulary the testing kit reads a tree in.
#[cfg(feature = "suite")]
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

/// Whether this pass builds AccessKit nodes: AccessKit was on when the pass began. Reads only, unlike `create_node`.
pub(crate) fn builds_nodes(ctx: &Context) -> bool {
    ctx.with_accesskit_parents(|_| ()).is_some()
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

/// Makes the node of `id` a generic container without actions: assistive technology passes over it and reports its
/// children in its place, as it does for a `Ui`'s node.
pub(crate) fn pass_through(ctx: &Context, id: Id) {
    crate::runtime::egui::widgets::note_node(ctx, id);
    ctx.accesskit_node_builder(id, |node| {
        node.set_role(accesskit::Role::GenericContainer);
        node.clear_actions();
    });
}

/// Marks the node of `id` modal, when the pass has written it.
pub(crate) fn set_modal(ctx: &Context, id: Id) {
    if ctx.has_accesskit_node(id) {
        ctx.accesskit_node_builder(id, accesskit::Node::set_modal);
    }
}

/// Reports `id` as the focused node of this pass. A replay of the run that reported it reports it again: a view that
/// holds focus replays while nothing reaches it.
pub(crate) fn set_focus(ctx: &Context, id: Id) {
    crate::runtime::replay::journal(ctx, || id, |ctx, &id| set_focus(ctx, id));
    with_emitted(ctx, |emitted| emitted.focus = Some(id));
    ctx.set_accesskit_focus(id);
}

/// The node of the last finished pass of `viewport` that holds key focus.
#[derive(Clone, Copy)]
struct FocusedNode(Id);

fn focused_node_key(viewport: ViewportId) -> Id {
    memory::unique(("craft access focused node", viewport))
}

/// The node that held key focus at the end of the running viewport's last finished pass, the one reported to
/// assistive technology: a focused part's node ([`sub_node_id`](crate::runtime::access::sub_node_id)), the shown item
/// of a list that holds focus, else the focused component's own; `None` when nothing holds focus.
pub fn focused_node(ctx: &Context) -> Option<Id> {
    let viewport = pass::current_viewport(ctx);
    memory::peek_copy::<FocusedNode>(ctx, focused_node_key(viewport)).map(|node| node.0)
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
    /// The focused node the runtime reported this pass.
    focus: Option<Id>,
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

/// The trials' pass-end check that egui's AccessKit parents of the pass hold no cycle; panics naming the ids of the
/// first one found. egui places a node under the first ancestor that has a node by walking the parents, which a cycle
/// sends into endless recursion when no node of the cycle exists yet: a replay adds a run's nodes in the order they
/// were created, and a run only tolerates a cycle while some node of it already exists.
#[cfg(feature = "testing")]
pub(crate) fn check_parents_acyclic(ctx: &Context) {
    let cycle = ctx.with_accesskit_parents(|parents| {
        let mut acyclic = IdSet::default();
        for &start in parents.keys() {
            let mut path: Vec<Id> = Vec::new();
            let mut on_path = IdSet::default();
            let mut at = Some(start);
            while let Some(id) = at.filter(|id| !acyclic.contains(id)) {
                if !on_path.insert(id) {
                    let from = path.iter().position(|on| *on == id).unwrap_or_default();
                    return Some(path.split_off(from));
                }
                path.push(id);
                at = parents.get(&id).copied();
            }
            acyclic.extend(path);
        }
        None
    });
    let cycle = cycle.flatten();
    assert!(
        cycle.is_none(),
        "the AccessKit parents of the pass hold a cycle: {cycle:?}, each the parent of the one before and the first the parent of the last"
    );
}

/// Notes that the runtime emitted the node of `id`.
fn emit(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| emitted.emit(id));
    crate::runtime::replay::journal(ctx, || id, |ctx, &id| emit(ctx, id));
}

/// Notes that the node of the text widget `widget` folds into the node of `owner`.
fn note_folded(ctx: &Context, widget: Id, owner: Id) {
    with_emitted(ctx, |emitted| {
        emitted.folded.insert(widget, owner);
    });
    crate::runtime::replay::journal(ctx, || (widget, owner), |ctx, &(widget, owner)| note_folded(ctx, widget, owner));
}

/// Notes that the runtime wrote the node of `id`.
pub(crate) fn note_written(ctx: &Context, id: Id) {
    with_emitted(ctx, |emitted| {
        emitted.written.insert(id);
    });
    crate::runtime::replay::journal(ctx, || id, |ctx, &id| note_written(ctx, id));
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
    let widget = ctx.memory(|memory| memory.focused());
    let folded = widget.and_then(|widget| emitted.folded.get(&widget).copied());
    if emitted.focus.is_none()
        && let Some(owner) = folded
    {
        ctx.set_accesskit_focus(owner);
    }
    let focused = widget.map(|widget| emitted.focus.or(folded).unwrap_or(widget));
    match focused {
        Some(node) => memory::put(ctx, focused_node_key(viewport), FocusedNode(node)),
        None => drop(memory::take::<FocusedNode>(ctx, focused_node_key(viewport))),
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
    crate::runtime::replay::journal(ctx, || id, |ctx, &id| offer_context_menu(ctx, id));
}

/// Whether `id` has a `Trigger::Context` opener recorded in this pass.
pub(crate) fn offers_context_menu(ctx: &Context, id: Id) -> bool {
    with_emitted(ctx, |emitted| emitted.context.contains(&id))
}

/// Turns AccessKit output on, so every viewport's pass settles an update ([`settle_output`]) that automation reads
/// ([`kept_tree`]).
pub(crate) fn keep_trees(ctx: &Context) {
    ctx.enable_accesskit();
}

/// The settled update of `viewport`'s last pass as a tree to walk; `None` before its first pass or while AccessKit is
/// off.
pub(crate) fn kept_tree(ctx: &Context, viewport: ViewportId) -> Option<AccessTree> {
    settled_tree(ctx, viewport).map(AccessTree::new)
}

/// The tree update of a finished frame.
#[cfg(feature = "testing")]
pub(crate) fn tree_of(output: &mut FullOutput) -> Option<AccessTree> {
    output.platform_output.accesskit_update.take().map(AccessTree::new)
}

/// Drops relations that name a node this update does not carry.
///
/// A menu reports its popup open on a frame where the popup's own node is not emitted (the ordering
/// differs per menu), and `runtime::access::emit` puts that id in `controls`. A host's UIA adapter
/// unwraps every `controls` lookup (`accesskit_consumer`'s `Node::controls`), so one id with no node
/// in the same update aborts the app: photocraft#15597, where clicking Image, Edit or Layer — and on
/// one machine Filter — closed PhotoCraft on the click that opened the menu.
pub(crate) fn drop_dangling_relations(output: &mut FullOutput) {
    let Some(update) = output.platform_output.accesskit_update.as_mut() else { return };
    prune_controls(update);
}

/// The step itself: every `controls` id of `update` that names no node of `update` is dropped. Other
/// relations keep their ids: they are built from nodes this pass wrote.
pub fn prune_controls(update: &mut egui::accesskit::TreeUpdate) {
    let ids: std::collections::HashSet<egui::accesskit::NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
    for (_, node) in update.nodes.iter_mut() {
        if node.controls().iter().all(|id| ids.contains(id)) {
            continue;
        }
        let kept: Vec<egui::accesskit::NodeId> = node.controls().iter().copied().filter(|id| ids.contains(id)).collect();
        node.set_controls(kept);
    }
}
