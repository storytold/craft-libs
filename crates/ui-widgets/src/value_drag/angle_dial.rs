//! `AngleDial`: a round dial whose needle points in a direction. A press or drag points the needle at the pointer; the
//! arrow and Page keys, the wheel and AccessKit step the angle, round the range when it wraps. Its degree field is a
//! `NumberField` the caller shows beside it, bound to the same value.

use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, PaintUi, PartsCx, Pointer as _,
    Role, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::UnitKind;
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, SurfaceLook};
use craft_behavior::geometry::crop::snap_angle;
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps, snap_to_step, step_value};
use craft_behavior::{HitShape, Metrics, MoveMods, NavMove, PartKey, PressPhase, Sense, Wheel, decimals_for_step, wheel_step, wrap_value};
use craft_theme::{Stateful, States};
use egui::{CornerRadius, Pos2, Rect, Vec2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

/// The face: the circle the pointer points at.
const FACE: PartKey = PartKey::new("face");

/// One turn in degrees.
const TURN: f64 = 360.0;
/// The grid Shift snaps the pointer to, and the step the Page keys take.
const OCTANT: f64 = 45.0;
/// The range and the step of a dial that is given none a usable one.
const DEFAULT_RANGE: (f64, f64) = (-180.0, 180.0);
const DEFAULT_STEP: f64 = 1.0;
/// The least a range's span may differ from a turn for the dial to count it as one.
const TURN_TOLERANCE: f64 = 1e-9;

/// Where the dial's zero is and which way the value grows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AngleOrientation {
    /// 0 at 3 o'clock, counter-clockwise: a mathematician's angle with y up.
    #[default]
    CounterClockwise,
    /// 0 at 12 o'clock, clockwise.
    Clockwise,
}

impl AngleOrientation {
    /// The screen direction (y down) of the needle at `degrees`.
    pub fn direction(self, degrees: f64) -> Vec2 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        match self {
            AngleOrientation::CounterClockwise => Vec2::new(cos as f32, -sin as f32),
            AngleOrientation::Clockwise => Vec2::new(sin as f32, -cos as f32),
        }
    }

    /// The angle in degrees, in `(-180, 180]`, of the screen offset `offset` (y down) from the centre.
    pub fn angle_of(self, offset: Vec2) -> f64 {
        let (across, up) = (f64::from(offset.x), f64::from(-offset.y));
        match self {
            AngleOrientation::CounterClockwise => up.atan2(across).to_degrees(),
            AngleOrientation::Clockwise => across.atan2(up).to_degrees(),
        }
    }
}

/// The style of an angle dial.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct AngleDialStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The needle.
    needle: Stateful<StyleColor>,
    /// The dot at the centre.
    hub_fill: Stateful<StyleColor>,
    /// From the face's edge to the needle's tip.
    needle_inset: f32,
    /// The radius of the dot at the centre.
    hub: f32,
    /// The radius round the centre within which the pointer changes nothing.
    dead_zone: f32,
}

/// An angle in degrees: a dial whose needle points where the pointer is.
///
/// Shift while pointing snaps the direction to 45 degrees, as Photoshop's angle dial does, so Alt is the fine factor
/// of the keys and the wheel (a tenth of a step); Shift on a key or a wheel notch takes the large step.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct AngleDial<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f64>,
    /// The values the angle may take; the default is one turn, -180 to 180. Bounds that are not finite give the
    /// default.
    #[prop]
    range: RangeInclusive<f64>,
    /// The step values snap to and one arrow key changes; the default is one degree.
    #[prop(quantity = "degrees")]
    step: f64,
    /// Where zero degrees sits and which way angles increase.
    #[prop]
    orientation: AngleOrientation,
    /// Whether the value wraps round the range: a step past one end continues from the other, and a direction maps
    /// into the range modulo its span. By default only a range of exactly one turn wraps.
    #[prop]
    wrap: Option<bool>,
}

impl<'a> AngleDial<'a> {
    /// A dial over `value`, in degrees.
    pub fn new(value: Bind<'a, f64>) -> Self {
        AngleDial {
            common: Common::new(),
            value,
            range: DEFAULT_RANGE.0..=DEFAULT_RANGE.1,
            step: DEFAULT_STEP,
            orientation: AngleOrientation::default(),
            wrap: None,
        }
    }

    fn bounds(&self) -> (f64, f64) {
        let (a, b) = (*self.range.start(), *self.range.end());
        if a.is_finite() && b.is_finite() { (a.min(b), a.max(b)) } else { DEFAULT_RANGE }
    }

