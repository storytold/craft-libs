//! Stories of `NamedPresetRow`.

use super::{NamedPresetRow, NamedPresetStyle, PresetError};
use crate::contract::Bind;
use crate::list::ThumbAspect;
use crate::patterns::presets::{PickerField, PresetExport, PresetFiles, PresetPicker, PresetRow, PresetState};
use crate::testing::pictures::paint_contour;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const TRACE: [&str; 6] = ["Default", "High Fidelity Photo", "Low Fidelity Photo", "3 Colors", "Sketched Art", "Line Art"];

fn taken(rows: &[PresetRow<u32>], name: &str) -> Option<PresetError> {
    rows.iter().any(|row| row.label.eq_ignore_ascii_case(name)).then_some(PresetError::NameTaken)
}

/// The contour editor's row: a thumbnail picker, then Load…, Save… and New… as buttons.
#[story(NamedPresetRow<'static, u32>, covers = [], drive = Drive::Never)]
fn contour_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows: Vec<PresetRow<u32>> = (1..=4_u32)
        .map(|n| PresetRow::item(n, format!("Contour {n}"), 0).preview(ThumbAspect::SQUARE, move |cx, rect| paint_contour(cx, rect, n)))
        .collect();
    let files =
        PresetFiles::new("contours", "Contours", &["shc"]).export(|_| Some(PresetExport { file_name: "Contours.shc".to_owned(), bytes: Vec::new() }));
    let (contour, state) = data.value("contour", (None::<u32>, PresetState::<u32>::new()));
    let picker = PresetPicker::new("Contour", Bind::bound(contour), &rows, Bind::bound(state)).field(PickerField::Thumbnail);
    NamedPresetRow::new("Contour", picker)
        .style(NamedPresetStyle::Buttons)
        .suggest_name("Contour 5")
        .check_name(|name| taken(&rows, name))
        .built_in(|_| true)
        .files(&files)
        .show(ui)
        .into()
}

/// Image Trace's row: a name picker and one Manage presets menu.
#[story(NamedPresetRow<'static, u32>, covers = [], drive = Drive::Never)]
fn trace_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows: Vec<PresetRow<u32>> = (1..).zip(TRACE).map(|(n, name)| PresetRow::item(n, name, 0)).collect();
    let files = PresetFiles::new("trace-presets", "Trace presets", &["json"])
        .export(|_| Some(PresetExport { file_name: "Trace.json".to_owned(), bytes: Vec::new() }));
    let (preset, state) = data.value("preset", (Some(1_u32), PresetState::<u32>::new()));
    let picker = PresetPicker::new("Trace preset", Bind::bound(preset), &rows, Bind::bound(state)).field(PickerField::Name);
    NamedPresetRow::new("Trace preset", picker)
        .style(NamedPresetStyle::Menu)
        .suggest_name("Preset 7")
        .check_name(|name| taken(&rows, name))
        .built_in(|_| true)
        .files(&files)
        .show(ui)
        .into()
}
