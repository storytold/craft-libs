//! `RangeSlider`: a lower and an upper value on one track, with a thumb for each. The thumbs never cross. A press on a
//! thumb grabs it and a drag moves it by the pointer's travel; a press on the track moves the nearer thumb there.
//! Where the thumbs meet, the direction of the drag picks the one that moves. Each end is mixed or not on its own.

use super::track::{self, Along, Fill, MarkInk, MarkSpace, Marks, Pick, Press, Scale, SliderScale, TrackFace, Travel};
use super::words::ValueDragWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, PaintUi, PartsCx,
    Pointer as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TextLook, TrackLook, UiTokens};
use craft_behavior::{Flow, HitLevel, HitPriority, Metrics, Mixed, PartKey, PressPhase, PressState, Sense};
use craft_theme::Stateful;
use egui::{CornerRadius, Pos2, Rect, Vec2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

/// The track both thumbs ride on.
pub(crate) const TRACK: PartKey = PartKey::new("track");
/// The lower thumb.
pub(crate) const LOWER: PartKey = PartKey::new("lower");
/// The upper thumb.
pub(crate) const UPPER: PartKey = PartKey::new("upper");

/// The style of a range slider.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RangeSliderStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The labels under labelled marks.
    #[style(text)]
    mark_text: TextLook,
    /// The track and the part of it between the thumbs.
    track: TrackLook,
    /// The room marks keep.
    space: MarkSpace,
    /// The fill of a thumb.
    thumb: Stateful<StyleColor>,
    /// The edge of a thumb.
    thumb_edge: Stateful<StyleColor>,
    /// The width of a drawn thumb.
    thumb_width: f32,
    /// The height of a drawn thumb.
    thumb_height: f32,
    /// The corner of a drawn thumb.
    thumb_corner: CornerRadius,
    /// The length a slider asks for along its axis.
    length: f32,
}

/// The two ends of the range, each mixed or not.
type Pair = (Mixed<f64>, Mixed<f64>);

/// The running drag: the pair when it began and what the press moves; and the thumb moved last, which the wheel moves
/// when no thumb has focus.
#[derive(Clone, Default)]
struct Held {
    drag: Option<(Pair, Press)>,
    upper_last: bool,
}

/// The thumbs as drawn, in order and inside the range. A mixed end sits at its end of the range, so both thumbs stay
/// free to move.
fn placed(scale: &Scale, (lower, upper): &Pair) -> (f64, f64) {
    match (lower.same(), upper.same()) {
        (Some(&a), Some(&b)) => {
            let (a, b) = (scale.place(a), scale.place(b));
            if a <= b { (a, b) } else { (b, a) }
        }
        (a, b) => (a.map_or(scale.min, |&a| scale.place(a)), b.map_or(scale.max, |&b| scale.place(b))),
    }
}

/// `pair` with the thumb `upper` set to `value`; the other end keeps what it was, mixed or not.
fn with((lower, upper_end): Pair, upper: bool, value: f64) -> Pair {
    if upper { (lower, Mixed::Same(value)) } else { (Mixed::Same(value), upper_end) }
}

/// The key of a thumb.
fn key_of(upper: bool) -> PartKey {
    if upper { UPPER } else { LOWER }
}

/// A range slider over `range`.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct RangeSlider<'a> {
    #[common]
    common: Common,
    value: Bind<'a, (Mixed<f64>, Mixed<f64>)>,
    range: RangeInclusive<f64>,
    /// The change of one arrow press, and the stops the values snap to; the default is a hundredth of the range and
    /// no stops.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// How positions along the track map to values.
    #[prop]
    scale: SliderScale,
    /// Whether the part of the track between the thumbs is filled.
    #[prop]
    fill: bool,
    /// Evenly spaced ticks under the track: one more than this, none for 0.
    #[prop]
    ticks: usize,
    marks: Marks,
    /// Paints the groove with `face`, a component shown as the look of the track. The fill is not drawn over it.
    #[prop]
    track_face: Option<TrackFace<'a>>,
}

