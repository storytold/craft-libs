//! [`ColorWheel`]: a hue ring round a saturation and brightness triangle that turns with the hue, as PhotoCraft's
//! Color Wheel.

use super::area::{AreaShape, ColorArea};
use super::channels::{channel_step, finite, primary, settled, spoken};
use super::disc::nearest_within;
use super::ramp::Gamut;
use crate::AreaMode;
use craft_behavior::nav::{NavShape, Roving};
use craft_behavior::{HitLevel, HitPriority, HitShape, Metrics, Mixed, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{ColorModel, HsbModel, ModelValues, Srgba};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, Outcome,
    PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{Marker, MarkerSize, PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, MarkerLook, StyleColor, StyleCx};
use egui::{Color32, Mesh, Pos2, Rect, Response, Vec2, vec2};
use std::f32::consts::{SQRT_2, TAU};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The hue ring: a press on it sets the hue to the pointer's angle.
const RING: PartKey = PartKey::new("ring");
/// The triangle: a press on it sets saturation and brightness. With a square inside the ring it is the catch area
/// round the square: a press there sets the square's closest point.
const TRIANGLE: PartKey = PartKey::new("triangle");
/// A marker on the ring: `MARKER.at(i)`, a member of the markers' roving group.
const MARKER: PartKey = PartKey::new("marker");

/// Steps round the hue ring's mesh (PhotoCraft's 96).
const RING_STEPS: u32 = 96;
/// Cells along each side of the triangle's mesh (PhotoCraft's 16).
const TRIANGLE_CELLS: usize = 16;
/// The ring's width as a proportion of the outer radius.
const RING_RATIO: f32 = 0.09;
/// Degrees a Page key moves the hue.
const HUE_PAGE: f32 = 36.0;
/// Percents a Page key moves the brightness.
const BRIGHTNESS_PAGE: f32 = 10.0;
/// A Shift arrow moves ten steps.
const FAR: f32 = 10.0;
/// The last hue a key reaches: the hue wraps at 360 and has no 360 of its own.
const HUE_LAST: f32 = 359.0;
/// Degrees a ring drag with Shift snaps the hue to.
const HUE_SNAP: f32 = 45.0;

craft_ui::words! {
    /// The words of colour wheels.
    pub struct ColorWheelWords {
        hue: "colorWheel.hue" = "Hue";
        triangle: "colorWheel.saturationBrightness" = "Saturation and brightness";
        saturation: "colorWheel.saturation" = "Saturation";
        brightness: "colorWheel.brightness" = "Brightness";
    }
}

/// The hue ring and the triangle: where a press began decides which one a drag edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WheelPart {
    /// The ring: a press or drag sets the hue to the pointer's angle.
    Ring,
    /// The triangle: a press or drag sets saturation and brightness.
    Triangle,
}

/// What a [`ColorWheel`] shows inside its ring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WheelInner {
    /// The saturation and brightness triangle.
    #[default]
    Triangle,
    /// The largest square inside the ring, a `ColorArea` over the same values.
    Square,
}

/// What a [`ColorWheel`] returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorWheelOutput {
    /// The edit of the values, carrying the ring's and the adopted square's edits.
    pub edit: EditResponse<ModelValues>,
    /// The marker a click picked.
    pub picked: Option<usize>,
    /// The marker a press is holding.
    pub held: Option<usize>,
}

impl Outcome for ColorWheelOutput {
    type Value = ModelValues;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// Where a wheel lies in its area: pure numbers, shared with the colour disc.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelGeometry {
    /// Centre of the wheel.
    pub center: Pos2,
    /// Radius of the ring's inner edge.
    pub inner: f32,
    /// Radius of the ring's outer edge.
    pub outer: f32,
    /// Radius of the circle round the triangle's corners.
    pub triangle: f32,
}

impl WheelGeometry {
    /// The wheel in `area`: the outer radius is half the shorter side less `outer_inset`; the
    /// ring is `ratio` of it wide, between `ring_min` and `ring_max` and at most half the
    /// radius; the triangle's circle lies `triangle_inset` inside the ring.
    pub fn new(area: Rect, outer_inset: f32, ring: (f32, f32), ratio: f32, triangle_inset: f32) -> Self {
        let outer = (area.width().min(area.height()) / 2.0 - outer_inset).max(1.0);
        let width = (outer * ratio).max(ring.0).min(ring.1).min(outer * 0.5);
        let inner = outer - width;
        Self { center: area.center(), inner, outer, triangle: (inner - triangle_inset).max(0.0) }
    }

