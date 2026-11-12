//! The component's side of the pipeline. `Typed` and `TypedPass` give the pipeline in `run` what it needs of one
//! component type through `Erased` and `ErasedPass`, and the entry points wrap a component in them. This is the code
//! each component type instantiates; the pipeline itself is compiled once, in this crate.

use super::{Erased, ErasedPass, Host, HostCx, LeafHooks, Measuring, ShapeHooks, ShowAt, TypeFacts, blank_core, paint_ui};
use crate::contract::PaintUi;
use crate::contract::cx::{CxCore, PartsCx};
use crate::contract::steps::{self, MeasureIn, PartsStep, ShapeKind};
use crate::contract::{
    Access, BodyCx, Collection, Common, Component, Container, Cx, Inner, Internal, ItemFn, Meta, OverlayUi, Resolved, Show, TipFn, Validity, ValueOf,
};
use crate::primitives::{FaceRecord, face_id};
use crate::runtime::access::{Politeness, SetPosition};
use crate::runtime::commands::{self, ResolvedCommand};
use crate::runtime::env::{AppValues, Env, Message};
use crate::runtime::focus;
use crate::runtime::layout_memory::Inputs;
use crate::runtime::memory::{MemoryStage, sub};
use crate::runtime::overlay::CloseReason;
use crate::runtime::reads::{self, BindKey};
use crate::style::{Flags, StateSet, StyleCx, StyleView, UiTokens};
use craft_behavior::{EditEvent, Flow, ItemSource, KeyRole, PartRef, Proposal, Roving, Sense};
use craft_theme::Variant;
use egui::{Context, Id, Rect, Ui, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The component a `Typed` holds: its own for a show, which `finish` consumes, or the caller's for a measure.
enum Held<'c, C> {
    Owned(Option<C>),
    Lent(&'c mut C),
}

impl<C> Held<'_, C> {
    fn get(&mut self) -> &mut C {
        match self {
            Held::Owned(comp) => comp.as_mut().expect("the component is held until `finish`"),
            Held::Lent(comp) => comp,
        }
    }

    fn get_ref(&self) -> &C {
        match self {
            Held::Owned(comp) => comp.as_ref().expect("the component is held until `finish`"),
            Held::Lent(comp) => comp,
        }
    }

    fn take(&mut self) -> C {
        match self {
            Held::Owned(comp) => comp.take().expect("`finish` runs once"),
            Held::Lent(_) => unreachable!("a measure never finishes the component"),
        }
    }
}

/// A component and its shape's hooks, with the typed values steps 1 to 5 found.
struct Typed<'c, 'h, C: Component> {
    comp: Held<'c, C>,
    hooks: &'h mut dyn ShapeHooks<C>,
    style: Option<Arc<C::Style>>,
    shown: Option<Arc<ValueOf<C>>>,
    mixed: bool,
    cancelled: Vec<EditEvent<ValueOf<C>>>,
    output: Option<C::Output>,
}

impl<'c, 'h, C: Component> Typed<'c, 'h, C> {
    fn new(comp: Held<'c, C>, hooks: &'h mut dyn ShapeHooks<C>) -> Self {
        Typed { comp, hooks, style: None, shown: None, mixed: false, cancelled: Vec::new(), output: None }
    }
}

impl<C: Component> Erased for Typed<'_, '_, C> {
    fn facts(&self) -> TypeFacts {
        TypeFacts::of::<C>()
    }

    fn common(&self) -> &Common {
        self.comp.get_ref().common(Internal::new())
    }

    fn take_common(&mut self) -> Common {
        let common = std::mem::take(self.comp.get().common_mut(Internal::new()));
        // The sizing stays with the component: the layout hooks after step 1 read it through `Component::fills`.
        self.comp.get().common_mut(Internal::new()).sizing = common.sizing;
        common
    }

    fn variants(&self, asked: Option<Variant>) -> (Variant, Variant) {
        steps::variants::<C>(asked)
    }

    fn style(&mut self, ctx: &Context, scx: &StyleCx<'_>) -> Arc<dyn StyleView> {
        let style = steps::style::<C>(ctx, scx, self.comp.get_ref().style_key());
        self.style = Some(Arc::clone(&style));
        style
    }

