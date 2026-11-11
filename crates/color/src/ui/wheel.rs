//! [`ColorWheel`]: a hue ring round a saturation and brightness triangle that turns with the hue, as PhotoCraft's
//! Color Wheel.

use super::channels::{channel_step, finite, primary, settled, spoken};
use craft_behavior::{HitLevel, HitPriority, HitShape, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{ColorModel, HsbModel, ModelValues};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, Name, Numeric, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{Mesh, Pos2, Rect, Vec2, vec2};
use std::f32::consts::TAU;

#[cfg(feature = "screen-testing")]
mod stories;

/// The hue ring: a press on it sets the hue to the pointer's angle.
const RING: PartKey = PartKey::new("ring");
/// The triangle: a press on it sets saturation and brightness.
const TRIANGLE: PartKey = PartKey::new("triangle");

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
    /// The gap between the ring's inner edge and the triangle's circle.
    triangle_inset: f32,
    /// A press this far outside the ring still takes it, and this far inside its inner edge.
    grab: f32,
    /// The edge round the ring and the triangle.
    edge: StyleColor,
    /// The diameter of the knobs.
    knob: f32,
}

/// The part a press began in, for its whole drag.
#[derive(Clone, Copy, Default)]
struct Held(Option<WheelPart>);

/// A hue ring round a saturation and brightness triangle, editing a colour's HSB values.
///
/// The host gives the colour as HSB [`ModelValues`] and gets them back with only the picked components changed: the
/// ring changes the hue, the triangle saturation and brightness. A host that keeps the values it was sent keeps the
/// hue of a grey and the hue and saturation of black.
///
/// - Pointer (primary button): the part under the press is kept for the whole drag, so a drag that leaves the ring
///   keeps turning the hue; a press further outside the ring than `size.hit.target` starts nothing. A press sets the
///   value under the pointer (whole degrees, whole percents) and a drag follows it; Esc cancels. A press focuses the
///   part.
/// - Keys on the focused ring: Left and Down lower the hue by 1 degree, Right and Up raise it, wrapping at 360;
///   Shift steps ten; Page Up and Page Down 36 degrees; Home 0, End 359. On the triangle: Left and Right saturation,
///   Up and Down brightness; Shift ten; Page keys a tenth of brightness; Home and End saturation 0 and 100. The
///   directions are not screen directions: the triangle turns with the hue.
/// - Two Tab stops, the ring and then the triangle.
/// - AccessKit: a `Group` named `label` holding a `Slider` for the hue (0 to 360, step 1, increment, decrement and
///   set value) and a `Slider` for saturation and brightness (value text, no numeric value).
/// - Right to left nothing mirrors: red stays at 3 o'clock.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorWheel<'a> {
    #[common]
    common: Common,
    values: Bind<'a, ModelValues>,
}

impl<'a> ColorWheel<'a> {
    /// A wheel named `label` ("Color wheel") showing `values`, HSB values.
    pub fn new(label: impl Into<Label>, values: Bind<'a, ModelValues>) -> Self {
        ColorWheel { common: Common::new(), values }.label(label).ltr_axis()
    }

    /// Where the wheel lies in `area`.
    pub fn geometry(style: &ColorWheelStyle, area: Rect) -> WheelGeometry {
        WheelGeometry::new(area, style.outer_inset, (style.ring_min, style.ring_max), RING_RATIO, style.triangle_inset)
    }
}

/// The HSB values the wheel shows when the host gives none.
fn blank() -> ModelValues {
    ModelValues::new(HsbModel.id(), &[])
}

