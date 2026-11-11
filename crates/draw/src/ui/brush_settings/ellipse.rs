//! Photoshop's tip angle and roundness widget ([`TipEllipse`]) and its geometry.

use craft_behavior::{HitLevel, HitPriority, HitShape, Metrics, MoveMods, NavMove, PartKey, PressPhase, Sense};
use craft_theme::{Stateful, States};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, PaintUi, PartsCx, Pointer as _,
    Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, Mix as _, StyleColor, StyleCx, SurfaceLook};
use egui::{CornerRadius, Pos2, Rect, Vec2, vec2};

use super::words::BrushSettingsWords;

/// The arrow along the major axis, and the rest of the disc off the dots.
pub(super) const ARROW: PartKey = PartKey::new("arrow");
/// The two roundness dots on the minor axis.
const DOT: PartKey = PartKey::new("dot");

/// The disc is this many rotary-control sides across: the widget is the size of two angle dials.
const DISC_IN_DIALS: f32 = 2.0;
/// The ellipse's major radius over the disc's.
const ELLIPSE_SHARE: f32 = 0.85;
/// How much of the text colour the ellipse's fill takes over the disc.
const ELLIPSE_WASH: f32 = 0.18;
/// The points the ellipse's outline has.
const SEGMENTS: usize = 48;
/// The turn one Shift step makes, in degrees.
const COARSE_TURN: f64 = 15.0;
/// The roundness one step changes, as a fraction.
const ROUND_STEP: f64 = 0.01;
/// The least roundness.
const ROUND_MIN: f64 = 0.01;

/// A tip's angle and roundness.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TipAngle {
    /// The major axis's angle in whole degrees, counter-clockwise from the right, −180 to 180.
    pub degrees: f64,
    /// The minor axis over the major, 0.01 to 1.
    pub roundness: f64,
}

impl TipAngle {
    /// The angle `degrees` and roundness `roundness`.
    pub const fn new(degrees: f64, roundness: f64) -> Self {
        Self { degrees, roundness }
    }
}

impl Default for TipAngle {
    fn default() -> Self {
        Self { degrees: 0.0, roundness: 1.0 }
    }
}

/// What a press on the widget grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TipEllipseGrab {
    /// The arrow: a drag turns the angle.
    Angle,
    /// A roundness dot: a drag squashes the ellipse.
    Roundness,
}

/// The two roundness dots, relative to the widget's centre in points (screen y down), for an
/// ellipse of major radius `radius`: `±minor × radius × clamp(roundness, 0, 1)`, where the minor
/// axis is the major turned 90° counter-clockwise, `(−sin a, −cos a)`.
pub fn ellipse_handles(angle: TipAngle, radius: f32) -> [Vec2; 2] {
    let (sin, cos) = (angle.degrees as f32).to_radians().sin_cos();
    let minor = vec2(-sin, -cos) * radius * (angle.roundness as f32).clamp(0.0, 1.0);
    [minor, -minor]
}

/// What a press at `at` (relative to the centre) grabs: a roundness dot within `reach` of either,
/// else the arrow.
pub fn ellipse_grab(at: Vec2, angle: TipAngle, radius: f32, reach: f32) -> TipEllipseGrab {
    if ellipse_handles(angle, radius).iter().any(|h| (*h - at).length() <= reach) { TipEllipseGrab::Roundness } else { TipEllipseGrab::Angle }
}

/// The value after a drag to `at` (relative to the centre) holding `grab`. Angle: the direction
/// of `at` in whole degrees, counter-clockwise, `atan2(−y, x)` rounded; unchanged within 1 pt of
/// the centre. Roundness: `|at · minor| / radius` clamped to 0.01 to 1; the angle stays.
pub fn ellipse_drag(grab: TipEllipseGrab, at: Vec2, angle: TipAngle, radius: f32) -> TipAngle {
    if !(at.x.is_finite() && at.y.is_finite()) {
        return angle;
    }
    match grab {
        TipEllipseGrab::Angle if at.length() > 1.0 => TipAngle { degrees: f64::from((-at.y).atan2(at.x).to_degrees().round()), ..angle },
        TipEllipseGrab::Angle => angle,
        TipEllipseGrab::Roundness => {
            let (sin, cos) = (angle.degrees as f32).to_radians().sin_cos();
            let along = (at.x * -sin + at.y * -cos).abs();
            TipAngle { roundness: f64::from(along / radius.max(f32::EPSILON)).clamp(ROUND_MIN, 1.0), ..angle }
        }
    }
}

