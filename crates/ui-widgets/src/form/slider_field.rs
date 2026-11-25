//! `SliderField`: a slider and a number field over one value.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::contract::{
    Bind, BindCheck, BindMut, Common, Component, Cx, Edit as _, EditResponse, FieldLayout, MeasureCx, Outcome, PaintUi, Props as _, Role, SideLabel,
    Sizing, Validity, WordSetCx as _,
};
use crate::runtime::env::{Label, UnitKind};
use crate::style::{ComponentStyle, StyleCx};
use crate::text::{NumberField, RangeAlert};
use crate::value_drag::{Slider, SliderScale, SliderThumb, TrackFace, TrackPress};
use craft_behavior::{Flow, Metrics, Mixed, Proposal, Sense, decimals_for_step};
use egui::{Rect, Response, Vec2, pos2};
use std::ops::{Deref, RangeInclusive};

/// The step of the number field when the field sets none: hundredths on a short range, whole numbers on a long one.
const SHORT_RANGE: f64 = 10.0;
const SHORT_STEP: f64 = 0.01;
const LONG_STEP: f64 = 1.0;

/// How a slider field arranges its caption, number field and slider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SliderFieldLayout {
    /// The caption at the start and the number field at the end of one line (the label step's `Line` placement), the
    /// slider across the full width on the next.
    #[default]
    Stacked,
    /// The slider and then the number field on one line, for a row whose enclosing `Field` shows the label (a
    /// Photoshop dialog row: "Opacity: slider, field, unit").
    Inline,
}

/// The style of a slider field: the gap between its parts, the line height and the stacked number field's width.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SliderFieldStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The gap between the number field and the slider on one line.
    inline_gap: f32,
    /// The height of the caption line.
    row: f32,
    /// The width of a stacked row with no more room offered: the same for every row, so a column of rows lines up.
    width: f32,
}

/// A caption and number field over a full-width slider, on one value. A drag on the slider, a step key, typing in the field or a
/// scrub by the enclosing `Field`'s label is one edit stream (begin, updates, commit) of the one binding.
///
/// The step moves the slider's keys and wheel and the field's keys, wheel and scrub; typed values snap to it unless
/// `typed_decimals` sets their independent precision. Without a step the field steps by a hundredth on a range 10 wide
/// or less and by 1 on a longer one, and by default rounds typed values to the places that step needs. The field and
/// then the slider are Tab stops. A log or power scale curves the
/// slider's track only: the field, typing and steps stay linear.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct SliderField<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f64>,
    range: RangeInclusive<f64>,
    /// Whether the caption and the number field share a line over the slider, or the number field comes first on the
    /// slider's line for a row labelled by an enclosing `Field`.
    #[prop]
    layout: SliderFieldLayout,
    /// How positions along the slider's track map to values.
    #[prop]
    scale: SliderScale,
    /// The change of one arrow press, wheel notch or label scrub point, and by default the grid typed values snap to.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// The decimal places retained and shown for typed values (at most 12), independently of the step; typing no longer snaps to it.
    #[prop]
    typed_decimals: Option<usize>,
    /// The change of one wheel notch over the slider where it is coarser than the step.
    #[prop(quantity = "step")]
    wheel_step: Option<f64>,
    /// When a press on the slider's track sets the value (`Slider::track_press`): on its release or drag by default.
    #[prop]
    track_press: TrackPress,
    /// What the number counts: the field shows its unit and reads its spellings.
    #[prop]
    unit_kind: Option<UnitKind>,
    /// A unit shown after the number, for a quantity with no `UnitKind` (a font axis).
    #[prop(into)]
    unit: Option<Label>,
    /// Whether the part of the track before the thumb is filled.
    #[prop]
    fill: bool,
    /// The look of the track in place of the groove and the fill (a colour gradient).
    #[prop]
    track_face: Option<TrackFace<'a>>,
    /// The shape of the slider's thumb: a knob, or the pointer under the track of a colour channel.
    #[prop]
    thumb: SliderThumb,
    /// The number field's sizing on the `Inline` line: `Sizing::Content` sizes it to its widest value (a dense panel
    /// row, the Color panel's channels) instead of the number cell, and the slider takes the room it leaves.
    #[prop]
    field_sizing: Option<Sizing>,
    /// The marks under the slider's track: one list, the last of `marks` and `marks_labelled` wins.
    marks: Option<FieldMarks>,
    /// The slider asks for a short length ([`Slider::short`]): with a content-sized number field, the line is as wide as
    /// a short field and the number, as production's Opacity, not the room it is offered.
    #[prop]
    short: bool,
    /// The number field selects its whole value when it takes focus, so typing replaces it
    /// ([`NumberField::select_on_focus`]): a click on the value selects it, a drag across it keeps the range.
    #[prop(flag)]
    select_on_focus: bool,
    /// The number field's width from this pass's measure (its `Proposal::Ideal` call, which runs before `paint`).
    field_width: f32,
    /// The field's range alert of this pass, which `finish` reports.
    out_of_range: Option<RangeAlert>,
}

