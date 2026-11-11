//! The sensing core (1.9, 8.2, 8.3): the arena's registrations, the press lifecycle over last pass's shapes, and
//! the part results and member responses built from them.
//!
//! A component's root senses through one egui widget; its parts and its members have none. The root resolves
//! hover and presses itself against the shapes registered last pass, so the winner never depends on show order.
//! Every root runs the lifecycle, arena root or not: a leaf is an arena of its own id with no members.

use crate::runtime::egui::areas::{self, Layer, LayerMap};
use crate::runtime::egui::pass;
use crate::runtime::egui::response::{self, MemberView};
use crate::runtime::env::ArenaId;
use crate::runtime::frame::PerViewport;
use crate::runtime::memory::sub;
use crate::runtime::pointer;
use crate::runtime::pointer::PassPointer;
use craft_behavior::{
    Cursor, HitLevel, HitPriority, HitShape, PartKey, PartRef, PassInput, PointerSample, PressMachine, PressPhase, PressState, ResolveFor, Sense,
    Shape, resolve_mapped,
};
use craft_keymap::Mods;
use craft_theme::Flags;
use egui::{Context, Id, Pos2, Rect, Response, Ui, ViewportId};
use std::collections::HashMap;
use std::sync::Arc;

/// A part's results this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PartResult {
    /// The part's bounds.
    pub rect: Rect,
    /// Its press state.
    pub press: PressState,
    /// The pointer is over it and nothing above takes the hover.
    pub hovered: bool,
    /// The pointer's position while `hovered`, in the owner's layer space.
    pub hover_pos: Option<Pos2>,
    /// The value flags the component set on the part.
    pub flags: Flags,
    /// The component declared the part inactive: it paints with the Disabled state and `PaintUi::draw_part` draws data
    /// colours with the disabled fill, and it keeps sensing.
    pub inactive: bool,
}

impl PartResult {
    /// What a read returns for a part with no press history: face mode, or a part first registered this pass.
    pub const IDLE: PartResult =
        PartResult { rect: Rect::NOTHING, press: PressState::IDLE, hovered: false, hover_pos: None, flags: Flags::empty(), inactive: false };
}

/// A text part's result this pass.
#[derive(Clone)]
#[non_exhaustive]
pub struct TextOutcome<V> {
    /// The part's rect.
    pub rect: Rect,
    /// The draft being typed, if any.
    pub draft: Option<String>,
    /// The value the draft parsed to.
    pub value: Option<V>,
    /// The laid-out text, so an editor draws gutters and line numbers beside it.
    pub layout: Option<super::TextLayout>,
    /// Whether the caret stroke shows this pass (the caret blink).
    pub caret_visible: bool,
    /// The draft is not a value of the codec.
    pub invalid: bool,
    /// The tag of the unit the text named in the pass it committed (`Codec::parse_tagged`).
    pub committed_tag: Option<u32>,
    /// An IME composition is under way.
    pub composing: bool,
    /// The completion candidates the list shows; empty while it is closed.
    pub completions: Vec<super::Completion>,
}

impl<V> TextOutcome<V> {
    /// An outcome over `rect` with no draft or value.
    pub const fn new(rect: Rect) -> Self {
        TextOutcome {
            rect,
            draft: None,
            value: None,
            layout: None,
            caret_visible: true,
            invalid: false,
            committed_tag: None,
            composing: false,
            completions: Vec::new(),
        }
    }
}

/// A part as the sensing core registers it.
// F2-MERGE: F3 (`PartBuilder` adds tooltip and `pointer_only`)
#[derive(Clone, Debug)]
pub(crate) struct PartDecl {
    pub key: PartKey,
    pub index: Option<u64>,
    pub shape: HitShape,
    pub sense: Sense,
    pub priority: HitPriority,
    pub cursor: Option<Cursor>,
    /// Drawn by `paint_over`: stacks above the body members.
    pub over: bool,
    pub flags: Flags,
    /// The part paints with the Disabled state and its data colours take the theme's disabled fill. It keeps sensing.
    pub inactive: bool,
}

impl PartDecl {
    /// A part of `key` over `shape` sensing `sense`.
    pub(crate) fn new(key: PartKey, shape: impl Into<HitShape>, sense: Sense) -> Self {
        PartDecl {
            key,
            index: None,
            shape: shape.into(),
            sense,
            priority: HitPriority::BODY,
            cursor: None,
            over: false,
            flags: Flags::default(),
            inactive: false,
        }
    }

    fn part_ref(&self) -> PartRef {
        PartRef { key: self.key, index: self.index }
    }
}

