//! The shell of the pipeline (1.6): one `run` calls the 17 steps for every shape and every component type. What
//! differs by shape (the body step, the item step, the size of a body or collection) goes through `ShapeHooks`; what
//! differs by component type goes through `Erased` and `ErasedPass` (`typed`), so the pipeline is compiled once.

use super::cx::PartsCx;
use super::steps::{self, MeasureIn, PartsStep, ShapeKind, Step, StopsIn, Trace};
use super::{Activation, Common, Component, Cx, CxCore, OverlayUi, PaintUi, Resolved, TipFn, ValueOf};
use super::{overlay, tooltip};
use crate::layout::InFlow;
use crate::primitives::sensing::{self, MemberSpec, RootSpec};
use crate::primitives::{PaintRecord, PartResult};
use crate::runtime::access::{self, Politeness, Role, SetPosition, scope};
use crate::runtime::commands::ResolvedCommand;
use crate::runtime::env::{AppValues, Env, EnvView};
use crate::runtime::layout_memory::{Call, Inputs};
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::CloseReason;
use crate::runtime::reads::{self, BindKey, Sample};
use crate::runtime::{commands, focus, inspect, keys, memory};
use crate::style::{Ink, StateSet, StyleCx, StyleView};
use craft_behavior::{Flow, Input, KeyRole, NavMove, PartRef, PressState, Proposal, Roving, Sense};
use craft_theme::{ControlSize, Variant};
use egui::{Context, Id, Pos2, Rect, Response, Ui, Vec2};
use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod typed;

pub(crate) use typed::{measure_container, measure_under, measured_extent, paint_face, show_container, show_container_taking, show_items, show_leaf};

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
        lent: Default::default(),
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
        member_tip: None,
        press_cancel: None,
        scroll_moves: Vec::new(),
        value_parts: Default::default(),
        own_reset_parts: Default::default(),
        state_layer_parts: Default::default(),
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

/// What the pipeline reads of a component type's constants.
#[derive(Clone, Copy)]
pub(crate) struct TypeFacts {
    pub name: &'static str,
    pub role: Role,
    pub alt_roles: &'static [Role],
    pub input: Input,
    pub overlays: bool,
    pub parts: bool,
    pub stops: bool,
    pub canvas_control: bool,
    pub labelled_after: bool,
    /// The style key's type, which `Auto` ids are counted per.
    pub style_key: TypeId,
}

impl TypeFacts {
    pub(crate) fn of<C: Component>() -> Self {
        TypeFacts {
            name: std::any::type_name::<C>(),
            role: C::ROLE,
            alt_roles: C::ALT_ROLES,
            input: C::INPUT,
            overlays: C::OVERLAYS,
            parts: C::PARTS,
            stops: C::STOPS,
            canvas_control: C::CANVAS_CONTROL,
            labelled_after: C::LABELLED_AFTER,
            style_key: TypeId::of::<C::StyleKey>(),
        }
    }

    /// Whether the component's input is a text part or an editor.
    fn text(&self) -> bool {
        matches!(self.input, Input::Text | Input::Editor)
    }
}

