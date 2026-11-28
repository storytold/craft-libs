//! `CanvasSpace`: the canvas view a member of a canvas body reads, the rulers' units and zero, the presses on the
//! canvas background, and the text of ruler units.
//!
//! A `Canvas` given a view publishes its [`CanvasSpace`] to its body through `Component::content_env`. A canvas
//! control reads it with [`canvas_space`]; the canvas's tool reads it with `ToolContext::space`, handed over by
//! `CanvasOutput::space`.

use super::PasteboardColor;
use crate::contract::{Component, Cx, Focus as _, Pointer as _};
use crate::primitives::PaintSlot;
use crate::runtime::env::{AppValues, EnvValue, EnvView, UnitKind, Units};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CanvasView, Intent, MoveMods, NavMove, PartRef, PointerButton, PressPhase, PressState, RulerUnits, Ticks, ZoomScheme};
use craft_keymap::Mods;
use egui::{Pos2, Vec2};
use std::ops::RangeInclusive;

/// How a canvas shows its document, as its body and its tool see it: the view, and the interface scale it is
/// shown at.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CanvasSpace {
    /// The view on screen: document to the canvas root's screen space, part way to [`target`](Self::target) while a
    /// zoom eases, and [`pixel_aligned`].
    pub view: CanvasView,
    /// Device pixels per point.
    pub pixels_per_point: f32,
    target: CanvasView,
    easing: bool,
    follows_pointer: bool,
    keys: BodyKeys,
    document: Option<Size>,
    rulers: Option<CanvasRulers>,
    background: BackgroundPress,
    /// The canvas's id, under which its controls claim keys on the focused canvas.
    canvas: Option<egui::Id>,
    pasteboard: PasteboardColor,
    precise: bool,
    zoom_scheme: ZoomScheme,
    space_held: bool,
    /// The canvas's picture slot for this pass, which [`paint_picture`](Self::paint_picture) appends to; `None` in a
    /// measuring pass.
    picture: Option<PaintSlot>,
    wheel: Option<craft_behavior::Wheel>,
    zoom_anchor: Option<Pos2>,
}

impl CanvasSpace {
    /// The space of `view` at `pixels_per_point` device pixels per point, with no rulers published and no press on
    /// the background.
    pub fn new(view: CanvasView, pixels_per_point: f32) -> Self {
        Self {
            view,
            pixels_per_point,
            target: view,
            easing: false,
            follows_pointer: false,
            keys: BodyKeys::default(),
            document: None,
            rulers: None,
            background: BackgroundPress::default(),
            canvas: None,
            pasteboard: PasteboardColor::Default,
            precise: false,
            zoom_scheme: ZoomScheme::PHOTOSHOP,
            space_held: false,
            picture: None,
            wheel: None,
            zoom_anchor: None,
        }
    }

    /// The wheel share the canvas read this pass, for a body that pages by wheel to read once.
    pub fn wheel(&self) -> Option<craft_behavior::Wheel> {
        self.wheel
    }

    /// This space with the wheel share the canvas read this pass.
    #[must_use]
    pub fn with_wheel(mut self, wheel: Option<craft_behavior::Wheel>) -> Self {
        self.wheel = wheel;
        self
    }

    /// This space for a pass in which the app's tool moves the view with the pointer
    /// ([`Canvas::view_follows_pointer`](super::Canvas::view_follows_pointer)): [`set_view`](Self::set_view) then shows
    /// a view as it is, not [`pixel_aligned`].
    #[must_use]
    pub fn with_follows_pointer(mut self, follows: bool) -> Self {
        self.follows_pointer = follows;
        self
    }

    /// Sets the view of this pass: [`target`](Self::target) is `view`, and the view on screen is `view`
    /// [`pixel_aligned`], or `view` as it is while the pass follows the pointer. The canvas sets the view it settled
    /// on, and the tool host each view a tool sets, so the calls after it start from what the canvas will show.
    pub fn set_view(&mut self, view: CanvasView) {
        self.target = view;
        self.view = self.on_screen(view);
        self.easing = false;
    }

    /// Shows `shown`, part way to the target while a zoom eases, by the same rule as [`set_view`](Self::set_view).
    pub(crate) fn ease_through(&mut self, shown: CanvasView) {
        self.view = self.on_screen(shown);
        self.easing = true;
    }

    fn on_screen(&self, view: CanvasView) -> CanvasView {
        if self.follows_pointer { view } else { pixel_aligned(view, self.pixels_per_point) }
    }

