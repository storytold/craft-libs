//! `Slider`: a thumb on a track over an `f64` range. A press on the thumb grabs it and a drag moves it by the pointer's
//! travel; a press on the track jumps the thumb there; arrows, the wheel and AccessKit step the value; a double click
//! or the reset intent restores the binding's reset value. The vertical wheel steps it while it is focused or armed, so
//! a panel scrolls over it; inside a `WheelScope::Free` scope (an adjustment dialog) without focus too. The track maps
//! positions to values through a [`SliderScale`], linear by default.

use super::track::{self, Along, Fill, MarkInk, MarkSpace, Marks, Press, Scale, SliderScale, ThumbRing, TrackFace, Travel};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, PaintUi, PartsCx, Pointer as _,
    Remember as _, Role, Styled as _,
};
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, SurfaceStyle, TextLook, TrackLook};
use crate::text::NumberStepper;
use craft_behavior::intent::{MoveMods, NavMove};
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps, step_value};
use craft_behavior::{Flow, HitLevel, HitPriority, Metrics, PartKey, PressPhase, Proposal, Sense, WheelPhase};
use craft_theme::Stateful;
use egui::{Pos2, Rect, Vec2};
use std::ops::RangeInclusive;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The track the thumb rides on.
pub(crate) const TRACK: PartKey = PartKey::new("track");

/// The style of a slider.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SliderStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The labels under labelled marks.
    #[style(text)]
    mark_text: TextLook,
    /// The track and the part of it from the start edge to the thumb.
    track: TrackLook,
    /// The room marks keep.
    space: MarkSpace,
    /// The thumb: the theme's knob.
    knob: Stateful<SurfaceStyle>,
    /// The ring round the thumb while a press at the pointer would move it.
    ring: ThumbRing,
    /// The size of the drawn thumb.
    thumb: Vec2,
    /// The thickness of a colour channel's band, the track face over a triangle pointer: `size.ramp.bar`.
    band: f32,
    /// The length a slider asks for along its axis.
    length: f32,
    /// The lines at the ends of a trimmed span.
    trim: StyleColor,
}

/// The running drag: where the thumb was along the track when it began, and what the press moves.
#[derive(Clone, Default)]
struct Held {
    from: f64,
    press: Option<Press>,
}

/// A slider over `range`.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Slider<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f64>,
    range: RangeInclusive<f64>,
    /// The change of one arrow press, and the stops the value snaps to; the default is a hundredth of the range and
    /// no stops.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// Where keys, accessibility actions and the wheel step from the current value. The direction is 1 or -1;
    /// the magnitude counts steps. A ladder steps in that direction without wrapping. The callback is also
    /// queried for the available accessibility actions.
    /// It takes precedence over the fixed key and wheel steps. Pointer placement, including snapping,
    /// still uses the track's scale.
    #[prop(func = "Fn(f64, i32, f64) -> f64")]
    stepper: Option<NumberStepper<'a>>,
    /// How positions along the track map to values.
    #[prop]
    scale: SliderScale,
    /// Whether the part of the track from the start edge to the thumb is filled.
    #[prop]
    fill: bool,
    /// The shape of the thumb.
    #[prop]
    thumb: SliderThumb,
    marks: Marks,
    /// The change of one wheel notch where it is coarser than the step (an offset of 0.0001 steps that wants 0.01 a
    /// notch); the step by default. Shift and Alt scale it as they scale the step. A logarithmic track with no step
    /// moves along the track and ignores it. A custom stepper also overrides it.
    #[prop(quantity = "step")]
    wheel_step: Option<f64>,
    /// When a press on the track sets the value: on its release or drag (the default), or as the primary button goes
    /// down. Presses on the thumb and keyboard, wheel and editor input do not change.
    #[prop]
    track_press: TrackPress,
    /// The value as the caller words it ("50 %"), in place of the slider's own value text: the number in the step's
    /// decimals, in one decimal without a step, or at the local step's precision with a custom stepper. Assistive
    /// technology reads it, and so does a viewer that shows the value beside the slider.
    #[prop(into)]
    value_text: Option<Label>,
    /// Paints the groove with `face`, a component shown as the look of the track (a colour gradient). The fill is not
    /// drawn over it.
    #[prop]
    track_face: Option<TrackFace<'a>>,
    /// The kept part of the range, its ends drawn across the track ([`Slider::trim`]).
    trim: Option<(f64, f64)>,
    /// The track asks for a short field's length (`size.length.short`) in place of a track's (`size.length.track`):
    /// a slider beside a number in a document dialog's line, as production's Opacity.
    #[style_key]
    #[prop]
    short: bool,
}

