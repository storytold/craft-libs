//! Stories of `PresetBrowser`.

use super::{PresetBrowser, PresetReorder};
use crate::choice::SwatchFace;
use crate::contract::Bind;
use crate::list::ThumbAspect;
use crate::patterns::presets::{PresetFooterOptions, PresetRow, PresetState};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::Srgba;
use egui::Ui;

/// Two groups and a loose preset: the second group holds a folder.
fn sample_rows() -> Vec<PresetRow<u32>> {
    let thumb = |row: PresetRow<u32>, hex: &str| {
        let color = Srgba::from_hex(hex).unwrap_or(Srgba::BLACK);
        row.preview(ThumbAspect::SQUARE, move |cx, rect| SwatchFace::new(color).paint(cx, rect))
    };
    vec![
        thumb(PresetRow::item(1, "Loose", 0), "#808080"),
        PresetRow::group(10, "Warm", 0).count(2),
        thumb(PresetRow::item(11, "Amber", 1), "#FFBF00").caption("#FFBF00"),
        thumb(PresetRow::item(12, "Coral", 1), "#FF7F50").caption("#FF7F50"),
        PresetRow::group(20, "Cool", 0).count(2),
        thumb(PresetRow::item(21, "Teal", 1), "#008080").caption("#008080"),
        PresetRow::group(30, "Pale", 1).count(1),
        thumb(PresetRow::item(31, "Mist", 2), "#E6EEF2").caption("#E6EEF2"),
    ]
}

#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = sample_rows();
    let slider = *data.value("slider", true);
    let new_group = *data.value("new_group", true);
    PresetBrowser::new("Swatches", Bind::bound(data.value("state", PresetState::<u32>::new())), &rows)
        .selected(11)
        .toolbar(true)
        .reorder(PresetReorder::items_and_groups().nest(1))
        .footer(PresetFooterOptions::new().slider(slider).new_group(new_group))
        .show(ui)
        .into()
}
