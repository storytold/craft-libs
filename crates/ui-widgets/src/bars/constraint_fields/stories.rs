//! Stories of `ConstraintFields`.

use super::ConstraintFields;
use crate::bars::{Bar, BarKind};
use crate::contract::{Bind, Entry, ShowContainer as _};
use crate::runtime::env::{Env, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use craft_behavior::geometry::Constraint;
use egui::Ui;

const PIXELS_PER_INCH: f64 = 300.0;

/// The cells in an options bar over the story's constraint, starting at `start`; `units` gives Fixed Size units. The
/// readout names the constraint and the unit the fields show, as a host reads them.
fn bar(ui: &mut Ui, data: &mut StoryData, start: Constraint, units: bool, enabled: bool) -> StoryOutput {
    let list = [
        Entry::new(UnitKind::Pixels, "Pixels"),
        Entry::new(UnitKind::Inches, "Inches"),
        Entry::new(UnitKind::Centimetres, "Centimetres"),
        Entry::new(UnitKind::Percent, "Percent"),
    ];
    let unit = *data.value("unit", if units { UnitKind::Inches } else { UnitKind::Pixels });
    let value = data.value("constraint", start);
    let mut picked = None;
    let shown = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        let fields = ConstraintFields::new(Bind::bound(&mut *value)).enabled(enabled);
        let fields = if units { fields.size_units(unit, &list, PIXELS_PER_INCH, PixelSize::new(1920, 1080)) } else { fields };
        picked = fields.show(ui).unit_picked;
    });
    let constraint = *value;
    if let Some(kind) = picked {
        *data.value("unit", unit) = kind;
    }
    let symbol = Env::of(ui).units().symbol(picked.unwrap_or(unit));
    let readout = match constraint {
        Constraint::Ratio(w, h) => format!("Fixed Ratio: {w} x {h}\nUnit: {symbol}"),
        Constraint::Size(w, h) => format!("Fixed Size: {w} x {h} {symbol}\nUnit: {symbol}"),
        _ => format!("Normal: - x -\nUnit: {symbol}"),
    };
    StoryOutput::from(shown.output).readout(readout)
}

#[story(ConstraintFields<'static>, covers = [], drive = Drive::Never)]
fn normal(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, Constraint::Free, false, true)
}

#[story(ConstraintFields<'static>, covers = [], drive = Drive::Never)]
fn fixed_ratio(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, Constraint::Ratio(16.0, 9.0), false, true)
}

#[story(ConstraintFields<'static>, covers = [], drive = Drive::Never)]
fn fixed_size_with_units(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, Constraint::Size(600.0, 300.0), true, true)
}

#[story(ConstraintFields<'static>, covers = [], drive = Drive::Never)]
fn greyed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, Constraint::Ratio(1.0, 1.0), false, false)
}