/// When a press on a slider's track sets the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackPress {
    /// The value jumps to the press when it ends as a click, or follows the pointer once the press starts a drag; a
    /// press held still changes nothing yet.
    #[default]
    OnRelease,
    /// The value lands where the primary button goes down, in the same pass, and a drag goes on from there: an
    /// editor whose preview should answer the first touch. A double click still restores the reset value.
    OnPress,
}

/// The shape of a slider's thumb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SliderThumb {
    /// A knob on the track.
    #[default]
    Round,
    /// A pointer under the track, as colour channel sliders draw it: a track face shows as a colour band
    /// (`size.ramp.bar`) over the pointer, not at the groove's thickness.
    Triangle,
}

impl<'a> Slider<'a> {
    /// The thumb: a drag on it moves the value.
    pub const THUMB: PartKey = PartKey::new("thumb");

    /// A slider of `value` over `range`.
    pub fn new(value: Bind<'a, f64>, range: RangeInclusive<f64>) -> Self {
        Slider {
            common: Common::new(),
            value,
            range,
            step: None,
            stepper: None,
            scale: SliderScale::Linear,
            fill: true,
            thumb: SliderThumb::Round,
            marks: Marks::default(),
            wheel_step: None,
            track_press: TrackPress::OnRelease,
            value_text: None,
            track_face: None,
            trim: None,
            short: false,
        }
    }

    /// Values the thumb snaps to when a drag or a press brings it near, drawn as marks under the track. A mark outside
    /// the range is ignored. Keys and the wheel use the step only, so with a step the marks should lie on it.
    pub fn marks(mut self, marks: &[f64]) -> Self {
        self.marks.set(marks);
        self
    }

    /// `count + 1` evenly spaced ticks under the track, none for 0. The thumb does not snap to them.
    pub fn ticks(mut self, count: usize) -> Self {
        self.marks.ticks = count;
        self
    }

    /// [`Slider::marks`] at the values, each with a caption centred under its mark and kept inside the slider. The
    /// slider is taller by one caption line, and at least as wide as the captions need to sit apart with a gap between
    /// them. The captions are drawing only: assistive technology gets the value.
    pub fn marks_labelled<L: Into<Label>>(mut self, marks: impl IntoIterator<Item = (f64, L)>) -> Self {
        self.marks.set_labelled(marks);
        self
    }

    /// The kept part of the range, `kept`: a trimmed clip's In and Out on a seek track. Each end is a
    /// line across the track, a thumb high, in the warning colour; the thumb does not snap to them. An end outside
    /// the range is not drawn.
    pub fn trim(mut self, kept: std::ops::RangeInclusive<f64>) -> Self {
        self.trim = Some((*kept.start(), *kept.end()));
        self
    }

    fn track_scale(&self) -> Scale {
        Scale::new(&self.range, self.step, self.scale)
    }

    fn key_value(&self, from: Option<f64>, scale: &Scale, movement: NavMove, mods: MoveMods) -> Option<f64> {
        let Some(stepper) = self.stepper.as_ref() else {
            return scale.stepped(from.unwrap_or_else(|| scale.middle()), (scale.min, scale.max), movement, mods);
        };
        let StepResult::Set(steps) = step_value(&ladder_steps(), ValueAxes::Both, movement, mods) else {
            return None;
        };
        let next = match movement {
            NavMove::Start => Some(scale.min),
            NavMove::End => Some(scale.max),
            _ => ladder_value(stepper, scale, from, steps),
        };
        next.filter(|next| from != Some(*next))
    }

    /// Where the track face paints: the faced groove, or a colour band under a triangle pointer.
    fn face_rect(&self, style: &SliderStyle, along: Along) -> Rect {
        match self.thumb {
            SliderThumb::Triangle => Rect::from_center_size(along.span().center(), Vec2::new(along.span().width(), style.band)),
            SliderThumb::Round => track::ramp_rect(&style.track, along),
        }
    }

    /// The track in `rect` for a thumb of `style`, less the room of the mark labels.
    fn along(&self, rect: Rect, style: &SliderStyle, flow: Flow) -> Along {
        Along { area: self.marks.area(rect, &style.track), flow, thumb: style.thumb.x }
    }
}

