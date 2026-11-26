//! Stories of `NamedPresetRow`.

use super::{NameCheck, NamedPresetRow, NamedPresetStyle, PresetError};
use crate::list::ThumbAspect;
use crate::patterns::presets::{PickerField, PresetExport, PresetFiles, PresetFns, PresetPicker, PresetRow, PresetSource, PresetState};
use crate::primitives::FacePaint;
use crate::runtime::env::Env;
use crate::testing::pictures::paint_contour;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const TRACE: [&str; 6] = ["Default", "High Fidelity Photo", "Low Fidelity Photo", "3 Colors", "Sketched Art", "Line Art"];

fn taken<const N: usize>(names: &[&str; N], name: &str) -> Option<PresetError> {
    names.iter().any(|label| label.eq_ignore_ascii_case(name)).then_some(PresetError::NameTaken)
}

/// The four contours.
fn contour_rows(_: &(), _: &Env) -> Vec<PresetRow<u32>> {
    (1..=4_u32)
        .map(|n| {
            PresetRow::item(n, format!("Contour {n}"), 0)
                .preview_face(ThumbAspect::SQUARE, FacePaint::new(n.into(), move |cx, rect| paint_contour(cx, rect, n)))
        })
        .collect()
}

/// The Image Trace presets.
fn trace_rows(_: &(), _: &Env) -> Vec<PresetRow<u32>> {
    (1..).zip(TRACE).map(|(n, name)| PresetRow::item(n, name, 0)).collect()
}

/// The contour editor's row: a thumbnail picker, then Load…, Save… and New… as buttons.
#[story(NamedPresetRow<'static, u32>, covers = [], drive = Drive::Never)]
fn contour_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(contour_rows));
    let files =
        PresetFiles::new("contours", "Contours", &["shc"]).export(|_| Some(PresetExport { file_name: "Contours.shc".to_owned(), bytes: Vec::new() }));
    let ((contour, state), contour_at) = data.value_at("contour", (None::<u32>, PresetState::<u32>::new()));
    let picker = PresetPicker::new("Contour", contour_at.bind(contour), &rows, contour_at.bind(state)).field(PickerField::Thumbnail);
    NamedPresetRow::new("Contour", picker)
        .style(NamedPresetStyle::Buttons)
        .suggest_name("Contour 5")
        .check_name(NameCheck::new(["Contour 1", "Contour 2", "Contour 3", "Contour 4"], taken))
        .built_in(|_| true)
        .files(&files)
        .show(ui)
        .into()
}

/// Image Trace's row: a name picker and one Manage presets menu.
#[story(NamedPresetRow<'static, u32>, covers = [], drive = Drive::Never)]
fn trace_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(trace_rows));
    let files = PresetFiles::new("trace-presets", "Trace presets", &["json"])
        .export(|_| Some(PresetExport { file_name: "Trace.json".to_owned(), bytes: Vec::new() }));
    let ((preset, state), preset_at) = data.value_at("preset", (Some(1_u32), PresetState::<u32>::new()));
    let picker = PresetPicker::new("Trace preset", preset_at.bind(preset), &rows, preset_at.bind(state)).field(PickerField::Name);
    NamedPresetRow::new("Trace preset", picker)
        .style(NamedPresetStyle::Menu)
        .suggest_name("Preset 7")
        .check_name(NameCheck::new(TRACE, taken))
        .built_in(|_| true)
        .files(&files)
        .show(ui)
        .into()
}