/// What a slider field reports: the edit of its value and its field's range alert.
#[non_exhaustive]
pub struct SliderFieldOutput {
    /// The edit response of the value.
    pub edit: EditResponse<f64>,
    /// A value typed into the field outside the range and clamped, in the pass it committed; never for a slider drag
    /// or a step.
    pub out_of_range: Option<RangeAlert>,
}

impl Deref for SliderFieldOutput {
    type Target = EditResponse<f64>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl Outcome for SliderFieldOutput {
    type Value = f64;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl<'a> SliderField<'a> {
    /// A slider and number field named `label` over `value` in `range`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, f64>, range: RangeInclusive<f64>) -> Self {
        SliderField {
            common: Common::new(),
            value,
            range,
            layout: SliderFieldLayout::Stacked,
            scale: SliderScale::Linear,
            step: None,
            typed_decimals: None,
            wheel_step: None,
            track_press: TrackPress::OnRelease,
            unit_kind: None,
            unit: None,
            fill: true,
            track_face: None,
            thumb: SliderThumb::Round,
            field_sizing: None,
            marks: None,
            short: false,
            select_on_focus: false,
            field_width: 0.0,
            out_of_range: None,
        }
        .label(label)
    }

    /// Values the field's slider snaps to, drawn as marks under the track: [`Slider::marks`].
    #[must_use]
    pub fn marks(mut self, marks: &[f64]) -> Self {
        self.marks = Some(FieldMarks::Plain(marks.to_vec()));
        self
    }

    /// [`marks`](Self::marks) at the values, each with a caption centred under its mark: [`Slider::marks_labelled`].
    #[must_use]
    pub fn marks_labelled<L: Into<Label>>(mut self, marks: impl IntoIterator<Item = (f64, L)>) -> Self {
        self.marks = Some(FieldMarks::Labelled(marks.into_iter().map(|(at, caption)| (at, caption.into())).collect()));
        self
    }

    /// What the children are built from, apart from the binding and the track face.
    fn look(&self) -> Look {
        Look {
            track_press: self.track_press,
            range: self.range.clone(),
            scale: self.scale,
            step: self.step,
            typed_decimals: self.typed_decimals,
            wheel_step: self.wheel_step,
            unit_kind: self.unit_kind,
            unit: self.unit.clone(),
            fill: self.fill,
            thumb: self.thumb,
            reset: self.value.reset_value().copied(),
            field_sizing: self.field_sizing,
            marks: self.marks.clone(),
            short: self.short,
            select_on_focus: self.select_on_focus,
        }
    }
}

/// The marks of a slider field's slider.
#[derive(Clone)]
enum FieldMarks {
    /// Marks the thumb snaps to.
    Plain(Vec<f64>),
    /// Marks with a caption under each.
    Labelled(Vec<(f64, Label)>),
}

/// What a `SliderField` builds its two children from.
struct Look {
    track_press: TrackPress,
    range: RangeInclusive<f64>,
    scale: SliderScale,
    step: Option<f64>,
    typed_decimals: Option<usize>,
    wheel_step: Option<f64>,
    unit_kind: Option<UnitKind>,
    unit: Option<Label>,
    fill: bool,
    thumb: SliderThumb,
    /// The reset value of the composite's own binding, which both children restore on a double click.
    reset: Option<f64>,
    field_sizing: Option<Sizing>,
    marks: Option<FieldMarks>,
    short: bool,
    select_on_focus: bool,
}

/// The composite's check, which its children check their values with.
type Check<'c> = Option<BindCheck<'c, f64>>;

impl Look {
    /// The step the host set, when it is a usable one.
    fn set_step(&self) -> Option<f64> {
        self.step.filter(|step| step.is_finite() && *step > 0.0)
    }