/// A component and its shape's hooks, with the component type erased, as steps 1 to 5, the measuring runs and step 17
/// reach them. It keeps the typed values (the shown value, the cancels, the style) between the calls.
pub(crate) trait Erased {
    fn facts(&self) -> TypeFacts;
    fn common(&self) -> &Common;
    /// Moves `Common` out of the component (step 1).
    fn take_common(&mut self) -> Common;
    /// The variant asked for (or the style's first) and the variant the type accepts for it.
    fn variants(&self, asked: Option<Variant>) -> (Variant, Variant);
    /// Step 2.
    fn style(&mut self, ctx: &Context, scx: &StyleCx<'_>) -> Arc<dyn StyleView>;
    /// Step 4: keeps the shown value, and outside a sizing pass the cancels it read. Returns whether the binding is mixed.
    fn value(&mut self, stage: &mut MemoryStage, id: Id, command: Option<&ResolvedCommand>, sizing: bool) -> bool;
    /// Step 4 as a measure reads it: a peek. Returns whether the binding is mixed.
    fn peek_value(&mut self, ctx: &Context, id: Id, command: Option<&ResolvedCommand>) -> bool;
    fn bind_key(&mut self, id: Id) -> Option<BindKey>;
    fn fills(&self) -> Option<craft_behavior::Axis>;
    /// What `C::measure` gives for the shown value.
    fn measure_content(&mut self, input: &MeasureIn<'_>, resolved: &Resolved<'_>) -> (Vec2, Inputs);
    fn measured_extent(&mut self, at: &Measuring<'_, '_>, stage: &mut MemoryStage) -> Vec2;
    fn pass_extent(&mut self, at: &Measuring<'_, '_>) -> Vec2;
    /// Builds the component's `Cx` over `core` and `style` (what `style` gave), runs `rest` with it, then `finish`.
    /// `rest` returns whether a real pass ran.
    fn with_cx<'a>(&mut self, core: CxCore<'a>, style: &'a dyn StyleView, rest: &mut dyn FnMut(&mut dyn ErasedPass<'a>) -> bool);

    fn shape(&self) -> ShapeKind;
    fn declares_parts(&self) -> bool;
    fn read(&mut self, stage: &mut MemoryStage, id: Id, env: &Env);
    fn instance_scope(&self) -> Option<commands::CommandScope>;
    fn in_flow(&self) -> bool;
    /// Whether the shape fills the width of the room it is shown in (a body not sized from its content).
    fn fills_width(&self) -> bool;
    fn sized(&self, ui: &Ui, id: Id, size: Vec2) -> Vec2;
    fn arena(&self) -> bool;
    fn extent(&mut self, ui: &Ui, at: ShowAt, id: Id, scx: &StyleCx<'_>) -> Vec2;
    fn host(&mut self, at: HostCx<'_>) -> Box<dyn Host>;
    fn record(&mut self, stage: &mut MemoryStage, ctx: &Context, id: Id);
}

/// A component, its shape's hooks and its `Cx`, with the component type erased, as steps 6 to 17 reach them.
pub(crate) trait ErasedPass<'a> {
    fn core(&mut self) -> &mut CxCore<'a>;
    fn core_ref(&self) -> &CxCore<'a>;
    /// The part of the component's `Shown` record that does not depend on its value type.
    fn shown_state(&mut self) -> &mut steps::ShownState;
    fn value_text(&self) -> Option<String>;
    fn keys(&self) -> focus::KeyUse;
    /// The root's sense: the shape's from the component's own.
    fn sense(&self) -> Sense;
    /// The component's own sense.
    fn comp_sense(&self) -> Sense;
    /// Whether the component is bound, and whether its binding has a reset value.
    fn bound(&mut self) -> (bool, bool);
    fn paint_outset(&self, tokens: &crate::style::UiTokens) -> f32;
    /// Step 7: the component's parts, then the shape's.
    fn parts(&mut self, rect: Rect, flow: Flow, content_rect: Rect, parts: &mut PartsStep, input: &MeasureIn<'_>, rtl: bool);
    /// Step 8b for a text part.
    fn text(&mut self, ui: &mut Ui, step: &mut super::text::TextStep, keys: focus::KeyUse, rtl: bool, ink: egui::Color32, underline: f32);
    /// Step 8b for an editor part.
    fn editor(&mut self, ui: &mut Ui, step: &mut super::text::TextStep, keys: focus::KeyUse);
    fn gesture_gate(&mut self);
    fn edit_intents(&mut self);
    fn scrub_by_label(&mut self);
    fn update(&mut self);
    fn overlay(&mut self, overlays: &mut OverlayUi<'_>);
    fn apply_values(&mut self) -> steps::ValueEdit;
    fn register_reset(&mut self);
    fn value_flags(&self) -> crate::style::Flags;
    fn content_env(&self, values: &mut AppValues);
    fn paint(&mut self, ui: &mut Ui, env: &Env);
    fn paint_over(&mut self, ui: &mut Ui, env: &Env);
    fn settle_values(&mut self) -> steps::ValueEdit;
    /// The component's access facts and the words for its value.
    fn access(&self) -> (super::Access, Option<String>);
    /// The binding's validity message when it warns or refuses, and whether it warns.
    fn validity(&self) -> Option<(&crate::runtime::env::Message, bool)>;
    fn shows_label(&self) -> bool;
    fn activated(&self) -> bool;
    /// Whether the pass's events moved the value.
    fn changed(&self) -> bool;
    fn measure_content(&mut self, input: &MeasureIn<'_>, resolved: &Resolved<'_>) -> (Vec2, Inputs);

    fn shape(&self) -> ShapeKind;
    fn declares_parts(&self) -> bool;
    fn in_flow(&self) -> bool;
    fn regions(&self, id: Id) -> Vec<(Id, Vec2)>;
    fn focus(&self) -> Option<focus::FocusPolicy>;
    fn sized(&self, ui: &Ui, id: Id, size: Vec2) -> Vec2;
    fn fills_width(&self) -> bool;
    fn on_line(&self) -> bool;
    fn fills_width_only(&self) -> bool;
    fn arena(&self) -> bool;
    fn key_role(&self) -> KeyRole;
    fn roving(&self) -> Option<Roving>;
    fn expanded(&self) -> Option<bool>;
    fn live(&self) -> Option<Politeness>;
    fn position(&self) -> Option<SetPosition>;
    fn scrolls(&self) -> (bool, bool);
    fn overflow(&self) -> f32;
    fn sensed(&mut self);
    fn updated(&mut self);
    fn apply(&mut self);
    fn content(&mut self, ui: &mut Ui, env: &Env, values: &AppValues);
    fn settle(&mut self);
    fn dismissed(&mut self, why: CloseReason);
}

/// The paint view of a component's pass.
pub(super) fn paint_ui<'a>(ui: &'a mut Ui, env: &'a Env, core: &'a CxCore<'_>, style: &'a dyn StyleView) -> PaintUi<'a> {
    let focus = core.active.map(|part| (part, core.state.flags.contains(crate::style::Flags::FOCUS_VISIBLE)));
    PaintUi {
        ui,
        env,
        ink: core.ink,
        id: core.id,
        rect: core.rect,
        record: &core.paint,
        results: &core.results,
        response: &core.response,
        active: focus.filter(|_| core.state.flags.contains(crate::style::Flags::FOCUSED)),
        // Only a composite with parts paints faces, so the others skip the look list.
        text: if core.results.is_empty() { None } else { style.texts().first().map(|text| &text.color) },
        muted: if core.results.is_empty() { None } else { style.texts().get(1).map(|text| &text.color) },
        flags: core.state.flags & HOST_FLAGS,
        enabled: core.resolved.enabled,
        size: core.resolved.size,
        kids: None,
    }
}

