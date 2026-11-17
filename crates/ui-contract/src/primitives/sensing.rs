//! The sensing core (1.9, 8.2, 8.3): the arena's registrations, the press lifecycle over last pass's shapes, and
//! the part results and member responses built from them.
//!
//! A component's root senses through one egui widget; its parts and its members have none. What lies under the
//! pointer is resolved once per point for the whole viewport, over the shapes registered last pass (`pointer::Under`),
//! and every root reads its hover and its press from that answer, so the winner never depends on show order.
//! Every root runs the lifecycle, arena root or not: a leaf is an arena of its own id with no members.

use crate::runtime::edit::{RemovalWatch, gesture_key};
use crate::runtime::egui::areas::{self, Layer, LayerMap};
use crate::runtime::egui::pass;
use crate::runtime::egui::response::{self, MemberView, PressView};
use crate::runtime::env::ArenaId;
use crate::runtime::frame::PerViewport;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::pointer;
use crate::runtime::pointer::PassPointer;
use craft_behavior::{
    Cursor, HitLevel, HitPriority, HitShape, PartKey, PartRef, PassInput, PointerSample, PressMachine, PressPhase, PressState, Sense, Shape, Winner,
};
use craft_keymap::Mods;
use craft_theme::Flags;
use egui::{Context, Id, IdMap, Pos2, Rect, Response, Ui, ViewportId};
use smallvec::SmallVec;
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
    /// What the part senses, as declared.
    pub sense: Sense,
    /// The component declared the part inactive: it paints with the Disabled state and `PaintUi::draw_part` draws data
    /// colours with the disabled fill, and it keeps sensing.
    pub inactive: bool,
    /// The component holds a press on the part that the part does not sense itself: it paints pressed.
    pub held: bool,
}

impl PartResult {
    /// What a read returns for a part with no press history: face mode, or a part first registered this pass.
    pub const IDLE: PartResult = PartResult {
        rect: Rect::NOTHING,
        press: PressState::IDLE,
        hovered: false,
        hover_pos: None,
        flags: Flags::empty(),
        sense: Sense::empty(),
        inactive: false,
        held: false,
    };

    /// The part paints pressed: its own press, or one its component holds on it.
    pub fn pressed(&self) -> bool {
        self.press.phase.is_press() || self.held
    }
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
    /// What the parse learned in the pass the draft committed through Enter, a separator or blur; `None` otherwise.
    pub committed: Option<craft_behavior::ParseNote>,
    /// An IME composition is under way.
    pub composing: bool,
    /// The completion candidates the list shows; empty while it is closed.
    pub completions: Vec<super::Completion>,
    /// A primary click on the text ended this pass.
    pub clicked: bool,
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
            committed: None,
            composing: false,
            completions: Vec::new(),
            clicked: false,
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
    /// The component holds a press on the part (`PartBuilder::held`).
    pub held: bool,
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
            held: false,
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
    pub parts: SmallVec<[PartDecl; 4]>,
    /// The shapes lie in the body layer's space; `join` sets it.
    pub local: bool,
    /// The stack position of the member's `over` parts: just above the member until its body has run, then above
    /// every member its body showed (`place_over_widgets`).
    pub over_at: u32,
}

impl MemberDecl {
    /// A member `id` over `rect` sensing `sense`, with `parts`.
    pub(crate) fn new(id: Id, rect: Rect, sense: Sense, parts: &[PartDecl]) -> Self {
        MemberDecl { id, rect, sense, parts: parts.iter().cloned().collect(), local: false, over_at: 0 }
    }
}

/// What a root registers at step 8: its id and rect.
#[derive(Clone, Copy, Debug)]
struct RootInfo {
    id: Id,
    rect: Rect,
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

impl From<Winner<Id>> for Hit {
    fn from(winner: Winner<Id>) -> Self {
        Hit { owner: winner.owner, part: winner.part, sense: winner.sense, local: winner.local }
    }
}

/// The arena's shapes that show hover this pass, from what lies under the pointer (`pointer::Under`): the shape a
/// press at the pointer goes to, and the arena's shape that hovers beside it (`Under::hover_top`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hovered {
    press: Option<Hit>,
    only: Option<Hit>,
}