    /// This space with the precise cursor on or off for this pass ([`precise`](Self::precise)).
    #[must_use]
    pub fn with_precise(mut self, precise: bool) -> Self {
        self.precise = precise;
        self
    }

    /// Whether pointers over the canvas show their precise cursor (a crosshair) in this pass: Preferences > Cursors >
    /// Other Cursors is Precise ([`Canvas::precise_cursors`](super::Canvas::precise_cursors)), toggled by Caps Lock.
    /// The one source for the canvas's tool (`ToolContext::precise`) and its controls.
    pub fn precise(&self) -> bool {
        self.precise
    }

    /// This space with Space held or released for this pass. A `Canvas` supplies the same real and injected key
    /// state its tool reads; a space built with [`new`](Self::new) starts with Space released.
    #[must_use]
    pub fn with_space_held(mut self, held: bool) -> Self {
        self.space_held = held;
        self
    }

    /// Whether Space is held this pass, including an injected held key. A canvas control that owns a drag can
    /// read it without turning a key press into a repeat or changing who owns the pointer. This does not claim
    /// the key or start a pan; the canvas's tool host still handles momentary tools.
    pub fn space_held(&self) -> bool {
        self.space_held
    }

    /// This space with the keys the canvas left to its body in the pass.
    #[must_use]
    pub fn with_keys(mut self, keys: BodyKeys) -> Self {
        self.keys = keys;
        self
    }

    /// This space with the size of the document the canvas shows, in document units: the one size the zoom ladder's
    /// lower limit and the zoom tools read.
    #[must_use]
    pub fn with_document(mut self, document: Option<Size>) -> Self {
        self.document = document;
        self
    }

    /// This space with the rulers `rulers` published.
    #[must_use]
    pub fn with_rulers(mut self, rulers: Option<CanvasRulers>) -> Self {
        self.rulers = rulers;
        self
    }

    /// This space with the presses `background` reported for the pass.
    #[must_use]
    pub fn with_background(mut self, background: BackgroundPress) -> Self {
        self.background = background;
        self
    }

    /// This space with the pasteboard colour the canvas was given.
    #[must_use]
    pub fn with_pasteboard(mut self, color: PasteboardColor) -> Self {
        self.pasteboard = color;
        self
    }

    /// The pasteboard colour set once on the canvas: the pasteboard and the crop shield read it from the space.
    pub fn pasteboard(&self) -> PasteboardColor {
        self.pasteboard
    }

    /// Appends `shapes` to the canvas's picture slot, so they paint under every canvas control of the body, in show
    /// order with the displays that use [`RasterDisplay::picture`](super::RasterDisplay::picture). It takes the
    /// context because the space holds none. Outside a canvas's paint (a measuring pass) there is no slot, and it
    /// paints nothing.
    pub fn paint_picture(&self, ctx: &egui::Context, shapes: Vec<egui::Shape>) {
        if let Some(slot) = self.picture {
            slot.extend(ctx, shapes);
        }
    }

    /// The zoom scheme of the root env, published for the body and the tool: the rungs, range and 100 % they zoom by.
    pub fn zoom_scheme(&self) -> ZoomScheme {
        self.zoom_scheme
    }

    /// This space with the root env's zoom `scheme`.
    #[must_use]
    pub(crate) fn with_zoom_scheme(mut self, scheme: ZoomScheme) -> Self {
        self.zoom_scheme = scheme;
        self
    }

    /// This space with the canvas's picture slot of this pass.
    #[must_use]
    pub(crate) fn with_picture(mut self, slot: PaintSlot) -> Self {
        self.picture = Some(slot);
        self
    }

    /// The slot the canvas reserved this pass, `None` before its paint step ran or outside a canvas: a display that
    /// paints under the controls paints in place without one.
    pub fn picture_slot(&self) -> Option<PaintSlot> {
        self.picture
    }

    /// This space published by the canvas `id`.
    #[must_use]
    pub(crate) fn with_canvas(mut self, id: egui::Id) -> Self {
        self.canvas = Some(id);
        self
    }

    /// The id of the canvas that published this space: what a control claims keys on the focused canvas with. A body
    /// hands a control keyed by its canvas ([`Guides::new`](crate::canvas_controls::Guides::new)) the id it named the
    /// canvas with, not this one: the space exists only once the view has settled, after the body's measuring run.
    pub(crate) fn canvas(&self) -> Option<egui::Id> {
        self.canvas
    }

