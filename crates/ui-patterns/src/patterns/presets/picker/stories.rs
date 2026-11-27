//! Stories of `PresetPicker`.

use super::{PickerField, PresetPicker};
use crate::bars::{Bar, BarKind};
use crate::choice::SwatchFace;
use crate::contract::{Role, ShowContainer as _, Validity};
use crate::list::ThumbAspect;
use crate::patterns::presets::{PresetFns, PresetRow, PresetSource, PresetState};
use crate::primitives::FacePaint;
use crate::runtime::env::Env;
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use egui::Ui;

const AMBER: u32 = 11;
const CORAL: u32 = 12;
const TEAL: u32 = 21;

/// Two groups of swatches.
fn rows(_: &(), _: &Env) -> Vec<PresetRow<u32>> {
    let thumb = |row: PresetRow<u32>, hex: &str| {
        let color = Srgba::from_hex(hex).unwrap_or(Srgba::BLACK);
        let key = row.key.into();
        row.preview_face(ThumbAspect::SQUARE, FacePaint::new(key, move |cx, rect| SwatchFace::new(color).paint(cx, rect)))
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
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), swatch_at) = data.value_at("swatch", (Some(AMBER), PresetState::<u32>::new()));
    PresetPicker::new("Swatch", swatch_at.bind(swatch), &rows, swatch_at.bind(state)).field(PickerField::ThumbnailAndName).show(ui).into()
}

/// The compact chip in an options bar: the swatch over its caption, on the bar's line.
#[story(PresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_picker_chip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), chip_at) = data.value_at("chip", (Some(TEAL), PresetState::<u32>::new()));
    let mut output = None;
    Bar::new("Brush options", BarKind::Options).show(ui, |ui, _| {
        output = Some(
            PresetPicker::new("Swatch", chip_at.bind(&mut *swatch), &rows, chip_at.bind(&mut *state)).field(PickerField::Chip).caption("20").show(ui),
        );
    });
    output.map(StoryOutput::from).unwrap_or_default()
}

/// The chip with a caption range: it holds the width of the widest number of the range ("5000", in tabular figures),
/// so a caption that follows a dragged size ("7") moves nothing beside it on the bar.
#[story(PresetPicker<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_picker_chip_caption_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), at) = data.value_at("chip range", (Some(TEAL), PresetState::<u32>::new()));
    let mut output = None;
    Bar::new("Brush options", BarKind::Options).show(ui, |ui, _| {
        output = Some(
            PresetPicker::new("Swatch", at.bind(&mut *swatch), &rows, at.bind(&mut *state))
                .field(PickerField::Chip)
                .caption("7")
                .caption_range((1.0..=5000.0, 0))
                .show(ui),
        );
    });
    output.map(StoryOutput::from).unwrap_or_default()
}

#[story(PresetPicker<'static, u32>, covers = [Mixed], drive = Drive::Never)]
fn preset_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), mixed_at) = data.value_at("mixed", (Mixed::<Option<u32>>::Mixed, PresetState::<u32>::new()));
    PresetPicker::new("Swatch", mixed_at.bind_mixed(swatch), &rows, mixed_at.bind(state)).show(ui).into()
}

#[story(PresetPicker<'static, u32>, covers = [Invalid], drive = Drive::Never)]
fn preset_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), invalid_at) = data.value_at("invalid", (Some(TEAL), PresetState::<u32>::new()));
    let refuse_teal = |swatch: &Option<u32>| match swatch {
        Some(TEAL) => Validity::Refuse(Message::new("Teal is not available here")),
        _ => Validity::Ok,
    };
    PresetPicker::new("Swatch", invalid_at.bind(swatch).check(refuse_teal), &rows, invalid_at.bind(state)).show(ui).into()
}

fn open_browser(fixture: &mut Fixture) {
    if let Some(field) = fixture.by_role(Role::COMBO_BOX).first().copied() {
        fixture.click(&field);
    }
}

#[story(PresetPicker<'static, u32>, covers = [Open], drive = Drive::Custom(open_browser))]
fn preset_picker_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let ((swatch, state), open_at) = data.value_at("open", (Some(CORAL), PresetState::<u32>::new()));
    PresetPicker::new("Swatch", open_at.bind(swatch), &rows, open_at.bind(state)).show(ui).into()
}

#[story(PresetPicker<'static, u32>, covers = [], disabled_text = "Pick a layer first")]
fn preset_picker_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(rows));
    let tooltip = data.tooltip("The swatch of the layer");
    let ((swatch, state), reason_at) = data.value_at("reason", (None::<u32>, PresetState::<u32>::new()));
    PresetPicker::new("Swatch", reason_at.bind(swatch), &rows, reason_at.bind(state)).field(PickerField::Name).tooltip(tooltip).show(ui).into()
}