/// One member's registration for this pass.
#[derive(Clone, Debug)]
pub(crate) struct MemberDecl {
    pub id: Id,
    pub rect: Rect,
    pub sense: Sense,
    pub parts: Vec<PartDecl>,
    /// The shapes lie in the body layer's space; `join` sets it.
    pub local: bool,
    /// The stack position of the member's `over` parts: just above the member until its body has run, then above
    /// every member its body showed (`place_over_widgets`).
    pub over_at: u32,
}

impl MemberDecl {
    /// A member `id` over `rect` sensing `sense`, with `parts`.
    pub(crate) fn new(id: Id, rect: Rect, sense: Sense, parts: Vec<PartDecl>) -> Self {
        MemberDecl { id, rect, sense, parts, local: false, over_at: 0 }
    }
}

/// What a root registers at step 8: its rect and its own sense.
#[derive(Clone, Copy, Debug)]
struct RootInfo {
    id: Id,
    rect: Rect,
    own_sense: Sense,
}

/// A shape's owner and, for a part, its key.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub owner: Id,
    pub part: Option<PartRef>,
    pub sense: Sense,
    pub local: bool,
}

impl Hit {
    fn same(&self, other: &Hit) -> bool {
        self.owner == other.owner && self.part == other.part
    }
}

/// What the arena resolved this pass; the root computes it once, members and the root read it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Outcome {
    pass: u64,
    pub hover: Option<Hit>,
    /// The press winner while a press is open, or the one released or cancelled this pass.
    pub target: Option<Hit>,
    pub phase: Option<PressPhase>,
    pub dragging: bool,
    /// The press winner released as a click this pass.
    pub activated: Option<Hit>,
    xf: Option<LayerMap>,
}

#[derive(Default)]
struct ArenaState {
    /// The pass the registration lists below belong to.
    pass: u64,
    members: Vec<MemberDecl>,
    previous: Vec<MemberDecl>,
    own: Vec<PartDecl>,
    previous_own: Vec<PartDecl>,
    /// The stack position of the root's own `over` parts: above the members its body showed and below the ones
    /// `paint_over` shows (`place_over_widgets`). Above everything until the owner has placed them.
    own_over_at: u32,
    previous_own_over_at: u32,
    /// The senses and bounds of the members that joined this pass and last pass.
    union_sense: Sense,
    previous_union_sense: Sense,
    layer: Option<Layer>,
    /// The layer and the egui sense the root's widgets had at its last step 8, for `sense_early`.
    last_layer: Option<Layer>,
    last_sense: Sense,
    /// The pass whose outcome `sense_early` stepped before the root's step 8.
    early: u64,
    root: Option<RootInfo>,
    /// The egui widgets of the root: the root's, the transform forwarder and the edge widgets.
    widgets: Vec<Id>,
    machine: PressMachine<Hit>,
    outcome: Option<Outcome>,
    /// The rects of the roots shown in the arena's body this pass that are not its members and take a press. Their
    /// egui widgets lie over the members shown before them; a member shown after one of them over its rect gets an
    /// egui widget of its own, so egui's press target there is the member's (8.3).
    nested: Vec<Rect>,
}

#[derive(Default)]
struct Arenas {
    by_root: HashMap<Id, ArenaState>,
    swept: u64,
    /// The end sweep is registered; the runtime layer below cannot register it at install.
    sweep_registered: bool,
}

fn with_arena<R>(ctx: &Context, arena: ArenaId, pass_nr: u64, f: impl FnOnce(&mut ArenaState) -> R) -> R {
    with_arena_in(ctx, pass::current_viewport(ctx), arena, pass_nr, f)
}

fn with_arena_in<R>(ctx: &Context, viewport: ViewportId, arena: ArenaId, pass_nr: u64, f: impl FnOnce(&mut ArenaState) -> R) -> R {
    PerViewport::<Arenas>::with(ctx, viewport, |arenas| {
        if !arenas.sweep_registered {
            arenas.sweep_registered = true;
            crate::runtime::frame::register_end_sweep(ctx, std::any::TypeId::of::<Arenas>(), sweep_undrawn);
            pointer::register_press_probe(ctx, any_press_open);
        }
        if arenas.swept != pass_nr {
            arenas.swept = pass_nr;
            arenas.by_root.retain(|_, state| state.pass + 2 >= pass_nr);
        }
        f(arenas.by_root.entry(arena.0).or_default())
    })
}

fn rotate(state: &mut ArenaState, pass_nr: u64) {
    if state.pass != pass_nr {
        state.previous = std::mem::take(&mut state.members);
        state.previous_own = std::mem::take(&mut state.own);
        state.previous_own_over_at = state.own_over_at;
        state.own_over_at = u32::MAX;
        state.previous_union_sense = std::mem::take(&mut state.union_sense);
        state.nested.clear();
        state.pass = pass_nr;
    }
}

fn own_part_shape(part: &PartDecl, root: Id, position: u32, target: f32) -> Shape<Id> {
    Shape::new(part.shape.clone(), part.sense, root, Some(part.part_ref()), part.priority, position).min_across(target)
}