impl Hovered {
    /// The hovered shape `pick` takes, the press's first.
    pub(crate) fn find(&self, pick: impl Fn(&Hit) -> bool) -> Option<Hit> {
        self.press.into_iter().chain(self.only).find(|hit| pick(hit))
    }

    /// Whether any shape of the arena shows hover.
    pub(crate) fn any(&self) -> bool {
        self.press.is_some() || self.only.is_some()
    }

    fn filter(self, keep: impl Fn(&Hit) -> bool) -> Self {
        Hovered { press: self.press.filter(&keep), only: self.only.filter(&keep) }
    }
}

/// What the arena resolved this pass; the root computes it once, members and the root read it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Outcome {
    pass: u64,
    pub hover: Hovered,
    /// A shape of the arena lies under the pointer, on top or not.
    pub contains: bool,
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
    /// The host asked to end the open press (`cancel_press_of`): the arena's next lifecycle run ends it.
    host_cancel: bool,
}

#[derive(Default)]
pub(crate) struct Arenas {
    by_root: IdMap<ArenaState>,
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
        // Last pass's lists become `previous`; the older lists' buffers take this pass's registrations.
        std::mem::swap(&mut state.previous, &mut state.members);
        state.members.clear();
        std::mem::swap(&mut state.previous_own, &mut state.own);
        state.own.clear();
        state.own_over_at = u32::MAX;
        state.previous_union_sense = std::mem::take(&mut state.union_sense);
        state.pass = pass_nr;
    }
}

fn own_part_shape(part: &PartDecl, root: Id, position: u32, target: f32) -> Shape<Id> {
    Shape::new(part.shape.clone(), part.sense, root, Some(part.part_ref()), part.priority, position).min_across(target)
}

/// The root's own parts but the `over` ones, at position 0.
fn own_shapes(own: &[PartDecl], root: Id, target: f32) -> impl Iterator<Item = Shape<Id>> + '_ {
    own.iter().filter(|part| !part.over).map(move |part| own_part_shape(part, root, 0, target))
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
fn member_shapes(member: &MemberDecl, position: u32, target: f32) -> SmallVec<[Shape<Id>; 4]> {
    let root = (!member.sense.is_empty())
        .then(|| Shape::new(member.rect.into(), member.sense, member.id, None, HitPriority::BODY, position).local(member.local));
    let parts = member.parts.iter().filter(|part| !part.over).map(|part| member_part_shape(member, part, position, target));
    root.into_iter().chain(parts).collect()
}

/// The member's `over` parts, at the position its body left them.
fn member_over_shapes(member: &MemberDecl, target: f32) -> impl Iterator<Item = Shape<Id>> + '_ {
    member.parts.iter().filter(|part| part.over).map(move |part| member_part_shape(member, part, member.over_at, target))
}

/// The root's own background over `rect`, below every part and member, sensing `sense`.
fn background_shape(root: Id, rect: Rect, sense: Sense) -> Shape<Id> {
    Shape::new(rect.into(), sense, root, None, HitPriority::new(HitLevel::Background, 0), 0)
}

/// The members of `arena` in `viewport` as of its latest run: while the root's run of this pass is under way, the
/// members of last pass and those registered so far; empty for an arena that has not run. A read creates no arena.
pub(crate) fn members(ctx: &Context, viewport: ViewportId, arena: ArenaId) -> SmallVec<[Id; 8]> {
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Arenas>::with(ctx, viewport, |arenas| {
        let Some(state) = arenas.by_root.get(&arena.0) else { return SmallVec::new() };
        let mut ids: SmallVec<[Id; 8]> = state.members.iter().map(|member| member.id).collect();
        if state.pass == pass_nr {
            ids.extend(state.previous.iter().map(|member| member.id).filter(|id| !state.members.iter().any(|member| member.id == *id)));
        }
        ids
    })
}

/// Whether a member of the arena root `arena` (a canvas control in a canvas, say) has a running gesture, in
/// `viewport`: begun and not yet committed, cancelled or removed. It reads each member's own gesture record, so a
/// reader before or after the arena's pass reads the same fact.
pub fn member_gesture_running(ctx: &Context, viewport: ViewportId, arena: Id) -> bool {
    members(ctx, viewport, ArenaId::new(arena))
        .into_iter()
        .any(|member| MemoryStage::peek_with::<RemovalWatch, _>(ctx, gesture_key(member), RemovalWatch::armed))
}

