//! Stories of `PresetPicker`.

use super::{PickerField, PresetPicker};
use crate::choice::SwatchFace;
use crate::contract::{Bind, Props as _, Role, Show as _, Validity};
use crate::list::ThumbAspect;
use crate::patterns::presets::{PresetRow, PresetState};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use egui::Ui;

const AMBER: u32 = 11;
const CORAL: u32 = 12;
const TEAL: u32 = 21;

/// Two groups of swatches.
fn rows() -> Vec<PresetRow<u32>> {
    let thumb = |row: PresetRow<u32>, hex: &str| {
        let color = Srgba::from_hex(hex).unwrap_or(Srgba::BLACK);
        row.preview(ThumbAspect::SQUARE, move |cx, rect| SwatchFace::new(color).paint(cx, rect))
    };
    vec![
        PresetRow::group(10, "Warm", 0).count(2),
        thumb(PresetRow::item(AMBER, "Amber", 1), "#FFBF00"),
        thumb(PresetRow::item(CORAL, "Coral", 1), "#FF7F50"),
        PresetRow::group(20, "Cool", 0).count(1),
        thumb(PresetRow::item(TEAL, "Teal", 1), "#008080"),
    ]
}

#[story(PresetPicker<'static, u32>, covers = [Default], drive = Drive::Never)]
fn preset_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = rows();
    let (swatch, state) = data.value("swatch", (Some(AMBER), PresetState::<u32>::new()));
    PresetPicker::new("Swatch", Bind::bound(swatch), &rows, Bind::bound(state)).field(PickerField::ThumbnailAndName).show(ui).into()
}

#[story(PresetPicker<'static, u32>, covers = [Mixed], drive = Drive::Never)]
fn preset_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = rows();
    let (swatch, state) = data.value("mixed", (Mixed::<Option<u32>>::Mixed, PresetState::<u32>::new()));
    PresetPicker::new("Swatch", Bind::bound_mixed(swatch), &rows, Bind::bound(state)).show(ui).into()
}

#[story(PresetPicker<'static, u32>, covers = [Invalid], drive = Drive::Never)]
fn preset_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = rows();
    let (swatch, state) = data.value("invalid", (Some(TEAL), PresetState::<u32>::new()));
    let refuse_teal = |swatch: &Option<u32>| match swatch {
        Some(TEAL) => Validity::Refuse(Message::new("Teal is not available here")),
        _ => Validity::Ok,
    };
    PresetPicker::new("Swatch", Bind::bound(swatch).check(refuse_teal), &rows, Bind::bound(state)).show(ui).into()
}

fn open_browser(fixture: &mut Fixture) {
    if let Some(field) = fixture.by_role(Role::COMBO_BOX).first().copied() {
        fixture.click(&field);
    }
}

#[story(PresetPicker<'static, u32>, covers = [Open], drive = Drive::Custom(open_browser))]
fn preset_picker_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = rows();
    let (swatch, state) = data.value("open", (Some(CORAL), PresetState::<u32>::new()));
    PresetPicker::new("Swatch", Bind::bound(swatch), &rows, Bind::bound(state)).show(ui).into()
}

#[story(PresetPicker<'static, u32>, covers = [], disabled_text = "Pick a layer first")]
fn preset_picker_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = rows();
    let tooltip = data.tooltip("The swatch of the layer");
    let (swatch, state) = data.value("reason", (None::<u32>, PresetState::<u32>::new()));
    PresetPicker::new("Swatch", Bind::bound(swatch), &rows, Bind::bound(state)).field(PickerField::Name).tooltip(tooltip).show(ui).into()
}