    /// The size of the document, in document units, as given to [`Canvas::document`](super::Canvas::document).
    pub fn document(&self) -> Option<Size> {
        self.document
    }

    /// The units and the zero point the canvas measures its document in: what [`Canvas::rulers`](super::Canvas::rulers)
    /// published, or pixels from the document's origin for a canvas that publishes none.
    pub fn rulers(&self) -> CanvasRulers {
        self.rulers.unwrap_or_default()
    }

    /// The presses on the canvas background in this pass.
    pub fn background(&self) -> BackgroundPress {
        self.background
    }

    /// The view the canvas settled on in this pass: what the app keeps, and what a body that keeps its own view state
    /// decides from. It differs from `view` while a zoom eases ([`easing`](Self::easing)) and, by less than half a
    /// device pixel, where `view` is [`pixel_aligned`].
    pub fn target(&self) -> CanvasView {
        self.target
    }

    /// Whether a zoom eases in this pass, so `view` is part way to [`target`](Self::target).
    pub fn easing(&self) -> bool {
        self.easing
    }

    /// The screen point the wheel or a pinch zoomed about in this pass, in the canvas root's screen space; `None` when
    /// neither zoomed (a view command zooms about the centre). A body that keeps its own view state anchors its zoom
    /// here: the canvas clamps the zoomed view to the document after, which can move the centre (a document no wider
    /// than the view stays centred), so the change of centre no longer tells which point stayed still.
    pub fn zoom_anchor(&self) -> Option<Pos2> {
        self.zoom_anchor
    }

    /// This space with the point a zoom of this pass kept still ([`zoom_anchor`](Self::zoom_anchor)).
    #[must_use]
    pub fn with_zoom_anchor(mut self, anchor: Option<Pos2>) -> Self {
        self.zoom_anchor = anchor;
        self
    }

    /// The page keys the canvas left to its body in this pass ([`Canvas::keys_to_body`](super::Canvas)).
    pub fn keys(&self) -> BodyKeys {
        self.keys
    }

    /// Takes this pass's arrow that nothing in the canvas's body took: the one routed to the focused canvas, or, with
    /// nothing in the window focused, the window's. A body that pages its document itself
    /// ([`Canvas::keys_to_body`](super::Canvas)) runs its tool host and controls first, which take the arrows a tool
    /// or a session claims, and moves its view with what is left in the same pass. `None` when the arrow was taken,
    /// or there was none; a measuring run reads it without taking it.
    pub fn take_nudge(&self, ctx: &egui::Context) -> Option<(Vec2, Mods)> {
        let canvas = self.canvas?;
        let mut own = crate::runtime::keys::intents_of(ctx, Some(canvas), false);
        if let Some(nudge) = own.take_nudge() {
            crate::runtime::keys::settle(ctx, Some(canvas), &own);
            return Some(nudge);
        }
        let mut nudge = None;
        crate::runtime::keys::with_canvas_window_intents(ctx, false, |window| {
            nudge = window.take_nudge();
            nudge.is_some()
        });
        nudge
    }

    /// Device pixels per document unit (1 is 100 %) of the view at this scale.
    pub fn zoom_device(&self) -> f64 {
        zoom::device(&self.view, self.pixels_per_point)
    }
}

/// `view` as the canvas shows it: at a whole number of device pixels per document unit on both axes, with no rotation,
/// moved by less than half a device pixel so the document's pixel edges land on device pixel edges. Every other view is
/// shown as it is. A canvas gives its displays, overlays and tools this view as [`CanvasSpace::view`]; the view the app
/// keeps stays as it was set, so the alignment never adds up into a pan. An app that maps through a view outside the
/// canvas's pass maps through this one, to land where the canvas will draw.
pub fn pixel_aligned(view: CanvasView, pixels_per_point: f32) -> CanvasView {
    let ppp = f64::from(pixels_per_point);
    if !(ppp.is_finite() && ppp > 0.0 && view.zoom.is_finite() && view.zoom > 0.0) || view.rotation.abs().is_nan() || view.rotation.abs() >= 1e-8 {
        return view;
    }
    let whole = |scale: f64| scale.is_finite() && scale.round() >= 1.0 && (scale - scale.round()).abs() <= 1e-5;
    if !(whole(view.zoom_x() * ppp) && whole(view.zoom * ppp)) {
        return view;
    }
    let origin = view.affine() * Point::ZERO;
    let aligned = Point::new((origin.x * ppp).round() / ppp, (origin.y * ppp).round() / ppp);
    let shift = aligned - origin;
    let center = view.center - view.unmap_vec(Vec2::new(shift.x as f32, shift.y as f32));
    let mut shown = view;
    if center.x.is_finite() && center.y.is_finite() {
        shown.center = center;
    }
    shown
}

