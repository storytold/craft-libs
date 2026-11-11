//! Stories of `NamedPresetRow`.

use super::{NamedPresetRow, PresetError};
use crate::contract::Bind;
use crate::list::ThumbAspect;
use crate::patterns::presets::{PickerField, PresetPicker, PresetRow, PresetState};
use crate::testing::pictures::paint_contour;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const LINEAR: u32 = 1;
const MINE: u32 = 2;

#[story(NamedPresetRow<'static, u32>, covers = [], drive = Drive::Never)]
fn named_preset_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let thumb = |row: PresetRow<u32>, n: u32| row.preview(ThumbAspect::SQUARE, move |cx, rect| paint_contour(cx, rect, n));
    let rows = vec![thumb(PresetRow::item(LINEAR, "Linear", 0), 0), thumb(PresetRow::item(MINE, "Mine", 0), 1)];
    let (contour, state) = data.value("contour", (Some(MINE), PresetState::<u32>::new()));
    let picker = PresetPicker::new("Contour", Bind::bound(contour), &rows, Bind::bound(state)).field(PickerField::ThumbnailAndName);
    NamedPresetRow::new("Contour preset", picker)
        .suggest_name("Contour 3")
        .check_name(|name| name.eq_ignore_ascii_case("linear").then_some(PresetError::NameTaken))
        .built_in(|key| *key == LINEAR)
        .show(ui)
        .into()
}