    fn value(&mut self, stage: &mut MemoryStage, id: Id, command: Option<&ResolvedCommand>, sizing: bool) -> bool {
        let value = steps::value(self.comp.get(), stage, id, command);
        self.shown = value.value;
        self.mixed = value.mixed;
        self.cancelled = value.cancelled.into_iter().filter(|_| !sizing).collect();
        self.mixed
    }

    fn peek_value(&mut self, ctx: &Context, id: Id, command: Option<&ResolvedCommand>) -> bool {
        (self.shown, self.mixed) = steps::peek_value(self.comp.get(), ctx, id, command);
        self.mixed
    }

    fn bind_key(&mut self, id: Id) -> Option<BindKey> {
        self.comp.get().bind().and_then(|bind| bind.key(id))
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        self.comp.get_ref().fills()
    }

    fn cut_first(&self) -> bool {
        self.comp.get_ref().cut_first()
    }

    fn measure_content(&mut self, input: &MeasureIn<'_>, resolved: &Resolved<'_>) -> (Vec2, Inputs) {
        let style = self.style.as_deref().expect("step 2 ran");
        steps::measure_content(self.comp.get(), input, style, resolved, self.shown.as_deref())
    }

    fn measured_extent(&mut self, at: &Measuring<'_, '_>, stage: &mut MemoryStage) -> Vec2 {
        let style = self.style.as_deref().expect("step 2 ran");
        let (extent, shown) = measured_extent(self.comp.get(), &mut *self.hooks, at, stage, style, self.shown.take());
        self.shown = shown;
        extent
    }

    fn pass_extent(&mut self, at: &Measuring<'_, '_>) -> Vec2 {
        let style = self.style.as_deref().expect("step 2 ran");
        self.hooks.pass_extent(self.comp.get(), at, style, self.shown.clone())
    }

    fn with_cx<'a>(&mut self, core: CxCore<'a>, style: &'a dyn StyleView, rest: &mut dyn FnMut(&mut dyn ErasedPass<'a>) -> bool) {
        let style = style.as_any().downcast_ref::<C::Style>().expect("the pipeline holds the style `Erased::style` gave");
        let events = std::mem::take(&mut self.cancelled);
        // A cancel step 4 raised over a document that moved is reported, not applied: the app's value stays. One that
        // carries the value the gesture began with restores it at step 11.
        let applied = if events.iter().any(|event| matches!(event, EditEvent::Cancel { before: Some(_), .. })) { 0 } else { events.len() };
        let mut cx = Cx::<C> { core, style, shown: self.shown.take(), mixed: self.mixed, events, applied, validity: Validity::Ok };
        let mut comp = self.comp.take();
        let real = rest(&mut TypedPass { comp: &mut comp, hooks: &mut *self.hooks, cx: &mut cx });
        let id = cx.core.id;
        let output = comp.finish(cx);
        debug_assert!(!real || crate::contract::Outcome::response(&output).id == id, "an output's Response is the component's own");
        self.output = Some(output);
    }

    fn shape(&self) -> ShapeKind {
        self.hooks.shape()
    }

    fn declares_parts(&self) -> bool {
        self.hooks.declares_parts()
    }

    fn read(&mut self, stage: &mut MemoryStage, id: Id, env: &Env) {
        self.hooks.read(self.comp.get(), stage, id, env);
    }

    fn instance_scope(&self) -> Option<commands::CommandScope> {
        self.hooks.instance_scope(self.comp.get_ref())
    }

    fn in_flow(&self) -> bool {
        self.hooks.in_flow(self.comp.get_ref())
    }

    fn fills_width(&self) -> bool {
        self.hooks.fills_width(self.comp.get_ref())
    }

    fn overflow(&self) -> f32 {
        self.hooks.overflow()
    }

    fn sized(&self, ui: &Ui, id: Id, size: Vec2) -> Vec2 {
        self.hooks.sized(self.comp.get_ref(), ui, id, size)
    }

    fn arena(&self) -> bool {
        self.hooks.arena(self.comp.get_ref())
    }

    fn extent(&mut self, ui: &Ui, at: ShowAt, id: Id, scx: &StyleCx<'_>) -> Vec2 {
        self.hooks.extent(self.comp.get_ref(), ui, at, id, scx)
    }

    fn pad(&self, tokens: &UiTokens) -> f32 {
        self.hooks.pad(self.comp.get_ref(), tokens)
    }

    fn host(&mut self, at: HostCx<'_>) -> Box<dyn Host> {
        self.hooks.host(self.comp.get_ref(), at)
    }

    fn record(&mut self, stage: &mut MemoryStage, ctx: &Context, id: Id) {
        self.hooks.record(stage, ctx, id);
    }
}