/// The units and the zero point a canvas measures its document in: the one place the rulers, the guides and the
/// measures read them from.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CanvasRulers {
    /// The x axis' ruler (the top ruler, and the position of a vertical guide).
    pub x: RulerText,
    /// The y axis' ruler (the left ruler, and the position of a horizontal guide).
    pub y: RulerText,
    /// The document point the rulers count from.
    pub zero: Point,
}

impl CanvasRulers {
    /// Rulers measuring the x axis in `x` and the y axis in `y`, counted from `zero` (document units).
    pub const fn new(x: RulerText, y: RulerText, zero: Point) -> Self {
        Self { x, y, zero }
    }

    /// The ruler a position across a guide running `along` is written in, and the zero it counts from: the x ruler
    /// for a vertical guide (along `Axis::Y`).
    pub fn across(&self, along: Axis) -> (RulerText, f64) {
        match along {
            Axis::X => (self.y, self.zero.y),
            _ => (self.x, self.zero.x),
        }
    }
}

impl Default for CanvasRulers {
    /// Pixels on both axes, from the document's origin.
    fn default() -> Self {
        let pixels = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
        Self::new(pixels, pixels, Point::ZERO)
    }
}

/// The presses that reached the canvas background in a pass: ones no canvas control claimed. A middle-button press
/// pans and is never one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BackgroundPress {
    /// The button that went down on the background in this pass: a control that keeps a selection clears it.
    pub pressed: Option<PointerButton>,
    /// Where the secondary button was released on the background without a drag: a control that offers a
    /// canvas menu opens it there.
    pub context: Option<Pos2>,
    /// The Menu key or Shift+F10 asked the focused canvas for its menu (the canvas leaves the request untaken): a
    /// control that offers a canvas menu opens it at the view's centre.
    pub context_key: bool,
}

/// The page keys a canvas that leaves them to its body took in a pass: a key held down repeats over passes, not
/// within one. The arrows stay with the body's tool host and controls first; the body takes what they left with
/// [`CanvasSpace::take_nudge`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BodyKeys {
    /// Page Up, Page Down, Home or End, with the modifiers held.
    pub page: Option<(NavMove, MoveMods)>,
}

impl BackgroundPress {
    /// What `press`, the press on the background part after the middle button's pan is taken out, and the canvas's
    /// keyed menu request report.
    pub(super) fn of(press: &PressState, context_key: bool) -> Self {
        let pressed = match press.phase {
            PressPhase::Down { start, .. } => Some(start.button),
            _ => None,
        };
        Self { pressed, context: context_click(press), context_key }
    }
}

/// Where `press` is the release of a secondary press: the one reading of a context press from a part's press. The press
/// machine gives a secondary press only to a part that senses `CONTEXT_CLICK`, so the release is the request whether
/// or not the pointer moved. A secondary press made during another open press (a right-click during a drag) comes up
/// beside that press's phase, as `PressState::context`, and is the request too.
pub fn context_click(press: &PressState) -> Option<Pos2> {
    match press.phase {
        PressPhase::Release { start, pos, .. } if start.button == PointerButton::Secondary => Some(pos),
        _ => press.context,
    }
}

/// Where `part` of the component was asked for its context menu in this pass: a secondary click on it, or the Menu key
/// or Shift+F10 while it holds the focus (then its bottom start corner). The intent is taken, so the component's
/// generic context request is not recorded.
pub(crate) fn context_menu<C: Component>(cx: &mut Cx<'_, C>, part: impl Into<PartRef>) -> Option<Pos2> {
    let part = part.into();
    let result = cx.part_if_declared(part)?;
    let (press, rect) = (result.press, result.rect);
    if let Some(at) = context_click(&press) {
        return Some(at);
    }
    (cx.active_part() == Some(part) && cx.intents().take(Intent::ContextMenu)).then(|| rect.left_bottom())
}

/// The env value that carries the space of the innermost canvas; `None` outside a canvas and inside a canvas that
/// has no view.
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
#[memo_props(every_pass = "a context the canvas rebuilds each pass")]
struct InCanvas(Option<CanvasSpace>);

impl EnvValue for InCanvas {
    fn root() -> Self {
        InCanvas(None)
    }
}

