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
use crate::runtime::env::{AppValues, ColumnSweep, Env, EnvView, ThemeScope};
use crate::runtime::layout_memory::{Call, Inputs};
use crate::runtime::lazy::LineItem;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::CloseReason;
use crate::runtime::reads::{self, BindKey, Sample};
use crate::runtime::{commands, focus, inspect, keys, memory};
use crate::style::{Ink, StateSet, StyleCx, StyleView, UiTokens};
use craft_behavior::{Flow, Input, KeyRole, NavMove, PartRef, PressState, Proposal, Roving, Sense};
use craft_theme::Variant;
use egui::{Context, Id, Pos2, Rect, Response, Ui, Vec2};
use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod typed;

pub(crate) use typed::{
    MemberNode, MemberPrep, MemberShown, measure_container, measure_face, measure_under, measured_extent, member_prepare, member_show, paint_face,
    show_container, show_container_taking, show_items, show_leaf,
};

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
    /// The env the component's parent installed for `ui`, under which `ui`'s children are placed.
    pub place: &'a Env,
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
    /// parent's at step 5, and the host's at step 6 when the host runs the pass in a `Ui` of its own. `place` is the
    /// env `ui`'s children are placed under.
    fn sized(&self, _comp: &C, _ui: &Ui, _place: &Env, _id: Id, size: Vec2) -> Vec2 {
        size
    }

    /// Whether the root takes all the width the flow offers: in a content-sized column it then counts at its
    /// measured width and never sets the column's.
    fn fills_width(&self, comp: &C) -> bool {
        comp.fills() == Some(craft_behavior::Axis::X)
    }

    /// How far below the root's top the centre of its first line lies, at `height` tall: a root whose first line lies
    /// at half its height is one line, which sits with its centre on a `Row`'s line; any other root sits at the
    /// line's top. A leaf is one control on the line.
    fn first_line(&self, _comp: &C, _ctx: &Context, _id: Id, height: f32) -> Option<f32> {
        Some(height / 2.0)
    }

    /// Whether the root's first line is the line of its own unwrapped `Row` body: it sits on a `Row`'s line by that
    /// line even where a child of more than one line makes it taller, so the members of a row nested in a row share
    /// the outer row's line.
    fn first_is_row_line(&self, _comp: &C) -> bool {
        false
    }

    /// Whether the root is one member of a bar's line, shown whole or not at all: a leaf is; a body is when it has a
    /// side label (a captioned field), and otherwise only lays out the members it holds.
    fn member(&self, _comp: &C) -> bool {
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

    /// The padding the shape's props add on every side of the content, over the style's (`Resolved::pad`): step 5
    /// adds it to the measured size and step 7 insets the content rect by it.
    fn pad(&self, _comp: &C, _tokens: &UiTokens) -> f32 {
        0.0
    }

    /// What the shape adds to the content size at step 5, before padding and `min_size`. `place` is the env the parent
    /// `ui`'s children are placed under.
    fn extent(&mut self, _comp: &C, _ui: &Ui, _place: &Env, _at: ShowAt, _id: Id, _scx: &StyleCx<'_>) -> Vec2 {
        Vec2::ZERO
    }

    /// What the shape adds to the content size in a measuring run (a sizing pass, or `measure_in`), from this pass's
    /// inputs only: a body runs its regions in an invisible sizing pass and measures what they took. The default is
    /// `extent`.
    fn measured_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, cx: &mut Cx<'_, C>) -> Vec2 {
        self.extent(comp, at.ui, at.place, at.at, cx.core.id, at.scx)
    }

    /// A real pass's step 5 for a body or an item shape: what the shape adds to the content size, from a measuring run
    /// of its contents in this pass (or the record one left in this pass). The default is `extent`.
    fn pass_extent(&mut self, comp: &mut C, at: &Measuring<'_, '_>, _style: &C::Style, _shown: Option<Arc<ValueOf<C>>>) -> Vec2 {
        self.extent(comp, at.ui, at.place, at.at, at.resolved.id, at.scx)
    }

    /// The width by which the shape's content wants to be wider than the size it measured to, because the room cut
    /// it: the parent `Ui` counts it in the size it took, so a parent that sizes from its content widens.
    fn overflow(&self) -> f32 {
        0.0
    }

    /// What the shape lays out before the component's `parts` runs, from its rect and content rect: a collection's
    /// rows this pass, for `PartsCx::rect_of`; a container's header slots, for `PartsCx::header_slot`.
    fn geometry(
        &mut self,
        _comp: &C,
        _cx: &crate::contract::MeasureCx<'_, C>,
        _rects: (Rect, Rect),
        _rtl: bool,
    ) -> Option<std::rc::Rc<dyn std::any::Any>> {
        None
    }

    /// Step 4: the shape's part of the value step (selection snapshot, scroll offset).
    fn read(&mut self, _comp: &mut C, _stage: &mut MemoryStage, _id: Id, _env: &Env) {}

    /// The command scope the component is an instance of: its own `Cx` and the items below see it in the chain.
    fn instance_scope(&self, _comp: &C) -> Option<commands::CommandScope> {
        None
    }

    /// The token scope of the component and everything it shows (`ContainerProps::theme`).
    fn theme_scope(&self, _comp: &C) -> Option<ThemeScope> {
        None
    }

    /// The rank by which the component leaves a shrinking row whole (`ContainerProps::fit_rank`).
    fn fit_rank(&self, _comp: &C) -> Option<u8> {
        None
    }

    /// The component whose memory holds this one's shown memory, when it is another: a popup body's memory belongs to
    /// the component that shows the popup, so it lasts while that component shows, open or closed.
    fn memory_owner(&self, _comp: &C) -> Option<Id> {
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

    fn focus(&self, comp: &C) -> Option<crate::runtime::focus::FocusPolicy> {
        let scope = comp.focus_scope();
        (scope != crate::runtime::focus::FocusPolicy::NONE).then_some(scope)
    }

    fn sized(&self, comp: &C, ui: &Ui, place: &Env, id: Id, size: Vec2) -> Vec2 {
        use craft_behavior::Axis;
        let room = crate::layout::leaf_fill_room(ui, place);
        // A leaf is given an axis its enclosing body does not size from its content: there it fills, elsewhere it keeps
        // its measured size, so it never sets that size.
        let content = place.content;
        let fill = |measured: f32, offered: f32, along: Axis| {
            let given = if along == Axis::X { !content.0 } else { !content.1 };
            crate::layout::fill_size(ui, place, id, along, measured, offered, given)
        };
        // The control beside an inspector's label fills the cell the label leaves, unless its caller sized it to its
        // content.
        let sizing = comp.common(super::Internal::new()).sizing;
        if sizing == Some(super::Sizing::Fill) {
            return Vec2::new(fill(size.x, room.x, Axis::X), fill(size.y, room.y, Axis::Y));
        }
        let in_cell = place.cell_fills && sizing != Some(super::Sizing::Content);
        match comp.fills().or(in_cell.then_some(Axis::X)) {
            Some(Axis::X) => Vec2::new(fill(size.x, room.x, Axis::X), size.y),
            Some(Axis::Y) => Vec2::new(size.x, fill(size.y, room.y, Axis::Y)),
            _ => size,
        }
    }
}

/// The run's `CxCore`, on the heap: a measuring run and a body recurse while their run's core is alive, so a core in
/// their frames would sit on the stack once per nesting level.
#[allow(clippy::too_many_arguments, reason = "one argument per field of the context the pipeline builds once per show")]
fn blank_core<'a>(
    env: EnvView<'a>,
    resolved: &'a Resolved<'a>,
    memory: &'a mut MemoryStage,
    ctx: Context,
    ink: Ink,
    response: Response,
    command: Option<Arc<crate::runtime::commands::ResolvedCommand>>,
) -> Box<CxCore<'a>> {
    let rect = Rect::NOTHING;
    Box::new(CxCore {
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
        parted: false,
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
        pointer_group: None,
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
        header_stops: Vec::new(),
        roving_member: false,
        press_cancel: None,
        scroll_moves: Vec::new(),
        value_parts: Default::default(),
        own_reset_parts: Default::default(),
        state_layer_parts: Default::default(),
        toggles: Vec::new(),
        reset_part: None,
        root_sense: Sense::empty(),
        roving_lead: None,
        collection_events: None,
        visible_rows: 0..0,
        header: Default::default(),
        column_requests: Vec::new(),
        gestures: Default::default(),
        texts: None,
        selections: Vec::new(),
        faded: false,
        node: false,
        fade: None,
        focus_shapes: Vec::new(),
        focus_shape: None,
        surface_slot: None,
        editor: None,
        editor_inputs: Vec::new(),
        text_invalid: false,
        text_unparsed: None,
        text_takes_pointer: false,
        grid: crate::runtime::access::GridPlace::default(),
        escape_to_list: false,
        text_access: std::cell::RefCell::new(None),
        value_text: std::cell::RefCell::new(None),
        submitted: false,
        escaped: false,
    })
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
    pub binds_view: bool,
    /// `DISABLED` is `Faded`.
    pub faded: bool,
    /// `RULE`.
    pub rule: bool,
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
            binds_view: C::BINDS_VIEW,
            faded: C::DISABLED == super::DisabledLook::Faded,
            rule: C::RULE,
            style_key: TypeId::of::<C::StyleKey>(),
        }
    }

    /// Whether the component's input is a text part or an editor.
    fn text(&self) -> bool {
        matches!(self.input, Input::Text | Input::Editor)
    }

    /// Whether the component holds cells the keyboard moves between, its text part editing one cell at a time. Read
    /// only, it stays a Tab stop, so the keyboard still reaches its cells to read and copy them.
    fn holds_cells(&self) -> bool {
        [Role::GRID, Role::TABLE, Role::TREE].contains(&self.role)
    }
}