    /// The largest square inside the ring, a `space.gap.tight` inside its inner edge: where `WheelInner::Square` lies.
    /// The square's corners lie on the circle `triangle` is the radius of, which the style insets by that gap.
    pub fn square(&self) -> Rect {
        Rect::from_center_size(self.center, Vec2::splat(self.triangle * SQRT_2))
    }

    /// The part a press at `pos` takes: nothing beyond the ring's outer edge plus `grab_outer`,
    /// the ring from `grab_inner` inside its inner edge, the triangle within.
    pub fn part_at(&self, pos: Pos2, grab_outer: f32, grab_inner: f32) -> Option<WheelPart> {
        let d = pos.distance(self.center);
        if !d.is_finite() || d > self.outer + grab_outer {
            None
        } else if d >= self.inner - grab_inner {
            Some(WheelPart::Ring)
        } else {
            Some(WheelPart::Triangle)
        }
    }

    /// The middle of the ring at `hue` degrees: where its knob sits.
    pub fn ring_knob(&self, hue: f32) -> Pos2 {
        self.center + hue_direction(hue) * ((self.inner + self.outer) / 2.0)
    }

    /// The triangle's corners at `hue`, as `triangle_corners`, grown by `by` points.
    pub fn corners(&self, hue: f32, by: f32) -> [Pos2; 3] {
        triangle_corners(self.center, self.triangle + by, hue)
    }
}

/// The unit vector on screen at `hue` degrees: red (0) to the right, growing counter-clockwise.
pub fn hue_direction(hue: f32) -> Vec2 {
    let a = hue.to_radians();
    vec2(a.cos(), -a.sin())
}

/// The hue in whole degrees, 0 to 359, of the direction from `center` to `p`; `None` for a point
/// that isn't finite.
pub fn hue_at(center: Pos2, p: Pos2) -> Option<f32> {
    let a = (center.y - p.y).atan2(p.x - center.x);
    a.is_finite().then(|| (a.rem_euclid(TAU).to_degrees().round()).rem_euclid(360.0))
}

/// The triangle's corners for `hue` degrees inside radius `r`: the pure hue at the hue's angle,
/// white 120 degrees on and black 240 degrees on.
pub fn triangle_corners(center: Pos2, r: f32, hue: f32) -> [Pos2; 3] {
    [0.0, 120.0, 240.0].map(|turn| center + hue_direction(hue + turn) * r)
}

/// The point of saturation `s` and brightness `b` (0 to 1) in the triangle: black + b (white -
/// black) + b s (hue - white).
pub fn triangle_point([hue, white, black]: [Pos2; 3], s: f32, b: f32) -> Pos2 {
    black + (white - black) * b + (hue - white) * (b * s)
}

/// Saturation and brightness (0 to 1) at `p`, kept inside the triangle; `None` for a degenerate
/// triangle or a point that isn't finite.
pub fn triangle_sv([hue, white, black]: [Pos2; 3], p: Pos2) -> Option<(f32, f32)> {
    let (a, c, d) = (white - black, hue - white, p - black);
    let det = a.x * c.y - a.y * c.x;
    if det.abs() < 1e-6 {
        return None;
    }
    let (b, bs) = ((d.x * c.y - d.y * c.x) / det, (a.x * d.y - a.y * d.x) / det);
    if !b.is_finite() || !bs.is_finite() {
        return None;
    }
    let b = b.clamp(0.0, 1.0);
    let bs = bs.clamp(0.0, b);
    Some((if b > 0.0 { bs / b } else { 0.0 }, b))
}

/// The style of a colour wheel.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorWheelStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The least ring width, and the largest.
    ring_min: f32,
    ring_max: f32,
    /// The gap between the area's edge and the ring.
    outer_inset: f32,
    /// The gap between the ring's inner edge and the circle round the triangle's corners or the square's.
    triangle_inset: f32,
    /// A press this far outside the ring still takes it, and this far inside its inner edge.
    grab: f32,
    /// The edge round the ring and the triangle.
    edge: StyleColor,
    /// The knobs.
    knob: MarkerLook,
    /// The least side of the picture, and the most it takes of the width it is given.
    least: f32,
    most: f32,
}