    fn step_size(&self) -> f64 {
        if self.step.is_finite() && self.step > 0.0 { self.step } else { DEFAULT_STEP }
    }

    fn wraps(&self) -> bool {
        let (min, max) = self.bounds();
        self.wrap.unwrap_or_else(|| (max - min - TURN).abs() <= TURN_TOLERANCE)
    }

    /// `value` on the nearest stop of the grid of `step` from the range's start, in the range.
    fn snap(&self, value: f64, step: f64) -> f64 {
        let (min, max) = self.bounds();
        snap_to_step(value, step, min, min..=max)
    }

    /// The value `delta` degrees from `value` (`None`: mixed, which steps from the range's start), brought back into
    /// the range from the end it passed when the dial wraps, and on the step grid. A value inside the range, max
    /// included, stays; one past max continues from min by the overshoot and one below min from max, so both ends
    /// are reachable. The grid is the step, or the fine step when `fine`.
    fn stepped(&self, value: Option<f64>, delta: f64, fine: bool) -> f64 {
        let (min, max) = self.bounds();
        let target = value.unwrap_or(min) + delta;
        let span = max - min;
        let wrapped = if self.wraps() && !(min..=max).contains(&target) { wrap_value(target, min, span) } else { target };
        self.snap(wrapped, if fine { self.step_size() / LARGE_STEPS } else { self.step_size() })
    }

    fn pointing(&self, rect: Rect, dead_zone: f32) -> Pointing {
        let (min, max) = self.bounds();
        Pointing { min, max, wraps: self.wraps(), orientation: self.orientation, dead_zone, center: rect.center() }
    }

    /// The steps of one key or wheel notch from `value` (`None`: mixed, which steps from the range's start). The bounds
    /// are open so a step past an end reaches `stepped`, which wraps or stops.
    fn steps(&self, value: Option<f64>) -> ValueSteps {
        let step = self.step_size();
        ValueSteps::new(value.unwrap_or(self.bounds().0), f64::NEG_INFINITY, f64::INFINITY, step, step * LARGE_STEPS)
    }

    /// The angle after an arrow or Page key, or `None` for a key that does not step it. Right and Up raise the angle
    /// in either reading direction, because the dial is not mirrored. Home and End are none: the ends of a turn point
    /// the same way. A Page key is 45 degrees.
    fn key(&self, value: Option<f64>, mv: NavMove, mods: MoveMods, rtl: bool) -> Option<f64> {
        let raise = match mv {
            NavMove::PageUp => return Some(self.stepped(value, OCTANT, false)),
            NavMove::PageDown => return Some(self.stepped(value, -OCTANT, false)),
            NavMove::Forward => !rtl,
            NavMove::Back => rtl,
            NavMove::Up => true,
            NavMove::Down => false,
            _ => return None,
        };
        let steps = self.steps(value);
        let mv = if raise { NavMove::Up } else { NavMove::Down };
        match step_value(&steps, ValueAxes::Vertical, mv, mods) {
            StepResult::Set(next) => Some(self.stepped(value, next - steps.value, mods.fine && !mods.extend)),
            _ => None,
        }
    }

    /// The angle after a wheel notch on either axis.
    fn wheeled(&self, value: Option<f64>, wheel: &Wheel) -> Option<f64> {
        let steps = self.steps(value);
        match wheel_step(&steps, ValueAxes::Both, wheel, false) {
            StepResult::Set(next) => Some(self.stepped(value, next - steps.value, wheel.mods.alt && !wheel.mods.shift)),
            _ => None,
        }
    }
}

/// Where the pointer points the dial.
struct Pointing {
    min: f64,
    max: f64,
    wraps: bool,
    orientation: AngleOrientation,
    dead_zone: f32,
    center: Pos2,
}

impl Pointing {
    /// The value for the pointer at `pos`, in the range and not yet on the step grid, or `None` within the dead zone.
    /// `current` (`None`: mixed, which counts as the range's start) picks among the angles a range wider than a turn
    /// holds for the direction; `snap` rounds the direction to 45 degrees first.
    fn at(&self, pos: Pos2, current: Option<f64>, snap: bool) -> Option<f64> {
        let offset = pos - self.center;
        if offset.length() <= self.dead_zone {
            return None;
        }
        let mut angle = self.orientation.angle_of(offset);
        if snap {
            angle = snap_angle(angle, OCTANT);
        }
        let current = current.unwrap_or(self.min);
        let nearest = (-2..=2)
            .map(|turns| angle + TURN * f64::from(turns))
            .filter(|candidate| (self.min..=self.max).contains(candidate))
            .min_by(|a, b| (a - current).abs().total_cmp(&(b - current).abs()));
        Some(match nearest {
            Some(value) => value,
            None if self.wraps && self.max - self.min < TURN => wrap_value(angle, self.min, self.max - self.min),
            None => angle.clamp(self.min, self.max),
        })
    }
}

