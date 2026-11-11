//! Stories of the transform fields: the panel form, and the options-bar form in percent and in a length unit.

use super::{TransformFields, TransformRow, size_unit_of};
use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::Rect;
use craft_behavior::{RulerUnits, Ticks};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::RulerText;
use craft_ui::contract::{Bind, Entry, ShowContainer as _};
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
    let state = data.value("state", sample());
    TransformFields::new("Transform", Bind::bound(&mut state.0), START, Bind::bound(&mut state.1)).show(ui).into()
}

#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn bar_form_in_percent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("state", sample());
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            TransformFields::new("Transform", Bind::bound(&mut state.0), START, Bind::bound(&mut state.1)).bar().show(ui);
        })
        .output
        .into()
}

#[story(TransformFields<'static>, covers = [], drive = Drive::Never)]
fn bar_form_in_a_length_unit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("state", sample());
    let units = [Entry::new(UnitKind::Percent, "Percent"), Entry::new(UnitKind::Pixels, "Pixels"), Entry::new(UnitKind::Inches, "Inches")];
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            TransformFields::new("Transform", Bind::bound(&mut state.0), START, Bind::bound(&mut state.1))
                .bar()
                .units(inches(), inches())
                .size_unit(size_unit_of(UnitKind::Inches, inches(), UnitKind::POINTS_PER_INCH))
                .size_units(&units)
                .show(ui);
        })
        .output
        .into()
}