impl<'a> RangeSlider<'a> {
    /// A range slider of `value`, lower end first, over `range`. An end that is mixed shows a blank thumb at its end of
    /// the range; a key, the wheel or a drag on one thumb gives that end a number and leaves the other as it was.
    pub fn new(value: Bind<'a, (Mixed<f64>, Mixed<f64>)>, range: RangeInclusive<f64>) -> Self {
        RangeSlider {
            common: Common::new(),
            value,
            range,
            step: None,
            scale: SliderScale::Linear,
            fill: true,
            ticks: 0,
            marks: Marks::default(),
            track_face: None,
        }
    }

    /// Values a thumb snaps to when a drag brings it near, drawn as marks under the track; see [`Slider::marks`].
    ///
    /// [`Slider::marks`]: super::Slider::marks
    pub fn marks(mut self, marks: &[f64]) -> Self {
        self.marks.set(marks);
        self
    }

    /// [`RangeSlider::marks`] at the values, each with a caption centred under its mark. The slider is taller by one
    /// caption line.
    pub fn marks_labelled<L: Into<Label>>(mut self, marks: impl IntoIterator<Item = (f64, L)>) -> Self {
        self.marks.set_labelled(marks);
        self
    }

    fn track_scale(&self) -> Scale {
        Scale::new(&self.range, self.step, self.scale)
    }

    /// The track in `rect` for thumbs of `style`, less the room of the mark labels.
    fn along(&self, rect: Rect, style: &RangeSliderStyle, flow: Flow) -> Along {
        Along { area: self.marks.area(rect, &style.track), flow, thumb: style.thumb_width }
    }

    /// The ends the next interaction starts from: the binding's, each mixed or not; both mixed without one.
    fn current(cx: &Cx<'_, Self>) -> Pair {
        cx.value().cloned().unwrap_or((Mixed::Mixed, Mixed::Mixed))
    }

    /// The range thumb `upper` moves in: it stops at the other thumb.
    fn bounds(scale: &Scale, pair: (f64, f64), upper: bool) -> (f64, f64) {
        if upper { (pair.0, scale.max) } else { (scale.min, pair.1) }
    }
}