/// A hue ring round a saturation and brightness triangle, editing a colour's HSB values.
///
/// The host gives the colour as HSB [`ModelValues`] and gets them back with only the picked components changed: the
/// ring changes the hue, the triangle saturation and brightness. A host that keeps the values it was sent keeps the
/// hue of a grey and the hue and saturation of black.
///
/// - Pointer (primary button): the part under the press is kept for the whole drag, so a drag that leaves the ring
///   keeps turning the hue; a press further outside the ring than `size.hit.target` starts nothing. A press sets the
///   value under the pointer (whole degrees, whole percents) and a drag follows it; Esc cancels. A press focuses the
///   part. A ring drag with Shift snaps the hue to 45 degrees.
/// - Keys on the focused ring: Left and Down lower the hue by 1 degree, Right and Up raise it, wrapping at 360;
///   Shift steps ten; Page Up and Page Down 36 degrees; Home 0, End 359. On the triangle: Left and Right saturation,
///   Up and Down brightness; Shift ten; Page keys a tenth of brightness; Home and End saturation 0 and 100. The
///   directions are not screen directions: the triangle turns with the hue.
/// - Two Tab stops, the ring and then the triangle; then the markers, one roving stop, when there are any.
/// - [`WheelInner::Square`]: the largest square inside the ring is a `ColorArea` with its own stop, keys and node; a
///   press between the square and the ring sets the square's closest point.
/// - Markers: a click, Enter or Space on one reports [`picked`](ColorWheelOutput::picked); a press on one still drags
///   the ring and reports it as [`held`](ColorWheelOutput::held).
/// - AccessKit: a `Group` named `label` holding a `Slider` for the hue (0 to 360, step 1, increment, decrement and
///   set value; no numeric value while the value is mixed), a `Slider` for saturation and brightness (value text, no
///   numeric value) on the triangle, and a `Button` per marker.
/// - Right to left nothing mirrors: red stays at 3 o'clock.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorWheel<'a> {
    #[common]
    common: Common,
    values: Bind<'a, ModelValues>,
    /// The host's gamut, passed to the square's `ColorArea`.
    #[prop]
    gamut: Option<Gamut<'a>>,
    /// What the ring holds: the triangle, or a square `ColorArea`.
    #[prop]
    #[style_key]
    inner: WheelInner,
    /// Markers on the ring, one roving Tab stop after it; a click, Enter or Space picks one.
    #[prop]
    markers: &'a [Srgba],
    /// The markers' accessible names, in order.
    #[prop(into)]
    marker_names: Vec<Label>,
    picked: Option<usize>,
    held: Option<usize>,
}

