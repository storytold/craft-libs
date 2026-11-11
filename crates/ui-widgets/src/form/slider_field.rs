//! `SliderField`: a slider and a number field over one value.

#[cfg(feature = "testing")]
mod stories;

use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, FieldLayout, MeasureCx, PaintUi, Props as _, Role, SideLabel, Validity,
    WordSetCx as _,
};
use crate::runtime::env::{Label, UnitKind};
use crate::style::{ComponentStyle, StyleCx};
use crate::text::NumberField;
use crate::value_drag::{Slider, SliderScale, TrackFace};
use craft_behavior::{Flow, Metrics, Mixed, Proposal, Sense, decimals_for_step};
use egui::{Rect, Vec2, pos2};
use std::ops::RangeInclusive;

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
    /// The number field and then the slider on one line, for a row whose enclosing `Field` shows the label.
    Inline,
}

/// The style of a slider field: the gap between its parts, the line height and the number field's width.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SliderFieldStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The width of the number field, the same for every row so a column of fields lines up.
    field: f32,
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
/// The step moves the slider's keys and wheel and the field's keys, wheel and scrub; typed values snap to it. Without
/// a step the field steps by a hundredth on a range 10 wide or less and by 1 on a longer one, and rounds typed values
/// to the places that step needs. The field and then the slider are Tab stops. A log or power scale curves the
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
    /// The change of one arrow press, wheel notch or label scrub point, and the grid typed values snap to.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// The change of one wheel notch over the slider where it is coarser than the step.
    #[prop(quantity = "step")]
    wheel_step: Option<f64>,
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
            wheel_step: None,
            unit_kind: None,
            unit: None,
            fill: true,
            track_face: None,
        }
        .label(label)
    }

    /// What the children are built from, apart from the binding and the track face.
    fn look(&self) -> Look {
        Look {
            range: self.range.clone(),
            scale: self.scale,
            step: self.step,
            wheel_step: self.wheel_step,
            unit_kind: self.unit_kind,
            unit: self.unit.clone(),
            fill: self.fill,
            reset: self.value.reset_value().copied(),
        }
    }
}

/// What a `SliderField` builds its two children from.
struct Look {
    range: RangeInclusive<f64>,
    scale: SliderScale,
    step: Option<f64>,
    wheel_step: Option<f64>,
    unit_kind: Option<UnitKind>,
    unit: Option<Label>,
    fill: bool,
    /// The reset value of the composite's own binding, which both children restore on a double click.
    reset: Option<f64>,
}

/// The composite's check on a number, as a child with the same value type reads it.
type Check<'c, V> = Option<&'c dyn Fn(&V) -> Validity<V>>;

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
    fn slider<'c>(&self, name: &Label, shown: Mixed<f64>, check: Check<'c, f64>, face: Option<TrackFace<'c>>) -> Slider<'c> {
        let mut bind = Bind::given(shown);
        if let Some(reset) = self.reset {
            bind = bind.reset(reset);
        }
        if let Some(check) = check {
            bind = bind.check(check);
        }
        let slider = Slider::new(bind, self.range.clone()).scale(self.scale).fill(self.fill).label(name.clone());
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
    fn field<'c>(&self, name: &Label, shown: Mixed<f64>, check: Check<'c, Option<f64>>) -> NumberField<'c> {
        let number = match shown {
            Mixed::Same(value) => Mixed::Same(Some(value)),
            Mixed::Mixed => Mixed::Mixed,
        };
        let mut bind = Bind::given(number);
        if let Some(reset) = self.reset {
            bind = bind.reset(Some(reset));
        }
        if let Some(check) = check {
            bind = bind.check(check);
        }
        let step = self.field_step();
        let field = NumberField::new(name.clone(), bind)
            .range(self.range.clone())
            .step(step)
            .snap_typed(self.set_step().is_some())
            .commit_decimals(decimals_for_step(step))
            .scrub_by_label();
        let field = match self.unit_kind {
            Some(kind) => field.unit_kind(kind),
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
        _ => Validity::Ok,
    }
}

impl Component for SliderField<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = SliderFieldStyle;
    type Output = EditResponse<f64>;

    fn style(cx: &StyleCx<'_>, _: SliderFieldStyleKey) -> SliderFieldStyle {
        let control = cx.control();
        SliderFieldStyle {
            metrics: Metrics::new().gap(control.gap),
            field: cx.tokens.size.length.number,
            inline_gap: cx.tokens.space.gap.section,
            row: control.height,
            width: cx.tokens.size.length.field,
        }
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
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
        match self.layout {
            SliderFieldLayout::Inline => Vec2::new(slider.x + cx.style.inline_gap + cx.style.field, slider.y.max(field.y)),
            SliderFieldLayout::Stacked => Vec2::new(slider.x.max(cx.style.width), cx.style.row.max(field.y) + cx.gap() + slider.y),
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let shown = shown_of(cx.value());
        let name = cx.label();
        let (track, field) = match self.layout {
            SliderFieldLayout::Inline => arrange(cx.rect.size(), cx.style.inline_gap, cx.style.field),
            SliderFieldLayout::Stacked => stack(cx.rect.size(), cx.style.metrics.gap, cx.style.row),
        };
        let flow = Flow::new(cx.rect, cx.env.direction().is_rtl());
        let place = |logical: Rect| flow.to_physical(logical.translate(cx.rect.min.to_vec2()));
        // The field is shown first, so it is the first Tab stop.
        // The group has no stop, so its tooltip, and with it the reason it is disabled, is carried by the first stop.
        let look = self.look();
        let face = self.track_face.take();
        let bind = self.value.reborrow();
        let on_slider = |value: &f64| bind.validity(value);
        let on_field = |value: &Option<f64>| value.map_or(Validity::Ok, |value| lifted(bind.validity(&value)));
        let field_view = look.field(&name, shown, Some(&on_field));
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
        let dragged = ui.child("slider", place(track), look.slider(&name, shown, Some(&on_slider), face));
        let start = *self.range.start();
        cx.adopt(&typed, &start, |value, mine| {
            if let Some(value) = value {
                *mine = *value;
            }
        });
        cx.adopt(&dragged, &start, |value, mine| *mine = *value);
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}

/// The value the children show: none while the binding is mixed.
fn shown_of(shown: Option<&f64>) -> Mixed<f64> {
    shown.map_or(Mixed::Mixed, |value| Mixed::Same(*value))
}

/// The slider's and the number field's rects in a room of `size`, in logical coordinates from the start edge: the
/// field at the start, the slider in the room after it.
fn arrange(size: Vec2, gap: f32, field_width: f32) -> (Rect, Rect) {
    let field = field_width.min(size.x);
    let track = (size.x - field - gap).max(0.0);
    let rect = |x: f32, width: f32| Rect::from_min_size(pos2(x, 0.0), Vec2::new(width, size.y));
    (rect(field + gap, track), rect(0.0, field))
}

/// The first line's and the slider's rects in a room of `size`, in logical coordinates from the start edge: a line of
/// height `row` for the caption and the number field, and the slider across the room below it.
fn stack(size: Vec2, gap: f32, row: f32) -> (Rect, Rect) {
    let below = row + gap;
    let line = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(size.x, row));
    (Rect::from_min_size(pos2(0.0, below), Vec2::new(size.x, (size.y - below).max(0.0))), line)
}