impl Component for Slider<'_> {
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = SliderStyle;
    type Output = EditResponse<f64>;

    fn style(cx: &StyleCx<'_>, key: SliderStyleKey) -> SliderStyle {
        let control = cx.control();
        let length = if key.short { cx.tokens.size.length.short } else { cx.tokens.size.length.track };
        SliderStyle {
            // Where a row shrinks it, the track narrows down to a square of the control's height.
            metrics: Metrics::new().min_size(Vec2::new(length, control.height)).least_width(control.height),
            focus: cx.focus_look(),
            mark_text: track::mark_text(cx),
            track: cx.track(),
            space: MarkSpace::new(cx),
            knob: track::knob(cx),
            ring: ThumbRing::new(cx),
            thumb: track::knob_size(cx),
            band: cx.tokens.size.ramp.bar,
            length,
            trim: cx.color(cx.tokens.color.status.warn.fg),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        let labels = self.marks.least_width(&self.track_scale());
        let along = if cx.proposal == Proposal::Min { labels } else { cx.style.length.max(labels) };
        Vec2::new(along, cx.icon() + self.marks.below(&cx.style.track))
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::WHEEL_Y
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        let scale = self.track_scale();
        let along = self.along(rect, cx.style, cx.flow);
        let value = cx.value().copied().unwrap_or_else(|| scale.place(f64::NAN));
        let thumb = along.thumb_rect(scale.position(value));
        if self.track_face.is_some() {
            cx.part(track::GROOVE, self.face_rect(cx.style, along));
        }
        let sense = Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS;
        let sense = if self.track_press == TrackPress::OnPress { sense | Sense::SETS_ON_PRESS } else { sense };
        cx.part(TRACK, along.area).sense(sense).value_part().pointer_only("the arrow keys step the value");
        cx.part(Self::THUMB, thumb)
            .sense(Sense::DRAG | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .value_part()
            .pointer_only("the arrow keys step the value");
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    /// A host that keeps the value in `f32` writes the proposal back rounded to it: that is the gesture's own value.
    fn same_value(&self, proposed: &f64, host: &f64) -> bool {
        craft_behavior::same_in_f32(*proposed, *host)
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let scale = self.track_scale();
        let bounds = (scale.min, scale.max);
        let along = self.along(cx.rect, cx.style, cx.flow);
        let rtl = cx.flow.is_rtl();
        let length = along.length().max(1.0);
        // The drag runs over positions along the track, so every scale shares the maths.
        let travel = Travel { track: along.travel().min..along.travel().max, units: 0.0..=1.0, rtl };
        let pick = along.pick(false);
        let marks = &self.marks;
        let at = |position: f64| marks.snapped(&scale, length, position).unwrap_or_else(|| scale.snap(scale.at_position(position)));
        let current = |cx: &Cx<'_, Self>| cx.value().copied().unwrap_or_else(|| scale.middle());
        let press_at = |cx: &Cx<'_, Self>, x: f32| pick.press(&[along.x(scale.position(current(cx)))], x);
        // Multi-clicks keep the legacy track path: reset wins on Down, and a later drag
        // can still begin from the reset value through the existing DragStart handling.
        let follow_track_on_down = self.track_press == TrackPress::OnPress
            && cx.part(TRACK).press.phase.start().is_none_or(|start| start.button != craft_behavior::PointerButton::Primary || start.clicks < 2);
        if follow_track_on_down {
            let press = cx.part(TRACK).press;
            if let Some(start) = press.phase.start().filter(|start| start.button == craft_behavior::PointerButton::Primary) {
                if matches!(press.phase, PressPhase::Down { .. }) && cx.in_wheel_run() {
                    // The pending wheel End must close the old run before follow_press starts the press gesture.
                    cx.commit();
                }
                let absolute = craft_behavior::ValueDrag::absolute(travel.track.clone(), travel.units.clone());
                if matches!(press.phase, PressPhase::Down { .. })
                    && let Some(position) = absolute.at(start.origin, rtl)
                {
                    // Fine/coarse travel continues from this real snapped landing, not the value before the press.
                    *cx.memory::<Held>() = Held { from: scale.position(at(position)), press: Some(Press::Jump(0)) };
                }
                let held = cx.memory::<Held>().clone();
                // The existing shared primitive owns Begin/update/commit and the cancelled-held-release latch.
                cx.follow_press(TRACK, |pos| {
                    let next = if matches!(press.phase, PressPhase::Down { .. }) {
                        absolute.at(pos, rtl)
                    } else {
                        held.press.and_then(|press_of| travel.value(press_of, held.from, &press, true))
                    };
                    next.map(at)
                });
            }
        }
        for key in [Self::THUMB, TRACK] {
            if key == TRACK && follow_track_on_down {
                continue;
            }
            let press = cx.part(key).press;
            match press.phase {
                PressPhase::DragStart { start, .. } => {
                    let held = Held { from: scale.position(current(&*cx)), press: press_at(&*cx, start.origin.x) };
                    *cx.memory::<Held>() = held;
                    cx.begin();
                }
                PressPhase::Drag { .. } => {
                    let held = cx.memory::<Held>().clone();
                    if let Some(next) = held.press.and_then(|press_of| travel.value(press_of, held.from, &press, true)) {
                        cx.propose(at(next));
                    }
                }
                PressPhase::Release { start, click, .. } => {
                    if cx.gesture().is_some() {
                        let held = cx.memory::<Held>().clone();
                        if let Some(next) = held.press.and_then(|press_of| travel.value(press_of, held.from, &press, true)) {
                            cx.propose(at(next));
                        }
                        cx.commit();
                    } else if click
                        && start.clicks < 2
                        && let Some(jump @ Press::Jump(_)) = press_at(&*cx, start.origin.x)
                        && let Some(next) = travel.value(jump, 0.0, &press, false)
                    {
                        // A press on the track jumps to the pointer; a click on the thumb changes nothing. A second
                        // click is the reset, which the pipeline applies.
                        cx.set(at(next));
                    }
                }
                _ => {}
            }
        }
        // A drag a press of the thumb or the track carries is one edit: keys, the wheel and SetValue wait for it. A
        // run the wheel began holds nothing back; a key or SetValue commits it first, so no one-shot change lands
        // beside its open proposal.
        // The press check reads the gesture slot, which holds for the whole press whatever this pass's press phase is.
        let pressing = cx.gesture().is_some() && !cx.in_wheel_run();
        let wheel = cx.take_wheel().filter(|_| !pressing);
        let set_value = cx.intents().take_set_value().filter(|_| !pressing);
        while let Some((mv, mods)) = cx.intents().take_move() {
            if pressing {
                continue;
            }
            if cx.in_wheel_run() {
                cx.commit();
            }
            if let Some(next) = self.key_value(cx.value().copied(), &scale, mv, mods) {
                cx.set(next);
            }
        }
        if let Some(next) = set_value {
            if cx.in_wheel_run() {
                cx.commit();
            }
            cx.set(scale.snap(next));
        }
        if let Some(wheel) = wheel {
            // The end share carries no scroll: it closes the run.
            let next = if wheel.phase == WheelPhase::End {
                None
            } else if let Some(stepper) = self.stepper.as_ref() {
                match craft_behavior::wheel_step(&ladder_steps(), ValueAxes::Both, &wheel, rtl) {
                    StepResult::Set(steps) => ladder_value(stepper, &scale, cx.value().copied(), steps),
                    _ => None,
                }
            } else {
                scale.wheeled(current(&*cx), bounds, &wheel, rtl, self.wheel_step)
            };
            cx.wheel_run(wheel.phase, next);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let scale = self.track_scale();
        let range = scale.min..=scale.max;
        let numeric = match cx.value() {
            Some(value) => Numeric::new(*value, range),
            None => Numeric::empty(range),
        };
        if let Some(stepper) = &self.stepper {
            let can_step = |steps| ladder_value(stepper, &scale, cx.value().copied(), steps).is_some();
            access.numeric(numeric).variable_steps(can_step(1.0), can_step(-1.0));
        } else {
            access.numeric(numeric.step(scale.small()));
        }
    }

    /// The app's words for the shown value when it gives them, else the value as the slider words it.
    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        let value = cx.value()?;
        match &self.value_text {
            Some(text) => Some(cx.env.resolve(text).as_str().to_owned()),
            None => self.word_value(value, cx),
        }
    }

    fn word_value(&self, value: &f64, cx: &Cx<'_, Self>) -> Option<String> {
        let scale = self.track_scale();
        let units = cx.env.units();
        let Some(stepper) = &self.stepper else { return Some(scale.text(&units, *value)) };
        let step = [1.0, -1.0]
            .into_iter()
            .filter_map(|steps| ladder_value(stepper, &scale, Some(*value), steps))
            .map(|next| (next - *value).abs())
            .filter(|step| step.is_finite() && *step > 0.0)
            .reduce(f64::min);
        Some(if let Some(step) = step {
            // Keep simple fractional stops even when the ladder strides by whole units.
            // A widened f32 echo does not need its tail of rounding digits.
            let current_places = craft_behavior::decimals_for_step(*value).min(7);
            let step_places = (1.0 - step.log10().floor()).max(0.0) as usize;
            let nonzero_places = if *value != 0.0 { (-value.abs().log10().floor()).max(0.0) as usize } else { 0 };
            units.number(*value, 0..=current_places.max(step_places).max(nonzero_places))
        } else {
            let decimal = value.to_string();
            let places = decimal.split_once('.').map_or(0, |(_, fraction)| fraction.len());
            cx.env.format().number(*value, places)
        })
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let flow = cx.flow;
        let scale = self.track_scale();
        let along = self.along(cx.rect, style, flow);
        let value = cx.value().copied();
        let faced = match &self.track_face {
            Some(face) => {
                face.paint(ui);
                true
            }
            None => false,
        };
        let draw = ui.draw();
        let centre = Pos2::new(along.x(scale.position(value.unwrap_or_else(|| scale.place(f64::NAN)))), along.area.center().y);
        if !faced {
            let reach = (centre.x - flow.start_x()).abs();
            let fill = value.filter(|_| self.fill).map(|_| Fill { color: cx.resolve(&style.track.filled), from: 0.0, to: reach });
            track::paint(&draw, &style.track, along, cx.resolve(&style.track.unfilled), fill);
        }
        let ink = MarkInk { marks: cx.resolve(&style.track.mark_color), labels: cx.resolve(&style.mark_text.color), role: style.mark_text.role };
        track::paint_marks(&draw, &style.track, &self.marks, &scale, along, cx.rect, &ink);
        if let Some(span) = self.trim {
            track::paint_trim(&draw, span, &scale, along, style.thumb.y, style.trim);
        }
        // A press anywhere on the slider moves the thumb, so hovering anywhere on it lights the thumb.
        let running = cx.gesture().and_then(|_| cx.recall::<Held>().press);
        let hovered = [Self::THUMB, TRACK].into_iter().find_map(|part| cx.hover_pos(part)).and_then(|at| along.pick(false).press(&[centre.x], at.x));
        let state = cx.core.part_state_lit(Self::THUMB.into(), track::lit(running, hovered, 0));
        let (knob, ring) = (state.resolve(&style.knob), state.resolve(&style.ring.color));
        match self.thumb {
            SliderThumb::Round => {
                let rect = Rect::from_center_size(centre, style.thumb);
                style.ring.paint_round(&draw, rect, knob.radius, ring);
                draw.surface_styled(rect, &knob);
            }
            SliderThumb::Triangle => {
                // The pointer starts under the band it points at: the colour face, or the plain groove.
                let band = if faced { self.face_rect(style, along) } else { track::groove_rect(&style.track, along.area) };
                let (half, top, x) = (style.thumb.x / 2.0, band.bottom(), centre.x);
                let points = vec![Pos2::new(x, top), Pos2::new(x + half, top + half), Pos2::new(x - half, top + half)];
                style.ring.paint_pointer(&draw, &points, ring);
                track::paint_pointer(&draw, points, &knob);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}

/// Read keys and the wheel in steps from zero, independently of the model's scale or snap grid.
fn ladder_steps() -> ValueSteps {
    ValueSteps::new(0.0, f64::NEG_INFINITY, f64::INFINITY, 1.0, LARGE_STEPS)
}

fn ladder_value(stepper: &NumberStepper<'_>, scale: &Scale, from: Option<f64>, steps: f64) -> Option<f64> {
    let start = scale.place(from.unwrap_or_else(|| scale.middle()));
    let direction = if steps > 0.0 { 1 } else { -1 };
    let next = stepper.next(start, direction, steps.abs()).clamp(scale.min, scale.max);
    let direction_matches = from.filter(|value| value.is_finite()).is_none_or(|value| if direction > 0 { next > value } else { next < value });
    (!next.is_nan() && direction_matches).then_some(next)
}