/// The style of the tip angle widget: the disc, the cross rules, the ellipse, the arrow and the dots.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TipEllipseStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The rules through the disc's centre.
    cross: Stateful<StyleColor>,
    /// The ellipse's outline.
    outline: Stateful<StyleColor>,
    /// The ellipse's fill.
    wash: Stateful<StyleColor>,
    /// The arrow and the dots' ring.
    accent: Stateful<StyleColor>,
    /// The dots' fill.
    dot_fill: Stateful<StyleColor>,
    /// The ellipse's major radius over the disc's.
    share: f32,
    /// The radius of a dot.
    dot: f32,
    /// The length and width of the arrow's head.
    head: Vec2,
    /// How far from a dot's centre a press still grabs it.
    reach: f32,
}

/// Photoshop's angle and roundness widget: a disc with cross lines, the tip's ellipse, an arrow
/// along the major axis and two dots on the minor axis. Drag the arrow (anywhere off the dots) to
/// turn the angle, a dot to set the roundness. A drag is one gesture (Escape cancels and the value
/// goes back); a click on the disc is a `Set`.
///
/// - **Keys.** One Tab stop. Up and Right turn the angle 1° counter-clockwise, Down and Left 1°
///   clockwise (Shift: 15°), wrapping at ±180; Alt with the arrows changes the roundness by 1 %
///   (Up and Right rounder).
/// - **AccessKit.** A slider named by its label with the value "{angle}, {roundness}" (the angle in
///   degrees, the roundness in percent, through the unit words); Increment and Decrement step the
///   angle, and SetValue sets the angle in whole degrees within ±180, keeping the roundness.
/// - **RTL.** Not mirrored: it shows document geometry.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct TipEllipse<'a> {
    #[common]
    common: Common,
    value: Bind<'a, TipAngle>,
}

impl<'a> TipEllipse<'a> {
    /// The widget named `label` over `value`.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, value: Bind<'a, TipAngle>) -> Self {
        use craft_ui::contract::Props as _;
        TipEllipse { common: Common::new(), value }.label(label)
    }
}

/// The major radius of the ellipse in a widget of `rect`.
fn major_radius(rect: Rect, style: &TipEllipseStyle) -> f32 {
    rect.width().min(rect.height()) / 2.0 * style.share
}

/// The angle after a key: Up and Right turn it counter-clockwise (the widget is not mirrored), Shift by a large
/// turn; Alt changes the roundness instead.
fn key(value: TipAngle, mv: NavMove, mods: MoveMods, rtl: bool) -> Option<TipAngle> {
    let raise = match mv {
        NavMove::Up => true,
        NavMove::Down => false,
        NavMove::Forward => !rtl,
        NavMove::Back => rtl,
        _ => return None,
    };
    let sign = if raise { 1.0 } else { -1.0 };
    if mods.fine {
        let roundness = (value.roundness + sign * ROUND_STEP).clamp(ROUND_MIN, 1.0);
        return Some(TipAngle { roundness: (roundness * 100.0).round() / 100.0, ..value });
    }
    let by = if mods.extend { COARSE_TURN } else { 1.0 };
    let mut degrees = value.degrees + sign * by;
    if degrees > 180.0 {
        degrees -= 360.0;
    } else if degrees < -180.0 {
        degrees += 360.0;
    }
    Some(TipAngle { degrees: degrees.round(), ..value })
}

