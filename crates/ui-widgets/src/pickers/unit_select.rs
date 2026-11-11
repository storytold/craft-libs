//! `UnitSelect`: a number field with a select of units beside it that picks the unit the field reads and writes.

use super::select::Select;
use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, MeasureCx, Outcome, PaintUi, Props as _, Role, WordSetCx as _,
};
use crate::runtime::env::{Label, UnitKind};
use crate::style::{ComponentStyle, StyleCx};
use crate::text::NumberField;
use craft_behavior::{EditEvents, Flow, Metrics, Mixed, Proposal, Sense};
use egui::{Rect, Response, Vec2, pos2};
use std::ops::{Deref, RangeInclusive};

#[cfg(feature = "testing")]
mod stories;

/// The style of a unit select: the gap between the field and the select, and the number field's width.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct UnitSelectStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The width of the number field; the select has the room after it.
    number: f32,
}

/// The output of a unit select: the number's edit, and the unit's edit when a unit was picked.
#[derive(Debug)]
#[non_exhaustive]
pub struct UnitSelectOutput {
    /// The edit of the number.
    pub edit: EditResponse<Option<f64>>,
    /// The unit the user picked this pass.
    pub unit: EditEvents<UnitKind>,
}

impl Deref for UnitSelectOutput {
    type Target = EditResponse<Option<f64>>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl Outcome for UnitSelectOutput {
    type Value = Option<f64>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// A number field with a select of units after it. The select picks the unit kind the field reads typed text in and
/// shows its number with, and the kind sets the digits and the step of the number; the number itself is the field's value, and the unit goes to its own binding, so the app
/// converts the number when the unit changes.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component]
pub struct UnitSelect<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Option<f64>>,
    unit: Bind<'a, UnitKind>,
    units: &'a [Entry<UnitKind>],
    /// The range typed, stepped and scrubbed numbers stay in.
    #[prop]
    range: Option<RangeInclusive<f64>>,
    /// The unit select's accessible name.
    #[prop(into)]
    unit_label: Option<Label>,
    /// Whether the number field takes edits; the unit select stays as the component's own enabled state leaves it.
    #[prop]
    number_enabled: Option<bool>,
    /// The unit child's event of this pass, kept from `paint` to `finish`.
    unit_event: EditEvents<UnitKind>,
}

impl<'a> UnitSelect<'a> {
    /// A number field named `label` over `value`, with a select of `units` over `unit` after it.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<f64>>, unit: Bind<'a, UnitKind>, units: &'a [Entry<UnitKind>]) -> Self {
        UnitSelect { common: Common::new(), value, unit, units, range: None, unit_label: None, number_enabled: None, unit_event: EditEvents::new() }
            .label(label)
    }

    fn number(&self, name: Label, value: Mixed<Option<f64>>, kind: Option<UnitKind>) -> NumberField<'static> {
        let field = NumberField::new(name, Bind::given(value));
        let field = match self.number_enabled {
            Some(on) => field.enabled(on),
            None => field,
        };
        let field = match &self.range {
            Some(range) => field.range(range.clone()),
            None => field,
        };
        match kind {
            Some(kind) => field.unit_kind(kind).step(kind.step()),
            None => field,
        }
    }

    fn select_name(&self, group: &Label) -> Label {
        self.unit_label.clone().unwrap_or_else(|| group.clone())
    }
}

/// What the number field shows: the value, or mixed when the component's is.
fn shown_of(value: Option<&Option<f64>>) -> Mixed<Option<f64>> {
    value.map_or(Mixed::Mixed, |value| Mixed::Same(*value))
}

impl Component for UnitSelect<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = UnitSelectStyle;
    type Output = UnitSelectOutput;

    fn style(cx: &StyleCx<'_>, _: UnitSelectStyleKey) -> UnitSelectStyle {
        UnitSelectStyle { metrics: Metrics::new().gap(cx.control().gap), number: cx.tokens.size.length.field }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let group = cx.label();
        let mut number = self.number(group.clone(), shown_of(cx.value()), None);
        let mut select = Select::new(self.select_name(&group), Bind::<UnitKind>::given(Mixed::Mixed), self.units);
        let number = cx.child("number", &mut number, Proposal::Ideal);
        let select = cx.child("unit", &mut select, Proposal::Ideal);
        Vec2::new(cx.style.number + cx.gap() + select.x, number.y.max(select.y))
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<f64>>> {
        Some(self.value.reborrow())
    }

    /// The number is shown first, so it is the first stop after the group, as it is first on screen. It reads the unit
    /// the binding holds before the pass, so a unit picked in this pass reaches it at the next one; the number's edit
    /// is adopted as the component's own.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let group = cx.label();
        let (number_rect, select_rect) = arrange(cx.rect.size(), cx.style.metrics.gap, cx.style.number);
        let flow = Flow::new(cx.rect, cx.flow.is_rtl());
        let place = |logical: Rect| flow.to_physical(logical.translate(cx.rect.min.to_vec2()));
        let kind = match self.unit.reborrow().get() {
            Some(Mixed::Same(kind)) => Some(kind),
            _ => None,
        };
        let shown = shown_of(cx.value());
        let start = cx.value().and_then(|value| *value);
        let number = ui.child("number", place(number_rect), self.number(group.clone(), shown, kind));
        cx.adopt(&number, &start, |value, mine| *mine = *value);
        let select = Select::new(self.select_name(&group), self.unit.reborrowed(), self.units);
        self.unit_event = ui.child("unit", place(select_rect), select).events;
    }

    fn finish(self, cx: Cx<'_, Self>) -> UnitSelectOutput {
        UnitSelectOutput { edit: cx.edit_response(), unit: self.unit_event }
    }
}

/// The number field's and the select's rects in a room of `size`, in logical coordinates from the start edge: the
/// number field first, the select in the room after it.
fn arrange(size: Vec2, gap: f32, number_width: f32) -> (Rect, Rect) {
    let number = number_width.min(size.x);
    let select = (size.x - number - gap).max(0.0);
    let rect = |x: f32, width: f32| Rect::from_min_size(pos2(x, 0.0), Vec2::new(width, size.y));
    (rect(0.0, number), rect(size.x - select, select))
}