/// The root's own parts but the `over` ones, at position 0.
fn own_shapes(own: &[PartDecl], root: Id, target: f32) -> Vec<Shape<Id>> {
    own.iter().filter(|part| !part.over).map(|part| own_part_shape(part, root, 0, target)).collect()
}

/// The root's own `over` parts, at the position the owner left them.
fn own_over_shapes(own: &[PartDecl], root: Id, position: u32, target: f32) -> Vec<Shape<Id>> {
    own.iter().filter(|part| part.over).map(|part| own_part_shape(part, root, position, target)).collect()
}

/// The stack position of the member at `index` in show order. Members take the even positions from 2, so the
/// `over` parts of a member fit between the last member its body showed and the next member (8.3).
fn member_position(index: usize) -> u32 {
    2 * (index as u32 + 1)
}

/// The stack position of the `over` parts of a member whose body has run once `count` members have joined.
fn over_position(count: usize) -> u32 {
    2 * count as u32 + 1
}

fn member_part_shape(member: &MemberDecl, part: &PartDecl, position: u32, target: f32) -> Shape<Id> {
    Shape::new(part.shape.clone(), part.sense, member.id, Some(part.part_ref()), part.priority, position).local(member.local).min_across(target)
}

/// The member's root shape and its parts but the `over` ones, at its stack position.
fn member_shapes(member: &MemberDecl, position: u32, target: f32) -> Vec<Shape<Id>> {
    let root = (!member.sense.is_empty())
        .then(|| Shape::new(member.rect.into(), member.sense, member.id, None, HitPriority::BODY, position).local(member.local));
    let parts = member.parts.iter().filter(|part| !part.over).map(|part| member_part_shape(member, part, position, target));
    root.into_iter().chain(parts).collect()
}

/// The member's `over` parts, at the position its body left them.
fn member_over_shapes(member: &MemberDecl, target: f32) -> impl Iterator<Item = Shape<Id>> + '_ {
    member.parts.iter().filter(|part| part.over).map(move |part| member_part_shape(member, part, member.over_at, target))
}

/// The shapes of last pass in resolution order: own parts, then members in show order, each member's `over` parts
/// above the members its body showed, and the own `over` parts at their position among them.
fn candidates(state: &ArenaState, target: f32) -> Vec<Shape<Id>> {
    let root = state.root.map_or(Id::NULL, |root| root.id);
    let mut shapes = own_shapes(&state.previous_own, root, target);
    for (at, member) in state.previous.iter().enumerate() {
        shapes.extend(member_shapes(member, member_position(at), target));
        shapes.extend(member_over_shapes(member, target));
    }
    shapes.extend(own_over_shapes(&state.previous_own, root, state.previous_own_over_at, target));
    shapes
}

/// Whether `widget` is the egui widget of a member that joined last pass or of one of its press-taking `over` parts:
/// such widgets stand for the arena root, as its own do.
fn member_widget_of(members: &[MemberDecl], widget: Id) -> bool {
    members.iter().any(|member| member_widget(member.id) == widget || over_parts(&member.parts).any(|part| over_widget(member.id, part) == widget))
}

/// The egui widget of a member shown over a root that is not a member of its arena.
fn member_widget(member: Id) -> Id {
    sub(member, "member")
}

/// Notes a root shown in the body of `arena` that is not one of its members and takes a press, over `rect`: the
/// members shown after it over that rect stack above it.
pub(crate) fn note_nested(ctx: &Context, arena: ArenaId, rect: Rect) {
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        state.nested.push(rect);
    });
}

/// Records the transformed body's egui layer.
pub(crate) fn set_layer(ctx: &Context, arena: ArenaId, layer: Layer) {
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| state.layer = Some(layer));
}

/// What step 8 reads of the running pass, fetched once per root: the viewport, the pass number and the pointer
/// record the begin hook built.
struct PassRead {
    viewport: ViewportId,
    pass_nr: u64,
    pointer: Arc<PassPointer>,
}

impl PassRead {
    fn of(ctx: &Context) -> Self {
        let viewport = pass::current_viewport(ctx);
        PassRead { viewport, pass_nr: pass::pass_nr(ctx, viewport), pointer: pointer::pass_pointer(ctx) }
    }

    fn target(&self) -> f32 {
        self.pointer.frame.as_ref().map_or(0.0, |frame| frame.target)
    }

    /// The modifiers, sample and hover position the part and press results read.
    fn inputs(&self) -> (Mods, Option<PointerSample>, Option<Pos2>) {
        let frame = self.pointer.frame.as_ref();
        (frame.map_or(Mods::NONE, |frame| frame.mods), frame.and_then(|frame| frame.sample), self.pointer.hover)
    }
}