impl Component for TipEllipse<'_> {
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = TipEllipseStyle;
    type Output = EditResponse<TipAngle>;

    fn style(cx: &StyleCx<'_>, _: TipEllipseStyleKey) -> TipEllipseStyle {
        let colors = &cx.tokens.color;
        let side = cx.tokens.size.dial * DISC_IN_DIALS;
        let states = |rest, off| Stateful::new(cx.color(rest)).off(States::new(cx.color(rest), cx.color(rest), cx.color(rest), cx.color(off)));
        let face = states(colors.surface.sunken, colors.state.disabled_fill);
        let edge = states(colors.line.control_edge, colors.line.control_edge);
        let ink = states(colors.text.normal, colors.text.disabled);
        let sunken = cx.color(colors.surface.sunken);
        let wash = sunken.mix(cx.color(colors.text.normal), ELLIPSE_WASH);
        let handle = cx.tokens.size.handle;
        TipEllipseStyle {
            surface: SurfaceLook::new(face).radius(CornerRadius::same((side / 2.0).round() as u8)).border(edge, cx.tokens.stroke.thin),
            metrics: Metrics::new().min_size(Vec2::splat(side)),
            focus: cx.focus_look(),
            cross: states(colors.line.divider, colors.line.divider),
            outline: ink,
            wash: Stateful::new(wash),
            accent: cx.accent_fill(),
            dot_fill: states(colors.overlay.handle_fill, colors.state.disabled_fill),
            share: ELLIPSE_SHARE,
            dot: handle.sm / 2.0,
            head: vec2(handle.md, handle.md),
            reach: cx.tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let center = rect.center();
        let disc = rect.width().min(rect.height()) / 2.0;
        let radius = major_radius(rect, cx.style);
        let reach = cx.style.reach;
        let value = cx.value().copied().unwrap_or_default();
        let dots = ellipse_handles(value, radius);
        cx.part(ARROW, HitShape::circle(center, disc))
            .sense(Sense::CLICK | Sense::DRAG)
            .value_part()
            .pointer_only("the arrow keys turn the angle, Alt with them squashes the ellipse");
        for (at, dot) in dots.into_iter().enumerate() {
            cx.part(DOT.at(at as u64), HitShape::circle(center + dot, reach))
                .sense(Sense::CLICK | Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .value_part()
                .pointer_only("Alt with the arrow keys squashes the ellipse");
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, TipAngle>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let center = cx.rect.center();
        let radius = major_radius(cx.rect, cx.style);
        let presses = [
            (TipEllipseGrab::Angle, cx.part(ARROW).press),
            (TipEllipseGrab::Roundness, cx.part(DOT.at(0)).press),
            (TipEllipseGrab::Roundness, cx.part(DOT.at(1)).press),
        ];
        for (grab, press) in presses {
            let current = cx.value().copied().unwrap_or_default();
            let at = |pos: Pos2| ellipse_drag(grab, pos - center, current, radius);
            match press.phase {
                PressPhase::DragStart { pos, .. } => {
                    cx.begin();
                    cx.propose(at(pos));
                }
                PressPhase::Drag { pos, .. } => cx.propose(at(pos)),
                PressPhase::Release { pos, click, .. } => {
                    let running = cx.gesture().is_some();
                    if running {
                        cx.propose(at(pos));
                        cx.commit();
                    } else if click {
                        cx.set(at(pos));
                    }
                }
                _ => {}
            }
        }
        let (running, rtl) = (cx.gesture().is_some(), cx.flow.is_rtl());
        let mut now = cx.value().copied().unwrap_or_default();
        // While a drag runs, keys and screen-reader requests change nothing; they are still taken.
        while let Some((mv, mods)) = cx.intents().take_move() {
            if let Some(next) = key(now, mv, mods, rtl).filter(|_| !running) {
                cx.set(next);
                now = next;
            }
        }
        if let Some(degrees) = cx.intents().take_set_value().filter(|degrees| degrees.is_finite() && !running) {
            cx.set(TipAngle { degrees: degrees.round().clamp(-180.0, 180.0), ..now });
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let numeric = match cx.value() {
            Some(value) => {
                let words = cx.words::<BrushSettingsWords>();
                let units = cx.env.units();
                let text = words
                    .tip_angle_value
                    .fill(&[("angle", &units.degrees(value.degrees)), ("roundness", &units.percent((value.roundness * 100.0).round()))]);
                Numeric::new(value.degrees, -180.0..=180.0).text(text)
            }
            None => Numeric::empty(-180.0..=180.0),
        };
        access.numeric(numeric.step(1.0));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(value) = cx.value().copied() else { return };
        let style = cx.style;
        let draw = ui.draw();
        let center = cx.rect.center();
        let disc = cx.rect.width().min(cx.rect.height()) / 2.0;
        let radius = major_radius(cx.rect, style);
        let cross = cx.resolve(&style.cross);
        draw.stroke(&PaintShape::Path(vec![center - vec2(disc, 0.0), center + vec2(disc, 0.0)]), StrokeWidth::Thin, cross);
        draw.stroke(&PaintShape::Path(vec![center - vec2(0.0, disc), center + vec2(0.0, disc)]), StrokeWidth::Thin, cross);
        let (sin, cos) = (value.degrees as f32).to_radians().sin_cos();
        let (major, minor) = (vec2(cos, -sin), vec2(-sin, -cos));
        let round = (value.roundness as f32).clamp(ROUND_MIN as f32, 1.0);
        let ring: Vec<Pos2> = (0..SEGMENTS)
            .map(|i| {
                let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                center + major * t.cos() * radius + minor * t.sin() * radius * round
            })
            .collect();
        let shape = PaintShape::Polygon(ring);
        draw.fill(&shape, cx.resolve(&style.wash));
        draw.stroke(&shape, StrokeWidth::Medium, cx.resolve(&style.outline));
        let accent = cx.resolve(&style.accent);
        let tip = center + major * disc;
        draw.stroke(&PaintShape::Path(vec![center, tip]), StrokeWidth::Medium, accent);
        let back = tip - major * style.head.x;
        let half = style.head.y / 2.0;
        draw.fill(&PaintShape::Polygon(vec![tip, back + minor * half, back - minor * half]), accent);
        for handle in ellipse_handles(value, radius) {
            let dot = PaintShape::Circle(center + handle, style.dot);
            draw.fill(&dot, cx.resolve(&style.dot_fill));
            draw.stroke(&dot, StrokeWidth::Thin, accent);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<TipAngle> {
        cx.edit_response()
    }
}
