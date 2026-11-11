//! The generic shell of the pipeline (1.6): one `run` calls the 17 steps for every shape. What differs by shape
//! (the body step, the item step, the size of a body or collection) goes through `ShapeHooks`.

use super::cx::PartsCx;
use super::steps::{self, MeasureIn, PartsStep, ShapeKind, Step, StopsIn, Trace};
use super::{
    Activation, BodyCx, Collection, Common, Component, Container, Cx, CxCore, Internal, Meta, OverlayUi, PaintUi, Resolved, Show, TipFn, Validity,
    ValueOf,
};
use super::{Inner, ItemFn};
use super::{overlay, tooltip};
use crate::layout::InFlow;
use crate::primitives::sensing::{self, MemberSpec, PartDecl, RootSpec};
use crate::primitives::{FaceRecord, PaintRecord, PartResult, face_id};
use crate::runtime::access::{self, Politeness, Role, SetPosition, scope};
use crate::runtime::env::{AppValues, Env, EnvView};
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::CloseReason;
use crate::runtime::reads::{self, Sample};
use crate::runtime::{commands, focus, inspect, keys, memory};
use crate::style::{ComponentStyle as _, Ink, StateSet, StyleCx};
use craft_behavior::{Flow, Input, ItemSource, KeyRole, NavMove, PartRef, PressState, Proposal, Roving, Sense};
use craft_theme::ControlSize;
use egui::{Context, Id, Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Where a component is placed.
#[derive(Clone, Copy)]
pub(crate) enum ShowAt {
    Flow,
    Rect(Rect),
}

/// The pass a host runs: the `Ui` it runs in, the placement that `Ui` imposes and the env it runs under.
pub(crate) type HostPass<'a> = dyn FnMut(&mut Ui, Option<ShowAt>, Option<&Env>) + 'a;

/// Where a component's steps 6 to 16 run: calls `pass` with the `Ui` they run in and the placement that `Ui`
/// imposes (`None` keeps the caller's), and the env it runs under when that is not the caller's (a viewport's own
/// window facts). A host that opens a new egui layer or viewport installs `env` there. F5 adds the layer and
/// viewport hosts, which call `pass` once per child pass.
pub(crate) trait Host {
    fn run(&mut self, id: Id, parent: &mut Ui, env: &Env, pass: &mut HostPass<'_>);

    /// Why the host asks the component to close, decided after its passes ran.
    fn dismissed(&self) -> Option<CloseReason> {
        None
    }
}

/// What a shape's `host` hook reads to choose a host.
pub(crate) struct HostCx<'a> {
    /// The parent `Ui`.
    pub ui: &'a Ui,
    pub env: &'a Env,
    pub id: Id,
    /// The component's label after step 1: a window's title.
    pub label: &'a crate::runtime::env::Label,
    /// The size step 5 measured: what the root takes in the parent `Ui`.
    pub size: Vec2,
}

/// The parent `Ui` is the host `Ui`.
pub(super) struct Inline;

impl Host for Inline {
    fn run(&mut self, _: Id, parent: &mut Ui, _: &Env, pass: &mut HostPass<'_>) {
        pass(parent, None, None);
    }
}

/// Where a measuring run measures: this pass's inputs only, never what an earlier pass recorded.
pub(crate) struct Measuring<'a, 's> {
    /// The parent `Ui`: the measuring run lays out in an invisible sizing pass of its own over this `Ui`'s layer.
    pub ui: &'a Ui,
    /// Where the room comes from, as at step 5.
    pub at: ShowAt,
    /// The width a `Proposal::Width` sets: the room is that wide and unbounded in height.
    pub width: Option<f32>,
    pub env: &'a Env,
    pub scx: &'a StyleCx<'s>,
    pub resolved: &'a Resolved<'s>,
    pub command: Option<Arc<crate::runtime::commands::ResolvedCommand>>,
    /// Whether the binding is mixed, as step 4 found it.
    pub mixed: bool,
}

/// What differs by shape. The item step grows through the defaulted hooks at steps 4, 7, 8, 9, 11 and 17 (F6), and
/// the body step through `content` and `host` (F5).
pub(crate) trait ShapeHooks<C: Component> {
    fn shape(&self) -> ShapeKind;

    /// What the shape's content took in this real run besides what its components measured, by id: content that
    /// allocates without a component (a raw egui allocation) changes this.
    fn regions(&self, _id: Id) -> Vec<(Id, Vec2)> {
        Vec::new()
    }

    /// Whether the shape takes room in its parent's flow: a layer or window mount does not.
    fn in_flow(&self, _comp: &C) -> bool {
        true
    }

    /// Whether the shape's own `parts` hook can declare parts, which the component's `PARTS = false` does not cover.
    fn declares_parts(&self) -> bool {
        true
    }

    /// Where steps 6 to 16 run.
    fn host(&mut self, comp: &C, at: HostCx<'_>) -> Box<dyn Host>;

    /// The host asked the component to close after its passes (a dismissal rule of the layer fired).
    fn dismissed(&mut self, _comp: &mut C, _cx: &mut Cx<'_, C>, _why: CloseReason) {}

    /// The focus scope pushed at step 6 and popped after step 16.
    fn focus(&self, _comp: &C) -> Option<crate::runtime::focus::FocusPolicy> {
        None
    }

    /// The size step 6 allocates, from the measured `size`: a body that fills reads the available rect of `ui`: the
    /// parent's at step 5, and the host's at step 6 when the host runs the pass in a `Ui` of its own.
    fn sized(&self, _comp: &C, _ui: &Ui, _id: Id, size: Vec2) -> Vec2 {
        size
    }

    /// Whether the root takes all the width the flow offers: in a content-sized column it then counts at its
    /// measured width and never sets the column's.
    fn fills_width(&self, comp: &C) -> bool {
        comp.fills() == Some(craft_behavior::Axis::X)
    }

    /// Whether the root is a line itself, which sits at the centre of a `Row`'s line, rather than a block or column,
    /// which sits at its top. A leaf is a control on the line.
    fn on_line(&self, _comp: &C) -> bool {
        true
    }

    /// Whether the root fills the width alone: a root that fills both axes takes the room of a `Content` grid column
    /// and sets its width.
    fn fills_width_only(&self, comp: &C) -> bool {
        comp.fills() == Some(craft_behavior::Axis::X)
    }

    /// The root's sense, from the component's own `own`.
    fn sense(&self, _comp: &C, own: Sense) -> Sense {
        own
    }

    /// Whether the root is an arena root: its sense covers its members.
    fn arena(&self, _comp: &C) -> bool {
        false
    }

    /// What the focused stop is, as far as the key table's rows differ.
    fn key_role(&self, _comp: &C) -> KeyRole {
        KeyRole::new()
    }

    /// The roving rules of the body's members, when the shape's body forms one group.
    fn roving(&self, _comp: &C) -> Option<Roving> {
        None
    }

    /// Whether a collapsible container is open, reported as its node's expanded state; `None` for a shape that does
    /// not collapse.
    fn expanded(&self, _comp: &C) -> Option<bool> {
        None
    }

    /// The politeness of a live region the shape's body is.
    fn live(&self, _comp: &C) -> Option<Politeness> {
        None
    }

    /// The position the shape's component reports among its siblings, when it knows one (a reorderable section).
    fn position(&self, _comp: &C) -> Option<SetPosition> {
        None
    }

    /// The axes the shape's body scrolls on, vertical then horizontal.
    fn scrolls(&self, _comp: &C) -> (bool, bool) {
        (false, false)
    }

    /// What the shape adds to the content size at step 5, before padding and `min_size`.
    fn extent(&mut self, _comp: &C, _ui: &Ui, _at: ShowAt, _id: Id, _scx: &StyleCx<'_>) -> Vec2 {
        Vec2::ZERO
    }

    /// What the shape adds to the content size in a measuring run (a sizing pass, or `measure_in`), from this pass's
    /// inputs only: a body runs its regions in an invisible sizing pass and measures what they took. The default is
    /// `extent`.
    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        self.extent(comp, at.ui, at.at, cx.core.id, at.scx)
    }

    /// A real pass's step 5 for a body or an item shape: what the shape adds to the content size, from a measuring run
    /// of its contents in this pass (or the record one left in this pass). The default is `extent`.
    fn pass_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, _style: &C::Style, _shown: Option<Arc<ValueOf<C>>>) -> Vec2 {
        self.extent(comp, at.ui, at.at, at.resolved.id, at.scx)
    }

    /// The width by which the shape's content wants to be wider than the size it measured to, because the room cut
    /// it: the parent `Ui` counts it in the size it took, so a parent that sizes from its content widens.
    fn overflow(&self) -> f32 {
        0.0
    }

    /// The rows this pass places, for `PartsCx::rect_of`; computed before the component's `parts` runs.
    fn geometry(&mut self, _comp: &C, _content: Rect, _rtl: bool) -> Option<std::rc::Rc<dyn std::any::Any>> {
        None
    }

    /// Step 4: the shape's part of the value step (selection snapshot, scroll offset).
    fn read(&mut self, _comp: &mut C, _stage: &mut MemoryStage, _id: Id, _env: &Env) {}

    /// The command scope the component is an instance of: its own `Cx` and the items below see it in the chain.
    fn instance_scope(&self, _comp: &C) -> Option<commands::CommandScope> {
        None
    }

    /// Step 7: the parts the shape declares itself (scroll parts, the item layout).
    fn parts(&mut self, _comp: &mut C, _rect: Rect, _pcx: &mut PartsCx<'_, C>) {}

    /// Step 8, after the pass fields of `Cx` are set: the item under a press.
    fn sensed(&mut self, _comp: &mut C, _cx: &mut Cx<'_, C>) {}

    /// Step 9, after `C::update`: navigation, selection, type-ahead and drops.
    fn updated(&mut self, _comp: &mut C, _cx: &mut Cx<'_, C>) {}

    /// Step 11: the selection change applied to its binding.
    fn apply(&mut self, _comp: &mut C, _cx: &mut Cx<'_, C>) {}

    /// Step 14: the body or the items.
    fn content(&mut self, comp: &mut C, ui: &mut Ui, env: &Env, cx: &mut Cx<'_, C>, values: &AppValues);

    /// Step 15, before the value events are applied: open, gate and scroll changes go to their owners.
    fn settle(&mut self, _comp: &mut C, _cx: &mut Cx<'_, C>) {}

    /// Step 17: what the shape records for next frame.
    fn record(&mut self, _stage: &mut MemoryStage, _ctx: &Context, _id: Id) {}
}

/// The leaf shape; `scope` is the command scope the leaf is an instance of, when its caller named one.
struct LeafHooks {
    scope: Option<commands::CommandScope>,
}

impl<C: Component> ShapeHooks<C> for LeafHooks {
    fn shape(&self) -> ShapeKind {
        ShapeKind::Leaf
    }

    fn declares_parts(&self) -> bool {
        false
    }

    fn host(&mut self, _: &C, _: HostCx<'_>) -> Box<dyn Host> {
        Box::new(Inline)
    }

    fn content(&mut self, _: &mut C, _: &mut Ui, _: &Env, _: &mut Cx<'_, C>, _: &AppValues) {}

    fn read(&mut self, _: &mut C, stage: &mut MemoryStage, id: Id, _: &Env) {
        if let Some(scope) = self.scope.filter(|_| !stage.is_peek()) {
            commands::register_instance(stage.ctx(), scope, id);
        }
    }

    fn instance_scope(&self, _: &C) -> Option<commands::CommandScope> {
        self.scope
    }

    fn sized(&self, comp: &C, ui: &Ui, id: Id, size: Vec2) -> Vec2 {
        let room = crate::layout::flow_fill_room(ui);
        // The line height of a content-height row is not known inside `sized` (the other children have not run), so a
        // child that fills across it keeps its measured size there.
        let content = Env::of(ui).content;
        let fill = |measured: f32, offered: f32, content: bool, along: craft_behavior::Axis| {
            if !offered.is_finite() || content {
                return measured;
            }
            match crate::layout::main_fill(ui, id, along) {
                crate::layout::MainFill::Measuring => 0.0,
                crate::layout::MainFill::Share(share) => share,
                crate::layout::MainFill::Room => measured.max(offered),
            }
        };
        match comp.fills() {
            Some(craft_behavior::Axis::X) => Vec2::new(fill(size.x, room.x, content.0, craft_behavior::Axis::X), size.y),
            Some(craft_behavior::Axis::Y) => Vec2::new(size.x, fill(size.y, room.y, content.1, craft_behavior::Axis::Y)),
            _ => size,
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "one argument per field of the context the pipeline builds once per show")]
fn blank_core<'a>(
    env: EnvView<'a>,
    resolved: &'a Resolved<'a>,
    memory: &'a mut MemoryStage,
    ctx: Context,
    ink: Ink,
    response: Response,
    command: Option<Arc<crate::runtime::commands::ResolvedCommand>>,
) -> CxCore<'a> {
    let rect = Rect::NOTHING;
    CxCore {
        env,
        resolved,
        id: resolved.id,
        rect,
        content_rect: rect,
        visible: rect,
        flow: Flow::new(rect, false),
        response,
        command,
        state: StateSet::default(),
        stated: false,
        ink,
        intents: keys::Intents::default(),
        memory,
        ctx,
        paint: RefCell::new(PaintRecord::default()),
        results: HashMap::new(),
        face_mode: false,
        active: None,
        focus: Default::default(),
        member_of: None,
        activation: None,
        injected: None,
        keep_open: false,
        part_clicked: false,
        collection_selection: None,
        activated_member: None,
        press: PressState::IDLE,
        cursor: None,
        part_cursors: Vec::new(),
        claims: Vec::new(),
        reveal: None,
        press_cancel: None,
        scroll_moves: Vec::new(),
        value_parts: Vec::new(),
        own_reset_parts: Vec::new(),
        toggles: Vec::new(),
        reset_part: None,
        root_sense: Sense::empty(),
        roving_arrival: None,
        roving_lead: None,
        collection_events: None,
        visible_rows: 0..0,
        gestures: Default::default(),
        texts: None,
        surface_slot: None,
        editor: None,
        editor_inputs: Vec::new(),
        text_invalid: false,
        grid: crate::runtime::access::GridPlace::default(),
        escape_to_list: false,
        text_access: std::cell::RefCell::new(None),
        value_text: std::cell::RefCell::new(None),
        submitted: false,
        escaped: false,
    }
}

fn paint_ui<'a, C: Component>(ui: &'a mut Ui, env: &'a Env, cx: &'a Cx<'_, C>) -> PaintUi<'a> {
    let focus = cx.core.active.map(|part| (part, cx.core.state.flags.contains(crate::style::Flags::FOCUS_VISIBLE)));
    PaintUi {
        ui,
        env,
        ink: cx.core.ink,
        id: cx.core.id,
        rect: cx.core.rect,
        record: &cx.core.paint,
        results: &cx.core.results,
        response: &cx.core.response,
        active: focus.filter(|_| cx.core.state.flags.contains(crate::style::Flags::FOCUSED)),
        // Only a composite with parts paints faces, so the others skip the look list.
        text: if cx.core.results.is_empty() { None } else { cx.style.texts().first().map(|text| &text.color) },
        muted: if cx.core.results.is_empty() { None } else { cx.style.texts().get(1).map(|text| &text.color) },
        flags: cx.core.state.flags & HOST_FLAGS,
        enabled: cx.core.resolved.enabled,
        size: cx.core.resolved.size,
        kids: None,
    }
}

/// A rate limit for the log of a rejected size: one line per id per second.
#[derive(Clone, Default)]
struct SizeLog(f64);

/// What steps 6 to 16 read and write besides the `Ui` the host gives. Built by steps 1 to 5 in the parent pass.
struct Pass<'r, 'a, C: Component> {
    comp: &'r mut C,
    cx: &'r mut Cx<'a, C>,
    hooks: &'r mut dyn ShapeHooks<C>,
    env: &'r Env,
    style: &'r C::Style,
    resolved: &'r Resolved<'r>,
    trace: Trace<'r>,
    tip: Option<TipFn<'r>>,
    /// The placement of the parent `Ui`, for a host that gives none.
    at: ShowAt,
    /// The width the parent offers, which step 5 and step 7's measure view both read.
    room: Option<f32>,
    /// The size step 5 measured, before `sized` made it fill.
    measured: Vec2,
    /// The size the root takes in the parent `Ui`.
    size: Vec2,
    /// The parent `Ui`: a host that runs the pass in a `Ui` of its own gives the room the root fills there.
    parent: Id,
    inert: bool,
    /// The parent `Ui` was in a sizing pass.
    sizing: bool,
    /// Set by a pass that ran past the sizing check.
    real: bool,
    activated: bool,
    /// The source of the first activation `Cx::activate` injected in the passes.
    injected: Option<commands::FireSource>,
}

/// Steps 6 to 16, in the `Ui` the host gives, inside the component's focus scope.
fn pass<C: Component>(p: &mut Pass<'_, '_, C>, ui: &mut Ui, host_at: Option<ShowAt>, host_env: Option<&Env>) {
    let scope = if p.sizing || ui.is_sizing_pass() { None } else { p.hooks.focus(p.comp) };
    let (ctx, id) = (p.cx.core.ctx.clone(), p.resolved.id);
    if let Some(policy) = scope {
        focus::push_scope(&ctx, id, policy, None);
    }
    run_in_host(p, ui, host_at, host_env);
    if scope.is_some() {
        focus::pop_scope(&ctx);
    }
}

/// A component wider than its room widens the flow `Ui` it was placed in, toward the end of the line.
fn expand_for_overflow(ui: &mut Ui, at: ShowAt, rect: Rect, overflow: f32, rtl: bool) {
    if overflow > 0.0 && matches!(at, ShowAt::Flow) {
        ui.expand_to_include_x(if rtl { rect.min.x - overflow } else { rect.max.x + overflow });
    }
}

/// What the component senses itself: a disabled one that shows a reason on focus stays a Tab stop and senses nothing else.
fn own_sense<C: Component>(hooks: &dyn ShapeHooks<C>, comp: &C, resolved: &Resolved, inert: bool) -> Sense {
    match (inert, resolved.enabled) {
        (true, _) => Sense::HOVER,
        (false, true) => hooks.sense(comp, comp.sense()),
        (false, false) if steps::takes_focus(resolved) => Sense::HOVER | comp.sense().intersection(Sense::FOCUS),
        (false, false) => Sense::HOVER,
    }
}

/// The widget that holds the component's text or editor and the keys it keeps, when it has either.
fn text_holder(id: Id, parts: &PartsStep, base: focus::KeyUse) -> (Option<Id>, Option<focus::KeyUse>) {
    let text = parts
        .text
        .decl
        .as_ref()
        .map(|decl| (crate::primitives::text_adapter::text_id(id, decl.key), crate::primitives::text_adapter::keys_of(base, &decl.options)));
    let editor =
        parts.text.editor.as_ref().map(|decl| (crate::primitives::ImeSurface::of(id, decl.key).widget(), super::editor::keys_of(base, decl.purpose)));
    match (text, editor) {
        (Some((widget, keys)), _) | (None, Some((widget, keys))) => (Some(widget), Some(keys)),
        (None, None) => (None, None),
    }
}

/// What the stops of a component read: the owner, its place in a member group and the text it holds.
struct StopsStep<'a> {
    ctx: &'a Context,
    id: Id,
    rtl: bool,
    env: &'a Env,
    resolved: &'a Resolved<'a>,
    inert: bool,
    holder_widget: Option<Id>,
    member: Option<crate::runtime::env::ArenaId>,
    own: Sense,
    text_keys: Option<focus::KeyUse>,
}

/// Registers the focus stops of a component that has any; one that sets `STOPS = false` has none, which a debug build checks.
fn stops_step<C: Component>(at: StopsStep<'_>, hooks: &dyn ShapeHooks<C>, comp: &mut C, parts: &PartsStep) {
    let StopsStep { ctx, id, rtl, env, resolved, inert, holder_widget, member, own, text_keys } = at;
    let body_roving = hooks.roving(comp);
    if !C::STOPS && body_roving.is_none() {
        debug_assert!(
            !own.contains(Sense::FOCUS) && parts.decls.iter().all(|(_, extras)| !extras.focusable),
            "`{}` sets `STOPS = false` and has a stop",
            std::any::type_name::<C>()
        );
        return;
    }
    let resettable = !matches!(C::INPUT, Input::Text | Input::Editor) && comp.bind().is_some_and(|bind| bind.reset_value().is_some());
    let text = holder_widget.is_some();
    let keys = text_keys.unwrap_or_else(|| comp.keys());
    let role = hooks.key_role(comp).nudge(keys.nudge);
    // A bound component that keeps the arrows reads them as value steps; a text value sets `steps_value` in its text options.
    let arrows = keys.keep == crate::runtime::focus::KeepKeys::BlockArrows;
    let value = role.value || keys.value || (comp.bind().is_some() && !role.nudge && !text && arrows);
    steps::register_stops(&StopsIn {
        ctx,
        id,
        holder: holder_widget.unwrap_or_else(|| member.map_or(id, |arena| arena.0)),
        focusable: !inert && steps::takes_focus(resolved) && own.contains(Sense::FOCUS) && env.item != Some(id),
        parts_focus: !inert && steps::takes_focus(resolved),
        text,
        keys,
        role: role.resettable(resettable).value(value).text(text).moves(role.moves || keys.moves).deletes(role.deletes || keys.deletes || text),
        rtl,
        parts,
        group: steps::member_group(ctx, env.roving, id),
        body: env.roving,
        row: env.row,
        item: env.item == Some(id),
        default_stop: resolved.default_stop,
    });
}

/// The key and index of every declared part whose extras `has` accepts.
fn declared_with(parts: &PartsStep, has: impl Fn(&super::cx::PartExtras) -> bool) -> Vec<(craft_behavior::PartKey, Option<u64>)> {
    parts.decls.iter().filter(|(_, extras)| has(extras)).map(|(decl, _)| (decl.key, decl.index)).collect()
}

/// The activation this pass: none for a component that takes none; a key or AccessKit activation of a member
/// (`keyed`); else the click, or the release of a press that opened a popup and ended over this row.
fn activation_of(ctx: &Context, ui: &Ui, rect: Rect, resolved: &Resolved<'_>, keyed: bool, root_sense: Sense, clicked: bool) -> Option<Activation> {
    if !steps::takes_activation(resolved) {
        return None;
    }
    if keyed {
        return Some(steps::key_activation(ctx));
    }
    // A release over the part clipped out of a scroll view is on whatever lies there, not on the part.
    let released = root_sense.contains(Sense::CLICK)
        && crate::runtime::overlay::released_inside(
            ctx,
            crate::runtime::egui::areas::layer_to_screen(ctx, ui.layer_id()) * rect.intersect(ui.clip_rect()),
        );
    (clicked || released).then(|| Activation {
        by_key: false,
        mods: crate::runtime::pointer::with_frame(ctx, |frame, _| frame.map_or(craft_keymap::Mods::NONE, |frame| frame.mods)),
    })
}

/// Step 8: the sense of a root, a member of an arena, or a component with nothing to sense.
fn sense_root(ui: &Ui, member: Option<crate::runtime::env::ArenaId>, spec: RootSpec<'_>, owns_arena: bool) -> sensing::RootResult {
    match member {
        Some(arena) => {
            let RootSpec { id, rect, own_sense, own, enabled } = spec;
            let sensed = sensing::sense_member(ui, arena, MemberSpec { id, rect, sense: own_sense, parts: own, enabled });
            sensing::RootResult {
                response: sensed.response,
                parts: sensed.parts,
                press: sensed.press,
                activation: sensed.activated,
                activated_member: None,
                sense: Sense::empty(),
            }
        }
        None if spec.own_sense.is_empty() && spec.own.is_empty() && !owns_arena => sensing::sense_none(ui, spec.id, spec.rect),
        None => sensing::sense_root(ui, spec),
    }
}

/// Step 6: the rect the root takes in `ui`, and whether it is a fill-width root held at its measured width.
fn place_root<C: Component>(p: &Pass<'_, '_, C>, ui: &mut Ui, at: ShowAt, env: &Env, size: Vec2, inert: bool, rtl: bool) -> (Rect, bool) {
    // In a content-sized column a fill-width child takes its measured width in the flow and shows across the width
    // the column settles on, so it never sets that width.
    let line = p.hooks.on_line(&*p.comp);
    let held = !inert
        && matches!(at, ShowAt::Flow)
        && ((env.content_column && p.hooks.fills_width(&*p.comp))
            || (crate::layout::in_grid_content_column(ui) && p.hooks.fills_width_only(&*p.comp)));
    let rect = if held {
        // The column's settled width, read before the child takes its place in it. A sizing or measuring run has no
        // settled width yet: there the child shows at its own size.
        let column = (!ui.is_sizing_pass() && !crate::runtime::measure::active(ui.ctx()))
            .then(|| crate::layout::flow_room(ui).x)
            .filter(|width| width.is_finite())
            .unwrap_or(0.0);
        let flowed = steps::place(ui, at, Vec2::new(p.measured.x.min(size.x), size.y), InFlow { fill: false, line });
        let across = Vec2::new(size.x.max(flowed.width()).max(column), flowed.height());
        if rtl { Rect::from_min_size(Pos2::new(flowed.max.x - across.x, flowed.min.y), across) } else { Rect::from_min_size(flowed.min, across) }
    } else {
        let fill = p.hooks.fills_width(&*p.comp) && !p.hooks.fills_width_only(&*p.comp);
        steps::place(ui, at, if inert { Vec2::ZERO } else { size }, InFlow { fill, line })
    };
    (rect, held)
}

/// Creates the component's accessibility node, and its label scope, before any child `Ui` runs.
fn create_node<C: Component>(ctx: &Context, ui: &Ui, id: Id, member: bool) -> bool {
    matches!(C::ROLE, Role::Node(_)) && {
        if member {
            access::register_parent(ctx, id, ui.unique_id());
        }
        let created = access::create_node(ctx, id);
        if created {
            scope::enter(ctx, id);
        }
        created
    }
}

/// Steps 6 to 16 of the pass, run in the `Ui` the host gives.
fn run_in_host<C: Component>(p: &mut Pass<'_, '_, C>, ui: &mut Ui, host_at: Option<ShowAt>, host_env: Option<&Env>) {
    let ctx = p.cx.core.ctx.clone();
    let (id, rtl) = (p.resolved.id, p.env.direction.is_rtl());
    let (env, resolved, inert) = (host_env.unwrap_or(p.env), p.resolved, p.inert);
    let metrics = p.style.metrics();
    // 6 place
    let at = host_at.unwrap_or(p.at);
    // A root that fills takes the room of the `Ui` its host runs it in: a pane's or a layer's own, not its parent's.
    let size = if ui.scope_id() == p.parent { p.size } else { p.hooks.sized(&*p.comp, ui, id, p.measured) };
    let (rect, held) = place_root(p, ui, at, env, size, inert, rtl);
    p.trace.mark(Step::Place);
    // What the content wants past the room counts where the root's width comes from its content: a content-sized
    // root, or a fill-width one held at its measured width. A root that fills the flow keeps to the room it fills.
    let overflow = if held || !p.hooks.fills_width(&*p.comp) { p.hooks.overflow() } else { 0.0 };
    expand_for_overflow(ui, host_at.unwrap_or(p.at), rect, overflow, rtl);
    // The component sits where the interface puts it; a component whose axis keeps its order flows its content left to right.
    let rtl = rtl && !p.cx.core.resolved.ltr_axis;
    // A sizing pass of the parent or host `Ui` allocates the size and stops: no parts, no body, and the stage
    // opened at step 3 is written back unchanged.
    if p.sizing || ui.is_sizing_pass() {
        // The output words the value in a measuring run too, so what is measured beside the component holds it.
        *p.cx.core.value_text.borrow_mut() = p.comp.value_text(p.cx);
        return;
    }
    p.real = true;
    let (comp, cx) = (&mut *p.comp, &mut *p.cx);
    // 7 parts
    p.trace.mark(Step::Parts);
    let flow = Flow::new(rect, rtl);
    let mut parts = PartsStep { decls: Vec::new(), rovings: Vec::new(), scrolls: Vec::new(), text: super::text::TextStep::default() };
    if !inert && matches!(C::INPUT, Input::Text | Input::Editor) {
        parts.text.commit_now = focus::commit_now(&ctx) == Some(id);
    }
    // A component that declares no parts skips the step; a debug build runs it anyway to check the declaration.
    let declares_parts = C::PARTS || p.hooks.declares_parts();
    if !inert && (declares_parts || cfg!(debug_assertions)) {
        let last = *cx.core.memory.shown::<steps::LastState>(sub(id, "last_state"));
        let input = MeasureIn { ui: &*ui, env, proposal: Proposal::Ideal, room: p.room };
        let measure = steps::measure_cx::<C>(&input, p.style, resolved, cx.shown.as_deref());
        let mut pcx = PartsCx {
            content_rect: flow.inset(metrics.padding),
            flow,
            decls: &mut parts.decls,
            rovings: &mut parts.rovings,
            scrolls: &mut parts.scrolls,
            text: &mut parts.text,
            last_state: (last.0, last.1),
            measure,
            items: p.hooks.geometry(comp, flow.inset(metrics.padding), rtl),
        };
        comp.parts(rect, &mut pcx);
        p.hooks.parts(comp, rect, &mut pcx);
    }
    if C::CANVAS_CONTROL && p.trace.recording() {
        inspect::canvas_control(&ctx, std::any::type_name::<C>());
    }
    debug_assert!(declares_parts || steps::declares_none(&parts), "`{}` sets `PARTS = false` and declares a part", std::any::type_name::<C>());
    steps::check_parts(&ctx, std::any::type_name::<C>(), &parts);
    let (holder_widget, text_keys) = text_holder(id, &parts, comp.keys());
    let own = own_sense(&*p.hooks, comp, resolved, inert);
    // An item never senses the whole row for a press: the collection's root takes the click and the focus (17.2). It
    // does sense the pointer over it, so `PartsCx::hovered` and the state step read the row's hover from the same
    // arena outcome whether the item stands alone or in a collection.
    let own = if env.item == Some(id) { own.difference(Sense::CLICK | Sense::FOCUS) | Sense::HOVER } else { own };
    let arena = p.hooks.arena(comp);
    let member = if arena || matches!(C::INPUT, Input::Text | Input::Editor) { None } else { env.arena };
    stops_step::<C>(StopsStep { ctx: &ctx, id, rtl, env, resolved, inert, holder_widget, member, own, text_keys }, &*p.hooks, comp, &parts);
    // A body's wheel share is its scroll taker (body step, step 7); the gate value does not take the wheel. A body root that
    // senses the wheel itself (a canvas) registers like any other component.
    if !inert && (p.hooks.shape() != ShapeKind::Body || own.intersects(Sense::WHEEL | Sense::WHEEL_X)) {
        steps::register_wheel(&ctx, id, rect, own, comp.bind().is_some(), env.wheel_gated);
    }
    // 8 sense
    p.trace.mark(Step::Sense);
    let decls: Vec<PartDecl> = parts.decls.iter().map(|(decl, _)| decl.clone()).collect();
    let enabled = resolved.enabled && !inert;
    let sensing::RootResult { mut response, parts: results, press, activation, activated_member, sense: root_sense } =
        sense_root(ui, member, RootSpec { id, rect, own_sense: own, own: &decls, enabled }, arena);
    // A root in an arena's body that is not one of its members lies over the members shown before it, and under those
    // shown after it (8.3).
    if member.is_none()
        && enabled
        && root_sense.intersects(Sense::CLICK | Sense::DRAG)
        && let Some(outer) = env.arena
    {
        sensing::note_nested(&ctx, outer, rect.intersect(ui.clip_rect()));
    }
    // The node exists before any child `Ui` runs, so the children find their parent (12.2). A member has no egui
    // widget, so it first registers the `Ui` it sits in as its parent.
    let node = create_node::<C>(&ctx, ui, id, member.is_some());
    // A root with no node whose egui widget senses still gets a node from egui, with actions nobody writes.
    if C::ROLE == Role::None && !root_sense.is_empty() {
        access::hide(&ctx, id);
    }
    cx.core.part_cursors = sensing::hovered_cursor(&decls, &results).into_iter().collect();
    cx.core.rect = rect;
    cx.core.content_rect = flow.inset(metrics.padding);
    cx.core.flow = flow;
    cx.core.visible = rect.intersect(ui.clip_rect());
    // A read-only component takes no activation: the response reports no click either.
    if !steps::takes_activation(resolved) {
        response.flags.remove(egui::response::Flags::CLICKED | egui::response::Flags::FAKE_PRIMARY_CLICKED);
    }
    cx.core.response = response;
    // The focus service holds last pass's registration: a part that is not in this pass's results is not active.
    cx.core.focus = focus::facts(&ctx, id);
    cx.core.active = cx.core.focus.part.filter(|part| results.contains_key(&(part.key, part.index)));
    cx.core.scroll_moves = super::cx::scroll_moves(&parts.scrolls, &results);
    cx.core.value_parts = declared_with(&parts, |extras| extras.value);
    cx.core.toggles = parts
        .decls
        .iter()
        .filter_map(|(decl, extras)| {
            extras.toggles.map(|toggle| super::cx::Toggle { part: PartRef { key: decl.key, index: decl.index }, decl: toggle, open: false })
        })
        .collect();
    cx.core.own_reset_parts = declared_with(&parts, |extras| extras.own_reset);
    cx.core.results = results;
    cx.core.root_sense = root_sense;
    cx.core.press = press;
    cx.core.member_of = member.map(|arena| arena.0);
    // A key or AccessKit activation of a member reaches its arena root, which names the member.
    let keyed = member.is_some_and(|arena| keys::activated_member(&ctx, arena.0) == Some(id));
    cx.core.activation = activation_of(&ctx, ui, rect, resolved, keyed, root_sense, activation);
    cx.core.activated_member = activated_member.or_else(|| arena.then(|| keys::activated_member(&ctx, id)).flatten());
    cx.core.intents = keys::intents_of(&ctx, Some(id), rtl);
    p.hooks.sensed(comp, cx);
    if inert {
        if node {
            steps::report_inert(cx);
        }
        return;
    }
    // The popups the text part and `update` register start over for this pass.
    if C::OVERLAYS {
        overlay::begin_owner(&ctx, id);
    }
    // 8b text part
    if parts.text.decl.is_some() {
        let ink = crate::style::ink_color(&env.tokens, env.ink);
        super::text::run(cx, ui, &mut parts.text, text_keys.unwrap_or_else(|| comp.keys()), rtl, ink, env.tokens.stroke.thin);
    } else if holder_widget.is_some() {
        super::editor::run(cx, ui, &mut parts.text, text_keys.unwrap_or_else(|| comp.keys()));
    }
    // 9 update
    p.trace.mark(Step::Update);
    steps::gesture_gate(comp, cx);
    steps::update_inputs(&mut cx.core);
    overlay::drive_toggles(cx);
    steps::edit_intents(comp, cx);
    steps::scrub_by_label(comp, cx);
    steps::activate_by_label(cx);
    comp.update(cx);
    if cfg!(debug_assertions) {
        let keys = text_keys.unwrap_or_else(|| comp.keys());
        let held = |mv: NavMove| match keys.keep {
            focus::KeepKeys::BlockArrows => true,
            focus::KeepKeys::InlineArrows => matches!(mv, NavMove::Back | NavMove::Forward),
            _ => false,
        };
        debug_assert!(
            p.hooks.key_role(comp).moves || keys.moves || cx.core.intents.taken_arrow_moves().iter().all(|mv| held(*mv)),
            "`{}` takes arrow moves with `take_move` that its `KeyUse` neither keeps nor declares as `moves`",
            std::any::type_name::<C>()
        );
    }
    p.hooks.updated(comp, cx);
    steps::record_context(&cx.core);
    // The body's group registers after `update`, which may change its rules from what the component knows now.
    if let Some(roving) = p.hooks.roving(comp) {
        let group = focus::Group::new(steps::body_group(id), roving).rtl(rtl);
        let group = match cx.core.roving_lead {
            Some(lead) => group.lead((lead, None)),
            None => group,
        };
        focus::register_group(
            &ctx,
            match cx.core.roving_arrival {
                Some(arrival) => group.selected((arrival, None)),
                None => group,
            },
        );
    }
    // 10 overlay
    p.trace.mark(Step::Overlay);
    if C::OVERLAYS {
        let open = overlay::open_map(&mut *cx.core.memory, id);
        let parts = cx
            .core
            .toggles
            .iter()
            .filter_map(|toggle| cx.core.results.get(&(toggle.part.key, toggle.part.index)).map(|result| (toggle.part, result.rect)))
            .collect();
        let mut overlays = OverlayUi::new(&ctx, overlay::OverlayOwner { id, anchor: rect, parts }, &*ui, resolved, env.clone(), open);
        comp.overlay(&mut overlays, cx);
        overlay::apply_closes(&ctx, overlays.closed);
    }
    // What `update` and `overlay` left untaken goes back to the key service.
    steps::after_update(&mut cx.core);
    // 11 apply
    p.trace.mark(Step::Apply);
    let edit = steps::apply_values(comp, cx);
    let leaf = p.hooks.shape() == ShapeKind::Leaf;
    steps::rerun_after_edit(comp, cx, edit, &MeasureIn { ui: &*ui, env: p.env, proposal: Proposal::Ideal, room: p.room }, resolved, leaf);
    p.hooks.apply(comp, cx);
    steps::register_reset(comp, cx);
    // 12 state
    p.trace.mark(Step::State);
    steps::state(comp, cx, C::OVERLAYS && overlay::popup_shown(&ctx, id), p.hooks.expanded(comp) == Some(true), env.ink);
    if cx.core.state.flags.contains(crate::style::Flags::FOCUSED) {
        commands::note_focus(&ctx, id, &env.commands);
    }
    // 13 paint
    p.trace.mark(Step::Paint);
    let mut values = AppValues::default();
    comp.content_env(cx, &mut values);
    steps::paint_surface(ui, env, cx);
    // Paint and content stay inside the rect (grown by the type's declared outset), however small the room is.
    let outer_clip = ui.clip_rect();
    let inside = cx.core.rect.expand(C::paint_outset(&env.tokens)).intersect(outer_clip);
    ui.set_clip_rect(inside);
    comp.paint(&mut paint_ui(&mut *ui, env, cx), cx);
    ui.set_clip_rect(outer_clip);
    steps::paint_focus(ui, cx);
    ui.set_clip_rect(inside);
    // 14 content, then `paint_over`
    if p.trace.mark(Step::Content) {
        if C::ROLE == Role::Merged {
            scope::begin(&ctx, id, id, scope::Kind::Merge);
        }
        // A real run checks a body's content against the kept transcript its records came from; a collection's rows
        // are its own measure, not the enclosing body's, and a layer's content is not part of its parent's flow.
        let scoped = (!p.sizing && !crate::runtime::measure::active(&ctx))
            .then(|| match p.hooks.shape() {
                ShapeKind::Body => Some((Some(id), !p.hooks.in_flow(comp))),
                ShapeKind::Item => Some((None, true)),
                _ => None,
            })
            .flatten();
        if let Some((body, boundary)) = scoped {
            crate::runtime::layout_memory::open_content(&ctx, body, boundary);
        }
        p.hooks.content(comp, ui, env, cx, &values);
        if scoped.is_some() {
            crate::runtime::layout_memory::close_content(&ctx, &p.hooks.regions(id));
        }
    }
    // The `over` parts are placed before `paint_over`, so the children it shows stack above them (8.3).
    ui.set_clip_rect(outer_clip);
    sensing::place_over_widgets(ui, id, member, &decls);
    ui.set_clip_rect(inside);
    comp.paint_over(&mut paint_ui(&mut *ui, env, cx), cx);
    ui.set_clip_rect(outer_clip);
    // 15 settle
    p.trace.mark(Step::Settle);
    p.hooks.settle(comp, cx);
    let edit = steps::settle(comp, cx);
    steps::rerun_after_edit(comp, cx, edit, &MeasureIn { ui: &*ui, env: p.env, proposal: Proposal::Ideal, room: p.room }, resolved, leaf);
    // 16 report, then the tooltip.
    p.trace.mark(Step::Report);
    steps::report(
        comp,
        cx,
        &steps::ReportIn {
            node,
            decls: &parts.decls,
            sense: own,
            live: p.hooks.live(comp),
            scrolls: p.hooks.scrolls(comp),
            expanded: p.hooks.expanded(comp),
            position: p.hooks.position(comp),
            group: steps::member_group(&ctx, env.roving, id),
        },
    );
    tooltip::run(cx, tooltip::TipIn { ui: &*ui, env, rich: p.tip.take(), parts: &parts.decls, shows_label: comp.shows_label() });
    // Activations accumulate across the passes of a viewport mount; step 17 fires the bound command once, with the
    // injected source when there is one.
    let natural = comp.activated(cx) && resolved.enabled;
    let injected = cx.core.injected.take().filter(|_| resolved.enabled);
    if (natural || injected.is_some()) && !cx.core.keep_open {
        crate::runtime::overlay::note_activated(&ctx, id);
    }
    p.activated |= natural || injected.is_some();
    p.injected = p.injected.or(injected);
}

fn run<C: Component>(mut comp: C, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>, hooks: &mut dyn ShapeHooks<C>) -> C::Output {
    let ctx = ui.ctx().clone();
    // Slots the last end hook evicted drop here, outside every lock and hook.
    memory::dispose_evicted(&ctx);
    let shape = hooks.shape();
    // Steps 1 to 5 read the sizing flag of the parent `Ui`; steps 6 to 16 read the host `Ui`'s. A sizing pass runs
    // steps 1 to 6 and allocates the size: no id registration, sensing or memory writes.
    // egui's own invisible pass: it allocates and registers nothing, whether or not the framework is measuring.
    let sizing = ui.is_sizing_pass();
    // The inspector flag is read once per run.
    let recording = !sizing && inspect::recording(&ctx);
    // 1 env: `Common` moves out of the component; `Resolved` borrows these locals.
    let env = Env::of(ui);
    let common: Common = std::mem::take(comp.common_mut(Internal::new()));
    let step1 = steps::env::<C>(&ctx, &env, &common);
    // 2 style
    let scx = steps::style_cx(&ctx, &env, &step1);
    let style = steps::style::<C>(&ctx, &scx, comp.style_key());
    // 3 id, and the memory stage for it
    let id = steps::id::<C>(ui, &common.id, !sizing, true);
    let trace = Trace::new(&ctx, id, shape, recording);
    if recording {
        inspect::begin(&ctx, id);
        for step in [Step::Env, Step::Style, Step::Id] {
            trace.mark(step);
        }
    }
    let mut stage = if sizing { MemoryStage::open_peek(&ctx, id) } else { MemoryStage::open(&ctx, id) };
    let resolved = steps::resolved(id, &step1, &common);
    // 4 value
    let value = steps::value(&mut comp, &mut stage, id, step1.command.as_deref());
    let (shown, mixed) = (value.value, value.mixed);
    // A sizing pass reads the cancel and ends nothing: its stage is a copy, and the real run reads the cancel again.
    let cancelled: Vec<_> = value.cancelled.into_iter().filter(|_| !sizing).collect();
    hooks.read(&mut comp, &mut stage, id, &env);
    let own_env = hooks.instance_scope(&comp).map(|scope| env.derive_local(|scoped| scoped.commands = scoped.commands.pushed(scope, id)));
    let env = own_env.as_ref().unwrap_or(&env);
    trace.mark(Step::Value);
    // 5 measure
    let room = match at {
        ShowAt::Rect(rect) => Some(rect.width()),
        // A measuring run in a content column asks for what the content needs: nothing there fills the room.
        ShowAt::Flow => {
            Some(crate::layout::flow_room(ui).x).filter(|width| width.is_finite() && !(env.content_column && crate::runtime::measure::active(&ctx)))
        }
    };
    // A leaf in flow in a row that shrinks its children to fit shows at most the width the row gave it.
    let shrink = env.shrink.filter(|_| shape == ShapeKind::Leaf && matches!(at, ShowAt::Flow) && comp.fills().is_none());
    let cap = match shrink {
        Some(crate::layout::Shrink::Apply(row)) => crate::layout::shrink_cap(&ctx, row, id),
        _ => None,
    };
    let room = match cap {
        Some(cap) => Some(room.map_or(cap, |room| room.min(cap))),
        None => room,
    };
    // A body root with parts steps its press machine before its measuring run, so the body reads this pass's part
    // presses in both runs (`BodyCx::part_press`). Step 8 finds the outcome. A member of another arena senses at step 8.
    if !sizing
        && shape == ShapeKind::Body
        && (C::PARTS || hooks.declares_parts())
        && (hooks.arena(&comp) || matches!(C::INPUT, Input::Text | Input::Editor) || env.arena.is_none())
    {
        sensing::sense_early(&ctx, id, resolved.enabled);
    }
    // Sizes come from this pass: a sizing pass measures the shape from its content; in a real pass a body or item
    // shape takes what its measuring run found.
    let (extent, shown) = if sizing {
        let measuring = Measuring { ui: &*ui, at, width: None, env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        measured_extent(&mut comp, &mut *hooks, &measuring, &mut stage, &style, shown)
    } else if matches!(shape, ShapeKind::Body | ShapeKind::Item) {
        let measuring = Measuring { ui: &*ui, at, width: None, env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        (hooks.pass_extent(&mut comp, &measuring, &style, shown.clone()), shown)
    } else {
        (hooks.extent(&comp, ui, at, id, &scx), shown)
    };
    let ask = |comp: &mut C, proposal: Proposal, room: Option<f32>, extent: Vec2| {
        let input = MeasureIn { ui: &*ui, env, proposal, room };
        steps::measure(comp, &input, &style, &resolved, shown.as_deref(), extent)
    };
    let (size, inputs) = ask(&mut comp, Proposal::Ideal, room, extent);
    // A leaf's measure calls are what its body's layout takes from it: a measuring run keeps them in the body's
    // transcript, and a real run its kept transcript serves asks them again. A leaf shown at a rect takes no room in
    // the flow: its body's layout takes the rect, and any measure its caller sized the rect by records itself.
    let leaf = shape == ShapeKind::Leaf && matches!(at, ShowAt::Flow) && (!sizing || crate::runtime::measure::active(&ctx));
    if leaf {
        crate::runtime::layout_memory::leaf_call(&ctx, id, Proposal::Ideal, room, inputs, size, |proposal, room| {
            ask(&mut comp, proposal, room, extent).0
        });
    }
    trace.mark(Step::Measure);
    // A control in a label scope reports its narrowest width, in a measuring run too: the real run's label step reads
    // it to decide whether the row fits the label beside it.
    let min = (shape == ShapeKind::Leaf && scope::in_label(&ctx)).then(|| {
        let (min, inputs) = ask(&mut comp, Proposal::Min, None, Vec2::ZERO);
        if leaf {
            crate::runtime::layout_memory::leaf_call(&ctx, id, Proposal::Min, None, inputs, min, |proposal, room| {
                ask(&mut comp, proposal, room, Vec2::ZERO).0
            });
        }
        scope::record_control_min(&ctx, min.x);
        min
    });
    // The read record: the binding step 4 read, and the sizes step 5 made from it.
    if let Some(key) = comp.bind().and_then(|bind| bind.key(id)) {
        let ideal = Sample { proposal: Proposal::Ideal, room, took: size };
        let measuring = sizing || crate::runtime::measure::active(&ctx);
        match min {
            Some(min) => reads::note(&ctx, key, id, measuring, &[ideal, Sample { proposal: Proposal::Min, room: None, took: min }]),
            None => reads::note(&ctx, key, id, measuring, &[ideal]),
        }
    }
    // The row gave the leaf no less than its least, so a leaf whose measure does not follow its room still fits.
    let size = cap.map_or(size, |cap| Vec2::new(size.x.min(cap), size.y));
    let measured = size;
    let size = hooks.sized(&comp, ui, id, size);
    if let Some(crate::layout::Shrink::Count(row)) = shrink {
        let mut width = |proposal| {
            let input = MeasureIn { ui: &*ui, env, proposal, room: None };
            steps::measure(&mut comp, &input, &style, &resolved, shown.as_deref(), Vec2::ZERO).0.x
        };
        let (ideal, least) = (width(Proposal::Ideal), width(Proposal::Min));
        crate::layout::report_shrink(&ctx, row, id, size.x, ideal, least);
    }
    // A component shown at a rect fills it: the rect is its size, and a leaf that measures nothing (a content
    // painter) still shows there.
    let inert = match at {
        ShowAt::Rect(rect) => !steps::guard(size, ShapeKind::Body) || !steps::guard(rect.size(), shape),
        ShowAt::Flow => !steps::guard(size, shape),
    };
    if inert && size != Vec2::ZERO {
        let now = crate::runtime::frame::clock(&ctx);
        let log = stage.shown::<SizeLog>(sub(id, "size_log"));
        if now - log.0 >= 1.0 || log.0 > now {
            log.0 = now;
            log::debug!("{} measured the size {size:?} and shows nothing", std::any::type_name::<C>());
        }
    }
    let mut cx = Cx::<C> {
        core: blank_core(env.view(), &resolved, &mut stage, ctx.clone(), env.ink, steps::inert_response(ui, id), step1.command.clone()),
        style: &style,
        shown,
        mixed,
        // A cancel step 4 raised over a document that moved is reported, not applied: the app's value stays. One that
        // carries the value the gesture began with restores it at step 11.
        applied: if cancelled.iter().any(|event| matches!(event, craft_behavior::EditEvent::Cancel { before: Some(_), .. })) {
            0
        } else {
            cancelled.len()
        },
        events: cancelled,
        validity: Validity::Ok,
    };
    // 6 to 16, in the host's `Ui`, once per call of the host.
    let mut host = hooks.host(&comp, HostCx { ui: &*ui, env, id, label: &step1.label, size });
    let mut run = Pass {
        comp: &mut comp,
        cx: &mut cx,
        hooks: &mut *hooks,
        env,
        style: &*style,
        resolved: &resolved,
        trace,
        tip,
        at,
        room,
        measured,
        size,
        parent: ui.scope_id(),
        inert,
        sizing,
        real: false,
        activated: false,
        injected: None,
    };
    host.run(id, ui, env, &mut |host_ui, host_at, host_env| pass(&mut run, host_ui, host_at, host_env));
    let (real_pass, activated, injected) = (run.real, run.activated, run.injected);
    drop(run);
    // A layer's dismissal is decided after its body so a field inside commits in the press pass; its open state is
    // written here, after the report (the one write point outside step 15, besides `request_close`).
    if real_pass && let Some(why) = host.dismissed() {
        hooks.dismissed(&mut comp, &mut cx, why);
    }
    // 17 finish, once: the stage is written back after `finish` returns, so writes made in `finish` are kept.
    if real_pass {
        trace.mark(Step::Finish);
        steps::before_finish(&mut cx, activated, injected);
        // A control painting text through its faces (a select, a segmented control) reports that baseline too.
        let baseline = cx.core.paint.borrow().baseline;
        if let Some(baseline) = baseline {
            scope::record_baseline(&ctx, baseline - cx.core.rect.top());
        } else if shape == ShapeKind::Leaf {
            scope::record_centre(&ctx, cx.core.rect.height() / 2.0);
        }
    }
    let output = comp.finish(cx);
    if real_pass {
        hooks.record(&mut stage, &ctx, id);
    }
    debug_assert!(!real_pass || crate::contract::Outcome::response(&output).id == id, "an output's Response is the component's own");
    if !sizing {
        stage.write_back();
    }
    output
}

pub(crate) fn show_leaf<C: Component + Meta<Shape = super::shape::Leaf>>(comp: C, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>) -> C::Output {
    let scope = comp.common(Internal::new()).command_scope;
    run(comp, ui, at, tip, &mut LeafHooks { scope })
}

/// Shows a container at `rect` and takes the rect's room in the flow of `ui`, through the one helper that moves a
/// parent's flow past a rect, so a sibling shown after it lies past it.
pub(crate) fn show_container_taking<C: Container, R>(
    comp: C,
    ui: &mut Ui,
    rect: Rect,
    body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
) -> Inner<C::Output, R> {
    let shown = show_container(comp, ui, ShowAt::Rect(rect), body, None);
    crate::runtime::egui::alloc::advance_past(ui, rect);
    shown
}

pub(crate) fn show_container<C: Container, R>(
    comp: C,
    ui: &mut Ui,
    at: ShowAt,
    mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
    tip: Option<TipFn<'_>>,
) -> Inner<C::Output, R> {
    let mut inner: Option<R> = None;
    let mut ran = false;
    let mut call = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
        // The measuring run and a sizing parent's run are peek stages; only a real run settles.
        ran |= !cx.memory.is_peek();
        inner = Some(body(ui, cx));
    };
    let ctx = ui.ctx().clone();
    let output = run(comp, ui, at, tip, &mut super::body::BodyHooks::new(&mut call));
    let id = crate::contract::Outcome::response(&output).id;
    let settle = ran.then(|| super::settle::SettleAt { ctx, body: sub(id, "body"), owner: id });
    Inner { output, inner, settle }
}

pub(crate) fn show_items<'r, C: Collection>(comp: C, ui: &mut Ui, source: &dyn ItemSource<C::Key>, item: &mut ItemFn<'r, C>) -> C::Output {
    let view = comp.filtered(source, &Env::of(ui));
    let source = view.as_deref().unwrap_or(source);
    run(comp, ui, ShowAt::Flow, None, &mut super::items::ItemsHooks::<C>::new(source, item))
}

/// Measures `comp` as `show` will show it, without allocating: steps 1 to 5 under the id `show` would use next. It
/// reads memory through a peek, registers no id and applies nothing. `under` is the composite's resolved
/// `(enabled, size)` when it measures a child it shows with `PaintUi::child`.
pub(crate) fn measure_under<C: Component>(comp: &mut C, ui: &Ui, id: Option<Id>, proposal: Proposal, under: Option<(bool, ControlSize)>) -> Vec2 {
    measure_shape(comp, ui, id, proposal, under, &mut LeafHooks { scope: None })
}

/// Measures a container with `body` as `show` will show it: steps 1 to 5, with the body, header, footer and end
/// regions run once in an invisible sizing pass under the container's layout.
pub(crate) fn measure_container<C: Container, R>(
    comp: &mut C,
    ui: &Ui,
    proposal: Proposal,
    mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
) -> Vec2 {
    let mut call = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
        body(ui, cx);
    };
    measure_shape(comp, ui, None, proposal, None, &mut super::body::BodyHooks::new(&mut call))
}

/// A measuring run's extent: the shape's `measured_extent` under a `Cx` that senses nothing and writes only to the
/// peek `stage`. Returns the shown value back with it.
pub(super) fn measured_extent<C: Component>(
    comp: &mut C,
    hooks: &mut dyn ShapeHooks<C>,
    at: &Measuring<'_, '_>,
    stage: &mut MemoryStage,
    style: &C::Style,
    shown: Option<Arc<ValueOf<C>>>,
) -> (Vec2, Option<Arc<ValueOf<C>>>) {
    let response = steps::inert_response(at.ui, at.resolved.id);
    let core = blank_core(at.env.view(), at.resolved, stage, at.ui.ctx().clone(), at.env.ink, response, at.command.clone());
    let mut cx = Cx::<C> { core, style, shown, mixed: at.mixed, events: Vec::new(), applied: 0, validity: Validity::Ok };
    let ctx = at.ui.ctx();
    let extent = reads::measuring(ctx, at.resolved.id, || steps::measuring(ctx, || hooks.measured_extent(comp, at, &mut cx)));
    (extent, cx.shown.take())
}

fn measure_shape<C: Component>(
    comp: &mut C,
    ui: &Ui,
    id: Option<Id>,
    proposal: Proposal,
    under: Option<(bool, ControlSize)>,
    hooks: &mut dyn ShapeHooks<C>,
) -> Vec2 {
    let env = match under {
        Some((enabled, size)) => super::cx::child_env(&Env::of(ui), enabled, size),
        None => Env::of(ui),
    };
    let id = id.unwrap_or_else(|| steps::id::<C>(ui, &comp.common(Internal::new()).id, false, false));
    let common = comp.common(Internal::new()).clone();
    let step1 = steps::env::<C>(ui.ctx(), &env, &common);
    let (shown, mixed) = steps::peek_value(comp, ui.ctx(), id, step1.command.as_deref());
    let scx = steps::style_cx(ui.ctx(), &env, &step1);
    let style = steps::style::<C>(ui.ctx(), &scx, comp.style_key());
    let resolved = steps::resolved(id, &step1, &common);
    let width = match proposal {
        Proposal::Width(width) => Some(width),
        _ => None,
    };
    // A leaf adds nothing to its own measure, so it opens no stage.
    let (extent, shown) = if hooks.shape() == ShapeKind::Leaf {
        (Vec2::ZERO, shown)
    } else {
        let mut stage = MemoryStage::open_peek(ui.ctx(), id);
        let measuring = Measuring { ui, at: ShowAt::Flow, width, env: &env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        measured_extent(comp, hooks, &measuring, &mut stage, &style, shown)
    };
    let ask = |comp: &mut C, proposal: Proposal| {
        let input = MeasureIn { ui, env: &env, proposal, room: None };
        steps::measure(comp, &input, &style, &resolved, shown.as_deref(), extent)
    };
    let (took, inputs) = ask(comp, proposal);
    if hooks.shape() == ShapeKind::Leaf && (!ui.is_sizing_pass() || crate::runtime::measure::active(ui.ctx())) {
        crate::runtime::layout_memory::leaf_call(ui.ctx(), id, proposal, None, inputs, took, |proposal, _| ask(comp, proposal).0);
    }
    if let Some(key) = comp.bind().and_then(|bind| bind.key(id)) {
        reads::note(ui.ctx(), key, id, true, &[Sample { proposal, room: None, took }]);
    }
    took
}

/// The flags a host passes to its faces: the validity of its value. The rest (focus, selection, open) belong to the
/// host's own parts.
pub(crate) const HOST_FLAGS: crate::style::Flags = crate::style::Flags::INVALID.union(crate::style::Flags::WARN);

/// What a face reads of the composite that paints it.
pub(crate) struct FaceSource<'a> {
    pub id: Id,
    pub result: &'a PartResult,
    /// `Some(keyboard visible)` when the part is the active part of the focused composite.
    pub focus: Option<bool>,
    pub response: &'a Response,
    /// The text colour of the host part, which a face reads through `StyleCx::host_ink`; `None` when the host has no
    /// text look.
    pub ink: Option<&'a crate::style::Stateful<crate::style::StyleColor>>,
    /// The muted text colour of the host part, which a face reads through `StyleCx::host_muted_ink`; `None` when the
    /// host has no muted text look.
    pub muted: Option<&'a crate::style::Stateful<crate::style::StyleColor>>,
    /// The flags the host owns for its faces: whether its value is invalid or warns.
    pub flags: crate::style::Flags,
    /// Whether the host is enabled: a face of a disabled host paints with the Disabled state.
    pub enabled: bool,
}

/// A face run is steps 1, 2, 4 (read only), 12 and 13 of the face component, inside the composite's step 13. It
/// registers no id, senses nothing, opens no memory stage and fires nothing.
pub(crate) fn paint_face<C: Show>(ui: &mut Ui, source: &FaceSource<'_>, part: PartRef, mut face: C) -> FaceRecord {
    let ctx = ui.ctx().clone();
    let env = Env::of(ui);
    let common = std::mem::take(face.common_mut(Internal::new()));
    debug_assert!(common.tooltip.is_none(), "a face instance carries no tooltip; use `PartBuilder::tooltip`");
    let step1 = steps::env::<C>(&ctx, &env, &common);
    let scx = steps::style_cx(&ctx, &env, &step1);
    let scx = source.ink.map_or(scx, |ink| scx.hosted(ink, source.muted));
    let style = steps::style::<C>(&ctx, &scx, face.style_key());
    let id = face_id(source.id, part);
    let resolved = steps::resolved(id, &step1, &common);
    let (shown, mixed) = steps::peek_value(&mut face, &ctx, id, step1.command.as_deref());
    if let Some(key) = face.bind().and_then(|bind| bind.key(id)) {
        reads::note(&ctx, key, id, crate::runtime::measure::active(&ctx), &[]);
    }
    // A peek that opens no lifetime stage and is dropped without a write-back; a write through it is a debug assertion.
    let mut stage = MemoryStage::open_peek(&ctx, id);
    stage.face = true;
    let rect = source.result.rect;
    let flow = Flow::new(rect, env.direction.is_rtl());
    let part_state = steps::part_state(source.result, step1.enabled && source.enabled, source.focus);
    let mut flags = part_state.flags | face.flags() | source.flags;
    // A face shows the value the step-6 peek found, as `steps::state` reads it for a component: a checked check box
    // takes the on look.
    if (shown.as_deref().map(|value| value as &dyn std::any::Any)).and_then(|value| value.downcast_ref::<bool>()) == Some(&true) {
        flags |= crate::style::Flags::CHECKED;
    }
    if mixed {
        flags |= crate::style::Flags::MIXED;
    }
    let on = f32::from(u8::from(flags.intersects(crate::style::Flags::ON)));
    let state = StateSet::new(part_state.state, flags, crate::style::StateMix::new(part_state.t.hover, part_state.t.press, on));
    let mut core = blank_core(env.view(), &resolved, &mut stage, ctx.clone(), env.ink, source.response.clone(), None);
    core.face_mode = true;
    core.rect = rect;
    core.content_rect = flow.inset(style.metrics().padding);
    core.flow = flow;
    core.state = state;
    core.stated = true;
    core.ink = steps::surface_ink(style.surface(), &state, env.ink);
    core.visible = rect;
    let cx = Cx::<C> { core, style: &style, shown, mixed, events: Vec::new(), applied: 0, validity: Validity::Ok };
    steps::paint_surface(ui, &env, &cx);
    {
        let results = HashMap::new();
        let mut pui = PaintUi {
            ui: &mut *ui,
            env: &env,
            ink: cx.core.ink,
            id,
            rect,
            record: &cx.core.paint,
            results: &results,
            response: source.response,
            active: None,
            text: None,
            muted: None,
            flags: crate::style::Flags::empty(),
            enabled: step1.enabled && source.enabled,
            size: step1.size,
            kids: None,
        };
        face.paint(&mut pui, &cx);
    }
    let (elided, baseline) = {
        let paint = cx.core.paint.borrow();
        (paint.elided.clone(), paint.baseline)
    };
    let name = elided.unwrap_or_else(|| step1.label.as_str().to_owned());
    // The face's own `access` adds its facts; the composite's step 16 fills the part's sub-node from the record.
    let mut access = super::Access::default();
    face.access(&mut access, &cx);
    FaceRecord { part, name, flags, role: C::ROLE, numeric: access.numeric.take(), expanded: access.expanded, enabled: step1.enabled, baseline }
}