/// The env value that carries the guide a drag out of the canvas's rulers is placing in this pass: a view under the
/// canvas runs in every pass of the drag (`GuideDrag` keys in every pass), and replays when no drag is placing one.
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
struct InRulerGuide(Option<super::GuideDrag>);

impl EnvValue for InRulerGuide {
    fn root() -> Self {
        InRulerGuide(None)
    }
}

/// Publishes the ruler guide of this pass (or none) to a canvas's body.
pub(super) fn publish_ruler_guide(values: &mut AppValues, guide: Option<super::GuideDrag>) {
    values.set(InRulerGuide(guide));
}

/// The guide a drag out of the rulers of the innermost `Canvas` around the component is placing in this pass, as the
/// rulers report it ([`RulersOutput::guide`](super::RulersOutput::guide)); `Guides` shows and commits it. `None`
/// outside a canvas, when the canvas shows no rulers, and while no drag runs.
pub fn ruler_guide(env: &EnvView<'_>) -> Option<super::GuideDrag> {
    env.get::<InRulerGuide>().0
}

/// Publishes `space` (or none) to a canvas's body.
pub(super) fn publish(values: &mut AppValues, space: Option<CanvasSpace>) {
    values.set(InCanvas(space));
}

/// The space of the innermost `Canvas` around the component that has a view, or `None` outside one.
///
/// A canvas control registers its parts in the canvas root's screen space: it maps document geometry with
/// `space.view.to_screen` (and screen positions back with `to_doc`) and shows itself at the canvas rect, so it works
/// under any rotation, flip or aspect. The body's layer transform exists only for an unrotated, unmirrored,
/// square view (`CanvasView::layer_transform`), for bodies of node-graph components that keep canvas-space geometry;
/// a control that registers in screen space must not also read through it.
pub fn canvas_space(env: &EnvView<'_>) -> Option<CanvasSpace> {
    env.get::<InCanvas>().0
}

/// Runs `body` with `space` as the canvas space inherited by its controls.
pub fn with_canvas_space<R>(ui: &mut egui::Ui, space: CanvasSpace, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    crate::runtime::env::with_env(
        ui,
        |overrides| {
            overrides.values.set(InCanvas(Some(space)));
        },
        body,
    )
}

/// The text of one axis' ruler units: its tick maths with the unit kind it is written in, over `Units` (the one
/// number formatter of the env).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RulerText {
    /// The ticks and the size of the unit.
    pub units: RulerUnits,
    /// What the unit counts, which gives its spelling and default decimals.
    pub kind: UnitKind,
}

impl PartialEq for RulerText {
    fn eq(&self, other: &Self) -> bool {
        let same_ticks = match (self.units.ticks, other.units.ticks) {
            (Ticks::Decimal { whole: a, .. }, Ticks::Decimal { whole: b, .. }) => a == b,
            (Ticks::Custom(a), Ticks::Custom(b)) => std::ptr::fn_addr_eq(a, b),
            _ => false,
        };
        self.kind == other.kind && self.units.per_doc == other.units.per_doc && same_ticks && self.units.labels == other.units.labels
    }
}

impl RulerText {
    /// Ruler `units` written as `kind`.
    pub const fn new(units: RulerUnits, kind: UnitKind) -> Self {
        Self { units, kind }
    }

    /// The distance of `position` (document units) from `zero`, with the unit and between the least and most digits
    /// of `decimals`: `103.5 px`.
    pub fn format(&self, numbers: &Units, position: f64, zero: f64, decimals: RangeInclusive<usize>) -> String {
        numbers.text(self.kind, self.units.distance(position, zero), decimals)
    }

    /// [`format`](Self::format) for a readout or a value: at least one digit after the decimal mark, so pixels show
    /// tenths (PhotoCraft's `X: 103.2 px`), other units at the digits of their kind.
    pub fn format_value(&self, numbers: &Units, position: f64, zero: f64) -> String {
        let digits = (*self.kind.decimals().end()).max(1);
        self.format(numbers, position, zero, digits..=digits)
    }

    /// The number a ruler prints at the tick `value` (ruler units from the zero point) of labelled step `step`, with
    /// no unit, as the units' [`TickLabels`](craft_behavior::TickLabels) write it.
    pub fn tick_label(&self, numbers: &Units, value: f64, step: f64) -> String {
        numbers.number_ungrouped(self.units.label_value(value, step), self.units.label_decimals(step))
    }

    /// The unit's short name: `px`.
    pub fn unit_name(&self, numbers: &Units) -> String {
        numbers.symbol(self.kind)
    }
}