/// Steps the press machine of the root `id` before its measuring run, from what last pass registered: the root rect,
/// widgets, layer and sense. A body reads the parts' presses of this pass in both of its runs through
/// [`part_press`]; step 8 finds the memoised outcome. Does nothing on a root's first pass and when the root was not
/// drawn last pass. `enabled` is what is known before the root's step 5; step 8 ends a press of a root that turns out
/// to be inert.
pub(crate) fn sense_early(ctx: &Context, id: Id, enabled: bool) {
    let arena = ArenaId::new(id);
    let read = PassRead::of(ctx);
    let last = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        if state.pass != read.pass_nr && state.pass + 1 != read.pass_nr {
            return None;
        }
        rotate(state, read.pass_nr);
        let root = state.root?;
        let layer = state.last_layer?;
        (root.rect.is_positive() && root.rect.is_finite() && !state.previous_own.is_empty())
            .then_some((layer, state.last_sense | state.previous_union_sense))
    });
    let Some((layer, sense)) = last else { return };
    outcome_in(ctx, &read, arena, layer, enabled, sense);
    with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| state.early = read.pass_nr);
}

/// The press, hover and hover position of one part of the root `owner` this pass, from the outcome of the pass
/// (stepped early when the root has parts), idle when the root has not stepped yet.
pub(crate) struct PartPress {
    pub press: PressState,
    pub hovered: bool,
    pub hover_pos: Option<Pos2>,
}

pub(crate) fn part_press(ctx: &Context, owner: Id, part: PartRef) -> PartPress {
    let read = PassRead::of(ctx);
    let arena = ArenaId::new(owner);
    let (outcome, known) = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        (state.outcome.filter(|outcome| outcome.pass == read.pass_nr), known_in(state, owner))
    });
    let (mods, sample, at) = read.inputs();
    match outcome {
        Some(outcome) => part_press_in(&outcome, &known, (mods, sample, at), owner, part),
        None => PartPress { press: PressState::new(PressPhase::Idle).mods(mods).sample(sample), hovered: false, hover_pos: None },
    }
}

/// What a root registers and reads at step 8.
pub(crate) struct RootSpec<'a> {
    pub id: Id,
    pub rect: Rect,
    pub own_sense: Sense,
    pub own: &'a [PartDecl],
    pub enabled: bool,
}

/// What a root reads back: the response, the lifecycle outcome and the owner's own results.
pub(crate) struct RootResult {
    pub response: Response,
    pub parts: PartResults,
    pub press: PressState,
    /// The root's own shapes won a click.
    pub activation: bool,
    /// The member a click activated.
    pub activated_member: Option<Id>,
    /// The sense the root's egui widget carries.
    pub sense: Sense,
}

pub(crate) type PartResults = HashMap<(PartKey, Option<u64>), PartResult>;

/// Step 8 for a root: its egui widgets, its registration, the lifecycle and what it reads back.
pub(crate) fn sense_root(ui: &Ui, spec: RootSpec<'_>) -> RootResult {
    let ctx = ui.ctx();
    let arena = ArenaId::new(spec.id);
    let read = PassRead::of(ctx);
    let (clip, layer) = (ui.clip_rect(), ui.layer_id());
    let last_union = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        rotate(state, read.pass_nr);
        state.previous_union_sense
    });
    // A disabled root only hovers, whatever its parts and members sense; it keeps the focus stop its own sense or a part's grants.
    let sense = if spec.enabled {
        spec.own.iter().fold(spec.own_sense | last_union, |sense, part| sense | part.sense)
    } else {
        Sense::HOVER | spec.own.iter().fold(spec.own_sense, |sense, part| sense | part.sense).intersection(Sense::FOCUS)
    };
    // A root outside its clip gets an empty response rect at its corner, never an inverted one.
    let shown = spec.rect.intersect(clip);
    let response = response::widget(ui, spec.id, Rect::from_min_max(shown.min, shown.max.max(shown.min)), sense);
    // Each edge or handle part that reaches outside the root rect gets its own widget, resolved as the root.
    let mut widgets = vec![spec.id, sub(spec.id, "transform")];
    for part in spec.own.iter().filter(|part| !part.over && part.priority.level >= HitLevel::Edge) {
        let bounds = part.shape.bounds();
        if !spec.rect.contains_rect(bounds) {
            let id = sub(spec.id, ("edge", part.key, part.index));
            response::widget(ui, id, bounds.intersect(clip), part.sense);
            crate::runtime::egui::access::hide(ctx, id);
            widgets.push(id);
        }
    }
    widgets.extend(over_parts(spec.own).map(|part| over_widget(spec.id, part)));
    let mut shapes = own_shapes(spec.own, spec.id, read.target());
    if !spec.own_sense.is_empty() {
        shapes.push(Shape::new(spec.rect.into(), spec.own_sense, spec.id, None, HitPriority::new(HitLevel::Background, 0), 0));
    }
    // The parts last pass registered, which this pass's results read.
    let known = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        state.own = spec.own.to_vec();
        state.root = Some(RootInfo { id: spec.id, rect: spec.rect, own_sense: spec.own_sense });
        state.widgets = widgets.clone();
        state.last_layer = Some(layer);
        state.last_sense = sense;
        // A root that stepped early while it was not yet known to be inert: an open press ends as the disabled root's
        // would, by stepping again with the cancel.
        if !spec.enabled && state.early == read.pass_nr && state.outcome.is_some_and(|outcome| outcome.pass == read.pass_nr) {
            state.outcome = None;
        }
        known_in(state, spec.id)
    });
    pointer::record_shapes_in(ctx, read.viewport, layer, spec.id, shapes);
    pointer::record_widgets_in(ctx, read.viewport, layer, spec.id, &widgets, sense.contains(Sense::CLICK));
    let mut response = response;
    let outcome = outcome_in(ctx, &read, arena, layer, spec.enabled, sense);
    response::keep_press_flags(&mut response, outcome.target.is_some_and(|target| target.owner == spec.id));
    let (activation, activated_member) = match outcome.activated {
        Some(hit) if hit.owner == spec.id => (true, None),
        // A press on a member's part is the part's activation; it does not pick the member.
        Some(hit) if hit.part.is_none() => (false, Some(hit.owner)),
        Some(_) => (false, None),
        None => (false, None),
    };
    let inputs = read.inputs();
    RootResult {
        response,
        parts: part_results_in(&outcome, &known, inputs, spec.id, spec.own),
        press: shape_press_in(&outcome, inputs.0, inputs.1, spec.id),
        activation,
        activated_member,
        sense,
    }
}