/// A component, its shape's hooks and its `Cx` for steps 6 to 17.
struct TypedPass<'r, 'a, C: Component> {
    comp: &'r mut C,
    hooks: &'r mut dyn ShapeHooks<C>,
    cx: &'r mut Cx<'a, C>,
}

impl<'a, C: Component> ErasedPass<'a> for TypedPass<'_, 'a, C> {
    fn core(&mut self) -> &mut CxCore<'a> {
        &mut self.cx.core
    }

    fn core_ref(&self) -> &CxCore<'a> {
        &self.cx.core
    }

    fn shown_state(&mut self) -> &mut steps::ShownState {
        let id = self.cx.core.id;
        &mut self.cx.core.memory.shown::<steps::Shown<ValueOf<C>>>(id).state
    }

    fn value_text(&self) -> Option<String> {
        self.comp.value_text(self.cx)
    }

    fn keys(&self) -> focus::KeyUse {
        self.comp.keys()
    }

    fn sense(&self) -> Sense {
        self.hooks.sense(self.comp, self.comp.sense())
    }

    fn comp_sense(&self) -> Sense {
        self.comp.sense()
    }

    fn bound(&mut self) -> (bool, bool) {
        self.comp.bind().map_or((false, false), |bind| (true, bind.reset_value().is_some()))
    }

    fn paint_outset(&self, tokens: &crate::style::UiTokens) -> f32 {
        C::paint_outset(tokens)
    }

    fn padding(&self, rect: Rect) -> craft_behavior::Insets {
        self.comp.padding(rect, self.cx.style)
    }

    fn parts(&mut self, rect: Rect, flow: Flow, content_rect: Rect, parts: &mut PartsStep, input: &MeasureIn<'_>, rtl: bool) {
        let id = self.cx.core.id;
        let last = self.cx.core.memory.shown::<steps::Shown<ValueOf<C>>>(id).state.last_state;
        let measure = steps::measure_cx::<C>(input, self.cx.style, self.cx.core.resolved, self.cx.shown.as_deref());
        let geometry = self.hooks.geometry(self.comp, &measure, (rect, content_rect), rtl);
        let mut pcx = PartsCx {
            content_rect,
            flow,
            decls: &mut parts.decls,
            rovings: &mut parts.rovings,
            scrolls: &mut parts.scrolls,
            text: &mut parts.text,
            last_state: last,
            measure,
            items: geometry,
        };
        self.comp.parts(rect, &mut pcx);
        self.hooks.parts(self.comp, rect, &mut pcx);
    }

    fn text(&mut self, ui: &mut Ui, step: &mut crate::contract::text::TextStep, keys: focus::KeyUse, rtl: bool, ink: egui::Color32, underline: f32) {
        crate::contract::text::run(self.cx, ui, step, keys, rtl, ink, underline);
    }

    fn editor(&mut self, ui: &mut Ui, step: &mut crate::contract::text::TextStep, keys: focus::KeyUse) {
        crate::contract::editor::run(self.cx, ui, step, keys);
    }

    fn gesture_gate(&mut self) {
        steps::gesture_gate(self.comp, self.cx);
    }

    fn edit_intents(&mut self) {
        steps::edit_intents(self.comp, self.cx);
    }

    fn scrub_by_label(&mut self) {
        steps::scrub_by_label(self.comp, self.cx);
    }

    fn update(&mut self) {
        self.comp.update(self.cx);
    }

    fn overlay(&mut self, overlays: &mut OverlayUi<'_>) {
        self.comp.overlay(overlays, self.cx);
    }

    fn apply_values(&mut self) -> steps::ValueEdit {
        steps::apply_values(self.comp, self.cx)
    }

    fn register_reset(&mut self) {
        steps::register_reset(self.comp, self.cx);
    }

    fn value_flags(&self) -> Flags {
        steps::value_flags(self.comp, self.cx)
    }

    fn content_env(&self, values: &mut AppValues) {
        self.comp.content_env(self.cx, values);
    }

    fn paint(&mut self, ui: &mut Ui, env: &Env) {
        self.comp.paint(&mut paint_ui(ui, env, &self.cx.core, self.cx.style), self.cx);
    }

    fn paint_over(&mut self, ui: &mut Ui, env: &Env) {
        self.comp.paint_over(&mut paint_ui(ui, env, &self.cx.core, self.cx.style), self.cx);
    }

    fn settle_values(&mut self) -> steps::ValueEdit {
        steps::settle(self.comp, self.cx)
    }

    fn access(&self) -> (Access, Option<String>) {
        let mut access = Access::default();
        self.comp.access(&mut access, self.cx);
        (access, self.comp.value_text(self.cx))
    }

    fn validity(&self) -> Option<(&Message, bool)> {
        steps::validity_of(&self.cx.validity).or_else(|| self.cx.core.text_unparsed.as_ref().map(|message| (message, false)))
    }

    fn shows_label(&self) -> bool {
        self.comp.shows_label()
    }

    fn activated(&self) -> bool {
        self.comp.activated(self.cx)
    }

    fn changed(&self) -> bool {
        steps::value_changed(&self.cx.events)
    }

    fn measure_content(&mut self, input: &MeasureIn<'_>, resolved: &Resolved<'_>) -> (Vec2, Inputs) {
        steps::measure_content(self.comp, input, self.cx.style, resolved, self.cx.shown.as_deref())
    }

    fn shape(&self) -> ShapeKind {
        self.hooks.shape()
    }

    fn declares_parts(&self) -> bool {
        self.hooks.declares_parts()
    }

    fn in_flow(&self) -> bool {
        self.hooks.in_flow(self.comp)
    }

    fn regions(&self, id: Id) -> Vec<(Id, Vec2)> {
        self.hooks.regions(id)
    }

    fn focus(&self) -> Option<focus::FocusPolicy> {
        self.hooks.focus(self.comp)
    }

    fn sized(&self, ui: &Ui, id: Id, size: Vec2) -> Vec2 {
        self.hooks.sized(self.comp, ui, id, size)
    }

    fn fills_width(&self) -> bool {
        self.hooks.fills_width(self.comp)
    }

    fn first_line(&self, ctx: &Context, id: Id, height: f32) -> Option<f32> {
        self.hooks.first_line(self.comp, ctx, id, height)
    }

    fn member(&self) -> bool {
        self.hooks.member(self.comp)
    }

    fn fills_width_only(&self) -> bool {
        self.hooks.fills_width_only(self.comp)
    }

    fn arena(&self) -> bool {
        self.hooks.arena(self.comp)
    }

    fn key_role(&self) -> KeyRole {
        self.hooks.key_role(self.comp)
    }

    fn roving(&self) -> Option<Roving> {
        self.hooks.roving(self.comp)
    }

    fn expanded(&self) -> Option<bool> {
        self.hooks.expanded(self.comp)
    }

    fn live(&self) -> Option<Politeness> {
        self.hooks.live(self.comp)
    }

    fn position(&self) -> Option<SetPosition> {
        self.hooks.position(self.comp)
    }

    fn scrolls(&self) -> (bool, bool) {
        self.hooks.scrolls(self.comp)
    }

    fn overflow(&self) -> f32 {
        self.hooks.overflow()
    }

    fn sensed(&mut self) {
        self.hooks.sensed(self.comp, self.cx);
    }

    fn updated(&mut self) {
        self.hooks.updated(self.comp, self.cx);
    }

    fn apply(&mut self) {
        self.hooks.apply(self.comp, self.cx);
    }

    fn content(&mut self, ui: &mut Ui, env: &Env, values: &AppValues) {
        self.hooks.content(self.comp, ui, env, self.cx, values);
    }

    fn settle(&mut self) {
        self.hooks.settle(self.comp, self.cx);
    }

    fn dismissed(&mut self, why: CloseReason) {
        self.hooks.dismissed(self.comp, self.cx, why);
    }
}