impl Component for AngleDial<'_> {
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = AngleDialStyle;
    type Output = EditResponse<f64>;

    fn style(cx: &StyleCx<'_>, _: AngleDialStyleKey) -> AngleDialStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let side = cx.tokens.size.dial;
        let disabled = colors.state.disabled_fill;
        let states = |rest, hover, pressed, off| {
            Stateful::new(cx.color(rest)).off(States::new(cx.color(rest), cx.color(hover), cx.color(pressed), cx.color(off)))
        };
        let face = states(colors.surface.sunken, colors.surface.sunken, colors.surface.sunken, disabled);
        let edge = states(colors.line.control_edge, colors.text.normal, colors.text.normal, colors.line.control_edge);
        AngleDialStyle {
            surface: SurfaceLook::new(face).radius(CornerRadius::same((side / 2.0).round() as u8)).border(edge, cx.tokens.stroke.thin),
            metrics: Metrics::new().min_size(Vec2::splat(side)),
            focus: cx.focus_look(),
            needle: cx.accent_fill(),
            hub_fill: states(colors.text.muted, colors.text.muted, colors.text.muted, colors.text.disabled),
            needle_inset: control.gap,
            hub: cx.tokens.size.handle.sm / 2.0,
            dead_zone: cx.tokens.size.hit.dead_zone,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let side = rect.width().min(rect.height());
        cx.part(FACE, HitShape::circle(rect.center(), side / 2.0))
            .sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS)
            .value_part()
            .pointer_only("the arrow keys step the angle");
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let pointing = self.pointing(cx.rect, cx.style.dead_zone);
        let press = cx.part(FACE).press;
        let shown = cx.value().copied();
        let at = |pos: Pos2| pointing.at(pos, shown, press.mods.shift).map(|value| self.snap(value, self.step_size()));
        match press.phase {
            PressPhase::DragStart { pos, .. } => {
                cx.begin();
                if let Some(next) = at(pos) {
                    cx.propose(next);
                }
            }
            PressPhase::Drag { pos, .. } => {
                if let Some(next) = at(pos) {
                    cx.propose(next);
                }
            }
            PressPhase::Release { pos, click, .. } => {
                let running = cx.gesture().is_some();
                if let Some(next) = at(pos) {
                    if running {
                        cx.propose(next);
                    } else if click {
                        cx.set(next);
                    }
                }
                if running {
                    cx.commit();
                }
            }
            _ => {}
        }
        let (running, rtl) = (cx.gesture().is_some(), cx.flow.is_rtl());
        let mut now = cx.value().copied();
        // While a drag runs, keys, the wheel and screen-reader requests change nothing; they are still taken.
        while let Some((mv, mods)) = cx.intents().take_move() {
            if let Some(next) = self.key(now, mv, mods, rtl).filter(|_| !running) {
                cx.set(next);
                now = Some(next);
            }
        }
        if let Some(wheel) = cx.take_wheel()
            && !running
            && let Some(next) = self.wheeled(now, &wheel)
        {
            cx.set(next);
        }
        if let Some(next) = cx.intents().take_set_value().filter(|next| next.is_finite() && !running) {
            cx.set(self.snap(next, self.step_size()));
        }
    }

    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        let decimals = decimals_for_step(self.step_size());
        cx.value().map(|value| cx.env.units().text(UnitKind::Degrees, *value, decimals..=decimals))
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (min, max) = self.bounds();
        let numeric = match cx.value() {
            Some(value) => Numeric::new(*value, min..=max),
            None => Numeric::empty(min..=max),
        };
        access.numeric(numeric.step(self.step_size()).wraps(self.wraps()));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let draw = ui.draw();
        let (center, radius) = (cx.rect.center(), cx.rect.width().min(cx.rect.height()) / 2.0);
        if let Some(value) = cx.value() {
            let tip = center + self.orientation.direction(*value) * (radius - style.needle_inset).max(0.0);
            draw.stroke(&PaintShape::Path(vec![center, tip]), StrokeWidth::Thick, cx.resolve(&style.needle));
        }
        draw.fill(&PaintShape::Circle(center, style.hub), cx.resolve(&style.hub_fill));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}