/// The `over` parts that take a press: above the body's members and nested roots (8.3).
fn over_parts(own: &[PartDecl]) -> impl Iterator<Item = &PartDecl> {
    own.iter().filter(|part| part.over && part.sense.takes_press())
}

fn over_widget(root: Id, part: &PartDecl) -> Id {
    sub(root, ("over", part.key, part.index))
}

/// Registers the egui widgets of the component's `over` parts after its body ran and before its `paint_over`. egui
/// picks the later widget, so a press on such a part goes to the component's arena and not to a root nested in the
/// body, and a child `paint_over` shows afterwards stacks above it: one rule for the stacking order (8.3). A root's
/// ids are part of its widgets from step 8. The `over` parts of a root or a member take their stack position here,
/// above every member its body showed; a member's widgets stand for the arena root.
pub(crate) fn place_over_widgets(ui: &Ui, id: Id, member: Option<ArenaId>, decls: &[PartDecl]) {
    let (ctx, clip) = (ui.ctx(), ui.clip_rect());
    let widgets: Vec<Id> = over_parts(decls)
        .map(|part| {
            let widget = over_widget(id, part);
            response::widget(ui, widget, part.shape.bounds().intersect(clip), part.sense);
            crate::runtime::egui::access::hide(ctx, widget);
            widget
        })
        .collect();
    if !decls.iter().any(|part| part.over) {
        return;
    }
    let arena = member.unwrap_or_else(|| ArenaId::new(id));
    let layer = ui.layer_id();
    let read = PassRead::of(ctx);
    let shapes = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        let at = over_position(state.members.len());
        if member.is_none() {
            state.own_over_at = at;
            return Some(own_over_shapes(&state.own, id, at, read.target()));
        }
        let joined = state.members.iter_mut().rev().find(|joined| joined.id == id)?;
        joined.over_at = at;
        Some(member_over_shapes(joined, read.target()).collect::<Vec<_>>())
    });
    let Some(shapes) = shapes else { return };
    pointer::record_shapes_in(ctx, read.viewport, layer, arena.0, shapes);
    // A root's widgets are in its widget record from step 8.
    if member.is_some() && !widgets.is_empty() {
        pointer::record_widgets_in(ctx, read.viewport, layer, arena.0, &widgets, decls.iter().any(|part| part.sense.contains(Sense::CLICK)));
    }
}

/// Step 8 for a root with no sense, no parts and no members: nothing can hit it, so it has no arena entry, shape
/// record or press lifecycle, only its egui widget and its entry in the widget record.
pub(crate) fn sense_none(ui: &Ui, id: Id, rect: Rect) -> RootResult {
    let ctx = ui.ctx();
    let response = response::widget(ui, id, rect.intersect(ui.clip_rect()), Sense::empty());
    pointer::record_widgets(ctx, ui.layer_id(), id, &[id, sub(id, "transform")], false);
    RootResult { response, parts: PartResults::new(), press: PressState::IDLE, activation: false, activated_member: None, sense: Sense::empty() }
}