/// Ends the press `owner` holds open the way Escape ends a captured press: the next lifecycle run of the arena it is
/// open in reports `PressPhase::Cancel` with `PressCancel::Host`, and step 9 cancels the gesture the press carries
/// (`CancelReason::HostChanged`). For a host whose own state no longer matches what the press acts on; it may ask
/// inside or outside a pass. Nothing happens when `owner` holds no open press.
pub fn cancel_press_of(ctx: &Context, owner: Id) {
    PerViewport::<Arenas>::each(ctx, |arenas| {
        for state in arenas.by_root.values_mut() {
            if state.machine.target().is_some_and(|target| target.owner == owner) {
                state.host_cancel = true;
            }
        }
    });
}

/// Records the transformed body's egui layer.
pub(crate) fn set_layer(ctx: &Context, arena: ArenaId, layer: Layer) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| set_layer(ctx, arena, layer));
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| state.layer = Some(layer));
}

// The arena registrations below are hit records like the shape record (`pointer::record_shapes`): a settled region
// that replays its last run (`runtime::replay`) makes them again, so the arenas it shows resolve the next pass's
// press over the members, parts and roots it shows. Without them a replayed pass leaves its members out of the arena
// around it, and an arena inside it is dropped once it has not run for two passes, while the shape record still
// names their shapes as the press winner.

/// What a root registers in its arena at step 8, kept for a replay.
#[derive(Clone)]
struct RootRegistration {
    info: RootInfo,
    own: Vec<PartDecl>,
    widgets: Vec<Id>,
    layer: Layer,
    sense: Sense,
}

/// Notes a root's step-8 registration in the open records.
fn journal_root(ctx: &Context, arena: ArenaId, info: RootInfo, own: &[PartDecl], widgets: &[Id], (layer, sense): (Layer, Sense)) {
    crate::runtime::replay::journal(ctx, || {
        let registration = RootRegistration { info, own: own.to_vec(), widgets: widgets.to_vec(), layer, sense };
        move |ctx: &Context| register_root_again(ctx, arena, registration.clone())
    });
}

/// A replayed root's step-8 registration: its own parts, rect, widgets, layer and sense, with no press run.
fn register_root_again(ctx: &Context, arena: ArenaId, registration: RootRegistration) {
    journal_root(ctx, arena, registration.info, &registration.own, &registration.widgets, (registration.layer, registration.sense));
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        state.own = registration.own;
        state.root = Some(registration.info);
        state.widgets = registration.widgets;
        state.last_layer = Some(registration.layer);
        state.last_sense = registration.sense;
    });
}

/// Notes in the open records the members that joined `arena`, as they joined.
fn journal_join(ctx: &Context, arena: ArenaId, joined: &[MemberDecl]) {
    crate::runtime::replay::journal(ctx, || {
        let joined = joined.to_vec();
        move |ctx: &Context| join_again(ctx, arena, joined.clone())
    });
}

/// Replayed members join their arena again.
fn join_again(ctx: &Context, arena: ArenaId, joined: Vec<MemberDecl>) {
    journal_join(ctx, arena, &joined);
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        for decl in joined {
            state.union_sense |= decl.parts.iter().fold(decl.sense, |sense, part| sense | part.sense);
            state.members.push(decl);
        }
    });
}