/// A rate limit for the log of a rejected size: one line per id per second.
#[derive(Clone, Default)]
struct SizeLog(f64);

/// What steps 6 to 16 read and write besides the `Ui` the host gives. Built by steps 1 to 5 in the parent pass.
struct Pass<'p, 'r, 'a> {
    p: &'p mut dyn ErasedPass<'a>,
    facts: &'r TypeFacts,
    env: &'r Env,
    style: &'r dyn StyleView,
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
fn pass(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, host_at: Option<ShowAt>, host_env: Option<&Env>) {
    let scope = if p.sizing || ui.is_sizing_pass() { None } else { p.p.focus() };
    let (ctx, id) = (p.p.core_ref().ctx.clone(), p.resolved.id);
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
fn own_sense(e: &dyn ErasedPass<'_>, resolved: &Resolved, inert: bool) -> Sense {
    match (inert, resolved.enabled) {
        (true, _) => Sense::HOVER,
        (false, true) => e.sense(),
        (false, false) if steps::takes_focus(resolved) => Sense::HOVER | e.comp_sense().intersection(Sense::FOCUS),
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
fn stops_step(at: StopsStep<'_>, e: &mut dyn ErasedPass<'_>, facts: &TypeFacts, parts: &PartsStep) {
    let StopsStep { ctx, id, rtl, env, resolved, inert, holder_widget, member, own, text_keys } = at;
    let body_roving = e.roving();
    if !facts.stops && body_roving.is_none() {
        debug_assert!(
            !own.contains(Sense::FOCUS) && parts.decls.iter().all(|(_, extras)| !extras.focusable),
            "`{}` sets `STOPS = false` and has a stop",
            facts.name
        );
        return;
    }
    let (bound, resets) = e.bound();
    let resettable = !facts.text() && resets;
    let text = holder_widget.is_some();
    let keys = text_keys.unwrap_or_else(|| e.keys());
    let role = e.key_role().nudge(keys.nudge);
    // A bound component that keeps the arrows reads them as value steps; a text value sets `steps_value` in its text options.
    let arrows = keys.keep == crate::runtime::focus::KeepKeys::BlockArrows;
    let value = role.value || keys.value || (bound && !role.nudge && !text && arrows);
    steps::register_stops(&StopsIn {
        ctx,
        id,
        holder: holder_widget.unwrap_or_else(|| member.map_or(id, |arena| arena.0)),
        focusable: !inert && steps::takes_focus(resolved) && own.contains(Sense::FOCUS) && env.item != Some(id),
        parts_focus: !inert && steps::takes_focus(resolved),
        text,
        press_only: text && resolved.read_only,
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
fn declared_with(parts: &PartsStep, has: impl Fn(&super::cx::PartExtras) -> bool) -> super::cx::PartKeys {
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
fn place_root(p: &Pass<'_, '_, '_>, ui: &mut Ui, at: ShowAt, env: &Env, size: Vec2, inert: bool, rtl: bool) -> (Rect, bool) {
    // In a content-sized column a fill-width child takes its measured width in the flow and shows across the width
    // the column settles on, so it never sets that width.
    let line = p.p.on_line();
    let held = !inert
        && matches!(at, ShowAt::Flow)
        && ((env.content_column && p.p.fills_width()) || (crate::layout::in_grid_content_column(ui) && p.p.fills_width_only()));
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
        let fill = p.p.fills_width() && !p.p.fills_width_only();
        steps::place(ui, at, if inert { Vec2::ZERO } else { size }, InFlow { fill, line })
    };
    (rect, held)
}

/// Creates the component's accessibility node, and its label scope, before any child `Ui` runs.
fn create_node(ctx: &Context, ui: &Ui, id: Id, member: bool, role: Role) -> bool {
    matches!(role, Role::Node(_)) && {
        if member {
            access::register_parent(ctx, id, ui.unique_id());
        }
        // Inside an inspected region the node is reported whether or not AccessKit runs, so its facts are recorded.
        let created = access::create_node(ctx, id) || inspect::records_nodes(ui);
        if created {
            scope::enter(ctx, id);
        }
        created
    }
}

/// Steps 6 to 16 of the pass, run in the `Ui` the host gives.
fn run_in_host(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, host_at: Option<ShowAt>, host_env: Option<&Env>) {
    let ctx = p.p.core_ref().ctx.clone();
    let (id, rtl) = (p.resolved.id, p.env.direction.is_rtl());
    let (env, resolved, inert, facts, style) = (host_env.unwrap_or(p.env), p.resolved, p.inert, p.facts, p.style);
    let metrics = style.metrics();
    // 6 place
    let at = host_at.unwrap_or(p.at);
    // A root that fills takes the room of the `Ui` its host runs it in: a pane's or a layer's own, not its parent's.
    let size = if ui.scope_id() == p.parent { p.size } else { p.p.sized(ui, id, p.measured) };
    let (rect, held) = place_root(p, ui, at, env, size, inert, rtl);
    p.trace.mark(Step::Place);
    // What the content wants past the room counts where the root's width comes from its content: a content-sized
    // root, or a fill-width one held at its measured width. A root that fills the flow keeps to the room it fills.
    let overflow = if held || !p.p.fills_width() { p.p.overflow() } else { 0.0 };
    expand_for_overflow(ui, host_at.unwrap_or(p.at), rect, overflow, rtl);
    // The component sits where the interface puts it; a component whose axis keeps its order flows its content left to right.
    let rtl = rtl && !resolved.ltr_axis;
    // A sizing pass of the parent or host `Ui` allocates the size and stops: no parts, no body, and the stage
    // opened at step 3 is written back unchanged.
    if p.sizing || ui.is_sizing_pass() {
        // The output words the value in a measuring run too, so what is measured beside the component holds it.
        let text = p.p.value_text();
        *p.p.core_ref().value_text.borrow_mut() = text;
        return;
    }
    p.real = true;
    let inspected = p.trace.recording() && inspect::open_element(ui, id, facts.name);
    let e = &mut *p.p;
    // 7 parts
    p.trace.mark(Step::Parts);
    let flow = Flow::new(rect, rtl);
    let content_rect = flow.inset(metrics.padding);
    let mut parts = PartsStep { decls: Default::default(), rovings: Vec::new(), scrolls: Vec::new(), text: super::text::TextStep::default() };
    if !inert && facts.text() {
        parts.text.commit_now = focus::commit_now(&ctx) == Some(id);
    }
    // A component that declares no parts skips the step; a debug build runs it anyway to check the declaration.
    let declares_parts = facts.parts || e.declares_parts();
    if !inert && (declares_parts || cfg!(debug_assertions)) {
        let input = MeasureIn { ui: &*ui, env, proposal: Proposal::Ideal, room: p.room };
        e.parts(rect, flow, content_rect, &mut parts, &input, rtl);
    }
    if facts.canvas_control && p.trace.recording() {
        inspect::canvas_control(&ctx, facts.name);
    }
    debug_assert!(declares_parts || steps::declares_none(&parts), "`{}` sets `PARTS = false` and declares a part", facts.name);
    steps::check_parts(&ctx, facts.name, &parts);
    let (holder_widget, text_keys) = text_holder(id, &parts, e.keys());
    let own = own_sense(e, resolved, inert);
    // An item never senses the whole row for a press: the collection's root takes the click and the focus (17.2). It
    // does sense the pointer over it, so `PartsCx::hovered` and the state step read the row's hover from the same
    // arena outcome whether the item stands alone or in a collection.
    let own = if env.item == Some(id) { own.difference(Sense::CLICK | Sense::FOCUS) | Sense::HOVER } else { own };
    let arena = e.arena();
    let member = if arena || facts.text() { None } else { env.arena };
    stops_step(StopsStep { ctx: &ctx, id, rtl, env, resolved, inert, holder_widget, member, own, text_keys }, e, facts, &parts);
    // A body's wheel share is its scroll taker (body step, step 7); the gate value does not take the wheel. A body root that
    // senses the wheel itself (a canvas) registers like any other component.
    if !inert && (e.shape() != ShapeKind::Body || own.intersects(Sense::WHEEL | Sense::WHEEL_X)) {
        steps::register_wheel(&ctx, id, rect, own, e.bound().0, env.wheel);
    }
    // 8 sense
    p.trace.mark(Step::Sense);
    let decls = parts.decls.parts();
    let enabled = resolved.enabled && !inert;
    let sensing::RootResult { mut response, parts: results, press, activation, activated_member, sense: root_sense } =
        sense_root(ui, member, RootSpec { id, rect, own_sense: own, own: decls, enabled }, arena);
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
    let node = create_node(&ctx, ui, id, member.is_some(), facts.role);
    // A root with no node whose egui widget senses still gets a node from egui, with actions nobody writes.
    if facts.role == Role::None && !root_sense.is_empty() {
        access::hide(&ctx, id);
    }
    let session = member.filter(|_| e.keys().session);
    let core = e.core();
    core.part_cursors = sensing::hovered_cursor(decls, &results).into_iter().collect();
    core.rect = rect;
    core.content_rect = content_rect;
    core.flow = flow;
    core.visible = rect.intersect(ui.clip_rect());
    // A read-only component takes no activation: the response reports no click either.
    if !steps::takes_activation(resolved) {
        response.flags.remove(egui::response::Flags::CLICKED | egui::response::Flags::FAKE_PRIMARY_CLICKED);
    }
    core.response = response;
    // The focus service holds last pass's registration: a part that is not in this pass's results is not active.
    core.focus = focus::facts(&ctx, id);
    core.active = core.focus.part.filter(|part| results.contains_key(&(part.key, part.index)));
    core.scroll_moves = super::cx::scroll_moves(&parts.scrolls, &results);
    core.value_parts = declared_with(&parts, |extras| extras.value);
    core.state_layer_parts = declared_with(&parts, |extras| extras.state_layer);
    core.toggles = parts
        .decls
        .iter()
        .filter_map(|(decl, extras)| {
            extras.toggles.map(|toggle| super::cx::Toggle { part: PartRef { key: decl.key, index: decl.index }, decl: toggle, open: false })
        })
        .collect();
    core.own_reset_parts = declared_with(&parts, |extras| extras.own_reset);
    core.results = results;
    core.root_sense = root_sense;
    core.press = press;
    core.member_of = member.map(|arena| arena.0);
    // A key or AccessKit activation of a member reaches its arena root, which names the member.
    let keyed = member.is_some_and(|arena| keys::activated_member(&ctx, arena.0) == Some(id));
    core.activation = activation_of(&ctx, ui, rect, resolved, keyed, root_sense, activation);
    core.activated_member = activated_member.or_else(|| arena.then(|| keys::activated_member(&ctx, id)).flatten());
    // A session member with no focused part takes the canvas's and the window's Enter, Escape and arrows first.
    if let Some(arena) = session.filter(|_| !inert && !core.focus.focused) {
        core.lent = keys::lend(&ctx, arena.0, id);
    }
    core.intents = keys::intents_of(&ctx, Some(id), rtl);
    e.sensed();
    if inert {
        if node {
            steps::report_inert(e.core_ref(), facts);
        }
        if inspected {
            inspect::close_element(&ctx, id, rect, rect);
        }
        return;
    }
    // The popups the text part and `update` register start over for this pass.
    if facts.overlays {
        overlay::begin_owner(&ctx, id);
    }
    // 8b text part
    if parts.text.decl.is_some() {
        let ink = crate::style::ink_color(&env.tokens, env.ink);
        let keys = text_keys.unwrap_or_else(|| e.keys());
        e.text(ui, &mut parts.text, keys, rtl, ink, env.tokens.stroke.thin);
    } else if holder_widget.is_some() {
        let keys = text_keys.unwrap_or_else(|| e.keys());
        e.editor(ui, &mut parts.text, keys);
    }
    // 9 update
    p.trace.mark(Step::Update);
    e.gesture_gate();
    steps::update_inputs(e.core());
    overlay::drive_toggles(e.core(), facts.overlays, facts.name);
    e.edit_intents();
    e.scrub_by_label();
    steps::activate_by_label(&ctx, facts.labelled_after);
    e.update();
    if cfg!(debug_assertions) {
        let keys = text_keys.unwrap_or_else(|| e.keys());
        let held = |mv: NavMove| match keys.keep {
            focus::KeepKeys::BlockArrows => true,
            focus::KeepKeys::InlineArrows => matches!(mv, NavMove::Back | NavMove::Forward),
            _ => false,
        };
        debug_assert!(
            e.key_role().moves || keys.moves || e.core_ref().intents.taken_arrow_moves().iter().all(|mv| held(*mv)),
            "`{}` takes arrow moves with `take_move` that its `KeyUse` neither keeps nor declares as `moves`",
            facts.name
        );
    }
    e.updated();
    steps::record_context(e.core_ref());
    // The body's group registers after `update`, which may change its rules from what the component knows now.
    if let Some(roving) = e.roving() {
        let core = e.core_ref();
        let group = focus::Group::new(steps::body_group(id), roving).rtl(rtl);
        let group = match core.roving_lead {
            Some(lead) => group.lead((lead, None)),
            None => group,
        };
        focus::register_group(
            &ctx,
            match core.roving_arrival {
                Some(arrival) => group.selected((arrival, None)),
                None => group,
            },
        );
    }
    // 10 overlay
    p.trace.mark(Step::Overlay);
    if facts.overlays {
        let core = e.core();
        let open = overlay::open_map(&mut *core.memory, id);
        let parts = core
            .toggles
            .iter()
            .filter_map(|toggle| core.results.get(&(toggle.part.key, toggle.part.index)).map(|result| (toggle.part, result.rect)))
            .collect();
        let mut overlays = OverlayUi::new(&ctx, overlay::OverlayOwner { id, anchor: rect, parts }, &*ui, resolved, env.clone(), open);
        e.overlay(&mut overlays);
        overlay::apply_closes(&ctx, overlays.closed);
    }
    // What `update` and `overlay` left untaken goes back to the key service.
    steps::after_update(e.core());
    // 11 apply
    p.trace.mark(Step::Apply);
    let edit = e.apply_values();
    let leaf = e.shape() == ShapeKind::Leaf;
    steps::rerun_after_edit(e, style, edit, &MeasureIn { ui: &*ui, env: p.env, proposal: Proposal::Ideal, room: p.room }, resolved, leaf);
    e.apply();
    e.register_reset();
    // 12 state
    p.trace.mark(Step::State);
    let open = facts.overlays && overlay::popup_shown(&ctx, id);
    let expanded = e.expanded() == Some(true);
    steps::state(e, style, open, expanded, env.ink);
    if e.core_ref().state.flags.contains(crate::style::Flags::FOCUSED) {
        commands::note_focus(&ctx, id, &env.commands);
    }
    // 13 paint
    p.trace.mark(Step::Paint);
    let mut values = AppValues::default();
    e.content_env(&mut values);
    steps::paint_surface(ui, env, e.core_ref(), style);
    steps::paint_part_layers(ui, env, e.core_ref());
    // Paint and content stay inside the rect (grown by the type's declared outset), however small the room is.
    let outer_clip = ui.clip_rect();
    let inside = e.core_ref().rect.expand(e.paint_outset(&env.tokens)).intersect(outer_clip);
    ui.set_clip_rect(inside);
    e.paint(ui, env);
    ui.set_clip_rect(outer_clip);
    steps::paint_focus(ui, e.core_ref(), style);
    ui.set_clip_rect(inside);
    // 14 content, then `paint_over`
    if p.trace.mark(Step::Content) {
        if facts.role == Role::Merged {
            scope::begin(&ctx, id, id, scope::Kind::Merge);
        }
        // A real run checks a body's content against the kept transcript its records came from; a collection's rows
        // are its own measure, not the enclosing body's, and a layer's content is not part of its parent's flow.
        let scoped = (!p.sizing && !crate::runtime::measure::active(&ctx))
            .then(|| match e.shape() {
                ShapeKind::Body => Some((Some(id), !e.in_flow())),
                ShapeKind::Item => Some((None, true)),
                _ => None,
            })
            .flatten();
        if let Some((body, boundary)) = scoped {
            crate::runtime::layout_memory::open_content(&ctx, body, boundary);
        }
        e.content(ui, env, &values);
        if scoped.is_some() {
            crate::runtime::layout_memory::close_content(&ctx, &e.regions(id));
        }
    }
    // The `over` parts are placed before `paint_over`, so the children it shows stack above them (8.3).
    ui.set_clip_rect(outer_clip);
    sensing::place_over_widgets(ui, id, member, decls);
    ui.set_clip_rect(inside);
    e.paint_over(ui, env);
    ui.set_clip_rect(outer_clip);
    // 15 settle
    p.trace.mark(Step::Settle);
    e.settle();
    let edit = e.settle_values();
    steps::rerun_after_edit(e, style, edit, &MeasureIn { ui: &*ui, env: p.env, proposal: Proposal::Ideal, room: p.room }, resolved, leaf);
    // 16 report, then the tooltip.
    p.trace.mark(Step::Report);
    steps::report(
        e,
        facts,
        &steps::ReportIn {
            node,
            decls: &parts.decls,
            sense: own,
            live: e.live(),
            scrolls: e.scrolls(),
            expanded: e.expanded(),
            position: e.position(),
            group: steps::member_group(&ctx, env.roving, id),
        },
    );
    let shows_label = e.shows_label();
    tooltip::run(e.core_ref(), tooltip::TipIn { ui: &*ui, env, rich: p.tip.take(), parts: &parts.decls, shows_label });
    // Activations accumulate across the passes of a viewport mount; step 17 fires the bound command once, with the
    // injected source when there is one.
    let natural = e.activated() && resolved.enabled;
    let injected = e.core().injected.take().filter(|_| resolved.enabled);
    if (natural || injected.is_some()) && !e.core_ref().keep_open {
        crate::runtime::overlay::note_activated(&ctx, id);
    }
    p.activated |= natural || injected.is_some();
    p.injected = p.injected.or(injected);
    if inspected {
        let core = e.core_ref();
        inspect::close_element(&ctx, id, core.rect, core.content_rect);
    }
}

/// Step 5's size for `input`: what the component measures plus `extent`, padded and bounded by its style.
fn measured(t: &mut dyn Erased, input: &MeasureIn<'_>, style: &dyn StyleView, resolved: &Resolved<'_>, extent: Vec2) -> (Vec2, Inputs) {
    let (content, inputs) = t.measure_content(input, resolved);
    (steps::measure(style, input, content, extent), inputs)
}

/// The pipeline: steps 1 to 17 for one component, whatever its type and shape.
pub(super) fn run(t: &mut dyn Erased, ui: &mut Ui, at: ShowAt, mut tip: Option<TipFn<'_>>) {
    let ctx = ui.ctx().clone();
    let facts = t.facts();
    crate::runtime::measure::count_run(&ctx, facts.name);
    // Slots the last end hook evicted drop here, outside every lock and hook.
    memory::dispose_evicted(&ctx);
    let shape = t.shape();
    // Steps 1 to 5 read the sizing flag of the parent `Ui`; steps 6 to 16 read the host `Ui`'s. A sizing pass runs
    // steps 1 to 6 and allocates the size: no id registration, sensing or memory writes.
    // egui's own invisible pass: it allocates and registers nothing, whether or not the framework is measuring.
    let sizing = ui.is_sizing_pass();
    // The inspector flag is read once per run.
    let recording = !sizing && inspect::recording(&ctx);
    // 1 env: `Common` moves out of the component; `Resolved` borrows these locals.
    let env = Env::of(ui);
    let common: Common = t.take_common();
    let step1 = steps::env(&ctx, &env, &common, t.variants(common.variant));
    // 2 style
    let scx = steps::style_cx(&ctx, &env, &step1);
    let style = t.style(&ctx, &scx);
    // 3 id, and the memory stage for it
    let id = steps::id(ui, &common.id, !sizing, true, facts.style_key, facts.name);
    let trace = Trace::new(&ctx, id, shape, recording);
    if recording {
        inspect::begin(&ctx, id);
        for step in [Step::Env, Step::Style, Step::Id] {
            trace.mark(step);
        }
    }
    let mut stage = if sizing { MemoryStage::open_peek(&ctx, id) } else { MemoryStage::open(&ctx, id) };
    let resolved = steps::resolved(id, &step1, &common);
    // 4 value. A sizing pass reads the cancel and ends nothing: its stage is a copy, and the real run reads the
    // cancel again.
    let mixed = t.value(&mut stage, id, step1.command.as_deref(), sizing);
    t.read(&mut stage, id, &env);
    let own_env = t.instance_scope().map(|scope| env.derive_local(|scoped| scoped.inherited_mut().commands = scoped.commands.pushed(scope, id)));
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
    // A leaf or body in flow that does not fill takes part in the shrink step of the body around it: in a row that
    // shrinks its children to fit, a leaf shows at most the width the row gave it, and a body takes that width as its
    // room (`mount_room`). A body that fills the width of a bounded room takes the room or its share of it (`sized`),
    // so it is no unit.
    let in_flow = matches!(at, ShowAt::Flow) && t.fills().is_none();
    let unit_body = || t.in_flow() && !(t.fills_width() && crate::layout::flow_room(ui).x.is_finite());
    let shrink = env.shrink.filter(|_| in_flow && (shape == ShapeKind::Leaf || (shape == ShapeKind::Body && unit_body())));
    let cap = match shrink {
        Some(crate::layout::Shrink::Apply(row)) if shape == ShapeKind::Leaf => crate::layout::shrink_cap(&ctx, row, id),
        _ => None,
    };
    let room = match cap {
        Some(cap) => Some(room.map_or(cap, |room| room.min(cap))),
        None => room,
    };
    // A body root with parts steps its press machine before its measuring run, so the body reads this pass's part
    // presses in both runs (`BodyCx::part_press`). Step 8 finds the outcome. A member of another arena senses at step 8.
    if !sizing && shape == ShapeKind::Body && (facts.parts || t.declares_parts()) && (t.arena() || facts.text() || env.arena.is_none()) {
        sensing::sense_early(&ctx, id, resolved.enabled);
    }
    // Sizes come from this pass: a sizing pass measures the shape from its content; in a real pass a body or item
    // shape takes what its measuring run found.
    let extent = if sizing || matches!(shape, ShapeKind::Body | ShapeKind::Item) {
        let measuring = Measuring { ui: &*ui, at, width: None, env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        if sizing { t.measured_extent(&measuring, &mut stage) } else { t.pass_extent(&measuring) }
    } else {
        t.extent(ui, at, id, &scx)
    };
    let ask = |t: &mut dyn Erased, proposal: Proposal, room: Option<f32>, extent: Vec2| {
        measured(t, &MeasureIn { ui: &*ui, env, proposal, room }, &*style, &resolved, extent)
    };
    let (size, inputs) = ask(&mut *t, Proposal::Ideal, room, extent);
    let ideal = Call { proposal: Proposal::Ideal, room, inputs, took: size };
    // A control in a label scope reports its narrowest width, in a measuring run too: the real run's label step reads
    // it to decide whether the row fits the label beside it.
    let min = (shape == ShapeKind::Leaf && scope::in_label(&ctx)).then(|| {
        let (took, inputs) = ask(&mut *t, Proposal::Min, None, Vec2::ZERO);
        Call { proposal: Proposal::Min, room: None, inputs, took }
    });
    let min_took = min.as_ref().map(|min| min.took);
    // A leaf's measure calls are what its body's layout takes from it: a measuring run keeps them in the body's
    // transcript, and a real run its kept transcript serves checks them against this run's. A leaf shown at a rect
    // takes no room in the flow: its body's layout takes the rect, and any measure its caller sized the rect by
    // records itself.
    let leaf = shape == ShapeKind::Leaf && matches!(at, ShowAt::Flow) && (!sizing || crate::runtime::measure::active(&ctx));
    if leaf {
        let mut both;
        let mut one;
        let calls: &mut [Call] = match min {
            Some(min) => {
                both = [ideal, min];
                &mut both
            }
            None => {
                one = [ideal];
                &mut one
            }
        };
        crate::runtime::layout_memory::leaf_call(&ctx, id, calls, |proposal, room| ask(&mut *t, proposal, room, extent).0);
    }
    trace.mark(Step::Measure);
    let min = min_took.inspect(|min| scope::record_control_min(&ctx, min.x));
    // The read record: the binding step 4 read, and the sizes step 5 made from it.
    if let Some(key) = t.bind_key(id) {
        let ideal = Sample { proposal: Proposal::Ideal, room, took: size };
        let measuring = sizing || crate::runtime::measure::active(&ctx);
        match min {
            Some(min) => reads::note(&ctx, key, id, measuring, &[ideal, Sample { proposal: Proposal::Min, room: None, took: min }]),
            None => reads::note(&ctx, key, id, measuring, &[ideal]),
        }
    }
    // In the least measure of a body every leaf in flow takes its least. In a stack a leaf wider than the stack's room
    // takes the room, down to its least.
    let leaf_in_flow = shape == ShapeKind::Leaf && in_flow;
    let least = |t: &mut dyn Erased| ask(t, Proposal::Min, None, Vec2::ZERO).0.x;
    let cap = match leaf_in_flow && env.least {
        true => {
            let least = least(&mut *t);
            Some(cap.map_or(least, |cap| cap.min(least)))
        }
        false => cap,
    };
    let fit_room =
        (leaf_in_flow && shrink == Some(crate::layout::Shrink::Fit)).then(|| crate::layout::flow_room(ui).x).filter(|room| room.is_finite());
    let cap = match fit_room.filter(|room| size.x > *room) {
        Some(room) => Some(cap.map_or(room, |cap| cap.min(room)).max(least(&mut *t))),
        None => cap,
    };
    // The row gave the leaf no less than its least, so a leaf whose measure does not follow its room still fits.
    let size = cap.map_or(size, |cap| Vec2::new(size.x.min(cap), size.y));
    let measured = size;
    // A component on a layer or in a window of its own takes no room in the parent's flow: the parent neither sizes it
    // nor counts it among the children that share its room. Its host sizes it in its own `Ui`.
    let size = if t.in_flow() { t.sized(ui, id, size) } else { size };
    match shrink {
        Some(crate::layout::Shrink::Count(row)) if shape == ShapeKind::Leaf => {
            let (ideal, least) = (ask(&mut *t, Proposal::Ideal, None, Vec2::ZERO).0.x, ask(&mut *t, Proposal::Min, None, Vec2::ZERO).0.x);
            crate::layout::report_shrink(&ctx, row, id, size.x, ideal, Some(least));
        }
        Some(crate::layout::Shrink::Count(row)) => crate::layout::report_shrink(&ctx, row, id, size.x, size.x, None),
        Some(crate::layout::Shrink::Least(row)) if shape == ShapeKind::Body => crate::layout::report_least(&ctx, row, id, size.x),
        _ => {}
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
            log::debug!("{} measured the size {size:?} and shows nothing", facts.name);
        }
    }
    let core = blank_core(env.view(), &resolved, &mut stage, ctx.clone(), env.ink, steps::inert_response(ui, id), step1.command.clone());
    // 6 to 16, in the host's `Ui`, once per call of the host.
    let mut host = t.host(HostCx { ui: &*ui, env, id, label: &step1.label, size });
    let parent = ui.scope_id();
    let mut real_pass = false;
    t.with_cx(core, &*style, &mut |e| {
        let mut run = Pass {
            p: &mut *e,
            facts: &facts,
            env,
            style: &*style,
            resolved: &resolved,
            trace,
            tip: tip.take(),
            at,
            room,
            measured,
            size,
            parent,
            inert,
            sizing,
            real: false,
            activated: false,
            injected: None,
        };
        host.run(id, ui, env, &mut |host_ui, host_at, host_env| pass(&mut run, host_ui, host_at, host_env));
        let (real, activated, injected) = (run.real, run.activated, run.injected);
        // A layer's dismissal is decided after its body so a field inside commits in the press pass; its open state is
        // written here, after the report (the one write point outside step 15, besides `request_close`).
        if real && let Some(why) = host.dismissed() {
            e.dismissed(why);
        }
        // 17 finish, once: the stage is written back after `finish` returns, so writes made in `finish` are kept.
        if real {
            trace.mark(Step::Finish);
            let changed = e.changed();
            steps::before_finish(e.core(), activated, injected, changed);
            // A control painting text through its faces (a select, a segmented control) reports that baseline too.
            let core = e.core_ref();
            let baseline = core.paint.borrow().baseline;
            if let Some(baseline) = baseline {
                scope::record_baseline(&ctx, baseline - core.rect.top());
            } else if shape == ShapeKind::Leaf {
                scope::record_centre(&ctx, core.rect.height() / 2.0);
            }
        }
        real_pass = real;
        real
    });
    if real_pass {
        t.record(&mut stage, &ctx, id);
    }
    if !sizing {
        stage.write_back();
    }
}

/// Measures the component in `t` as `show` will show it, without allocating: steps 1 to 5 under the id `show` would
/// use next (or `id`). It reads memory through a peek, registers no id and applies nothing. `under` is the
/// composite's resolved `(enabled, size)` when it measures a child it shows with `PaintUi::child`.
pub(super) fn measure_shape(t: &mut dyn Erased, ui: &Ui, id: Option<Id>, proposal: Proposal, under: Option<(bool, ControlSize)>) -> Vec2 {
    let facts = t.facts();
    let env = match under {
        Some((enabled, size)) => super::cx::child_env(&Env::of(ui), enabled, size),
        None => Env::of(ui),
    };
    let id = id.unwrap_or_else(|| steps::id(ui, &t.common().id, false, false, facts.style_key, facts.name));
    let common = t.common().clone();
    let step1 = steps::env(ui.ctx(), &env, &common, t.variants(common.variant));
    let mixed = t.peek_value(ui.ctx(), id, step1.command.as_deref());
    let scx = steps::style_cx(ui.ctx(), &env, &step1);
    let style = t.style(ui.ctx(), &scx);
    let resolved = steps::resolved(id, &step1, &common);
    let (at, width) = match proposal {
        Proposal::Width(width) => (ShowAt::Flow, Some(width)),
        Proposal::Size(size) => (ShowAt::Rect(Rect::from_min_size(ui.available_rect_before_wrap().min, size)), None),
        _ => (ShowAt::Flow, None),
    };
    let leaf = t.shape() == ShapeKind::Leaf;
    // A leaf adds nothing to its own measure, so it opens no stage.
    let extent = if leaf {
        Vec2::ZERO
    } else {
        let mut stage = MemoryStage::open_peek(ui.ctx(), id);
        let measuring = Measuring { ui, at, width, env: &env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        t.measured_extent(&measuring, &mut stage)
    };
    let ask = |t: &mut dyn Erased, proposal: Proposal| measured(t, &MeasureIn { ui, env: &env, proposal, room: None }, &*style, &resolved, extent);
    let (took, inputs) = ask(&mut *t, proposal);
    if leaf && (!ui.is_sizing_pass() || crate::runtime::measure::active(ui.ctx())) {
        let calls = &mut [Call { proposal, room: None, inputs, took }];
        crate::runtime::layout_memory::leaf_call(ui.ctx(), id, calls, |proposal, _| ask(&mut *t, proposal).0);
    }
    if let Some(key) = t.bind_key(id) {
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