/// A member's step 8: it joins the arena and builds its `Response` and results from the arena's outcome.
pub(crate) struct MemberSpec<'a> {
    pub id: Id,
    pub rect: Rect,
    pub sense: Sense,
    pub parts: &'a [PartDecl],
    pub enabled: bool,
}

pub(crate) struct MemberResult {
    pub response: Response,
    pub parts: PartResults,
    pub press: PressState,
    /// The press released as a click on the member's own shape or one of its parts.
    pub activated: bool,
}

pub(crate) fn sense_member(ui: &Ui, arena: ArenaId, spec: MemberSpec<'_>) -> MemberResult {
    let ctx = ui.ctx();
    debug_assert!(!crate::runtime::measure::active(ctx), "a member joins its arena in a real run only");
    let layer = ui.layer_id();
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    // One read of the pointer and one of the arena serve the join, the outcome and the part results.
    let (pointer_pos, target, mods, sample) = pointer::with_frame(ctx, |frame, hover| {
        (hover, frame.map_or(0.0, |frame| frame.target), frame.map_or(Mods::NONE, |frame| frame.mods), frame.and_then(|frame| frame.sample))
    });
    let mut decl = MemberDecl::new(spec.id, spec.rect, spec.sense, spec.parts.to_vec());
    let presses =
        spec.parts.iter().filter(|part| !part.over).fold(spec.sense, |sense, part| sense | part.sense).intersection(Sense::CLICK | Sense::DRAG);
    let (local, shapes, outcome, known, covers) = with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        let covers = state.nested.iter().any(|nested| nested.intersects(spec.rect));
        decl.local = state.layer == Some(layer);
        let position = member_position(state.members.len());
        decl.over_at = position + 1;
        let shapes = member_shapes(&decl, position, target);
        state.union_sense |= decl.sense;
        for part in &decl.parts {
            state.union_sense |= part.sense;
        }
        let local = decl.local;
        state.members.push(decl);
        let outcome = state.outcome.filter(|outcome| outcome.pass == pass_nr).unwrap_or_default();
        let known = known_in(state, spec.id);
        (local, shapes, outcome, known, covers)
    });
    pointer::record_shapes(ctx, layer, arena.0, shapes);
    // A member shown over an earlier root that is not a member takes egui's press there through a widget that stands
    // for the arena root; egui picks the later widget.
    if covers && spec.enabled && !presses.is_empty() {
        let widget = member_widget(spec.id);
        response::widget(ui, widget, spec.rect.intersect(ui.clip_rect()), presses);
        crate::runtime::egui::access::hide(ctx, widget);
        pointer::record_widgets_in(ctx, pass::current_viewport(ctx), layer, arena.0, &[widget], presses.contains(Sense::CLICK));
    }
    let view = member_view(&outcome, spec.id, local, pointer_pos);
    MemberResult {
        response: response::member(ui, spec.id, spec.rect, spec.enabled, view),
        parts: part_results_in(&outcome, &known, (mods, sample, pointer_pos), spec.id, spec.parts),
        press: shape_press_in(&outcome, mods, sample, spec.id),
        activated: view.clicked,
    }
}

fn member_view(outcome: &Outcome, id: Id, local: bool, pointer_pos: Option<Pos2>) -> MemberView {
    let phase = outcome.target.filter(|target| target.owner == id).and(outcome.phase);
    MemberView {
        hovered: outcome.hover.is_some_and(|hit| hit.owner == id),
        pressed: phase.is_some(),
        drag_started: matches!(phase, Some(PressPhase::DragStart { .. })),
        dragged: phase.is_some_and(|phase| phase.is_drag()),
        drag_stopped: matches!(phase, Some(PressPhase::Release { click: false, .. })),
        clicked: outcome.activated.is_some_and(|hit| hit.owner == id && hit.part.is_none()),
        pointer: pointer_pos.map(|at| if local { outcome.xf.map_or(at, |map| map.to_layer(at)) } else { at }),
    }
}

/// The resolve rule over last pass's shapes at `point`. A press inside the root rect that hits no shape goes to
/// the root's own background when the root senses a press.
fn resolve_in(state: &ArenaState, xf: Option<LayerMap>, point: Pos2, kind: ResolveFor, target: f32) -> Option<Hit> {
    let shapes = candidates(state, target);
    let won = resolve_mapped(&shapes, point, kind, |p| xf.map_or(p, |map| map.to_layer(p)));
    won.map(|winner| Hit { owner: winner.owner, part: winner.part, sense: winner.sense, local: winner.local })
}