/// `base` with what a press at `pos` in `part` picks: the hue for the ring, saturation and brightness for the
/// triangle (turned to `base`'s hue); `base` when `pos` picks nothing.
fn value_at(geometry: &WheelGeometry, base: &ModelValues, part: WheelPart, pos: Pos2) -> ModelValues {
    let mut out = *base;
    match part {
        WheelPart::Ring => {
            if let Some(hue) = hue_at(geometry.center, pos) {
                out.values[0] = hue;
            }
        }
        WheelPart::Triangle => {
            if let Some((s, b)) = triangle_sv(geometry.corners(finite(base.values[0]), 0.0), pos) {
                out.values[1] = (s * 100.0).round();
                out.values[2] = (b * 100.0).round();
            }
        }
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
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    const OVERLAYS: bool = false;
    type Style = ColorWheelStyle;
    type Output = EditResponse<ModelValues>;

    fn style(cx: &StyleCx<'_>, _: ColorWheelStyleKey) -> ColorWheelStyle {
        let tokens = cx.tokens;
        ColorWheelStyle {
            metrics: Metrics::new().min_size(Vec2::new(tokens.size.plot.width, tokens.size.plot.height)),
            focus: cx.focus_look(),
            ring_min: tokens.size.icon.sm,
            ring_max: tokens.size.control.md,
            outer_inset: tokens.space.gap.tight,
            triangle_inset: tokens.stroke.thin,
            grab: tokens.size.hit.target,
            edge: cx.color(tokens.color.line.control_edge),
            knob: tokens.size.handle.md,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, rect);
        let reach = cx.style.grab;
        // The ring takes every press within its reach; the triangle takes the circle inside the ring's reach.
        cx.part(RING, HitShape::circle(geometry.center, geometry.outer + reach))
            .sense(Sense::DRAG | Sense::CLICK)
            .focusable()
            .focus_shape(FocusShape::Circle);
        cx.part(TRIANGLE, HitShape::circle(geometry.center, (geometry.inner - reach).max(0.0)))
            .sense(Sense::DRAG | Sense::CLICK)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .focusable()
            .focus_shape(FocusShape::Circle);
    }

    fn bind(&mut self) -> Option<BindMut<'_, ModelValues>> {
        Some(self.values.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, cx.rect);
        let current = |cx: &Cx<'_, Self>| cx.value().copied().unwrap_or_else(blank);
        for (key, part) in [(RING, WheelPart::Ring), (TRIANGLE, WheelPart::Triangle)] {
            let press = cx.part(key).press;
            if !primary(&press) {
                continue;
            }
            if matches!(press.phase, PressPhase::Down { .. }) {
                cx.focus_part(key);
            }
            match press.phase {
                PressPhase::DragStart { pos, .. } => {
                    cx.memory::<Held>().0 = Some(part);
                    cx.begin();
                    cx.propose(value_at(&geometry, &current(&*cx), part, pos));
                }
                PressPhase::Drag { pos, .. } if cx.gesture().is_some() && cx.memory::<Held>().0 == Some(part) => {
                    cx.propose(value_at(&geometry, &current(&*cx), part, pos));
                }
                PressPhase::Release { pos, click, .. } => {
                    if cx.gesture().is_some() {
                        cx.propose(value_at(&geometry, &current(&*cx), part, pos));
                        cx.commit();
                    } else if click {
                        cx.set(value_at(&geometry, &current(&*cx), part, pos));
                    }
                    cx.memory::<Held>().0 = None;
                }
                _ => {}
            }
        }
        let running = cx.gesture().is_some();
        let focused = match cx.active_part() {
            Some(active) if active == RING.into() => Some(WheelPart::Ring),
            Some(active) if active == TRIANGLE.into() => Some(WheelPart::Triangle),
            _ => None,
        };
        let mut now = current(&*cx);
        while let Some((mv, mods)) = cx.intents().take_move() {
            if let Some(part) = focused.filter(|_| !running)
                && let Some(next) = keyed(&now, part, mv, mods.extend)
            {
                now = next;
                cx.set(now);
            }
        }
        if let Some(value) = cx.intents().take_set_value_at(RING).filter(|value| value.is_finite() && !running) {
            now.values[0] = value.round().rem_euclid(360.0) as f32;
            cx.set(now);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ColorWheelWords>();
        let values = cx.value().copied().unwrap_or_else(blank);
        let [hue, saturation, brightness] = [0, 1, 2].map(|channel| finite(values.values[channel]));
        let ring = access.node(RING, Role::SLIDER);
        ring.name = Name::Text(words.hue.as_str().to_owned());
        ring.numeric = Some(Numeric::new(f64::from(hue.rem_euclid(360.0)), 0.0..=360.0).step(1.0).wraps(true));
        let model = &HsbModel as &dyn ColorModel;
        let text =
            format!("{}, {}", spoken(words.saturation.as_str(), model, 1, saturation), spoken(words.brightness.as_str(), model, 2, brightness));
        let triangle = access.node(TRIANGLE, Role::GROUP);
        triangle.name = Name::Text(words.triangle.as_str().to_owned());
        triangle.value(text);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let geometry = ColorWheel::geometry(cx.style, cx.rect);
        let values = cx.value().copied().unwrap_or_else(blank);
        let [hue, saturation, brightness] = [0, 1, 2].map(|channel| finite(values.values[channel]));
        let corners = geometry.corners(hue, 0.0);
        let painter = ui.content_painter(cx);
        painter.painter.add(ring_mesh(&geometry));
        painter.painter.add(triangle_mesh(corners, hue));
        let draw = ui.draw();
        draw.stroke(&PaintShape::Circle(geometry.center, geometry.outer), StrokeWidth::Thin, cx.style.edge);
        draw.stroke(&PaintShape::Polygon(corners.to_vec()), StrokeWidth::Thin, cx.style.edge);
        let knobs = [geometry.ring_knob(hue), triangle_point(corners, saturation / 100.0, brightness / 100.0)];
        for at in knobs {
            painter.outline(&PaintShape::Circle(at, cx.style.knob / 2.0));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<ModelValues> {
        cx.edit_response()
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
