//! `Slider`: a thumb on a track over an `f64` range. A press on the thumb grabs it and a drag moves it by the pointer's
//! travel; a press on the track jumps the thumb there; arrows, the wheel and AccessKit step the value; a double click
//! or the reset intent restores the binding's reset value. The track maps positions to values through a
//! [`SliderScale`], linear by default.

use super::track::{self, Along, Fill, MarkInk, MarkSpace, Marks, Pick, Press, Scale, SliderScale, TrackFace, Travel};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, PaintUi, PartsCx, Pointer as _,
    Remember as _, Role, Styled as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TextLook, TrackLook};
use craft_behavior::{Flow, HitLevel, HitPriority, Metrics, PartKey, PressPhase, Sense};
use craft_theme::Stateful;
use egui::{Pos2, Rect, Vec2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

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
    /// The thumb's fill.
    thumb: Stateful<StyleColor>,
    /// The thumb's edge.
    thumb_edge: Stateful<StyleColor>,
    /// The diameter of the drawn thumb.
    diameter: f32,
    /// The length a slider asks for along its axis.
    length: f32,
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
    /// How positions along the track map to values.
    #[prop]
    scale: SliderScale,
    /// Whether the part of the track from the start edge to the thumb is filled.
    #[prop]
    fill: bool,
    /// The shape of the thumb.
    #[prop]
    thumb: SliderThumb,
    /// Evenly spaced ticks under the track: one more than this, none for 0.
    #[prop]
    ticks: usize,
    marks: Marks,
    /// The change of one wheel notch where it is coarser than the step (an offset of 0.0001 steps that wants 0.01 a
    /// notch); the step by default. Shift and Alt scale it as they scale the step. A logarithmic track with no step
    /// moves along the track and ignores it.
    #[prop(quantity = "step")]
    wheel_step: Option<f64>,
    /// The value as the caller words it ("50 %"), in place of the slider's own value text: the number in the step's
    /// decimals, or in one decimal without a step. Assistive technology reads it, and so does a viewer that
    /// shows the value beside the slider.
    #[prop(into)]
    value_text: Option<Label>,
    /// Paints the groove with `face`, a component shown as the look of the track (a colour gradient). The fill is not
    /// drawn over it.
    #[prop]
    track_face: Option<TrackFace<'a>>,
}

/// The shape of a slider's thumb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SliderThumb {
    /// A knob on the track.
    #[default]
    Round,
    /// A pointer under the track, as colour channel sliders draw it.
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
            scale: SliderScale::Linear,
            fill: true,
            thumb: SliderThumb::Round,
            ticks: 0,
            marks: Marks::default(),
            wheel_step: None,
            value_text: None,
            track_face: None,
        }
    }

    /// Values the thumb snaps to when a drag or a press brings it near, drawn as marks under the track. A mark outside
    /// the range is ignored. Keys and the wheel use the step only, so with a step the marks should lie on it.
    pub fn marks(mut self, marks: &[f64]) -> Self {
        self.marks.set(marks);
        self
    }

    /// [`Slider::marks`] at the values, each with a caption centred under its mark and kept inside the slider. The
    /// slider is taller by one caption line. The captions are drawing only: assistive technology gets the value.
    pub fn marks_labelled<L: Into<Label>>(mut self, marks: impl IntoIterator<Item = (f64, L)>) -> Self {
        self.marks.set_labelled(marks);
        self
    }

    fn track_scale(&self) -> Scale {
        Scale::new(&self.range, self.step, self.scale)
    }

    /// The track in `rect` for a thumb of `style`, less the room of the mark labels.
    fn along(&self, rect: Rect, style: &SliderStyle, flow: Flow) -> Along {
        Along { area: self.marks.area(rect, &style.track), flow, thumb: style.diameter }
    }
}