/// Runs the press lifecycle for this pass, once: later calls in the pass return the same outcome. A repeated
/// pass has no events, so phases hold and a release, click or activation does not repeat. `union_sense` is
/// what the root's egui widgets sense.
fn outcome_in(ctx: &Context, read: &PassRead, arena: ArenaId, layer: Layer, enabled: bool, union_sense: Sense) -> Outcome {
    let pass_nr = read.pass_nr;
    let done = with_arena_in(ctx, read.viewport, arena, pass_nr, |state| state.outcome.filter(|outcome| outcome.pass == pass_nr));
    if let Some(outcome) = done {
        return outcome;
    }
    let pass_pointer = &*read.pointer;
    let (slop, delta, pos, buttons, target) =
        pass_pointer.frame.as_ref().map_or((0.0, egui::Vec2::ZERO, None, craft_behavior::Buttons::empty(), 0.0), |frame| {
            (frame.slop, frame.delta, frame.pos, frame.buttons, frame.target)
        });
    let cancel = if enabled { pointer::cancel_of_pass(ctx) } else { Some(craft_behavior::PressCancel::Disabled) };
    let body_layer = with_arena(ctx, arena, pass_nr, |state| state.layer);
    let xf = body_layer.and_then(|layer| areas::layer_map(ctx, layer));
    // Hover needs the root's layer on top under the pointer, or egui's layer cover (a popup) would be ignored.
    let top = pass_pointer.hover.and_then(|at| areas::layer_at(ctx, at));
    let hover_layer_ok = top.is_some_and(|top| top == layer || Some(top) == body_layer);
    let mut cancelled = false;
    let outcome = with_arena(ctx, arena, pass_nr, |state| {
        let root = state.root.unwrap_or(RootInfo { id: Id::NULL, rect: Rect::NOTHING, own_sense: Sense::empty() });
        let pos = pos.unwrap_or(Pos2::ZERO);
        let input = PassInput::new(
            &pass_pointer.events,
            slop,
            pos,
            pass_pointer.primary_down || buttons.intersects(craft_behavior::Buttons::MIDDLE | craft_behavior::Buttons::SECONDARY),
        )
        .delta(delta)
        .cancel(cancel);
        let mut machine = std::mem::take(&mut state.machine);
        let step = machine.step(input, |at| {
            // The press-target rule: egui's click target decides, its drag target only for a drag-only root; the
            // root starts a press when that target is its own or there is none.
            let mine = |id: Option<Id>| match id {
                Some(id) => state.widgets.contains(&id) || member_widget_of(&state.previous, id),
                None => crate::runtime::sampler::takes_untargeted(ctx, state.root.map(|root| root.id), at),
            };
            let targets = pass_pointer.press_targets;
            let allowed = if union_sense.contains(Sense::CLICK) {
                mine(targets.click)
            } else if union_sense.contains(Sense::DRAG) {
                mine(targets.drag)
            } else {
                true
            };
            // A root whose own widget is not the click target still starts the press its parts win under it.
            let allowed = allowed
                || state
                    .root
                    .is_some_and(|own| own.rect.contains(at) && pointer::press_winner(ctx, at, targets).is_some_and(|(_, winner)| winner == own.id));
            if !enabled || !allowed {
                return None;
            }
            let hit = resolve_in(state, xf, at, ResolveFor::Press, target).or_else(|| {
                (root.rect.contains(at) && root.own_sense.takes_press()).then_some(Hit {
                    owner: root.id,
                    part: None,
                    sense: root.own_sense,
                    local: false,
                })
            });
            hit.map(|hit| (hit, hit.sense))
        });
        cancelled = matches!(step.phase, Some(PressPhase::Cancel { .. }));
        let open = machine.target();
        let dragging = step.dragging;
        state.machine = machine;
        let under = pass_pointer.hover.filter(|_| hover_layer_ok).and_then(|at| resolve_in(state, xf, at, ResolveFor::Hover, target));
        // While a press is open only its target hovers, and only while the pointer is over it or dragging.
        let hover = match open {
            Some(target) if dragging => Some(target),
            Some(target) => under.filter(|under| under.same(&target)),
            None => under,
        };
        let outcome = Outcome { pass: pass_nr, hover, target: step.target, phase: step.phase, dragging, activated: step.activated, xf };
        state.outcome = Some(outcome);
        outcome
    });
    if cancelled {
        crate::runtime::frame::note_cancelled_press(ctx);
    }
    outcome
}

/// The end hook's sweep: a press whose root, or whose member owner, was not drawn in the pass ends, so a removed
/// owner never keeps the pointer captured. egui's own drag ends at the next begin hook.
fn sweep_undrawn(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let gone = PerViewport::<Arenas>::with(ctx, viewport, |arenas| {
        let mut gone = false;
        for state in arenas.by_root.values_mut() {
            let orphaned = |state: &ArenaState| {
                state.machine.target().is_some_and(|target| {
                    state.pass != pass_nr
                        || (state.root.is_some_and(|root| root.id != target.owner) && !state.members.iter().any(|member| member.id == target.owner))
                })
            };
            if orphaned(state) {
                state.machine = PressMachine::default();
                gone = true;
            }
        }
        gone
    });
    if gone {
        crate::runtime::frame::note_cancelled_press(ctx);
    }
}