    fn field_step(&self) -> f64 {
        self.set_step().unwrap_or(if (self.range.end() - self.range.start()).abs() <= SHORT_RANGE { SHORT_STEP } else { LONG_STEP })
    }

    /// The slider showing `shown`. Its check is the composite's, so a value the composite refuses or warns about
    /// looks invalid or warned on the slider too, and a value it fixes is fixed before the composite adopts it.
    fn slider<'c>(&self, name: &Label, shown: Mixed<f64>, check: Check<'c>, face: Option<TrackFace<'c>>) -> Slider<'c> {
        let mut bind = Bind::given(shown);
        if let Some(reset) = self.reset {
            bind = bind.reset(reset);
        }
        if let Some(check) = check {
            bind = bind.check_with(check, BindCheck::validity);
        }
        let slider = Slider::new(bind, self.range.clone())
            .track_press(self.track_press)
            .scale(self.scale)
            .fill(self.fill)
            .thumb(self.thumb)
            .short(self.short)
            .label(name.clone());
        let slider = match &self.marks {
            Some(FieldMarks::Plain(marks)) => slider.marks(marks),
            Some(FieldMarks::Labelled(marks)) => slider.marks_labelled(marks.iter().cloned()),
            None => slider,
        };
        let slider = match face {
            Some(face) => slider.track_face(face),
            None => slider,
        };
        let slider = match self.set_step() {
            Some(step) => slider.step(step),
            None => slider,
        };
        match self.wheel_step {
            Some(step) => slider.wheel_step(step),
            None => slider,
        }
    }

    /// The number field showing `shown`, with the composite's check as the slider has it.
    fn field<'c>(&self, name: &Label, shown: Mixed<f64>, check: Check<'c>) -> NumberField<'c> {
        let number = match shown {
            Mixed::Same(value) => Mixed::Same(Some(value)),
            Mixed::Mixed => Mixed::Mixed,
        };
        let mut bind = Bind::given(number);
        if let Some(reset) = self.reset {
            bind = bind.reset(Some(reset));
        }
        if let Some(check) = check {
            bind = bind.check_with(check, |check, value| value.map_or(Validity::Ok, |value| lifted(check.validity(&value))));
        }
        let step = self.field_step();
        let typed_decimals = self.typed_decimals.map(|places| places.min(12));
        let field = NumberField::new(name.clone(), bind)
            .range(self.range.clone())
            .step(step)
            .snap_typed(typed_decimals.is_none() && self.set_step().is_some())
            .commit_decimals(typed_decimals.unwrap_or_else(|| decimals_for_step(step)))
            .scrub_by_label();
        let field = match typed_decimals {
            Some(decimals) => field.max_decimals(decimals),
            None => field,
        };
        let field = if self.select_on_focus { field.select_on_focus() } else { field };
        let field = match self.unit_kind {
            Some(kind) => field.unit_kind(kind),
            None => field,
        };
        let field = match self.field_sizing {
            Some(sizing) => field.sizing(sizing),
            None => field,
        };
        match &self.unit {
            Some(unit) => field.unit(unit.clone()),
            None => field,
        }
    }
}

/// The composite's verdict on a number as a number field's, which reads `Option<f64>`.
fn lifted(verdict: Validity<f64>) -> Validity<Option<f64>> {
    match verdict {
        Validity::Fix(number) => Validity::Fix(Some(number)),
        Validity::Warn(message) => Validity::Warn(message),
        Validity::Refuse(message) => Validity::Refuse(message),
        Validity::Error(message) => Validity::Error(message),
        _ => Validity::Ok,
    }
}