fn run<C: Component>(comp: C, ui: &mut Ui, at: ShowAt, tip: Option<TipFn<'_>>, hooks: &mut dyn ShapeHooks<C>) -> C::Output {
    let mut typed = Typed::new(Held::Owned(Some(comp)), hooks);
    super::run(&mut typed, ui, at, tip);
    typed.output.expect("the pipeline finishes every run")
}

fn measure_shape<C: Component>(
    comp: &mut C,
    ui: &Ui,
    id: Option<Id>,
    proposal: Proposal,
    under: Option<&crate::contract::cx::Under>,
    hooks: &mut dyn ShapeHooks<C>,
) -> Vec2 {
    super::measure_shape(&mut Typed::new(Held::Lent(comp), hooks), ui, id, proposal, under)
}

pub(crate) fn show_leaf<C: Component + Meta<Shape = crate::contract::shape::Leaf>>(
    comp: C,
    ui: &mut Ui,
    at: ShowAt,
    tip: Option<TipFn<'_>>,
) -> C::Output {
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
    mut comp: C,
    ui: &mut Ui,
    at: ShowAt,
    mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
    tip: Option<TipFn<'_>>,
) -> Inner<C::Output, R> {
    if let Some(output) = unshown(&mut comp, ui) {
        return Inner { output, inner: None, settle: None };
    }
    let mut inner: Option<R> = None;
    let mut ran = false;
    let mut call = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
        // The measuring run and a sizing parent's run are peek stages; only a real run settles.
        ran |= !cx.memory.is_peek();
        inner = Some(body(ui, cx));
    };
    let ctx = ui.ctx().clone();
    let props = Arc::new(comp.props(Internal::new()).clone());
    let output = run(comp, ui, at, tip, &mut crate::contract::body::BodyHooks::new(&mut call, props));
    let id = crate::contract::Outcome::response(&output).id;
    let settle = ran.then(|| crate::contract::settle::SettleAt { ctx, body: sub(id, "body"), owner: id });
    Inner { output, inner, settle }
}

