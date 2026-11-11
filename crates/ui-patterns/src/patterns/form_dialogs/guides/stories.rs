//! Stories of the guide dialogs.

use super::{GuideLayoutDialog, GuideLayoutValues, GuideRect, GuideUnits, NewGuideDialog, NewGuideValues};
use crate::canvas_data::{Guide, GuideColor, GuideKey};
use crate::contract::Bind;
use crate::runtime::env::UnitKind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Axis;
use craft_theme::Rgba;
use egui::Ui;

/// A 3000 by 2000 pixel document at 300 pixels per inch.
const WIDTH: f64 = 3000.0;
const HEIGHT: f64 = 2000.0;
const RESOLUTION: f64 = 300.0;
/// Where the edited guide stands, in pixels.
const EDITED_AT: f64 = 750.0;
/// The guide colour preference the edited guide is drawn in.
const PREFERENCE: Rgba = Rgba::rgb(0x4a, 0xff, 0xff);

const RULERS: [(&str, UnitKind); 4] =
    [("Pixels", UnitKind::Pixels), ("Inches", UnitKind::Inches), ("Centimetres", UnitKind::Centimetres), ("Millimetres", UnitKind::Millimetres)];

/// The units of the `ruler` value, centimetres at first.
fn units(data: &mut StoryData) -> GuideUnits {
    GuideUnits::new(*data.choice("ruler", UnitKind::Centimetres, &RULERS), WIDTH, HEIGHT, RESOLUTION)
}

#[story(NewGuideDialog<'static>, covers = [], drive = Drive::Never)]
fn new_guide_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units(data);
    let values = data.value("values", NewGuideValues::default());
    NewGuideDialog::new(units, Bind::bound(values)).opened(Bind::given(true)).show(ui).into()
}

#[story(NewGuideDialog<'static>, covers = [], drive = Drive::Never)]
fn new_guide_dialog_edit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units(data);
    let axis = if *data.value("horizontal_guide", true) { Axis::Y } else { Axis::X };
    let mut guide = Guide::new(GuideKey::new(1), axis, EDITED_AT);
    guide.color = Some(GuideColor::Magenta.rgba());
    let values = data.value("values", NewGuideValues::of_guide(&guide, PREFERENCE));
    NewGuideDialog::edit(units, guide, PREFERENCE, Bind::bound(values)).opened(Bind::given(true)).show(ui).into()
}

#[story(GuideLayoutDialog<'static>, covers = [], drive = Drive::Never)]
fn guide_layout_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units(data);
    let existing = *data.value("existing_guides", false);
    let artboards = *data.value("artboards", false);
    let values = data.value("values", GuideLayoutValues::default());
    let document = GuideRect::new(0.0, 0.0, WIDTH, HEIGHT);
    let mut dialog = GuideLayoutDialog::new(units, document, Bind::bound(values)).opened(Bind::given(true));
    if existing {
        dialog = dialog.existing(vec![Guide::new(GuideKey::new(1), Axis::X, WIDTH / 2.0), Guide::new(GuideKey::new(2), Axis::Y, HEIGHT / 2.0)]);
    }
    if artboards {
        dialog = dialog.artboards(vec![GuideRect::new(200.0, 200.0, 1400.0, 1800.0), GuideRect::new(1600.0, 200.0, 2800.0, 1800.0)]);
    }
    dialog.show(ui).into()
}