/// Places the `over` parts of the root of `arena` (`member` None) or of its member `member` above the members joined
/// so far, as `place_over_widgets` does; journaled for a replay.
fn place_over(ctx: &Context, arena: ArenaId, member: Option<Id>) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| place_over(ctx, arena, member));
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        let at = over_position(state.members.len());
        match member {
            None => state.own_over_at = at,
            Some(id) => {
                if let Some(joined) = state.members.iter_mut().rev().find(|joined| joined.id == id) {
                    joined.over_at = at;
                }
            }
        }
    });
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
    for part in spec.own.iter().filter(|part| reaches_out(spec.rect, part)) {
        let id = sub(spec.id, ("edge", part.key, part.index));
        response::widget(ui, id, part.shape.bounds().intersect(clip), part.sense);
        crate::runtime::egui::access::hide(ctx, id);
        widgets.push(id);
    }
    widgets.extend(over_parts(spec.own).map(|part| over_widget(spec.id, part)));
    // The root's background lies below its parts and members and hovers wherever nothing of the root above it takes
    // the pointer; it takes a press when the root senses one itself.
    let background = background_shape(spec.id, spec.rect, spec.own_sense | Sense::HOVER);
    pointer::record_widgets_in(ctx, read.viewport, layer, spec.id, &widgets, sense.contains(Sense::CLICK));
    let info = RootInfo { id: spec.id, rect: spec.rect };
    journal_root(ctx, arena, info, spec.own, &widgets, (layer, sense));
    // The parts last pass registered, which this pass's results read.
    let known = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        state.own.clear();
        state.own.extend_from_slice(spec.own);
        state.root = Some(info);
        state.widgets = widgets;
        state.last_layer = Some(layer);
        state.last_sense = sense;
        // A root that stepped early while it was not yet known to be inert: an open press ends as the disabled root's
        // would, by stepping again with the cancel.
        if !spec.enabled && state.early == read.pass_nr && state.outcome.is_some_and(|outcome| outcome.pass == read.pass_nr) {
            state.outcome = None;
        }
        known_in(state, spec.id)
    });
    pointer::record_shapes_in(
        ctx,
        read.viewport,
        layer,
        spec.id,
        clip,
        std::iter::once(background).chain(own_shapes(spec.own, spec.id, read.target())),
    );
    let mut response = response;
    let outcome = outcome_in(ctx, &read, arena, layer, spec.enabled, sense);
    // The root is hovered while a shape of its arena shows hover (`pointer::Under`): its background, a part or a member;
    // it contains the pointer while any shape of the arena lies under it.
    response::hover_flags(&mut response, outcome.hover.any(), outcome.contains);
    // The root's response reports the presses on the root and on every part of its own, whichever egui widget took
    // them; the part results say which part. A member reports in its own response.
    let on_root = |hit: &Hit| hit.owner == spec.id;
    response::press_flags(&mut response, press_view(&outcome, on_root, on_root));
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

/// Whether a root's own `part` is an edge or handle part, not `over`, that reaches outside the root's `rect`: it gets
/// an egui widget of its own, so egui's hit test names the root there and not a raw egui widget beneath.
fn reaches_out(rect: Rect, part: &PartDecl) -> bool {
    !part.over && part.priority.level >= HitLevel::Edge && !rect.contains_rect(part.shape.bounds())
}

fn over_widget(root: Id, part: &PartDecl) -> Id {
    sub(root, ("over", part.key, part.index))
}

/// Records the shapes of the component's `over` parts after its body ran and before its `paint_over`, so they stand
/// above everything the body showed and below what a later `paint_over` shows: one paint order for presses (8.3).
/// Each part gets an egui widget there too, so egui's hit test names the component over its body's widgets. A
/// root's ids are part of its widgets from step 8. The `over` parts of a root or a member take their stack position
/// here, above every member its body showed; a member's widgets stand for the arena root.
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
    place_over(ctx, arena, member.map(|_| id));
    let shapes = with_arena_in(ctx, read.viewport, arena, read.pass_nr, |state| {
        if member.is_none() {
            return Some(own_over_shapes(&state.own, id, state.own_over_at, read.target()));
        }
        let joined = state.members.iter().rev().find(|joined| joined.id == id)?;
        Some(member_over_shapes(joined, read.target()).collect::<Vec<_>>())
    });
    let Some(shapes) = shapes else { return };
    pointer::record_shapes_in(ctx, read.viewport, layer, arena.0, clip, shapes);
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

#[expect(clippy::unreachable, reason = "`sense_members` returns one result per spec")]
pub(crate) fn sense_member(ui: &Ui, arena: ArenaId, spec: MemberSpec<'_>) -> MemberResult {
    let mut results = sense_members(ui, arena, std::slice::from_ref(&spec));
    results.pop().unwrap_or_else(|| unreachable!("one result per member"))
}

