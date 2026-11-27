//! Stories of the transform fields: the panel form, and the options-bar form in percent and in a length unit.

use super::{FieldArrangement, TransformFields, TransformGroups, TransformRow, size_unit_of};
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::{Rect, Size};
use craft_behavior::{RulerUnits, Ticks};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::RulerText;
use craft_ui::contract::{Entry, ShowContainer as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The box before the transform, in document units.
const START: Rect = Rect::new(0.0, 0.0, 200.0, 100.0);

/// Document units per inch of the stories' document.
const PIXELS_PER_INCH: f64 = 72.0;

/// The box the stories start from: turned and leaning a little, so every field shows a value.
fn sample() -> (TransformRow, bool) {
    let state = BoxState::from_rect(START);
    let turned = state.with_parts(START, 0.0, {
        let mut parts = state.parts(START, 0.0);
        parts.angle = 15.0;
        parts.skew_h = 10.0;
        parts
    });
    (TransformRow::new(turned, 5.0), true)
}

fn inches() -> RulerText {
    RulerText::new(RulerUnits::new(1.0 / PIXELS_PER_INCH, Ticks::decimal(false)), UnitKind::Inches)
}

#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn panel_form(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", sample());
    TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1)).show(ui).into()
}

/// The panel form with X and W on the first row and Y and H on the second.
#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn panel_form_position_first(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", sample());
    TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1))
        .arrangement(FieldArrangement::PositionFirst)
        .show(ui)
        .into()
}

/// The panel form of a layer's properties: position and size only, percentages read against the document.
#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn panel_form_with_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", sample());
    let mut groups = TransformGroups::NONE;
    groups.reference = true;
    groups.position = true;
    groups.size = true;
    TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1))
        .groups(groups)
        .document(Size::new(1000.0, 400.0))
        .show(ui)
        .into()
}

/// The bar form with the Δ toggle on: X and Y show the reference point's offset.
#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn bar_form_relative(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", (sample().0, true, true));
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1))
                .bar()
                .relative(state_at.bind(&mut state.2))
                .show(ui);
        })
        .output
        .into()
}

#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn bar_form_in_percent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", sample());
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1)).bar().show(ui);
        })
        .output
        .into()
}

#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn bar_form_in_a_length_unit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (state, state_at) = data.value_at("state", sample());
    let units = [Entry::new(UnitKind::Percent, "Percent"), Entry::new(UnitKind::Pixels, "Pixels"), Entry::new(UnitKind::Inches, "Inches")];
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            TransformFields::new("Transform", state_at.bind(&mut state.0), START, state_at.bind(&mut state.1))
                .bar()
                .rulers(craft_ui::canvas::CanvasRulers::new(inches(), inches(), craft_behavior::kurbo::Point::ZERO))
                .size_unit(size_unit_of(UnitKind::Inches, inches(), UnitKind::POINTS_PER_INCH))
                .size_units(&units)
                .show(ui);
        })
        .output
        .into()
}