impl Component for RangeSlider<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = RangeSliderStyle;
    type Output = EditResponse<(Mixed<f64>, Mixed<f64>)>;

    fn style(cx: &StyleCx<'_>, _: RangeSliderStyleKey) -> RangeSliderStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let length = cx.tokens.size.length.track;
        RangeSliderStyle {
            metrics: Metrics::new().min_size(Vec2::new(length, control.height)),
            focus: cx.focus_look(),
            mark_text: track::mark_text(cx),
            track: cx.track(),
            space: MarkSpace::new(cx),
            thumb: track::states(cx, colors.text.normal, colors.text.normal, colors.text.normal),
            thumb_edge: track::states(cx, colors.text.muted, colors.text.muted, colors.text.muted),
            thumb_width: cx.tokens.size.handle.sm,
            thumb_height: cx.tokens.component.slider.thumb_height.get(cx.size),
            thumb_corner: UiTokens::corner(cx.tokens.shape.radius.sm),
            length,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        Vec2::new(cx.style.length, cx.icon() + self.marks.below(&cx.style.track))
    }

    fn sense(&self) -> Sense {
        Sense::WHEEL | Sense::WHEEL_X
    }

    /// The track, and a thumb each that is its own Tab stop and takes the arrow keys.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.marks.lay_out(&cx.style.mark_text, cx.style.space, |label| cx.env.resolve(label), |run| cx.layout(run).size());
        let scale = self.track_scale();
        let along = self.along(rect, cx.style, cx.flow);
        let pair = cx.value().cloned().unwrap_or((Mixed::Mixed, Mixed::Mixed));
        let (lower, upper) = placed(&scale, &pair);
        if self.track_face.is_some() {
            cx.part(track::GROOVE, track::ramp_rect(&cx.style.track, along));
        }
        cx.part(TRACK, along.area).sense(Sense::CLICK | Sense::DRAG).value_part().pointer_only("the arrow keys step the focused thumb");
        for (key, value) in [(LOWER, lower), (UPPER, upper)] {
            cx.part(key, along.thumb_rect(scale.position(value)))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focus_shape(FocusShape::Rect)
                .value_part()
                .focusable();
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, (Mixed<f64>, Mixed<f64>)>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let scale = self.track_scale();
        let along = self.along(cx.rect, cx.style, cx.flow);
        let (rtl, height) = (cx.flow.is_rtl(), along.area.height());
        let length = along.length().max(1.0);
        let marks = &self.marks;
        let snap = |position: f64| marks.snapped(&scale, length, position);
        // Thumbs follow the pointer's position along the track, in positions 0 to 1.
        let travel = Travel { track: along.travel().min..along.travel().max, units: 0.0..=1.0, rtl };
        let pick = Pick { reach: height / 2.0, overlap: height, period: 0.0, span: true };
        let thumbs_at = |pair: &Pair| {
            let (lower, upper) = placed(&scale, pair);
            [lower, upper].map(|value| along.x(scale.position(value)))
        };
        let press_at = |pair: &Pair, x: f32| pick.press(&thumbs_at(pair), x);
        // The ends a press gives: the thumb it moves, kept between the other thumb and the end of the range, or both
        // thumbs of a span, which keep their distance inside the range.
        let moved = |from: &Pair, press: Press, state: &PressState, fine: bool| -> Option<Pair> {
            let (origin, pos) = track::pointer(state)?;
            let pair = placed(&scale, from);
            let value_at = |position: f64| scale.snap(scale.at_position(position));
            if press == Press::Span {
                let position = travel.value(press, scale.position(pair.0), state, fine)?;
                let width = pair.1 - pair.0;
                let lower = value_at(position).min(scale.max - width).max(scale.min);
                return Some((Mixed::Same(lower), Mixed::Same((lower + width).min(scale.max))));
            }
            let upper = press.thumb(origin.x, pos.x)? == 1;
            let position = travel.value(press, scale.position(if upper { pair.1 } else { pair.0 }), state, fine)?;
            let value = snap(position).unwrap_or_else(|| value_at(position));
            let (low, high) = Self::bounds(&scale, pair, upper);
            Some(with(*from, upper, value.clamp(low, high)))
        };
        for key in [LOWER, UPPER, TRACK] {
            let press = cx.part(key).press;
            match press.phase {
                PressPhase::DragStart { start, pos, .. } => {
                    let from = Self::current(cx);
                    if let Some(picked) = press_at(&from, start.origin.x) {
                        let xs = thumbs_at(&from);
                        // A span focuses the thumb nearer the press.
                        let thumb = picked.thumb(start.origin.x, pos.x).or_else(|| {
                            (picked == Press::Span).then(|| usize::from((start.origin.x - xs[0]).abs() > (start.origin.x - xs[1]).abs()))
                        });
                        let held = cx.memory::<Held>();
                        held.drag = Some((from, picked));
                        if let Some(thumb) = thumb {
                            held.upper_last = thumb == 1;
                            cx.focus_part(key_of(thumb == 1));
                        }
                    }
                    cx.begin();
                }
                PressPhase::Drag { .. } => {
                    if let Some((from, picked)) = cx.memory::<Held>().drag
                        && let Some(next) = moved(&from, picked, &press, true)
                    {
                        cx.propose(next);
                    }
                }
                PressPhase::Release { start, click, .. } => {
                    if cx.gesture().is_some() {
                        if let Some((from, picked)) = cx.memory::<Held>().drag
                            && let Some(next) = moved(&from, picked, &press, true)
                        {
                            cx.propose(next);
                        }
                        cx.commit();
                    } else if click && start.clicks < 2 {
                        // A press on the track moves the nearer thumb there; a click on a thumb changes nothing.
                        let from = Self::current(cx);
                        if let Some(jump @ Press::Jump(thumb)) = press_at(&from, start.origin.x)
                            && let Some(next) = moved(&from, jump, &press, false)
                        {
                            cx.set(next);
                            cx.focus_part(key_of(thumb == 1));
                        }
                    }
                }
                _ => {}
            }
        }
        // A running drag is one edit: keys, the wheel and SetValue wait for it.
        let running = cx.gesture().is_some();
        let focused = cx.active_part().filter(|part| part.key == LOWER || part.key == UPPER).map(|part| part.key == UPPER);
        let wheel = cx.take_wheel().filter(|_| !running);
        while let Some((mv, mods)) = cx.intents().take_move() {
            if let (Some(upper), false) = (focused, running) {
                let pair = Self::current(cx);
                let shown = placed(&scale, &pair);
                let value = if upper { shown.1 } else { shown.0 };
                if let Some(next) = scale.stepped(value, Self::bounds(&scale, shown, upper), mv, mods) {
                    cx.set(with(pair, upper, next));
                }
            }
        }
        if let Some(wheel) = wheel {
            let upper = focused.unwrap_or(cx.memory::<Held>().upper_last);
            let pair = Self::current(cx);
            let shown = placed(&scale, &pair);
            let value = if upper { shown.1 } else { shown.0 };
            if let Some(next) = scale.wheeled(value, Self::bounds(&scale, shown, upper), &wheel, rtl, None) {
                cx.set(with(pair, upper, next));
            }
        }
        for upper in [false, true] {
            if let Some(next) = cx.intents().take_set_value_at(key_of(upper)).filter(|_| !running) {
                let pair = Self::current(cx);
                let (low, high) = Self::bounds(&scale, placed(&scale, &pair), upper);
                cx.set(with(pair, upper, scale.snap(next).clamp(low, high)));
            }
        }
    }

    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        let pair = Self::current(cx);
        pair.0.same()?;
        pair.1.same()?;
        let scale = self.track_scale();
        let (from, to) = placed(&scale, &pair);
        let units = cx.env.units();
        let words = cx.words::<ValueDragWords>();
        Some(words.range.fill(&[("from", scale.text(&units, from).as_str()), ("to", scale.text(&units, to).as_str())]))
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let scale = self.track_scale();
        let words = cx.words::<ValueDragWords>();
        let pair = Self::current(cx);
        let shown = placed(&scale, &pair);
        let label = cx.resolved.label;
        for (word, upper) in [(&words.lower, false), (&words.upper, true)] {
            let node = access.node(key_of(upper), Role::SLIDER);
            node.name =
                if label.is_empty() { Name::Text(word.as_str().to_owned()) } else { Name::Joined(vec![label.to_owned(), word.as_str().to_owned()]) };
            let (low, high) = Self::bounds(&scale, shown, upper);
            let end = if upper { &pair.1 } else { &pair.0 };
            node.numeric = Some(
                match end.same() {
                    Some(_) => Numeric::new(if upper { shown.1 } else { shown.0 }, low..=high),
                    None => Numeric::empty(low..=high),
                }
                .step(scale.small()),
            );
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let flow = cx.flow;
        let scale = self.track_scale();
        let along = self.along(cx.rect, style, flow);
        let pair = Self::current(cx);
        let (lower, upper) = placed(&scale, &pair);
        let faced = match &self.track_face {
            Some(face) => {
                face.paint(ui);
                true
            }
            None => false,
        };
        let draw = ui.draw();
        let thumbs = [(LOWER, lower, &pair.0), (UPPER, upper, &pair.1)]
            .map(|(key, value, end)| (key, Pos2::new(along.x(scale.position(value)), along.area.center().y), end.same().is_some()));
        if !faced {
            let reach = |centre: Pos2| (centre.x - flow.start_x()).abs();
            let both = thumbs[0].2 && thumbs[1].2;
            let fill = (both && self.fill).then(|| Fill { color: cx.resolve(&style.track.filled), from: reach(thumbs[0].1), to: reach(thumbs[1].1) });
            track::paint(&draw, &style.track, along, cx.resolve(&style.track.unfilled), fill);
        }
        let ink = MarkInk { marks: cx.resolve(&style.track.mark_color), labels: cx.resolve(&style.mark_text.color), role: style.mark_text.role };
        track::paint_marks(&draw, &style.track, &self.marks, &scale, along, cx.rect, &ink);
        for (key, centre, _) in thumbs {
            let drawn = Rect::from_center_size(centre, Vec2::new(style.thumb_width, style.thumb_height));
            let shape = PaintShape::Rect(drawn, style.thumb_corner);
            draw.fill(&shape, cx.resolve_part(key, &style.thumb));
            draw.stroke(&shape, StrokeWidth::Thin, cx.resolve_part(key, &style.thumb_edge));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<(Mixed<f64>, Mixed<f64>)> {
        cx.edit_response()
    }
}
