//! [`StrokeCells`]: the stroke width cell and the Stroke style button, shared by the Shape bar, the Appearance section
//! of the panel and the Pen tools' Shape mode.

use super::panel::NumberCell;
use super::{ShapeAppearance, ShapeChange, ShapeWords};
use crate::ui::stroke::StrokeButton;
use craft_behavior::{EditEvent, EditEvents};
use craft_ui::contract::{Bind, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::form::Field;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::text::RangeAlert;
use egui::Ui;

/// The largest stroke width in pixels.
const STROKE_WIDTH_MAX: f64 = 288.0;

/// The stroke width cell of `appearance`: 0 to 288 px.
pub(super) fn width_cell(words: &ShapeWords, units: &Units, pixels_per_inch: f64, appearance: &ShapeAppearance) -> NumberCell {
    NumberCell::pixels(units, pixels_per_inch, "stroke-width", &words.stroke_width, appearance.stroke_width, 0.0..=STROKE_WIDTH_MAX)
}

/// Adds the stroke width field's events to `out`. An arrow step that stays at 0 changes nothing, so it adds no stroke.
pub(super) fn take_width(appearance: &ShapeAppearance, events: EditEvents<f64>, out: &mut Vec<EditEvent<ShapeChange>>) {
    for event in events {
        let unchanged = matches!(&event, EditEvent::Set { after, .. } | EditEvent::Update { value: after, .. } if *after <= 0.0 && appearance.stroke_width <= 0.0);
        if !unchanged {
            out.push(event.map(ShapeChange::StrokeWidth));
        }
    }
}

/// Shows the Stroke style button over the appearance's stroke style and adds its events to `out`. It is off while the
/// width is 0; a pick then also carries the stroke's width in the same change.
pub(super) fn style_button(
    ui: &mut Ui,
    words: &ShapeWords,
    appearance: &ShapeAppearance,
    open: &mut Bind<'_, bool>,
    corners: bool,
    out: &mut Vec<EditEvent<ShapeChange>>,
) {
    let off = appearance.stroke_width <= 0.0;
    let style = Bind::given(appearance.stroke_style.clone());
    let button = StrokeButton::new(words.stroke_style.clone(), style, open.reborrowed()).corners(corners).stroked(!off);
    for event in button.show(ui).events {
        let width = (off && matches!(event, EditEvent::Set { .. } | EditEvent::Begin { .. })).then_some(ShapeChange::ENABLED_STROKE_WIDTH);
        out.push(event.map(|style| ShapeChange::StrokeStyle { style, width }));
    }
}

/// The stroke width cell and the Stroke style button of a shape's stroke, as one pattern shown as children of a
/// `Bar`.
///
/// - The width is 0 to 288 px; 0 is no stroke, and an edit that turns the stroke on carries
///   [`ShapeChange::ENABLED_STROKE_WIDTH`].
/// - The style button shows only with [`stroke_open`](Self::stroke_open) and is off while the width is 0.
/// - A width typed past its range is clamped and reported once in [`StrokeCellsOutput::out_of_range`].
#[must_use = "show the cells with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct StrokeCells<'a> {
    appearance: &'a ShapeAppearance,
    pixels_per_inch: f64,
    corners: bool,
    stroke_open: Option<Bind<'a, bool>>,
}

impl<'a> StrokeCells<'a> {
    /// The cells for `appearance`, in a document of `pixels_per_inch`.
    pub fn new(appearance: &'a ShapeAppearance, pixels_per_inch: f64) -> Self {
        StrokeCells { appearance, pixels_per_inch, corners: true, stroke_open: None }
    }

    /// Whether the style popover offers corner joins; off for an ellipse.
    pub fn corners(mut self, on: bool) -> Self {
        self.corners = on;
        self
    }

    /// The Stroke style popover's open flag; without it the button is not shown.
    pub fn stroke_open(mut self, open: Bind<'a, bool>) -> Self {
        self.stroke_open = Some(open);
        self
    }

    /// Adds the cells to the bar's row `ui` and reports each change.
    pub fn show(self, ui: &mut Ui) -> StrokeCellsOutput {
        let StrokeCells { appearance, pixels_per_inch, corners, mut stroke_open } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<ShapeWords>(), env.units());
        let mut events = Vec::new();
        let cell = width_cell(&words, &units, pixels_per_inch, appearance);
        let hidden = SideLabel::new().kind(LabelKind::Hidden);
        let (width, out_of_range) = Field::new(cell.caption.clone())
            .id_salt(cell.key)
            .side(hidden)
            .sizing(Sizing::Content)
            .show(ui, |ui, _| cell.show(ui))
            .inner
            .unwrap_or_default();
        take_width(appearance, width, &mut events);
        if let Some(open) = stroke_open.as_mut() {
            style_button(ui, &words, appearance, open, corners, &mut events);
        }
        StrokeCellsOutput { events, out_of_range }
    }
}

/// What [`StrokeCells::show`] reports.
#[derive(Debug, Default)]
#[non_exhaustive]
#[must_use = "the app routes each change to the selected layer or the tool settings"]
pub struct StrokeCellsOutput {
    /// Each changed cell this pass, in order.
    pub events: Vec<EditEvent<ShapeChange>>,
    /// A width typed past its range this pass, which the field clamped: the app answers with its range alert.
    pub out_of_range: Option<RangeAlert>,
}