fn any_press_open(ctx: &Context) -> bool {
    PerViewport::<Arenas>::with(ctx, pass::current_viewport(ctx), |arenas| arenas.by_root.values().any(|state| state.machine.target().is_some()))
}

/// The arena's outcome of the running pass, idle when the root has not run yet.
pub(crate) fn read_outcome(ctx: &Context, arena: ArenaId) -> Outcome {
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| state.outcome.filter(|outcome| outcome.pass == pass_nr)).unwrap_or_default()
}

/// A phase in the owner's space: a member of a transformed body reads positions in the body layer.
fn in_owner_space(phase: PressPhase, target: &Hit, xf: Option<LayerMap>) -> PressPhase {
    match xf {
        Some(map) if target.local => phase.mapped(|at| map.to_layer(at), |delta| map.delta_to_layer(delta)),
        _ => phase,
    }
}

fn known_in(state: &ArenaState, owner: Id) -> Vec<(PartKey, Option<u64>)> {
    let parts: &[PartDecl] = if state.root.is_some_and(|root| root.id == owner) {
        &state.previous_own
    } else {
        state.previous.iter().find(|member| member.id == owner).map_or(&[], |member| member.parts.as_slice())
    };
    parts.iter().map(|part| (part.key, part.index)).collect()
}

/// The part results of one owner: a part reads the press phase when it is the press winner, `Hover` when it is
/// the topmost shape under the pointer, else idle. A part first registered this pass is not in last pass's
/// registrations and reads idle.
fn part_results_in(
    outcome: &Outcome,
    known: &[(PartKey, Option<u64>)],
    (mods, sample, at): (Mods, Option<PointerSample>, Option<Pos2>),
    owner: Id,
    decls: &[PartDecl],
) -> PartResults {
    decls
        .iter()
        .map(|decl| {
            let read = part_press_in(outcome, known, (mods, sample, at), owner, decl.part_ref());
            let (key, hovered) = ((decl.key, decl.index), read.hovered);
            (
                key,
                PartResult {
                    rect: decl.shape.bounds(),
                    press: read.press,
                    hovered,
                    hover_pos: read.hover_pos,
                    flags: decl.flags,
                    inactive: decl.inactive,
                },
            )
        })
        .collect()
}

/// One part's press: the phase when it is the press winner, `Hover` when it is the topmost shape under the pointer,
/// else idle. A part not in `known` reads idle.
fn part_press_in(
    outcome: &Outcome,
    known: &[(PartKey, Option<u64>)],
    (mods, sample, at): (Mods, Option<PointerSample>, Option<Pos2>),
    owner: Id,
    part: PartRef,
) -> PartPress {
    let is_known = known.contains(&(part.key, part.index));
    let mine = |hit: &Hit| hit.owner == owner && hit.part == Some(part);
    let won = outcome.target.filter(|target| is_known && mine(target));
    let hover = outcome.hover.filter(|hit| is_known && mine(hit));
    let hovered = hover.is_some();
    let hover_pos = hover.zip(at).map(|(hit, at)| if hit.local { outcome.xf.map_or(at, |map| map.to_layer(at)) } else { at });
    let phase = match (won, outcome.phase) {
        (Some(target), Some(phase)) => in_owner_space(phase, &target, outcome.xf),
        _ if hovered => PressPhase::Hover,
        _ => PressPhase::Idle,
    };
    let captured = won.is_some() && outcome.dragging && phase.is_drag();
    let press = PressState::new(phase).mods(mods).sample(sample).captured(captured);
    PartPress { press, hovered, hover_pos }
}

/// The owner's own shape's press state: a member's root shape or the root background. `Idle` unless it won.
fn shape_press_in(outcome: &Outcome, mods: Mods, sample: Option<PointerSample>, owner: Id) -> PressState {
    let won = outcome.target.filter(|target| target.owner == owner && target.part.is_none());
    let phase = match (won, outcome.phase) {
        (Some(target), Some(phase)) => in_owner_space(phase, &target, outcome.xf),
        _ if outcome.hover.is_some_and(|hit| hit.owner == owner && hit.part.is_none()) => PressPhase::Hover,
        _ => PressPhase::Idle,
    };
    let captured = won.is_some() && phase.is_drag();
    PressState::new(phase).mods(mods).sample(sample).captured(captured)
}

/// The cursor a hovered part of `decls` declares, if any: the sensing core's request for the cursor service.
pub(crate) fn hovered_cursor(decls: &[PartDecl], results: &PartResults) -> Option<Cursor> {
    decls.iter().filter(|decl| results.get(&(decl.key, decl.index)).is_some_and(|result| result.hovered)).find_map(|decl| decl.cursor.clone())
}
