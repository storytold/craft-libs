//! `CanvasSpace`: the canvas view a member of a canvas body reads, the rulers' units and zero, the presses on the
//! canvas background, and the text of ruler units.
//!
//! A `Canvas` given a view publishes its [`CanvasSpace`] to its body through `Component::content_env`. A canvas
//! control reads it with [`canvas_space`]; the canvas's tool reads it with `ToolContext::space`, handed over by
//! `CanvasOutput::space`.

use crate::contract::{Component, Cx, Focus as _, Pointer as _};
use crate::runtime::env::{AppValues, EnvValue, EnvView, UnitKind, Units};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CanvasView, Intent, MoveMods, NavMove, PartRef, PointerButton, PressPhase, PressState, RulerUnits, Ticks};
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
        }
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

    /// This space published by the canvas `id`.
    #[must_use]
    pub(crate) fn with_canvas(mut self, id: egui::Id) -> Self {
        self.canvas = Some(id);
        self
    }

    /// The id of the canvas that published this space, for a control that claims keys on the focused canvas.
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

    /// The page and arrow keys the canvas left to its body in this pass ([`Canvas::keys_to_body`](super::Canvas)).
    pub fn keys(&self) -> BodyKeys {
        self.keys
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

/// The keys a canvas that leaves them to its body took in a pass, one of each kind: a key held down repeats over
/// passes, not within one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BodyKeys {
    /// Page Up, Page Down, Home or End, with the modifiers held.
    pub page: Option<(NavMove, MoveMods)>,
    /// An arrow: a unit direction on screen and the modifiers held.
    pub nudge: Option<(Vec2, Mods)>,
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
/// machine captures a secondary press only for a part that senses a drag or a `CONTEXT_CLICK`, so the release is the
/// request whether or not the pointer moved.
pub fn context_click(press: &PressState) -> Option<Pos2> {
    match press.phase {
        PressPhase::Release { start, pos, .. } if start.button == PointerButton::Secondary => Some(pos),
        _ => None,
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct InCanvas(Option<CanvasSpace>);

impl EnvValue for InCanvas {
    fn root() -> Self {
        InCanvas(None)
    }
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
        self.kind == other.kind && self.units.per_doc == other.units.per_doc && same_ticks
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
    /// no unit.
    pub fn tick_label(&self, numbers: &Units, value: f64, step: f64) -> String {
        let digits = self.units.label_decimals(step);
        numbers.number_ungrouped(self.units.label_value(value, step), digits..=digits)
    }

    /// The unit's short name: `px`.
    pub fn unit_name(&self, numbers: &Units) -> String {
        numbers.symbol(self.kind)
    }
}