/// A layer-like container that shows nothing this pass runs no pipeline: it takes its id as step 3 would, forgets
/// its layer and returns its output's unshown form. `None` when it shows, or when its output has no unshown form.
fn unshown<C: Container>(comp: &mut C, ui: &Ui) -> Option<C::Output> {
    let props = comp.props(Internal::new());
    if matches!(props.mount, crate::contract::Mount::Inline | crate::contract::Mount::Pane(_) | crate::contract::Mount::Window { .. }) {
        return None;
    }
    let props = props.clone();
    let facts = TypeFacts::of::<C>();
    let source = comp.common(Internal::new()).id.clone();
    let id = steps::id(ui, &source, false, false, facts.style_key, facts.name);
    let output = <C::Output as crate::contract::Outcome>::unshown(steps::inert_response(ui, id))?;
    let sizing = ui.is_sizing_pass();
    if !crate::contract::body::unshown(comp, ui.ctx(), id, &props, sizing) {
        return None;
    }
    steps::id(ui, &source, !sizing, true, facts.style_key, facts.name);
    if !sizing {
        crate::contract::overlay::forget_layer(ui.ctx(), id, id);
    }
    Some(output)
}

pub(crate) fn show_items<'r, C: Collection>(comp: C, ui: &mut Ui, source: &dyn ItemSource<C::Key>, item: &mut ItemFn<'r, C>) -> C::Output {
    let view = comp.filtered(source, &Env::of(ui));
    let source = view.as_deref().unwrap_or(source);
    run(comp, ui, ShowAt::Flow, None, &mut crate::contract::items::ItemsHooks::<C>::new(source, item))
}

/// Measures `comp` as `show` will show it, without allocating: steps 1 to 5 under the id `show` would use next. It
/// reads memory through a peek, registers no id and applies nothing. `under` is what the composite hands a child it
/// shows with `PaintUi::child` or `PaintUi::labelled` when it measures that child.
pub(crate) fn measure_under<C: Component>(
    comp: &mut C,
    ui: &Ui,
    id: Option<Id>,
    proposal: Proposal,
    under: Option<&crate::contract::cx::Under>,
) -> Vec2 {
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
    let props = Arc::new(comp.props(Internal::new()).clone());
    measure_shape(comp, ui, None, proposal, None, &mut crate::contract::body::BodyHooks::new(&mut call, props))
}