impl<'a> ColorWheel<'a> {
    /// A wheel named `label` ("Color wheel") showing `values`, HSB values.
    pub fn new(label: impl Into<Label>, values: Bind<'a, ModelValues>) -> Self {
        ColorWheel {
            common: Common::new(),
            values,
            gamut: None,
            inner: WheelInner::Triangle,
            markers: &[],
            marker_names: Vec::new(),
            picked: None,
            held: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// Where the wheel lies in `area`.
    pub fn geometry(style: &ColorWheelStyle, area: Rect) -> WheelGeometry {
        WheelGeometry::new(area, style.outer_inset, (style.ring_min, style.ring_max), RING_RATIO, style.triangle_inset)
    }

    /// Where each marker lies on the ring: at its colour's hue.
    fn places(&self, geometry: &WheelGeometry) -> Vec<Pos2> {
        self.markers.iter().map(|color| geometry.ring_knob(finite(HsbModel.from_srgb(*color).values[0]))).collect()
    }
}

/// The marker a press holds, kept until the press ends.
#[derive(Clone, Copy, Default)]
struct Held {
    marker: Option<usize>,
}

/// The HSB values the wheel shows when the host gives none.
fn blank() -> ModelValues {
    ModelValues::new(HsbModel.id(), &[])
}

/// The square's area over `values`: the one point-to-value mapping of the square, for the child and for a press
/// between it and the ring.
fn square_area(label: impl Into<Label>, values: impl Into<Mixed<ModelValues>>) -> ColorArea<'static> {
    ColorArea::new(label, AreaMode::Hue, Bind::given(values)).shape(AreaShape::Square)
}

/// `hue` rounded to the nearest multiple of [`HUE_SNAP`] degrees.
fn snapped(hue: f32) -> f32 {
    ((hue / HUE_SNAP).round() * HUE_SNAP).rem_euclid(360.0)
}

/// `base` with what a press at `pos` in `part` picks: the hue for the ring (to a multiple of 45 degrees when `snap`),
/// saturation and brightness for the triangle (turned to `base`'s hue) or the square's closest point; `base` when
/// `pos` picks nothing.
fn value_at(geometry: &WheelGeometry, base: &ModelValues, part: WheelPart, inner: WheelInner, pos: Pos2, snap: bool) -> ModelValues {
    let mut out = *base;
    match (part, inner) {
        (WheelPart::Ring, _) => {
            if let Some(hue) = hue_at(geometry.center, pos) {
                out.values[0] = if snap { snapped(hue) } else { hue };
            }
        }
        (WheelPart::Triangle, WheelInner::Triangle) => {
            if let Some((s, b)) = triangle_sv(geometry.corners(finite(base.values[0]), 0.0), pos) {
                out.values[1] = (s * 100.0).round();
                out.values[2] = (b * 100.0).round();
            }
        }
        (WheelPart::Triangle, WheelInner::Square) => out = square_area("", *base).value_at(geometry.square(), base, pos),
    }
    out
}

/// `values` after the key `mv` on the focused `part`.
fn keyed(values: &ModelValues, part: WheelPart, mv: NavMove, far: bool) -> Option<ModelValues> {
    let by = if far { FAR } else { 1.0 };
    let mut out = *values;
    match part {
        WheelPart::Ring => {
            let hue = finite(values.values[0]);
            let next = match mv {
                NavMove::Forward => hue + by,
                NavMove::Back => hue - by,
                NavMove::Up => hue + by,
                NavMove::Down => hue - by,
                NavMove::PageUp => hue + HUE_PAGE,
                NavMove::PageDown => hue - HUE_PAGE,
                NavMove::Start => 0.0,
                NavMove::End => HUE_LAST,
                _ => return None,
            };
            out.values[0] = next.round().rem_euclid(360.0);
        }
        WheelPart::Triangle => {
            let step = channel_step(&HsbModel, 1) as f32;
            let (s, b) = (finite(values.values[1]), finite(values.values[2]));
            let (s, b) = match mv {
                NavMove::Forward => (s + by * step, b),
                NavMove::Back => (s - by * step, b),
                NavMove::Up => (s, b + by * step),
                NavMove::Down => (s, b - by * step),
                NavMove::PageUp => (s, b + BRIGHTNESS_PAGE),
                NavMove::PageDown => (s, b - BRIGHTNESS_PAGE),
                NavMove::Start => (0.0, b),
                NavMove::End => (100.0, b),
                _ => return None,
            };
            out.values[1] = settled(&HsbModel, 1, s);
            out.values[2] = settled(&HsbModel, 2, b);
        }
    }
    Some(out)
}

impl Component for ColorWheel<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Both);
    const OVERLAYS: bool = false;
    type Style = ColorWheelStyle;
    type Output = ColorWheelOutput;

    fn style(cx: &StyleCx<'_>, key: ColorWheelStyleKey) -> ColorWheelStyle {
        let tokens = cx.tokens;
        ColorWheelStyle {
            metrics: Metrics::new(),
            least: tokens.size.plot.height,
            most: tokens.size.plot.width,
            focus: cx.focus_look(),
            ring_min: tokens.size.icon.sm,
            ring_max: tokens.size.control.md,
            outer_inset: tokens.space.gap.tight,
            triangle_inset: match key.inner {
                WheelInner::Triangle => tokens.stroke.thin,
                WheelInner::Square => tokens.space.gap.tight,
            },
            grab: tokens.size.hit.target,
            edge: cx.color(tokens.color.line.control_edge),
            knob: cx.marker(),
        }
    }

    /// A band across the width it is given, as tall as the square that fits it: the picture is centred in it.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let most = cx.style.most.max(cx.style.least);
        cx.picture_size(cx.style.least, f32::INFINITY, |width| width.min(most))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, rect);
        let reach = cx.style.grab;
        let hue = finite(cx.value().copied().unwrap_or_else(blank).values[0]);
        // The ring takes every press within its reach; the triangle takes the circle inside the ring's reach.
        cx.part(RING, HitShape::circle(geometry.center, geometry.outer + reach))
            .sense(Sense::DRAG | Sense::SETS_ON_PRESS | Sense::CLICK)
            .focusable()
            .focus_shape(FocusShape::Circle);
        let triangle = cx
            .part(TRIANGLE, HitShape::circle(geometry.center, (geometry.inner - reach).max(0.0)))
            .sense(Sense::DRAG | Sense::SETS_ON_PRESS | Sense::CLICK)
            .priority(HitPriority::new(HitLevel::Handle, 0));
        // The square is a child with its own stop; only the triangle is a stop of the wheel's.
        if self.inner == WheelInner::Triangle {
            // The triangle turns with the hue, so its ring is rebuilt each pass.
            triangle.focusable().focus_shape(FocusShape::Polygon(geometry.corners(hue, 0.0).to_vec()));
        } else {
            triangle.pointer_only("the square's own stop takes the keys");
        }
        let marker_reach = Marker::reach(&cx.style.knob, MarkerSize::Regular);
        for (index, place) in self.places(&geometry).into_iter().enumerate() {
            // A marker takes no press of its own: the ring takes it and drags on.
            let part = cx.part(MARKER.at(index as u64), HitShape::circle(place, marker_reach)).focusable().focus_shape(FocusShape::Circle);
            if index == 0 {
                part.selected();
            }
        }
        if !self.markers.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, ModelValues>> {
        Some(self.values.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, cx.rect);
        let inner = self.inner;
        let marker_reach = Marker::reach(&cx.style.knob, MarkerSize::Regular);
        let places = self.places(&geometry);
        let current = |cx: &Cx<'_, Self>| cx.value().copied().unwrap_or_else(blank);
        for (key, part) in [(RING, WheelPart::Ring), (TRIANGLE, WheelPart::Triangle)] {
            let press = cx.part(key).press;
            if !primary(&press) {
                continue;
            }
            if matches!(press.phase, PressPhase::Down { .. }) {
                if part == WheelPart::Ring {
                    cx.memory::<Held>().marker =
                        press.phase.start().and_then(|start| nearest_within(places.iter().map(|place| place.distance(start.origin)), marker_reach));
                }
                if part == WheelPart::Ring || inner == WheelInner::Triangle {
                    cx.focus_part(key);
                }
            }
            let now = current(&*cx);
            let snap = press.mods.shift;
            cx.follow_press(key, |pos| Some(value_at(&geometry, &now, part, inner, pos, snap)));
        }
        let held = cx.memory::<Held>().marker;
        self.held = held;
        let ring = cx.part(RING).press;
        if primary(&ring) && matches!(ring.phase, PressPhase::Release { click: true, .. }) {
            self.picked = held;
        }
        if matches!(ring.phase, PressPhase::Release { .. } | PressPhase::Cancel { .. }) {
            *cx.memory::<Held>() = Held::default();
        }
        for index in 0..self.markers.len() {
            if matches!(cx.part_activated(MARKER.at(index as u64)), Some(FireSource::Key | FireSource::Access)) {
                self.picked = Some(index);
            }
        }
        let running = cx.gesture().is_some();
        let focused = match cx.active_part() {
            Some(active) if active == RING.into() => Some(WheelPart::Ring),
            Some(active) if active == TRIANGLE.into() => Some(WheelPart::Triangle),
            _ => None,
        };
        let mut now = current(&*cx);
        // The moves are the focused part's; with a marker focused they are the markers' group's.
        if let Some(part) = focused {
            while let Some((mv, mods)) = cx.intents().take_move() {
                if running {
                    continue;
                }
                if let Some(next) = keyed(&now, part, mv, mods.extend) {
                    now = next;
                    cx.set(now);
                }
            }
        }
        if let Some(value) = cx.intents().take_set_value_at(RING).filter(|value| value.is_finite() && !running) {
            now.values[0] = value.round().rem_euclid(360.0) as f32;
            cx.set(now);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ColorWheelWords>();
        // A mixed value has no number to speak.
        let shown = cx.value().copied();
        let ring = access.node(RING, Role::SLIDER);
        ring.name = Name::Text(words.hue.as_str().to_owned());
        let hue = match shown {
            Some(values) => Numeric::new(f64::from(finite(values.values[0]).rem_euclid(360.0)), 0.0..=360.0),
            None => Numeric::empty(0.0..=360.0),
        };
        ring.numeric = Some(hue.step(1.0).wraps(true));
        if self.inner == WheelInner::Triangle {
            let triangle = access.node(TRIANGLE, Role::GROUP);
            triangle.name = Name::Text(words.triangle.as_str().to_owned());
            if let Some(values) = shown {
                let [saturation, brightness] = [1, 2].map(|channel| finite(values.values[channel]));
                let model = &HsbModel as &dyn ColorModel;
                triangle.value(format!(
                    "{}, {}",
                    spoken(words.saturation.as_str(), model, 1, saturation),
                    spoken(words.brightness.as_str(), model, 2, brightness)
                ));
            }
        }
        for (index, color) in self.markers.iter().enumerate() {
            let node = access.node(MARKER.at(index as u64), Role::BUTTON);
            match self.marker_names.get(index) {
                // A named marker speaks its colour as its value; an unnamed one is named by it.
                Some(name) => {
                    node.name = Name::Text(cx.env.resolve(name).as_str().to_owned());
                    node.value(color.to_hex());
                }
                None => node.name = Name::Text(color.to_hex()),
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, cx.rect);
        let shown = cx.value().copied();
        let values = shown.unwrap_or_else(blank);
        let [hue, saturation, brightness] = [0, 1, 2].map(|channel| finite(values.values[channel]));
        let corners = geometry.corners(hue, 0.0);
        {
            let painter = ui.content_painter(cx);
            painter.painter.add(ring_mesh(&geometry));
            if self.inner == WheelInner::Triangle {
                painter.painter.add(triangle_mesh(corners, hue));
            }
        }
        if self.inner == WheelInner::Square {
            let words = cx.words::<ColorWheelWords>();
            // `Gamut` is not `Clone`: the wheel is built anew every pass, so the child takes it.
            let area = square_area(words.triangle.clone(), shown.map_or(Mixed::Mixed, Mixed::from));
            let area = match self.gamut.take() {
                Some(gamut) => area.gamut(gamut),
                None => area,
            };
            let edit = ui.child("square", geometry.square(), area);
            cx.adopt(&edit, &blank(), |point, mine| *mine = *point);
        }
        let draw = ui.draw();
        draw.stroke(&PaintShape::Circle(geometry.center, geometry.outer), StrokeWidth::Thin, cx.style.edge);
        if self.inner == WheelInner::Triangle {
            draw.stroke(&PaintShape::Polygon(corners.to_vec()), StrokeWidth::Thin, cx.style.edge);
        }
        // A mixed value draws no knob.
        if shown.is_some() {
            draw.marker(&cx.style.knob, &Marker::hollow(geometry.ring_knob(hue)));
            if self.inner == WheelInner::Triangle {
                draw.marker(&cx.style.knob, &Marker::hollow(triangle_point(corners, saturation / 100.0, brightness / 100.0)));
            }
        }
        for (place, color) in self.places(&geometry).into_iter().zip(self.markers) {
            draw.marker(&cx.style.knob, &Marker::filled(place, Color32::from(*color)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorWheelOutput {
        let (picked, held) = (self.picked, self.held);
        ColorWheelOutput { edit: cx.edit_response(), picked, held }
    }
}

/// The colour of HSB `[h, s, b]` (degrees and percents) as a vertex colour.
pub(super) fn vertex(h: f32, s: f32, b: f32) -> egui::Color32 {
    HsbModel.to_srgb(&ModelValues::new("hsb", &[h, s, b]).values).into()
}

fn ring_mesh(geometry: &WheelGeometry) -> Mesh {
    let mut mesh = Mesh::default();
    for k in 0..=RING_STEPS {
        let hue = k as f32 / RING_STEPS as f32 * 360.0;
        let color = vertex(hue, 100.0, 100.0);
        let dir = hue_direction(hue);
        mesh.colored_vertex(geometry.center + dir * geometry.inner, color);
        mesh.colored_vertex(geometry.center + dir * geometry.outer, color);
    }
    for k in 0..RING_STEPS {
        let a = k * 2;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
    mesh
}

fn triangle_mesh(corners: [Pos2; 3], hue: f32) -> Mesh {
    let n = TRIANGLE_CELLS;
    let mut mesh = Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (s, b) = (i as f32 / n as f32, j as f32 / n as f32);
            mesh.colored_vertex(triangle_point(corners, s, b), vertex(hue, s * 100.0, b * 100.0));
        }
    }
    let row = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row + 1);
            mesh.add_triangle(a, a + row + 1, a + row);
        }
    }
    mesh
}