/// Step 8 of the members of `arena` in show order, joined in one read of the pointer and one of the arena: a member
/// run's own join, or the visible items of a collection whose items are members of its run.
pub(crate) fn sense_members(ui: &Ui, arena: ArenaId, specs: &[MemberSpec<'_>]) -> Vec<MemberResult> {
    let ctx = ui.ctx();
    debug_assert!(!crate::runtime::measure::active(ctx), "a member joins its arena in a real run only");
    let layer = ui.layer_id();
    let pass_nr = pass::pass_nr(ctx, pass::current_viewport(ctx));
    // One read of the pointer and one of the arena serve the joins, the outcome and the part results.
    let (pointer_pos, target, mods, sample) = pointer::with_frame(ctx, |frame, hover| {
        (hover, frame.map_or(0.0, |frame| frame.target), frame.map_or(Mods::NONE, |frame| frame.mods), frame.and_then(|frame| frame.sample))
    });
    let mut shapes = Vec::new();
    let mut joined = Vec::with_capacity(specs.len());
    let recording = crate::runtime::replay::recording(ctx);
    let mut recorded = Vec::new();
    let outcome = with_arena(ctx, arena, pass_nr, |state| {
        rotate(state, pass_nr);
        for spec in specs {
            let mut decl = MemberDecl::new(spec.id, spec.rect, spec.sense, spec.parts);
            decl.local = state.layer == Some(layer);
            let position = member_position(state.members.len());
            decl.over_at = position + 1;
            shapes.extend(member_shapes(&decl, position, target));
            state.union_sense |= decl.sense;
            for part in &decl.parts {
                state.union_sense |= part.sense;
            }
            joined.push((decl.local, known_in(state, spec.id)));
            if recording {
                recorded.push(decl.clone());
            }
            state.members.push(decl);
        }
        state.outcome.filter(|outcome| outcome.pass == pass_nr).unwrap_or_default()
    });
    if recording {
        journal_join(ctx, arena, &recorded);
    }
    pointer::record_shapes(ctx, layer, arena.0, ui.clip_rect(), shapes);
    specs
        .iter()
        .zip(joined)
        .map(|(spec, (local, known))| {
            let view = member_view(&outcome, spec.id, local, pointer_pos);
            MemberResult {
                response: response::member(ui, spec.id, spec.rect, spec.enabled, view),
                parts: part_results_in(&outcome, &known, (mods, sample, pointer_pos), spec.id, spec.parts),
                press: shape_press_in(&outcome, mods, sample, spec.id),
                activated: view.press.clicked,
            }
        })
        .collect()
}

fn member_view(outcome: &Outcome, id: Id, local: bool, pointer_pos: Option<Pos2>) -> MemberView {
    MemberView {
        hovered: outcome.hover.find(|hit| hit.owner == id).is_some(),
        press: press_view(outcome, |hit| hit.owner == id, |hit| hit.owner == id && hit.part.is_none()),
        pointer: pointer_pos.map(|at| if local { outcome.xf.map_or(at, |map| map.to_layer(at)) } else { at }),
    }
}

/// The press flags of the shapes `holds` takes, with the click of the shapes `clicks` takes.
fn press_view(outcome: &Outcome, holds: impl Fn(&Hit) -> bool, clicks: impl Fn(&Hit) -> bool) -> PressView {
    let phase = outcome.target.filter(|target| holds(target)).and(outcome.phase);
    PressView {
        pressed: phase.is_some_and(|phase| phase.button_down()),
        drag_started: matches!(phase, Some(PressPhase::DragStart { .. })),
        dragged: phase.is_some_and(|phase| phase.is_drag()),
        drag_stopped: matches!(phase, Some(PressPhase::Release { click: false, .. })),
        clicked: outcome.activated.is_some_and(|hit| clicks(&hit)),
    }
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
    let (slop, delta, pos, buttons) = pass_pointer
        .frame
        .as_ref()
        .map_or((0.0, egui::Vec2::ZERO, None, craft_behavior::Buttons::empty()), |frame| (frame.slop, frame.delta, frame.pos, frame.buttons));
    let (body_layer, host_cancel) = with_arena(ctx, arena, pass_nr, |state| (state.layer, std::mem::take(&mut state.host_cancel)));
    let cancel = if enabled {
        pointer::cancel_of_pass(ctx).or(host_cancel.then_some(craft_behavior::PressCancel::Host))
    } else {
        Some(craft_behavior::PressCancel::Disabled)
    };
    let xf = body_layer.and_then(|layer| areas::layer_map(ctx, layer));
    // What the viewport's one resolution of the pointer puts on this arena (`pointer::Under`): hover shows where a press
    // would go.
    let (under, contains, captured) = pointer::with_hover(ctx, read.viewport, |under, captured| {
        let hovered = under.map_or(Hovered::default(), |under| Hovered {
            press: under.press.hit_of(arena.0).map(Hit::from),
            only: under.hover_top_of(arena.0).map(Hit::from),
        });
        (hovered, under.is_some_and(|under| under.contains(arena.0)), captured)
    });
    let (mut cancelled, mut release_held) = (false, false);
    let outcome = with_arena(ctx, arena, pass_nr, |state| {
        // A press open when the pass began, on another arena, takes the hover from this one.
        let others_press = captured && state.machine.target().is_none();
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
            // The press-target rule (8.3): a press's click and its drag each start on one root, the one whose shape
            // is on top under it for that kind (`pointer::press_winners`); the root starts it for the kinds it won. A
            // root under a modal layer starts none; where egui names no target of the root's kind (its click target,
            // its drag target for a drag-only root; a root whose widgets sense neither is never named), only a root the
            // modal's canvas admits does.
            let root = state.root.map(|root| root.id);
            let targets = pass_pointer.press_targets;
            let untargeted = if union_sense.contains(Sense::CLICK) {
                targets.click.is_none()
            } else if union_sense.contains(Sense::DRAG) {
                targets.drag.is_none()
            } else {
                true
            };
            let winners = pointer::press_winners(ctx, at, targets);
            let won_kinds = root.map_or(Sense::empty(), |own| winners.senses_of(own));
            let allowed = crate::runtime::sampler::root_takes_press(ctx, root, layer, at, untargeted) && !won_kinds.is_empty();
            if !enabled || !allowed {
                return None;
            }
            // egui gives no click target where a drag-only widget is on top (a resize grip over a layer's or a
            // window's edge): a root under a raw egui one keeps its click there, but a drag is that widget's.
            let drag_elsewhere = targets.click.is_none() && targets.drag.is_some_and(|id| pointer::foreign_target(ctx, id));
            let won = root.and_then(|own| winners.hit_of(own)).map(Hit::from);
            // The kinds of press another root won are not this one's.
            let lost = (Sense::CLICK | Sense::DRAG).difference(won_kinds);
            won.map(|hit| {
                let sense = hit.sense.difference(lost);
                (hit, if drag_elsewhere { sense.difference(Sense::DRAG) } else { sense })
            })
        });
        cancelled = matches!(step.phase, Some(PressPhase::Cancel { .. }));
        release_held = machine.release_held();
        let open = machine.target();
        let dragging = step.dragging;
        state.machine = machine;
        // While a press is open only its target hovers, and only while the pointer is over it or dragging.
        let hover = match open {
            Some(target) if dragging => Hovered { press: Some(target), only: None },
            Some(target) => under.filter(|hit| hit.same(&target)),
            None if others_press => Hovered::default(),
            None => under,
        };
        let outcome = Outcome { pass: pass_nr, hover, contains, target: step.target, phase: step.phase, dragging, activated: step.activated, xf };
        state.outcome = Some(outcome);
        outcome
    });
    if cancelled {
        crate::runtime::frame::note_cancelled_press(ctx);
    }
    if release_held {
        crate::runtime::egui::repaint::request(ctx);
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
                state.host_cancel = false;
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
                    sense: decl.sense,
                    inactive: decl.inactive,
                    held: decl.held,
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
    let hover = outcome.hover.find(|hit| is_known && mine(hit));
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
        _ if outcome.hover.find(|hit| hit.owner == owner && hit.part.is_none()).is_some() => PressPhase::Hover,
        _ => PressPhase::Idle,
    };
    let captured = won.is_some() && phase.is_drag();
    PressState::new(phase).mods(mods).sample(sample).captured(captured)
}

/// The cursor a hovered part of `decls` declares, if any: the sensing core's request for the cursor service.
pub(crate) fn hovered_cursor(decls: &[PartDecl], results: &PartResults) -> Option<Cursor> {
    decls.iter().filter(|decl| results.get(&(decl.key, decl.index)).is_some_and(|result| result.hovered)).find_map(|decl| decl.cursor.clone())
}