impl Component for SliderField<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = SliderFieldStyle;
    type Output = SliderFieldOutput;

    fn style(cx: &StyleCx<'_>, _: SliderFieldStyleKey) -> SliderFieldStyle {
        let control = cx.control();
        SliderFieldStyle {
            metrics: Metrics::new().gap(control.gap),
            inline_gap: cx.tokens.space.gap.section,
            row: control.height,
            width: control.field,
        }
    }

    /// A short slider keeps to its own length; otherwise the line takes the width it is offered.
    fn fills(&self) -> Option<craft_behavior::Axis> {
        (!self.short).then_some(craft_behavior::Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let shown = shown_of(cx.value());
        let name = cx.label();
        let look = self.look();
        let slider = cx.child("slider", &mut look.slider(&name, shown, None, None), Proposal::Ideal);
        let field = cx.child("field", &mut look.field(&name, shown, None), Proposal::Ideal);
        if cx.proposal == Proposal::Ideal {
            self.field_width = field.x;
        }
        match self.layout {
            SliderFieldLayout::Inline => Vec2::new(slider.x + cx.style.inline_gap + field.x, slider.y.max(field.y)),
            SliderFieldLayout::Stacked => Vec2::new(slider.x.max(cx.style.width), cx.style.row.max(field.y) + cx.gap() + slider.y),
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    /// A host that keeps the value in `f32` writes the proposal back rounded to it: that is the gesture's own value.
    fn same_value(&self, proposed: &f64, host: &f64) -> bool {
        craft_behavior::same_in_f32(*proposed, *host)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let shown = shown_of(cx.value());
        let name = cx.label();
        let look = self.look();
        let (track, field) = match self.layout {
            // The field takes the width its own measure gives: a number cell, or its widest value and unit when
            // content-sized, which in a column group are the group's widest. So every row of the column ends its slider
            // at one place.
            SliderFieldLayout::Inline => arrange(cx.rect.size(), cx.style.inline_gap, self.field_width),
            SliderFieldLayout::Stacked => stack(cx.rect.size(), cx.style.metrics.gap, cx.style.row),
        };
        let flow = Flow::new(cx.rect, cx.env.direction().is_rtl());
        let place = |logical: Rect| flow.to_physical(logical.translate(cx.rect.min.to_vec2()));
        // The field is shown first, so it is the first Tab stop.
        // The group has no stop, so its tooltip, and with it the reason it is disabled, is carried by the first stop.
        let face = self.track_face.take();
        let bind = self.value.reborrow();
        let field_view = look.field(&name, shown, Some(bind.check_of()));
        let field_view = match cx.resolved.tooltip {
            Some(tooltip) => field_view.tooltip(tooltip.clone()),
            None => field_view,
        };
        let typed = match self.layout {
            // The label step puts the caption at the line's start and the field in the number cell at its end.
            SliderFieldLayout::Stacked => {
                ui.labelled("field", place(field), name.clone(), SideLabel::new().layout(FieldLayout::Line).scrub(true), field_view)
            }
            SliderFieldLayout::Inline => ui.child("field", place(field), field_view),
        };
        self.out_of_range = typed.out_of_range.clone();
        // A read-only field's track is read-only too: no press on it edits the value, whatever `track_press` is, and an
        // inert composite arms no first-Down gesture in its child for a later writable release.
        let slider = look.slider(&name, shown, Some(bind.check_of()), face).read_only(cx.resolved.read_only);
        let dragged = ui.child("slider", place(track), slider);
        let start = *self.range.start();
        cx.adopt(&typed, &start, |value, mine| {
            if let Some(value) = value {
                *mine = *value;
            }
        });
        cx.adopt(&dragged, &start, |value, mine| *mine = *value);
    }

    fn finish(self, cx: Cx<'_, Self>) -> SliderFieldOutput {
        SliderFieldOutput { edit: cx.edit_response(), out_of_range: self.out_of_range }
    }
}

/// The value the children show: none while the binding is mixed.
fn shown_of(shown: Option<&f64>) -> Mixed<f64> {
    shown.map_or(Mixed::Mixed, |value| Mixed::Same(*value))
}

/// The slider's and the number field's rects in a room of `size`, in logical coordinates from the start edge: the
/// field at the end, the slider in the room before it.
fn arrange(size: Vec2, gap: f32, field_width: f32) -> (Rect, Rect) {
    let field = field_width.min(size.x);
    let track = (size.x - field - gap).max(0.0);
    let rect = |x: f32, width: f32| Rect::from_min_size(pos2(x, 0.0), Vec2::new(width, size.y));
    (rect(0.0, track), rect(size.x - field, field))
}

/// The first line's and the slider's rects in a room of `size`, in logical coordinates from the start edge: a line of
/// height `row` for the caption and the number field, and the slider across the room below it.
fn stack(size: Vec2, gap: f32, row: f32) -> (Rect, Rect) {
    let below = row + gap;
    let line = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(size.x, row));
    (Rect::from_min_size(pos2(0.0, below), Vec2::new(size.x, (size.y - below).max(0.0))), line)
}