/// A measuring run's extent: the shape's `measured_extent` under a `Cx` that senses nothing and writes only to the
/// peek `stage`. Returns the shown value back with it.
pub(crate) fn measured_extent<C: Component>(
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

/// A face run is steps 1, 2, 4 (read only), 12 and 13 of the face component, inside the composite's step 13, under
/// the composite's `env`. It registers no id, senses nothing, opens no memory stage and fires nothing. Returns the
/// record step 16 fills the part's sub-node from, and the facts the face's `access` declared.
pub(crate) fn paint_face<C: Show>(ui: &mut Ui, env: &Env, source: &super::FaceSource<'_>, part: PartRef, mut face: C) -> FaceRecord {
    let ctx = ui.ctx().clone();
    let common = std::mem::take(face.common_mut(Internal::new()));
    debug_assert!(common.tooltip.is_none(), "a face instance carries no tooltip; use `PartBuilder::tooltip`");
    let step1 = steps::env(&ctx, &env, &common, steps::variants::<C>(common.variant));
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
    let part_state = steps::part_state(source.result, source.faded || (step1.enabled && source.enabled), source.focus);
    // The face reads its bound command (marked, checked, hint) as a shown component does.
    let mut core = blank_core(env.view(), &resolved, &mut stage, ctx.clone(), env.ink, source.response.clone(), step1.command.clone());
    core.face_mode = true;
    core.faded = source.faded;
    core.rect = rect;
    core.content_rect = flow.inset(face.padding(rect, &style));
    core.flow = flow;
    core.visible = rect;
    let mut cx = Cx::<C> { core, style: &style, shown, mixed, events: Vec::new(), applied: 0, validity: Validity::Ok };
    // A face shows its own inputs as `steps::state` reads them for a component: a checked check box takes the on
    // look, a selected face the selected one.
    let flags = part_state.flags | source.flags | steps::value_flags(&face, &cx);
    let on = f32::from(u8::from(flags.intersects(crate::style::Flags::ON)));
    let state = StateSet::new(part_state.state, flags, crate::style::StateMix::new(part_state.t.hover, part_state.t.press, on));
    cx.core.state = state;
    cx.core.stated = true;
    cx.core.ink = steps::surface_ink(StyleView::surface(&*style), &state, env.ink);
    steps::paint_surface(ui, &env, &cx.core, &*style);
    {
        let results = HashMap::new();
        let mut pui = PaintUi {
            ui: &mut *ui,
            env: &env,
            ink: cx.core.ink,
            // A face of an inactive part paints its data as `draw_part` does for the host.
            data: if source.result.inactive { crate::primitives::DataMode::Inactive } else { crate::primitives::DataMode::of(&state) },
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
            selections: &[],
            unfaded: None,
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
    let mut access = Access::default();
    face.access(&mut access, &cx);
    FaceRecord { part, name, flags, role: C::ROLE, numeric: access.numeric, expanded: access.expanded, enabled: step1.enabled, baseline }
}

/// A member of a collection's run (`Collection::MEMBERS`) after steps 1, 2, 4 (read only) and 7: what the batch join
/// reads and what [`member_show`] runs on.
pub(crate) struct MemberPrep<C: Component> {
    item: C,
    common: Common,
    step1: steps::EnvStep,
    style: Arc<C::Style>,
    shown: Option<Arc<ValueOf<C>>>,
    mixed: bool,
    pub decls: crate::contract::cx::Decls,
    /// What the member senses of its own: the item's sense less the press and the focus, which the collection's root
    /// takes (17.2), as for an item that runs.
    pub sense: Sense,
    pub tooltip: Option<crate::contract::Tooltip>,
}

/// Steps 1, 2, 4 (read only) and 7 of the member `id` at `row`; `hovered` is the arena's hover of the member this pass,
/// which `PartsCx::hovered` reads.
pub(crate) fn member_prepare<C: Show>(ui: &Ui, env: &Env, id: Id, row: Rect, hovered: bool, mut item: C) -> MemberPrep<C> {
    let ctx = ui.ctx();
    let mut common = std::mem::take(item.common_mut(Internal::new()));
    let tooltip = common.tooltip.take();
    let step1 = steps::env(ctx, env, &common, steps::variants::<C>(common.variant));
    let style = steps::style::<C>(ctx, &steps::style_cx(ctx, env, &step1), item.style_key());
    let (shown, mixed) = steps::peek_value(&mut item, ctx, id, step1.command.as_deref());
    if let Some(key) = item.bind().and_then(|bind| bind.key(id)) {
        reads::note(ctx, key, id, crate::runtime::measure::active(ctx), &[]);
    }
    let mut parts = PartsStep { decls: Default::default(), rovings: Vec::new(), scrolls: Vec::new(), text: Default::default() };
    if C::PARTS {
        let resolved = steps::resolved(id, &step1, &common);
        let flow = Flow::new(row, env.direction.is_rtl());
        let content_rect = flow.inset(item.padding(row, &style));
        let input = MeasureIn { ui, env, proposal: Proposal::Ideal, room: None };
        let mut pcx = PartsCx {
            content_rect,
            flow,
            decls: &mut parts.decls,
            rovings: &mut parts.rovings,
            scrolls: &mut parts.scrolls,
            text: &mut parts.text,
            last_state: (hovered, false),
            measure: steps::measure_cx::<C>(&input, &style, &resolved, shown.as_deref()),
            items: None,
        };
        item.parts(row, &mut pcx);
    }
    debug_assert!(
        parts.rovings.is_empty() && parts.scrolls.is_empty() && parts.text.decl.is_none() && parts.decls.iter().all(|(_, extras)| !extras.focusable),
        "a member of a collection's run declares no stop, roving group, scroll part or text part"
    );
    let sense = item.sense().difference(Sense::CLICK | Sense::FOCUS) | Sense::HOVER;
    MemberPrep { item, common, step1, style, shown, mixed, decls: parts.decls, sense, tooltip }
}

/// Where a member's sub-node goes: the collection's node `owner`, the item part it reports as, and its row facts.
pub(crate) struct MemberNode {
    pub owner: Id,
    pub part: PartRef,
    pub parent: Id,
    pub position: SetPosition,
    /// The row's flags: selected, lead, expanded, current, dragging, drop target.
    pub flags: Flags,
    pub enabled: bool,
    /// The member's sense as an item: whether its sub-node takes AccessKit focus and clicks, which the collection serves.
    pub item_sense: Sense,
}

/// What a member's show leaves for the collection: its output, and its tooltip while the pointer rests on it.
pub(crate) struct MemberShown<O> {
    pub output: O,
    pub tip: Option<crate::contract::tooltip::MemberTip>,
}

/// Steps 9 (update), 12, 13, 16 and the finish of a member that joined the arena as `sensed`: the item's `update`
/// reads its parts and its press, its `paint` and `paint_over` draw at its row, its node and its parts' nodes are
/// reported under the collection's node, and `finish` makes its output.
pub(crate) fn member_show<C: Show>(
    ui: &mut Ui,
    env: &Env,
    (id, row): (Id, Rect),
    prep: MemberPrep<C>,
    sensed: crate::primitives::sensing::MemberResult,
    node: &MemberNode,
) -> MemberShown<C::Output> {
    let ctx = ui.ctx().clone();
    let MemberPrep { mut item, common, step1, style, shown, mixed, decls, sense, tooltip } = prep;
    let rtl = env.direction.is_rtl();
    let resolved = steps::resolved(id, &step1, &common);
    let enabled = node.enabled && step1.enabled;
    let faded = !enabled && C::DISABLED == crate::contract::DisabledLook::Faded;
    let hovered = sensed.response.hovered() && !faded;
    let result = crate::primitives::PartResult {
        rect: row,
        press: sensed.press,
        hovered,
        hover_pos: None,
        flags: node.flags,
        sense,
        inactive: false,
        held: false,
    };
    let part_state = steps::part_state(&result, enabled || faded, None);
    // A peek that opens no lifetime stage and is dropped without a write-back, as a face's.
    let mut stage = MemoryStage::open_peek(&ctx, id);
    stage.face = true;
    let mut core = blank_core(env.view(), &resolved, &mut stage, ctx.clone(), env.ink, sensed.response, step1.command.clone());
    core.faded = faded;
    let flow = Flow::new(row, rtl);
    core.rect = row;
    core.content_rect = flow.inset(item.padding(row, &style));
    core.flow = flow;
    core.visible = row.intersect(ui.clip_rect());
    core.part_cursors = crate::primitives::sensing::hovered_cursor(decls.parts(), &sensed.parts).into_iter().collect();
    core.results = sensed.parts;
    core.press = sensed.press;
    core.member_of = Some(node.owner);
    core.intents = crate::runtime::keys::intents_of(&ctx, Some(id), rtl);
    let mut cx = Cx::<C> { core, style: &style, shown, mixed, events: Vec::new(), applied: 0, validity: Validity::Ok };
    let flags = part_state.flags | steps::value_flags(&item, &cx);
    let on = f32::from(u8::from(flags.intersects(Flags::ON)));
    let state = StateSet::new(part_state.state, flags, crate::style::StateMix::new(part_state.t.hover, part_state.t.press, on));
    cx.core.state = state;
    cx.core.stated = true;
    cx.core.ink = steps::surface_ink(StyleView::surface(&*style), &state, env.ink);
    item.update(&mut cx);
    let unfaded = steps::fade(ui, &mut cx.core);
    steps::paint_surface(ui, env, &cx.core, &*style);
    steps::paint_part_layers(ui, env, &cx.core);
    item.paint(&mut super::paint_ui(ui, env, &cx.core, &*style), &cx);
    item.paint_over(&mut super::paint_ui(ui, env, &cx.core, &*style), &cx);
    ui.set_opacity(unfaded);
    let mut access = Access::default();
    item.access(&mut access, &cx);
    let member_node = report_member(&cx.core, &access, node, C::ROLE);
    {
        let paint = cx.core.paint.borrow();
        steps::report_parts(&cx.core, &decls, &access.nodes, &paint.faces, member_node);
    }
    crate::contract::cx::write_cursor(&cx.core);
    let tip = member_tip(&cx.core, &decls, node.part, tooltip, row);
    MemberShown { output: item.finish(cx), tip }
}

/// The member's own sub-node of the collection's node: the facts the item's `access` declared, its row's place and
/// flags. Returns the node's id, which its parts' nodes attach to.
fn report_member(core: &CxCore<'_>, access: &Access, node: &MemberNode, role: crate::contract::Role) -> Id {
    use crate::runtime::access::{Name, SubDecl, SubNode};
    let name = match &access.name {
        Name::FromLabel => Name::Text(core.paint.borrow().elided.clone().unwrap_or_else(|| core.resolved.label.to_owned())),
        name => name.clone(),
    };
    let mut sub = SubNode::new(node.part, access.role.unwrap_or(role));
    sub.name = name;
    sub.description = access.description.clone();
    sub.value = access.value.clone();
    sub.color = access.color;
    sub.numeric = access.numeric.clone();
    sub.expanded = access.expanded;
    sub.position = Some(node.position);
    sub.flags = node.flags;
    let decl = SubDecl {
        owner: node.owner,
        component: "",
        node: &sub,
        rect: core.rect,
        enabled: node.enabled,
        focusable: node.enabled && node.item_sense.contains(Sense::FOCUS),
        clickable: node.enabled && node.item_sense.contains(Sense::CLICK),
        active: false,
        face: None,
        label: "",
        parent: node.parent,
    };
    crate::runtime::access::report_sub(&core.ctx, &decl);
    crate::runtime::access::sub_node_id(node.owner, node.part)
}

/// The tooltip of a hovered member: its hovered part's, else its own. A part's subject is keyed by the member, so two
/// members' same part are two subjects.
fn member_tip(
    core: &CxCore<'_>,
    decls: &crate::contract::cx::Decls,
    member: PartRef,
    own: Option<crate::contract::Tooltip>,
    row: Rect,
) -> Option<crate::contract::tooltip::MemberTip> {
    if !core.response.hovered() {
        return None;
    }
    let pressed = core.press.phase.is_press();
    let hash = member.index.unwrap_or(0);
    let part = decls.iter().find_map(|(decl, extras)| {
        let result = core.results.get(&(decl.key, decl.index)).filter(|result| result.hovered)?;
        let tip = extras.tooltip.as_ref()?;
        let index = decl.index.map_or(hash, |index| hash ^ index.rotate_left(32));
        Some(crate::contract::tooltip::MemberTip { part: decl.key.at(index), spec: tip.spec().clone(), rect: result.rect, pressed })
    });
    part.or_else(|| own.map(|tip| crate::contract::tooltip::MemberTip { part: member, spec: tip.spec().clone(), rect: row, pressed }))
}