impl Component for Slider<'_> {
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = SliderStyle;
    type Output = EditResponse<f64>;

    fn style(cx: &StyleCx<'_>, _: SliderStyleKey) -> SliderStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let accent = &colors.accent;
        let length = cx.tokens.size.length.track;
        SliderStyle {
            metrics: Metrics::new().min_size(Vec2::new(length, control.height)),
            focus: cx.focus_look(),
            mark_text: track::mark_text(cx),
            track: cx.track(),
            space: MarkSpace::new(cx),
            thumb: track::states(cx, colors.surface.knob, colors.surface.knob, colors.surface.knob),
            thumb_edge: track::states(cx, colors.line.knob_edge, accent.strong_hover, accent.strong_pressed),
            diameter: cx.tokens.component.slider.thumb_height.get(cx.size),
            length,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        Vec2::new(cx.style.length, cx.icon() + self.marks.below(&cx.style.track))
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::WHEEL | Sense::WHEEL_X
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        let scale = self.track_scale();
        let along = self.along(rect, cx.style, cx.flow);
        let value = cx.value().copied().unwrap_or_else(|| scale.place(f64::NAN));
        let thumb = along.thumb_rect(scale.position(value));
        if self.track_face.is_some() {
            cx.part(track::GROOVE, track::ramp_rect(&cx.style.track, along));
        }
        cx.part(TRACK, along.area).sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS).value_part().pointer_only("the arrow keys step the value");
        cx.part(Self::THUMB, thumb)
            .sense(Sense::DRAG | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .value_part()
            .pointer_only("the arrow keys step the value");
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let scale = self.track_scale();
        let bounds = (scale.min, scale.max);
        let along = self.along(cx.rect, cx.style, cx.flow);
        let (rtl, height) = (cx.flow.is_rtl(), along.area.height());
        let length = along.length().max(1.0);
        // The drag runs over positions along the track, so every scale shares the maths.
        let travel = Travel { track: along.travel().min..along.travel().max, units: 0.0..=1.0, rtl };
        let pick = Pick { reach: height / 2.0, overlap: height, period: 0.0, span: false };
        let marks = &self.marks;
        let at = |position: f64| marks.snapped(&scale, length, position).unwrap_or_else(|| scale.snap(scale.at_position(position)));
        let current = |cx: &Cx<'_, Self>| cx.value().copied().unwrap_or_else(|| scale.middle());
        let press_at = |cx: &Cx<'_, Self>, x: f32| pick.press(&[along.x(scale.position(current(cx)))], x);
        for key in [Self::THUMB, TRACK] {
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
        // A running drag is one edit: keys, the wheel and SetValue wait for it.
        let running = cx.gesture().is_some();
        let wheel = cx.take_wheel().filter(|_| !running);
        let set_value = cx.intents().take_set_value().filter(|_| !running);
        while let Some((mv, mods)) = cx.intents().take_move() {
            if running {
                continue;
            }
            if let Some(next) = scale.stepped(current(&*cx), bounds, mv, mods) {
                cx.set(next);
            }
        }
        if let Some(wheel) = wheel
            && let Some(next) = scale.wheeled(current(&*cx), bounds, &wheel, rtl, self.wheel_step)
        {
            cx.set(next);
        }
        if let Some(next) = set_value {
            cx.set(scale.snap(next));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let scale = self.track_scale();
        let range = scale.min..=scale.max;
        let numeric = match cx.value() {
            Some(value) => Numeric::new(*value, range),
            None => Numeric::empty(range),
        };
        access.numeric(numeric.step(scale.small()));
    }

    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        let value = cx.value()?;
        Some(match &self.value_text {
            Some(text) => cx.env.resolve(text).as_str().to_owned(),
            None => self.track_scale().text(&cx.env.units(), *value),
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
        let (fill, edge) = (cx.resolve_part(Self::THUMB, &style.thumb), cx.resolve_part(Self::THUMB, &style.thumb_edge));
        let knob = match self.thumb {
            SliderThumb::Round => PaintShape::Circle(centre, style.diameter / 2.0),
            SliderThumb::Triangle => {
                let (half, top, x) = (style.diameter / 2.0, track::groove_rect(&style.track, along.area).bottom(), centre.x);
                PaintShape::Polygon(vec![Pos2::new(x, top), Pos2::new(x + half, top + half), Pos2::new(x - half, top + half)])
            }
        };
        draw.fill(&knob, fill);
        draw.stroke(&knob, StrokeWidth::Thin, edge);
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}