/// A component and its shape's hooks, with the component type erased, as steps 1 to 5, the measuring runs and step 17
/// reach them. It keeps the typed values (the shown value, the cancels, the style) between the calls.
pub(crate) trait Erased {
    fn facts(&self) -> TypeFacts;
    fn common(&self) -> &Common;
    /// Moves `Common` out of the component (step 1).
    fn take_common(&mut self) -> Common;
    /// The variant asked for (or the type's default in `env`, or the style's first) and the variant the type accepts
    /// for it.
    fn variants(&self, asked: Option<Variant>, env: &Env) -> (Variant, Variant);
    /// Step 2.
    fn style(&mut self, ctx: &Context, scx: &StyleCx<'_>) -> Arc<dyn StyleView>;
    /// Step 4: keeps the shown value, and outside a sizing pass the cancels it read. Returns whether the binding is mixed.
    fn value(&mut self, stage: &mut MemoryStage, id: Id, command: Option<&ResolvedCommand>, sizing: bool) -> bool;
    /// Step 4 as a measure reads it: a peek. Returns whether the binding is mixed.
    fn peek_value(&mut self, ctx: &Context, id: Id, command: Option<&ResolvedCommand>) -> bool;
    fn bind_key(&mut self, id: Id) -> Option<BindKey>;
    fn fills(&self) -> Option<craft_behavior::Axis>;
    fn cut_first(&self) -> bool;
    /// What `C::measure` gives for the shown value.
    fn measure_content(&mut self, input: &MeasureIn<'_>, resolved: &Resolved<'_>) -> (Vec2, Inputs);
    fn measured_extent(&mut self, at: &Measuring<'_, '_>, stage: &mut MemoryStage) -> Vec2;
    fn pass_extent(&mut self, at: &Measuring<'_, '_>) -> Vec2;
    /// Builds the component's `Cx` over `core` and `style` (what `style` gave), runs `rest` with it, then `finish`.
    /// `rest` returns whether a real pass ran.
    fn with_cx<'a>(&mut self, core: &'a mut CxCore<'a>, style: &'a dyn StyleView, rest: &mut dyn FnMut(&mut dyn ErasedPass<'a>) -> bool);

    fn shape(&self) -> ShapeKind;
    fn declares_parts(&self) -> bool;
    fn read(&mut self, stage: &mut MemoryStage, id: Id, env: &Env);
    fn instance_scope(&self) -> Option<commands::CommandScope>;
    fn theme_scope(&self) -> Option<ThemeScope>;
    /// The rank by which the component leaves a shrinking row whole.
    fn fit_rank(&self) -> Option<u8> {
        None
    }
    fn memory_owner(&self) -> Option<Id>;
    fn in_flow(&self) -> bool;
    /// Whether the shape fills the width of the room it is shown in (a body not sized from its content).
    fn fills_width(&self) -> bool;
    /// How much wider than the size it measured to the shape's content wants to be (`ShapeHooks::overflow`).
    fn overflow(&self) -> f32;
    fn sized(&self, ui: &Ui, place: &Env, id: Id, size: Vec2) -> Vec2;
    fn arena(&self) -> bool;
    fn extent(&mut self, ui: &Ui, place: &Env, at: ShowAt, id: Id, scx: &StyleCx<'_>) -> Vec2;
    fn pad(&self, tokens: &UiTokens) -> f32;
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
    /// `Component::wheel_notch`.
    fn wheel_notch(&self) -> craft_behavior::WheelNotch;
    /// Whether the component is bound, and whether its binding has a reset value.
    fn bound(&mut self) -> (bool, bool);
    fn paint_outset(&self, tokens: &crate::style::UiTokens) -> f32;
    /// The axis the component fills (`Component::fills`).
    fn fills(&self) -> Option<craft_behavior::Axis>;
    /// `Component::padding` at `rect`.
    fn padding(&self, rect: Rect) -> craft_behavior::Insets;
    /// Step 7: the component's parts, then the shape's.
    fn parts(&mut self, rect: Rect, flow: Flow, content_rect: Rect, parts: &mut PartsStep, input: &MeasureIn<'_>, rtl: bool);
    fn parts_value_changed(&self) -> bool;
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
    /// The shown value when it is a `bool`.
    fn shown_bool(&self) -> Option<bool>;
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
    fn sized(&self, ui: &Ui, place: &Env, id: Id, size: Vec2) -> Vec2;
    fn fills_width(&self) -> bool;
    fn first_line(&self, ctx: &Context, id: Id, height: f32) -> Option<f32>;
    fn first_is_row_line(&self) -> bool;
    fn member(&self) -> bool;
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
        data: crate::primitives::DataMode::of(&core.state),
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
        selections: &core.selections,
        fade: core.fade,
        node: core.node,
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
    /// The env the parent installed for the parent `Ui`.
    place: &'r Env,
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
    /// The row around the leaf gave it no width: it takes no place in the flow (`layout::shrink_cap`).
    dropped: bool,
    /// The parent `Ui` was in a sizing pass.
    sizing: bool,
    /// Set by a pass that ran past the sizing check.
    real: bool,
    activated: bool,
    /// The source of the first activation `Cx::activate` injected in the passes.
    injected: Option<commands::FireSource>,
}

/// A component wider than its room widens the flow `Ui` it was placed in, toward the end of the line.
fn expand_for_overflow(ui: &mut Ui, at: ShowAt, rect: Rect, overflow: f32, rtl: bool) {
    if overflow > 0.0 && matches!(at, ShowAt::Flow) {
        ui.expand_to_include_x(if rtl { rect.min.x - overflow } else { rect.max.x + overflow });
    }
}

/// What the component senses itself: a disabled one that shows a reason on focus stays a Tab stop and senses nothing else.
/// One bound to an edit command keeps the focus where it is on a press, so the edit reaches the focused component.
fn own_sense(e: &dyn ErasedPass<'_>, resolved: &Resolved, inert: bool) -> Sense {
    match (inert, resolved.enabled) {
        (true, _) => Sense::HOVER,
        (false, true) if resolved.command.is_some_and(|command| command.edit) => e.sense().difference(Sense::TAKE_FOCUS).union(Sense::KEEP_FOCUS),
        (false, true) => e.sense(),
        (false, false) if steps::takes_focus(resolved) => Sense::HOVER | e.comp_sense().intersection(Sense::FOCUS),
        (false, false) => Sense::HOVER,
    }
}

/// The widget that holds the component's text or editor and the keys it keeps, when it has either. `drafting` is
/// whether the text part holds a draft.
fn text_holder(id: Id, parts: &PartsStep, base: focus::KeyUse, drafting: bool) -> (Option<Id>, Option<focus::KeyUse>) {
    let text = parts.text.decl.as_ref().map(|decl| {
        let decl = &decl.part;
        (crate::primitives::text_adapter::text_id(id, decl.key), crate::primitives::text_adapter::keys_of(base, &decl.options, drafting))
    });
    let editor =
        parts.text.editor.as_ref().map(|decl| (crate::primitives::ImeSurface::of(id, decl.key).widget(), super::editor::keys_of(base, decl.purpose)));
    match (text, editor) {
        (Some((widget, keys)), _) | (None, Some((widget, keys))) => (Some(widget), Some(keys)),
        (None, None) => (None, None),
    }
}

/// `keys`, those of the component's text part, with the edits it cannot run now inactive (`text_adapter::with_idle`),
/// read after the text step, so they are as the pass's edits left the selection and the undo steps. The text part's
/// keys are read again from `base` with the draft the text step left, which decides whether a rewriting Tab is kept.
fn idle_keys(ctx: &Context, id: Id, parts: &PartsStep, keys: focus::KeyUse, base: focus::KeyUse, memory: &mut MemoryStage) -> focus::KeyUse {
    let Some(decl) = &parts.text.decl else { return keys };
    let decl = &decl.part;
    let held = crate::contract::text::held_selection(memory, id);
    let draft = crate::contract::text::draft(memory, id);
    let keys = crate::primitives::text_adapter::keys_of(base, &decl.options, draft.is_some());
    let widget = crate::primitives::text_adapter::text_id(id, decl.key);
    crate::primitives::text_adapter::with_idle(ctx, widget, held, draft.as_deref().unwrap_or(&decl.shown), keys)
}

/// What the stops of a component read: the owner, its place in a member group and the text it holds.
struct StopsStep<'a> {
    ctx: &'a Context,
    id: Id,
    rtl: bool,
    env: &'a Env,
    resolved: &'a Resolved<'a>,
    holder_widget: Option<Id>,
    member: Option<crate::runtime::env::ArenaId>,
    own: Sense,
    text_keys: Option<focus::KeyUse>,
}

/// Registers the focus stops of a component that has any; one that sets `STOPS = false` has none, which a debug build checks.
fn stops_step(at: StopsStep<'_>, e: &mut dyn ErasedPass<'_>, facts: &TypeFacts, parts: &PartsStep) {
    let StopsStep { ctx, id, rtl, env, resolved, holder_widget, member, own, text_keys } = at;
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
    // A stop in the body of a canvas with a view is on a document; the canvas's own role says it itself.
    let role = e.key_role().nudge(keys.nudge);
    let role = role.document(role.document || env.in_view_body());
    let role = role.type_ahead(role.type_ahead || keys.type_ahead);
    // A bound component that keeps the arrows reads them as value steps; a text value sets `steps_value` in its text options.
    let arrows = keys.keep == crate::runtime::focus::KeepKeys::BlockArrows;
    let value = role.value || keys.value || (bound && !role.nudge && !text && arrows);
    e.core().header_stops = steps::register_stops(&StopsIn {
        ctx,
        id,
        holder: holder_widget.unwrap_or_else(|| member.map_or(id, |arena| arena.0)),
        focusable: steps::takes_focus(resolved) && own.contains(Sense::FOCUS) && env.item != Some(id),
        parts_focus: steps::takes_focus(resolved),
        text,
        press_only: text && resolved.read_only && !facts.holds_cells(),
        keys,
        role: role.resettable(resettable).value(value).text(text).moves(role.moves || keys.moves).deletes(role.deletes || keys.deletes || text),
        rtl,
        parts,
        group: env.roving.filter(|_| e.core_ref().roving_member),
        body: env.roving,
        arena: env.arena.map(|arena| arena.0).filter(|arena| *arena != id),
        row: env.row,
        item: env.item == Some(id),
        default_stop: resolved.default_stop,
        focus_request: resolved.focus_request,
        doc_key: resolved.doc_key,
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

/// Step 6: the rect the root takes in `ui`, whose children are placed under `place`, and whether it is a fill-width
/// root held at its measured width.
fn place_root(p: &Pass<'_, '_, '_>, ui: &mut Ui, (at, place): (ShowAt, &Env), env: &Env, size: Vec2, inert: bool, rtl: bool) -> (Rect, bool) {
    let (ctx, id) = (ui.ctx().clone(), p.resolved.id);
    if p.dropped {
        return (Rect::from_min_size(ui.cursor().min, Vec2::ZERO), false);
    }
    let first = |height: f32| p.p.first_line(&ctx, id, height);
    // In a content-sized column a fill-width child takes its measured width in the flow and shows across the width
    // the column settles on, so it never sets that width.
    let held = !inert
        && matches!(at, ShowAt::Flow)
        && ((env.content_column && p.p.fills_width()) || (crate::layout::in_grid_content_column(&ctx, place) && p.p.fills_width_only()));
    let rect = if held {
        // The column's settled width, read before the child takes its place in it. A sizing or measuring run has no
        // settled width yet: there the child shows at its own size.
        let column = (!ui.is_sizing_pass() && !crate::runtime::measure::active(ui.ctx()))
            .then(|| crate::layout::flow_room(ui, place).x)
            .filter(|width| width.is_finite())
            .unwrap_or(0.0);
        let line = p.p.first_is_row_line();
        let flowed = steps::place(ui, place, at, Vec2::new(p.measured.x.min(size.x), size.y), InFlow { fill: false, first: first(size.y), line });
        // The column's first sweep learns which of its held children measured narrower than it settles on.
        if let Some(ColumnSweep::Count(column)) = env.column {
            crate::layout::report_held(&ctx, column, id, crate::layout::HeldKind::Fill, flowed.width());
        }
        let across = Vec2::new(size.x.max(flowed.width()).max(column), flowed.height());
        if rtl { Rect::from_min_size(Pos2::new(flowed.max.x - across.x, flowed.min.y), across) } else { Rect::from_min_size(flowed.min, across) }
    } else {
        let fill = p.p.fills_width() && !p.p.fills_width_only();
        let size = if inert { Vec2::ZERO } else { size };
        let placed = steps::place(ui, place, at, size, InFlow { fill, first: first(size.y), line: p.p.first_is_row_line() });
        // A picture its measure held (`MeasureCx::picture_size`) reports its placed width, padding included.
        if let Some(ColumnSweep::Count(column)) = env.column
            && crate::layout::held_kind(&ctx, column, id) == Some(crate::layout::HeldKind::Picture)
        {
            crate::layout::report_held(&ctx, column, id, crate::layout::HeldKind::Picture, placed.width());
        }
        placed
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

/// Steps 6 to 16, in the `Ui` the host gives, inside the component's focus scope. Children run inside the overlay
/// step, `paint`, the content and `paint_over`; each step group is its own function, so during a child's run this
/// level keeps only its `HostStep`, on the heap.
fn run_in_host(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, host_at: Option<ShowAt>, host_env: Option<&Env>) {
    let scope = enter_scope(p, ui);
    let env = host_env.unwrap_or(p.env);
    // A host that runs the pass in a `Ui` of its own installs the env it gives for it; else the pass runs in the parent.
    let place = host_env.unwrap_or(p.place);
    if let Some(mut h) = sense_steps(p, ui, (host_at, place), env) {
        update_steps(p, env, &h);
        overlay_step(p, ui, env, &h);
        state_steps(p, ui, env, &h);
        paint_step(p, ui, env, &mut h);
        content_step(p, ui, env, &h);
        paint_over_step(p, ui, env, &h);
        report_steps(p, ui, env, &h);
    }
    if let Some(ctx) = scope {
        focus::pop_scope(&ctx);
    }
}

/// Opens the component's focus scope, outside a sizing pass. Returns the context to close it with.
fn enter_scope(p: &Pass<'_, '_, '_>, ui: &Ui) -> Option<Context> {
    let policy = if p.sizing || ui.is_sizing_pass() { None } else { p.p.focus() };
    policy.map(|policy| {
        let ctx = p.p.core_ref().ctx.clone();
        focus::push_scope(&ctx, p.resolved.id, policy, None);
        ctx
    })
}

/// What steps 9 to 16 of one pass read of steps 6 to 8b.
struct HostStep {
    ctx: Context,
    id: Id,
    /// The component's own direction: right to left unless its axis keeps its order.
    rtl: bool,
    rect: Rect,
    /// What step 7 declared: the parts, roving groups and text parts. The stops register from it after `update`.
    parts: PartsStep,
    /// The egui widget of the component's text part or editor, which holds key focus for it.
    holder_widget: Option<Id>,
    text_keys: Option<focus::KeyUse>,
    own: Sense,
    /// The sense the access node reports: an item's own, with the click the collection takes for it, so the node
    /// still offers the redirected action.
    report_sense: Sense,
    member: Option<crate::runtime::env::ArenaId>,
    /// The bar line the member sits in, in the run that paints: a rule notes itself to it instead of painting.
    line: Option<Id>,
    /// A bar's view cut the member: it shows nothing until the clip is restored.
    cut: bool,
    shown_clip: Rect,
    inspected: bool,
    /// The values `content_env` gave the content at step 13.
    values: AppValues,
    /// The clip around the component, and the clip inside its rect grown by its paint outset.
    outer_clip: Rect,
    inside: Rect,
}

/// Steps 6 to 8b: place, parts, sense and the text part. `None` when the pass stops before 8b: a sizing pass, or an
/// inert component.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn sense_steps(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, (host_at, place): (Option<ShowAt>, &Env), env: &Env) -> Option<Box<HostStep>> {
    let ctx = p.p.core_ref().ctx.clone();
    let (id, rtl) = (p.resolved.id, p.env.direction.is_rtl());
    let (resolved, inert, facts) = (p.resolved, p.inert, p.facts);
    // 6 place
    let at = host_at.unwrap_or(p.at);
    // A root that fills takes the room of the `Ui` its host runs it in: a pane's or a layer's own, not its parent's.
    let size = if ui.scope_id() == p.parent { p.size } else { p.p.sized(ui, place, id, p.measured) };
    let (rect, held) = place_root(p, ui, (at, place), env, size, inert, rtl);
    p.trace.mark(Step::Place);
    // What the content wants past the room counts where the root's width comes from its content: a content-sized
    // root, or a fill-width one held at its measured width. A root that fills the flow keeps to the room it fills, and
    // a unit its row dropped takes no place.
    let overflow = if !p.dropped && (held || !p.p.fills_width()) { p.p.overflow() } else { 0.0 };
    expand_for_overflow(ui, host_at.unwrap_or(p.at), rect, overflow, rtl);
    // The component sits where the interface puts it; a component whose axis keeps its order flows its content left to right.
    let rtl = rtl && !resolved.ltr_axis;
    // A sizing pass of the parent or host `Ui` allocates the size and stops: no parts, no body, and the stage
    // opened at step 3 is written back unchanged.
    if p.sizing || ui.is_sizing_pass() {
        // The output words the value in a measuring run too, so what is measured beside the component holds it.
        let text = p.p.value_text();
        *p.p.core_ref().value_text.borrow_mut() = text;
        return None;
    }
    p.real = true;
    let inspected = p.trace.recording() && inspect::open_element(ui, id, facts.name);
    let e = &mut *p.p;
    // An inert run skips the parts step and leaves early, so it declares no parts: its parts read idle.
    e.core().parted = !inert;
    // 7 parts
    p.trace.mark(Step::Parts);
    let flow = Flow::new(rect, rtl);
    let content_rect = flow.inset(steps::padding(e.padding(rect), resolved.pad));
    let mut parts = PartsStep { decls: Default::default(), rovings: Vec::new(), scrolls: Vec::new(), text: super::text::TextStep::default() };
    if !inert && facts.text() {
        parts.text.commit_now = focus::commit_now(&ctx) == Some(id);
    }
    let declares_parts = facts.parts || e.declares_parts();
    #[cfg(feature = "testing")]
    let checking_parts = !inert && !declares_parts && crate::testing::contract::checking_parts();
    #[cfg(not(feature = "testing"))]
    let checking_parts = false;
    if !inert && (declares_parts || checking_parts) {
        let input = MeasureIn { ui: &*ui, env, proposal: Proposal::Ideal, room: p.room };
        e.parts(rect, flow, content_rect, &mut parts, &input, rtl);
    }
    #[cfg(feature = "testing")]
    if checking_parts && !declares_parts && !steps::declares_none(&parts) {
        crate::testing::contract::bad_parts(facts.name);
    }
    if facts.canvas_control && !ui.is_sizing_pass() {
        inspect::canvas_control(&ctx, facts.name);
    }
    steps::check_parts(&ctx, facts.name, &parts);
    let keys = e.keys();
    let drafting = parts.text.decl.is_some() && crate::contract::text::draft(e.core().memory, id).is_some();
    let (holder_widget, text_keys) = text_holder(id, &parts, keys, drafting);
    let own = own_sense(e, resolved, inert);
    // The collection handles a row's click, but its access node must still offer the redirected action.
    let unnarrowed = own;
    // An item never senses the whole row for a press: the collection's root takes the click and the focus (17.2). It
    // does sense the pointer over it, so `PartsCx::hovered` and the state step read the row's hover from the same
    // arena outcome whether the item stands alone or in a collection.
    let own = if env.item == Some(id) { own.difference(Sense::CLICK | Sense::FOCUS) | Sense::HOVER } else { own };
    let report_sense = own | (unnarrowed & Sense::CLICK);
    let arena = e.arena();
    let member = if arena || facts.text() { None } else { env.arena };
    // A body's wheel share is its scroll taker (body step, step 7); the gate value does not take the wheel. A body root that
    // senses the wheel itself (a canvas) registers like any other component. A collection that senses the wheel itself
    // registers in the item step, with its notch.
    let registers = match e.shape() {
        ShapeKind::Leaf => true,
        ShapeKind::Body => own.intersects(Sense::WHEEL | Sense::WHEEL_X),
        ShapeKind::Item => !own.intersects(Sense::WHEELS),
    };
    if !inert && registers {
        let sensed = craft_behavior::WheelTaker::for_sense(id, rect, own).map(|taker| taker.notch(e.wheel_notch()));
        steps::register_wheel(&ctx, id, rect, sensed, e.bound().0 && !facts.binds_view, facts.text(), env.wheel);
    }
    // 8 sense
    p.trace.mark(Step::Sense);
    let decls = parts.decls.parts();
    let enabled = resolved.enabled && !inert;
    // A control that draws only its mark senses over its own label too: indicator, gap and label are one target.
    let label = if facts.labelled_after && !inert { scope::claim(&ctx) } else { None };
    let hit = label.map_or(rect, |label| rect.union(label));
    // Only a member in flow is judged: a part shown at a rect, and a control in a label scope, belong to the member
    // around them (a labelled control's pair is judged by the label step).
    let shown_clip = ui.clip_rect();
    let judged = e.member() && matches!(at, ShowAt::Flow) && !scope::in_label(&ctx);
    let cut = judged && bar_member(&ctx, env, hit, facts.rule);
    // A bar's line takes its rules only in the run that paints.
    let line = bar_line(env).filter(|_| judged && !crate::runtime::measure::active(&ctx));
    if cut {
        ui.set_clip_rect(Rect::NOTHING);
    }
    let sensing::RootResult { mut response, parts: results, press, activation, activated_member, sense: root_sense } =
        sense_root(ui, member, RootSpec { id, rect: hit, own_sense: own, own: decls, enabled }, arena);
    if label.is_some() {
        let shown = response.rect.intersect(rect);
        response.rect = Rect::from_min_max(shown.min, shown.max.max(shown.min));
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
    core.node = node;
    // A hovered part's declared cursor, else the role's over the root's own shape.
    let own_hovered = enabled && response.hovered() && !results.values().any(|result| result.hovered);
    core.part_cursors =
        sensing::hovered_cursor(decls, &results).or_else(|| facts.role.hover_cursor().filter(|_| own_hovered).map(Into::into)).into_iter().collect();
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
    core.focus_shapes =
        parts.decls.iter().filter_map(|(decl, extras)| extras.focus_shape.clone().map(|shape| ((decl.key, decl.index), shape))).collect();
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
    core.activation = activation_of(&ctx, ui, hit, resolved, keyed, root_sense, activation);
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
        return None;
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
    if !parts.text.selections.is_empty() {
        super::text::run_selections(e.core(), &parts.text.selections);
        e.core().selections = parts.text.selections.iter().map(|decl| decl.part).collect();
    }
    Some(Box::new(HostStep {
        ctx,
        id,
        rtl,
        rect,
        parts,
        holder_widget,
        text_keys,
        own,
        report_sense,
        member,
        line,
        cut,
        shown_clip,
        inspected,
        values: AppValues::default(),
        outer_clip: Rect::NOTHING,
        inside: Rect::NOTHING,
    }))
}

/// Step 9, update, with the body's roving group, which registers after `update`.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn update_steps(p: &mut Pass<'_, '_, '_>, env: &Env, h: &HostStep) {
    let (facts, ctx, id, rtl) = (p.facts, &h.ctx, h.id, h.rtl);
    let e = &mut *p.p;
    // 9 update
    p.trace.mark(Step::Update);
    e.gesture_gate();
    steps::update_inputs(e.core());
    overlay::drive_toggles(e.core(), facts.overlays, facts.name);
    e.edit_intents();
    e.scrub_by_label();
    e.update();
    if cfg!(debug_assertions) {
        let keys = h.text_keys.unwrap_or_else(|| e.keys());
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
    // The stops register after `update`, which says whether the component joins its body's roving group, and after
    // the text step, so a Select All of the pass makes Copy active.
    let base = e.keys();
    let text_keys = h.text_keys.map(|keys| idle_keys(ctx, id, &h.parts, keys, base, &mut *e.core().memory));
    let at = StopsStep { ctx, id, rtl, env, resolved: p.resolved, holder_widget: h.holder_widget, member: h.member, own: h.own, text_keys };
    stops_step(at, e, facts, &h.parts);
    // The body's group registers after `update`, which may change its rules from what the component knows now.
    if let Some(roving) = e.roving() {
        let core = e.core_ref();
        let group = focus::Group::new(steps::body_group(id), roving).owner(id).rtl(rtl);
        let group = match core.roving_lead {
            Some(lead) => group.lead((lead, None)),
            None => group,
        };
        focus::register_group(ctx, group);
    }
}

/// Step 10, overlay: the component's popups show their content here.
fn overlay_step(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &HostStep) {
    p.trace.mark(Step::Overlay);
    if p.facts.overlays {
        let (ctx, id) = (&h.ctx, h.id);
        let core = p.p.core();
        let open = overlay::open_map(&mut *core.memory, id);
        let parts = core
            .toggles
            .iter()
            .filter_map(|toggle| core.results.get(&(toggle.part.key, toggle.part.index)).map(|result| (toggle.part, result.rect)))
            .collect();
        let mut overlays = OverlayUi::new(ctx, overlay::OverlayOwner { id, anchor: h.rect, parts }, &mut *ui, p.resolved, env.clone(), open);
        p.p.overlay(&mut overlays);
        overlay::apply_closes(ctx, overlays.closed);
    }
}

/// Steps 11 (apply) and 12 (state), after what `update` and `overlay` left untaken goes back to the key service.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn state_steps(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &HostStep) {
    let (facts, resolved, style) = (p.facts, p.resolved, p.style);
    let e = &mut *p.p;
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
    let open = facts.overlays && overlay::popup_shown(&h.ctx, h.id);
    let expanded = e.expanded() == Some(true);
    steps::state(e, style, open, expanded, env.ink);
    if e.core_ref().state.flags.contains(crate::style::Flags::FOCUSED) {
        commands::note_focus(&h.ctx, h.id, &env.commands);
    }
}

/// Step 13, paint: a composite's `paint` shows its children here. Leaves the clip inside the component's rect for
/// the content, and the values `content_env` gives it.
fn paint_step(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &mut HostStep) {
    p.trace.mark(Step::Paint);
    let e = &mut *p.p;
    let fade = steps::fade(ui, env, e.core());
    // A rule in a bar's line is the bar's to draw, with the member after it (`draw_bar_rules`).
    let bar_rule = h.line.filter(|_| p.facts.rule && !h.cut);
    if bar_rule.is_none() {
        steps::paint_surface(ui, env, e.core_ref(), p.style);
        steps::paint_part_layers(ui, env, e.core_ref());
    }
    // Paint and content stay inside the rect (grown by the type's declared outset), however small the room is.
    h.outer_clip = ui.clip_rect();
    h.inside = e.core_ref().rect.expand(e.paint_outset(&env.tokens)).intersect(h.outer_clip);
    ui.set_clip_rect(h.inside);
    match bar_rule {
        Some(line) => {
            let rect = e.core_ref().rect;
            let axis = e.fills().unwrap_or(craft_behavior::Axis::Y);
            let look = crate::style::rule_look(&env.tokens, crate::primitives::RuleKind::Divider);
            let item = LineItem::Rule { rect, clip: crate::runtime::Clip::new(h.inside), axis, look };
            crate::runtime::lazy::note_line(&h.ctx, line, item);
        }
        None => e.paint(ui, env),
    }
    // The content's values may read what `paint` found this pass.
    e.content_env(&mut h.values);
    steps::unfade(ui, e.core(), fade);
    ui.set_clip_rect(h.outer_clip);
    if bar_rule.is_none() {
        steps::paint_focus(ui, e.core_ref(), p.style, e.shape() != ShapeKind::Item);
    }
    ui.set_clip_rect(h.inside);
}

/// Step 14, content: a body's regions or a collection's rows.
fn content_step(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &HostStep) {
    if !p.trace.mark(Step::Content) {
        return;
    }
    let (ctx, id) = (&h.ctx, h.id);
    let e = &mut *p.p;
    if p.facts.role == Role::Merged {
        scope::begin(ctx, id, id, scope::Kind::Merge);
    }
    // A real run checks a body's content against the kept transcript its records came from; a collection's rows
    // are its own measure, not the enclosing body's, and a layer's content is not part of its parent's flow.
    let scoped = (!p.sizing && !crate::runtime::measure::active(ctx))
        .then(|| match e.shape() {
            ShapeKind::Body => Some((Some(id), !e.in_flow())),
            ShapeKind::Item => Some((None, true)),
            _ => None,
        })
        .flatten();
    if let Some((body, boundary)) = scoped {
        crate::runtime::layout_memory::open_content(ctx, body, boundary);
    }
    e.content(ui, env, &h.values);
    if scoped.is_some() {
        crate::runtime::layout_memory::close_content(ctx, &e.regions(id));
    }
}

/// `paint_over`, after the `over` parts are placed, so the children it shows stack above them (8.3).
fn paint_over_step(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &HostStep) {
    ui.set_clip_rect(h.outer_clip);
    sensing::place_over_widgets(ui, h.id, h.member, h.parts.decls.parts());
    ui.set_clip_rect(h.inside);
    let e = &mut *p.p;
    let fade = steps::fade(ui, env, e.core());
    e.paint_over(ui, env);
    steps::unfade(ui, e.core(), fade);
    ui.set_clip_rect(h.outer_clip);
    // Every member of the root's arena has joined: its widgets take their senses.
    if h.member.is_none() {
        sensing::widen_root(&h.ctx, h.id, p.facts.role == Role::None);
    }
}

/// Steps 15 (settle) and 16 (report, then the tooltip), and the activation step 17 fires.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn report_steps(p: &mut Pass<'_, '_, '_>, ui: &mut Ui, env: &Env, h: &HostStep) {
    let (facts, resolved, style) = (p.facts, p.resolved, p.style);
    let (ctx, id) = (&h.ctx, h.id);
    let e = &mut *p.p;
    // 15 settle
    p.trace.mark(Step::Settle);
    e.settle();
    let edit = e.settle_values();
    let leaf = e.shape() == ShapeKind::Leaf;
    steps::rerun_after_edit(e, style, edit, &MeasureIn { ui: &*ui, env: p.env, proposal: Proposal::Ideal, room: p.room }, resolved, leaf);
    // Sensing used the parts before the edit. Report their new bounds alongside the value painted in this pass. An
    // inert run never reaches this step.
    if (facts.parts || e.declares_parts()) && e.parts_value_changed() {
        let flow = Flow::new(h.rect, h.rtl);
        let content_rect = flow.inset(steps::padding(e.padding(h.rect), resolved.pad));
        let mut current = PartsStep { decls: Default::default(), rovings: Vec::new(), scrolls: Vec::new(), text: super::text::TextStep::default() };
        let input = MeasureIn { ui: &*ui, env, proposal: Proposal::Ideal, room: p.room };
        e.parts(h.rect, flow, content_rect, &mut current, &input, h.rtl);
        for decl in current.decls.parts() {
            if let Some(result) = e.core().results.get_mut(&(decl.key, decl.index)) {
                result.rect = decl.shape.bounds();
            }
        }
    }
    // 16 report, then the tooltip.
    p.trace.mark(Step::Report);
    let message = steps::report(
        e,
        facts,
        &steps::ReportIn {
            decls: &h.parts.decls,
            sense: h.report_sense,
            live: e.live(),
            scrolls: e.scrolls(),
            expanded: e.expanded(),
            position: e.position(),
            group: env.roving.filter(|_| e.core_ref().roving_member),
            owns: e.roving().map(|_| steps::body_group(id)),
        },
    );
    let shows_label = e.shows_label();
    let tip_in =
        tooltip::TipIn { ui: &*ui, env, rich: p.tip.take(), parts: &h.parts.decls, shows_label, canvas_control: facts.canvas_control, message };
    tooltip::run(e.core_ref(), tip_in);
    // Activations accumulate across the passes of a viewport mount; step 17 fires the bound command once, with the
    // injected source when there is one.
    let natural = e.activated() && resolved.enabled;
    let injected = e.core().injected.take().filter(|_| resolved.enabled);
    if (natural || injected.is_some()) && !e.core_ref().keep_open {
        crate::runtime::overlay::note_activated(ctx, id);
    }
    p.activated |= natural || injected.is_some();
    p.injected = p.injected.or(injected);
    if h.inspected {
        let core = e.core_ref();
        inspect::close_element(ctx, id, core.rect, core.content_rect);
    }
    if h.cut {
        ui.set_clip_rect(h.shown_clip);
    }
}

/// The bar line a component in `env` sits in: the scrolling body of a bar, by its id.
fn bar_line(env: &Env) -> Option<Id> {
    if !env.values.get::<crate::contract::InBar>().0 {
        return None;
    }
    env.lazy.filter(|view| view.axes.0).map(|view| view.owner)
}

/// Judges a member of a bar's line whose hit target is `hit` (a control with the label it claims): whether its view
/// shows it only in part. Such a member is not shown until a scroll brings it in whole; it keeps its stop, so Tab
/// reaches it and the reveal scrolls it in. In the run that paints, a member that is not itself a `rule` is noted to
/// the line, so the rules before it show with it or not (`draw_bar_rules`). Outside a bar's line nothing is cut.
pub(crate) fn bar_member(ctx: &Context, env: &Env, hit: Rect, rule: bool) -> bool {
    let Some(line) = bar_line(env) else { return false };
    let cut = cut_by_bar_view(env, hit);
    if !rule && !crate::runtime::measure::active(ctx) {
        crate::runtime::lazy::note_line(ctx, line, LineItem::Member { cut });
    }
    cut
}

/// Closes a bar's line in `ui`, the line's own: draws each rule of `line` (what its run and the replays in it noted, in
/// run order) that a member the view shows comes after, in the look, clip and place the rule had. A rule before a
/// member the view cut, or with no member after it, is not drawn.
pub(crate) fn draw_bar_rules(ui: &Ui, line: &[LineItem]) {
    let mut waiting = Vec::new();
    for item in line {
        match item {
            LineItem::Rule { .. } => waiting.push(item),
            LineItem::Member { cut } => {
                if !cut {
                    waiting.iter().for_each(|rule| draw_bar_rule(ui, rule));
                }
                waiting.clear();
            }
        }
    }
}

/// Draws the rule `rule` of a bar's line in `ui`.
fn draw_bar_rule(ui: &Ui, rule: &LineItem) {
    let LineItem::Rule { rect, clip, axis, look } = rule else { return };
    let (from, to) = match axis {
        craft_behavior::Axis::X => (rect.left_center(), rect.right_center()),
        _ => (rect.center_top(), rect.center_bottom()),
    };
    let painter = ui.painter().with_clip_rect(clip.rect());
    painter.extend(crate::primitives::Rule::shapes(look, from, to, painter.pixels_per_point()));
}

/// Whether `rect`, in the scrolling body of a bar, lies partly outside the body's view along x. A member wider than the
/// view shows in part, since no scroll shows it whole.
pub(crate) fn cut_by_bar_view(env: &Env, rect: Rect) -> bool {
    if !env.values.get::<crate::contract::InBar>().0 {
        return false;
    }
    let Some(view) = env.lazy.filter(|view| view.axes.0) else { return false };
    // Half a point: less than a pixel shows at any scale the box runs.
    let outside = rect.min.x < view.rect.min.x - 0.5 || rect.max.x > view.rect.max.x + 0.5;
    outside && rect.width() <= view.rect.width() + 0.5
}

/// Step 5's size for `input`: what the component measures plus `extent`, padded and bounded by its style.
fn measured(t: &mut dyn Erased, input: &MeasureIn<'_>, style: &dyn StyleView, resolved: &Resolved<'_>, extent: Vec2) -> (Vec2, Inputs) {
    let (content, inputs) = t.measure_content(input, resolved);
    (steps::measure(style, input, content, extent, resolved), inputs)
}

/// The pipeline: steps 1 to 17 for one component, whatever its type and shape. A component that takes a cell of a
/// `Grid` runs under the column group of its cell's column (`ColumnGroup::cell`).
/// Runs `t` in `ui`, whose children are placed under `place`: the env its parent installed for it, which every step
/// of the run reads the place from.
pub(super) fn run(t: &mut dyn Erased, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>, place: Env) {
    if crate::runtime::pass_state::in_component_tree() {
        return run_in_context(t, ui, at, tip, place);
    }
    let ctx = ui.ctx().clone();
    crate::runtime::pass_state::with_component_tree(&ctx, || run_in_context(t, ui, at, tip, place));
}

fn run_in_context(t: &mut dyn Erased, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>, place: Env) {
    let ctx = ui.ctx().clone();
    let cell = (matches!(at, ShowAt::Flow) && t.in_flow()).then(|| crate::layout::grid_cell(&ctx, &place)).flatten();
    match cell {
        Some((grid, column, sized)) => crate::layout::ColumnGroup::cell(&ctx, grid, column, sized, || run_steps(t, ui, at, tip, place)),
        None => run_steps(t, ui, at, tip, place),
    }
}

/// Steps 1 to 17 of `run`. A component's children run inside step 5 (a body's measuring run) and inside the passes
/// of steps 6 to 16, so the frames on that path keep only what the run reads after them: each step runs in its own
/// function, and `run_steps` holds what steps 1 to 5 found.
fn run_steps(t: &mut dyn Erased, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>, place: Env) {
    let begun = begin(t, ui, place);
    let mut stage = begun.open_stage(t);
    let mixed = value_step(t, &begun, &mut stage);
    let resolved = begun.resolved(t);
    let room = room_step(t, ui, at, &begun);
    let extent = extent_step(t, ui, (at, mixed), (&begun, &resolved), &mut stage);
    let sizes = measure_step(t, ui, at, (&begun, &resolved, &room), extent, &mut stage);
    let real = run_passes(t, ui, at, tip, (&begun, &resolved, &sizes), &mut stage);
    if real {
        t.record(&mut stage, &begun.ctx, resolved.id);
    }
    if !begun.sizing {
        stage.write_back();
    }
}

/// What steps 1 to 4 found, which the rest of the run reads. It lives on the heap ([`begin`]): `run_steps` holds it
/// while step 5's measuring runs recurse.
struct Begun {
    ctx: Context,
    facts: TypeFacts,
    shape: ShapeKind,
    /// The parent `Ui` is in egui's invisible sizing pass: steps 1 to 6 run and allocate the size, with no id
    /// registration, sensing or memory writes.
    sizing: bool,
    /// The inspector records this run; read once per run.
    recording: bool,
    /// The env the parent installed for the parent `Ui`, under which the run is placed.
    place: Env,
    /// The place env in the component's token scope, which the style reads.
    env: Env,
    /// The env with the component's own command instance, when it has one.
    own_env: Option<Env>,
    /// `Common`, moved out of the component; `Resolved` borrows it.
    common: Common,
    step1: steps::EnvStep,
    style: Arc<dyn StyleView>,
    id: Id,
}

impl Begun {
    /// The env steps 4 to 17 run under.
    fn env(&self) -> &Env {
        self.own_env.as_ref().unwrap_or(&self.env)
    }

    fn trace(&self) -> Trace<'_> {
        Trace::new(&self.ctx, self.id, self.shape, self.recording)
    }

    /// The memory stage of step 3: a copy in a sizing pass, the component's own stage otherwise.
    fn open_stage(&self, t: &dyn Erased) -> MemoryStage {
        if self.sizing {
            MemoryStage::open_peek(&self.ctx, self.id)
        } else {
            MemoryStage::open_owned(&self.ctx, self.id, t.memory_owner().unwrap_or(self.id))
        }
    }

    /// `Common` after step 1, with the padding the shape adds.
    fn resolved(&self, t: &dyn Erased) -> Resolved<'_> {
        Resolved { pad: t.pad(&self.env().tokens), ..steps::resolved(self.id, &self.step1, &self.common) }
    }
}

/// Steps 1 to 3: env, style and id.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn begin(t: &mut dyn Erased, ui: &Ui, place: Env) -> Box<Begun> {
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
    // 1 env: `Common` moves out of the component; `Resolved` borrows it. A themed component resolves its own look in its
    // scope, and everything it shows inherits it.
    let env = scoped_env(&ctx, place.clone(), t.theme_scope());
    let common: Common = t.take_common();
    let step1 = steps::env(&ctx, &env, &common, t.variants(common.variant, &env));
    // 2 style
    let style = t.style(&ctx, &steps::style_cx(&ctx, &env, &step1));
    // 3 id
    let id = steps::id(ui, &common.id, !sizing, true, facts.style_key, facts.name);
    if recording {
        inspect::begin(&ctx, id);
        let trace = Trace::new(&ctx, id, shape, recording);
        for step in [Step::Env, Step::Style, Step::Id] {
            trace.mark(step);
        }
    }
    // The command instance the component is, which steps 4 to 17 run under.
    let own_env = t.instance_scope().map(|scope| env.derive_local(|scoped| scoped.inherited_mut().commands = scoped.commands.pushed(scope, id)));
    Box::new(Begun { ctx, facts, shape, sizing, recording, place, env, own_env, common, step1, style, id })
}

/// Step 4, value: returns whether the binding holds a mixed value. A sizing pass reads the cancel and ends nothing:
/// its stage is a copy, and the real run reads the cancel again.
fn value_step(t: &mut dyn Erased, begun: &Begun, stage: &mut MemoryStage) -> bool {
    let mixed = t.value(stage, begun.id, begun.step1.command.as_deref(), begun.sizing);
    t.read(stage, begun.id, &begun.env);
    begun.trace().mark(Step::Value);
    mixed
}

/// What step 5 found: the room, the measured size and the size the root takes.
struct Sizes {
    /// The width the parent offers, which step 5 and step 7's measure view both read.
    room: Option<f32>,
    /// The size step 5 measured, before `sized` made it fill.
    measured: Vec2,
    /// The size the root takes in the parent `Ui`.
    size: Vec2,
    inert: bool,
    /// The row around the leaf gave it no width: it takes no place in the flow (`layout::shrink_cap`).
    dropped: bool,
}

/// What step 5 reads of the room before the measuring run: the width the parent offers and the shrink step of the
/// body around.
#[derive(Clone, Copy)]
struct Room {
    room: Option<f32>,
    in_flow: bool,
    fill_body: bool,
    shrink: Option<crate::layout::Shrink>,
    cap: Option<f32>,
    dropped: bool,
}

/// Step 5's room: what the parent offers, capped by the width the row around gave a leaf.
fn room_step(t: &mut dyn Erased, ui: &Ui, at: ShowAt, begun: &Begun) -> Room {
    let (ctx, shape, id) = (&begun.ctx, begun.shape, begun.id);
    let env = begun.env();
    let room = match at {
        ShowAt::Rect(rect) => Some(rect.width()),
        // A measuring run in a content column asks for what the content needs: nothing there fills the room. Once the
        // column settled its width, a child that fills its width, or a picture its first sweep held, measures at
        // that width.
        ShowAt::Flow => match env.column {
            Some(ColumnSweep::Column(column, width))
                if env.content_column && (t.fills_width() || crate::layout::held_kind(ctx, column, id) == Some(crate::layout::HeldKind::Picture)) =>
            {
                Some(width)
            }
            _ => Some(crate::layout::flow_room(ui, &begun.place).x)
                .filter(|width| width.is_finite() && !(env.content_column && crate::runtime::measure::active(ctx))),
        },
    };
    // A leaf or body in flow that does not fill takes part in the shrink step of the body around it: in a row that
    // shrinks its children to fit, a leaf shows at most the width the row gave it, and a body takes that width as its
    // room (`mount_room`). A body that fills the width of a bounded room takes the room or its share of it (`sized`),
    // so it is no unit.
    let in_flow = matches!(at, ShowAt::Flow) && t.fills().is_none();
    let fill_body = shape == ShapeKind::Body && in_flow && t.in_flow() && t.fills_width() && crate::layout::flow_room(ui, &begun.place).x.is_finite();
    let shrink = env.shrink.filter(|_| in_flow && (shape == ShapeKind::Leaf || (shape == ShapeKind::Body && t.in_flow() && !fill_body)));
    let given = match shrink {
        Some(crate::layout::Shrink::Apply(row)) => crate::layout::shrink_cap(ctx, row, id),
        _ => None,
    };
    let dropped = given.is_some_and(|given| given.dropped);
    // A body takes its width from its own size step; the cap only tells it whether it was dropped.
    let cap = given.filter(|_| shape == ShapeKind::Leaf).map(|given| given.width);
    let room = match cap {
        Some(cap) => Some(room.map_or(cap, |room| room.min(cap))),
        None => room,
    };
    Room { room, in_flow, fill_body, shrink, cap, dropped }
}

/// Step 5's extent: a sizing pass measures the shape from its content; in a real pass a body or item shape takes
/// what its measuring run found. A body's children run here, in its measuring run.
fn extent_step(
    t: &mut dyn Erased,
    ui: &Ui,
    (at, mixed): (ShowAt, bool),
    (begun, resolved): (&Begun, &Resolved<'_>),
    stage: &mut MemoryStage,
) -> Vec2 {
    let (ctx, facts, shape, sizing, id) = (&begun.ctx, &begun.facts, begun.shape, begun.sizing, begun.id);
    let env = begun.env();
    // A body root with parts steps its press machine before its measuring run, so the body reads this pass's part
    // presses in both runs (`BodyCx::part_press`) and its host reads them before it places the body. Step 8 finds the
    // outcome. A member of another arena senses at step 8. A root its host shows on a layer of its own (not in flow)
    // senses in its own arena: its steps 6 to 16 run in the layer's env, which is outside its opener's arena.
    let own_arena = t.arena() || facts.text() || env.arena.is_none() || !t.in_flow();
    if !sizing && shape == ShapeKind::Body && (facts.parts || t.declares_parts()) && own_arena {
        sensing::sense_early(ctx, id, resolved.enabled);
    }
    let scx = steps::style_cx(ctx, &begun.env, &begun.step1);
    if sizing || matches!(shape, ShapeKind::Body | ShapeKind::Item) {
        let measuring = Measuring { ui, at, width: None, place: &begun.place, env, scx: &scx, resolved, command: begun.step1.command.clone(), mixed };
        if sizing { t.measured_extent(&measuring, stage) } else { t.pass_extent(&measuring) }
    } else {
        t.extent(ui, &begun.place, at, id, &scx)
    }
}

/// Step 5's sizes from the `extent`, and what the rows and bodies around the component take from it.
// Kept out of the recursive frame above it: opt-level 2 inlines it there, so its locals stay on the stack while children run.
#[inline(never)]
fn measure_step(
    t: &mut dyn Erased,
    ui: &Ui,
    at: ShowAt,
    (begun, resolved, room): (&Begun, &Resolved<'_>, &Room),
    extent: Vec2,
    stage: &mut MemoryStage,
) -> Sizes {
    let (ctx, facts, shape, sizing, id) = (&begun.ctx, &begun.facts, begun.shape, begun.sizing, begun.id);
    let env = begun.env();
    let style = &*begun.style;
    let Room { room, in_flow, fill_body, shrink, cap, dropped } = *room;
    let ask = |t: &mut dyn Erased, proposal: Proposal, room: Option<f32>, extent: Vec2| {
        measured(t, &MeasureIn { ui: &*ui, env, proposal, room }, style, resolved, extent)
    };
    let (size, inputs) = ask(&mut *t, Proposal::Ideal, room, extent);
    let ideal = Call { proposal: Proposal::Ideal, room, inputs, took: size };
    // A control in a label scope reports its narrowest width, in a measuring run too: the real run's label step reads
    // it to decide whether the row fits the label beside it.
    let in_label = shape == ShapeKind::Leaf && scope::in_label(ctx);
    // A leaf that fills the main axis of its body tells the body its least in the body's first sweep: its share never
    // cuts it below that (`layout::fill_size`). The call joins the body's transcript, which holds it while the inputs
    // hold.
    let fills = |axis: craft_behavior::Axis| t.fills() == Some(axis) || begun.common.sizing == Some(super::Sizing::Fill);
    let tells_least = shape == ShapeKind::Leaf
        && matches!(at, ShowAt::Flow)
        && [craft_behavior::Axis::X, craft_behavior::Axis::Y]
            .into_iter()
            .any(|axis| fills(axis) && crate::layout::counting_fills(ctx, &begun.place, axis));
    let min = (in_label || tells_least).then(|| {
        let (took, inputs) = ask(&mut *t, Proposal::Min, None, Vec2::ZERO);
        Call { proposal: Proposal::Min, room: None, inputs, took }
    });
    let min_took = min.as_ref().filter(|_| in_label).map(|min| min.took);
    // A leaf's measure calls are what its body's layout takes from it: a measuring run keeps them in the body's
    // transcript, and a real run its kept transcript serves checks them against this run's. A leaf shown at a rect
    // takes no room in the flow: its body's layout takes the rect, and any measure its caller sized the rect by
    // records itself.
    let leaf = shape == ShapeKind::Leaf && matches!(at, ShowAt::Flow) && (!sizing || crate::runtime::measure::active(ctx));
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
        crate::runtime::layout_memory::leaf_call(ctx, id, calls, |proposal, room| ask(&mut *t, proposal, room, extent).0);
    }
    begun.trace().mark(Step::Measure);
    let min = min_took.inspect(|min| scope::record_control_min(ctx, min.x));
    // The read record: the binding step 4 read, and the sizes step 5 made from it.
    if let Some(key) = t.bind_key(id) {
        let ideal = Sample { proposal: Proposal::Ideal, room, took: size };
        let measuring = sizing || crate::runtime::measure::active(ctx);
        match min {
            Some(min) => reads::note(ctx, key, id, measuring, &[ideal, Sample { proposal: Proposal::Min, room: None, took: min }]),
            None => reads::note(ctx, key, id, measuring, &[ideal]),
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
    let fit_room = (leaf_in_flow && shrink == Some(crate::layout::Shrink::Fit))
        .then(|| crate::layout::flow_room(ui, &begun.place).x)
        .filter(|room| room.is_finite());
    let cap = match fit_room.filter(|room| size.x > *room) {
        Some(room) => Some(cap.map_or(room, |cap| cap.min(room)).max(least(&mut *t))),
        None => cap,
    };
    // The row gave the leaf no less than its least, so a leaf whose measure does not follow its room still fits.
    let size = cap.map_or(size, |cap| Vec2::new(size.x.min(cap), size.y));
    let measured = size;
    // A component on a layer or in a window of its own takes no room in the parent's flow: the parent neither sizes it
    // nor counts it among the children that share its room. Its host sizes it in its own `Ui`.
    let size = if t.in_flow() { t.sized(ui, &begun.place, id, size) } else { size };
    match shrink {
        Some(crate::layout::Shrink::Count(row)) if shape == ShapeKind::Leaf => {
            // The `Ideal` size in no room, and the least width at which its content shows whole.
            let input = MeasureIn { ui, env, proposal: Proposal::Ideal, room: None };
            let (content, _) = t.measure_content(&input, resolved);
            let ideal = steps::measure(style, &input, content, Vec2::ZERO, resolved).x;
            let whole = steps::whole(style, content, Vec2::ZERO, resolved);
            let least = ask(&mut *t, Proposal::Min, None, Vec2::ZERO).0.x;
            crate::layout::report_shrink(ctx, row, id, size.x, ideal, Some((whole, least)), (t.cut_first(), None));
        }
        Some(crate::layout::Shrink::Count(row)) => crate::layout::report_shrink(ctx, row, id, size.x, size.x, None, (false, t.fit_rank())),
        Some(crate::layout::Shrink::Least(row)) if shape == ShapeKind::Body => crate::layout::report_least(ctx, row, id, size.x),
        _ => {}
    }
    // A leaf that fills a shrinking row is no unit of it, but a row under a label counts its least among the controls'.
    let fills_row = t.fills() == Some(craft_behavior::Axis::X) || begun.common.sizing == Some(super::Sizing::Fill);
    if let (Some(least), Some(crate::layout::Shrink::Count(row))) = (min_took.filter(|_| fills_row), env.shrink)
        && shrink.is_none()
    {
        crate::layout::report_fill_least(ctx, row, size.x, least.x);
    }
    // A body that fills the rest of a shrinking row needs at least its content: the row counts it at that width, so
    // the units narrow to leave it, and it takes what they leave.
    if let Some(crate::layout::Shrink::Count(row)) = env.shrink.filter(|_| fill_body) {
        let content = measured.x + t.overflow();
        crate::layout::report_shrink(ctx, row, id, size.x.min(content), content, Some((content, content)), (false, t.fit_rank()));
    }
    // A component shown at a rect fills it: the rect is its size, and a leaf that measures nothing (a content
    // painter) still shows there.
    let inert = match at {
        ShowAt::Rect(rect) => !steps::guard(size, ShapeKind::Body) || !steps::guard(rect.size(), shape),
        ShowAt::Flow => !steps::guard(size, shape),
    };
    if inert && size != Vec2::ZERO {
        let now = crate::runtime::frame::clock(ctx);
        let log = stage.shown::<SizeLog>(sub(id, "size_log"));
        if now - log.0 >= 1.0 || log.0 > now {
            log.0 = now;
            log::debug!("{} measured the size {size:?} and shows nothing", facts.name);
        }
    }
    Sizes { room, measured, size, inert, dropped }
}

/// Steps 6 to 17 under the run's one `CxCore`, which this frame owns: a sizing child that recurses inside step 5
/// finds no core of its level. Returns whether a pass ran past the sizing check.
// Holds the locals of steps 6 to 17, which opt-level 2 would merge into `run_steps`, on the stack of every measuring run
// below.
#[inline(never)]
fn run_passes(
    t: &mut dyn Erased,
    ui: &mut Ui,
    at: ShowAt,
    mut tip: Option<TipFn<'_>>,
    (begun, resolved, sizes): (&Begun, &Resolved<'_>, &Sizes),
    stage: &mut MemoryStage,
) -> bool {
    let env = begun.env();
    let id = resolved.id;
    let mut core = blank_core(env.view(), resolved, stage, begun.ctx.clone(), env.ink, steps::inert_response(ui, id), begun.step1.command.clone());
    core.pointer_group = Some(ui.scope_id());
    core.faded = begun.facts.faded && !resolved.enabled;
    // 6 to 16, in the host's `Ui`, once per call of the host.
    let mut host = t.host(HostCx { ui: &*ui, env, id, label: &begun.step1.label, size: sizes.size });
    let mut real = false;
    t.with_cx(&mut core, &*begun.style, &mut |e| {
        real = passes(e, &mut *host, ui, (at, tip.take()), (begun, resolved, sizes));
        real
    });
    real
}

/// Steps 6 to 16 in each call of the host, then the dismissal and step 17.
fn passes<'a>(
    e: &mut dyn ErasedPass<'a>,
    host: &mut dyn Host,
    ui: &mut Ui,
    (at, tip): (ShowAt, Option<TipFn<'_>>),
    (begun, resolved, sizes): (&Begun, &Resolved<'_>, &Sizes),
) -> bool {
    let mut run = Pass {
        p: e,
        facts: &begun.facts,
        env: begun.env(),
        place: &begun.place,
        style: &*begun.style,
        resolved,
        trace: begun.trace(),
        tip,
        at,
        room: sizes.room,
        measured: sizes.measured,
        size: sizes.size,
        parent: ui.scope_id(),
        inert: sizes.inert || sizes.dropped,
        dropped: sizes.dropped,
        sizing: begun.sizing,
        real: false,
        activated: false,
        injected: None,
    };
    host.run(resolved.id, ui, begun.env(), &mut |host_ui, host_at, host_env| run_in_host(&mut run, host_ui, host_at, host_env));
    finish_passes(&mut run, &*host, begun)
}

/// The dismissal and step 17 after the host's passes. Returns whether a pass ran past the sizing check.
fn finish_passes(run: &mut Pass<'_, '_, '_>, host: &dyn Host, begun: &Begun) -> bool {
    let (real, activated, injected) = (run.real, run.activated, run.injected);
    let e = &mut *run.p;
    // A layer's dismissal is decided after its body so a field inside commits in the press pass; its open state is
    // written here, after the report (the one write point outside step 15, besides `request_close`).
    if real && let Some(why) = host.dismissed() {
        e.dismissed(why);
    }
    // 17 finish, once: the stage is written back after `finish` returns, so writes made in `finish` are kept.
    if real {
        run.trace.mark(Step::Finish);
        let changed = e.changed();
        steps::before_finish(e.core(), activated, injected, changed);
        // A control painting text through its faces (a select, a segmented control) reports that baseline too.
        let core = e.core_ref();
        let baseline = core.paint.borrow().baseline;
        if baseline.is_some() || begun.shape == ShapeKind::Leaf {
            scope::record_line(&begun.ctx, core.rect, baseline);
        }
    }
    real
}

/// `env` with a component's token scope applied, the one place a `ThemeScope` takes effect.
fn scoped_env(ctx: &Context, env: Env, theme: Option<ThemeScope>) -> Env {
    match theme {
        Some(theme) => env.derive(|scoped| theme.apply(ctx, scoped)),
        None => env,
    }
}

/// What a measure outside the component's own show is for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeasureFor {
    /// A component its caller shows, or sizes a rect by: a leaf of the transcript of the body it is measured in.
    Component,
    /// A composite's face (`MeasureCx::face`), measured inside the composite's own run (measure, parts or header
    /// parts). Only the composite's measure call sizes the composite, and its answer and inputs are the transcript's
    /// record, so the face keeps no leaf of its own.
    Face,
}

/// Measures the component in `t` as `show` will show it, without allocating: steps 1 to 5 under the id `show` would
/// use next (or `id`). It reads memory through a peek, registers no id and applies nothing. `under` is the
/// composite's resolved `(enabled, size)` when it measures a child it shows with `PaintUi::child`.
pub(super) fn measure_shape(
    t: &mut dyn Erased,
    ui: &Ui,
    id: Option<Id>,
    proposal: Proposal,
    under: Option<&super::cx::Under>,
    purpose: MeasureFor,
) -> Vec2 {
    let facts = t.facts();
    // The one walk of a measure: what follows reads the place from it.
    let place = Env::of(ui);
    let env = match under {
        Some(under) => super::cx::child_env(&place, under),
        None => place.clone(),
    };
    let env = scoped_env(ui.ctx(), env, t.theme_scope());
    let id = id.unwrap_or_else(|| steps::id(ui, &t.common().id, false, false, facts.style_key, facts.name));
    let common = t.common().clone();
    let step1 = steps::env(ui.ctx(), &env, &common, t.variants(common.variant, &env));
    let mixed = t.peek_value(ui.ctx(), id, step1.command.as_deref());
    let scx = steps::style_cx(ui.ctx(), &env, &step1);
    let style = t.style(ui.ctx(), &scx);
    let resolved = Resolved { pad: t.pad(&env.tokens), ..steps::resolved(id, &step1, &common) };
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
        let measuring = Measuring { ui, at, width, place: &place, env: &env, scx: &scx, resolved: &resolved, command: step1.command.clone(), mixed };
        t.measured_extent(&measuring, &mut stage)
    };
    let ask = |t: &mut dyn Erased, proposal: Proposal| measured(t, &MeasureIn { ui, env: &env, proposal, room: None }, &*style, &resolved, extent);
    let (took, inputs) = ask(&mut *t, proposal);
    if leaf && purpose == MeasureFor::Component && (!ui.is_sizing_pass() || crate::runtime::measure::active(ui.ctx())) {
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
    /// The host is faded (`DisabledLook::Faded`): its faces paint their enabled look inside its fade.
    pub faded: bool,
    /// The host's step 16 fills the part's sub-node from the face's record: a reported face of a host with a node.
    pub reported: bool,
}
