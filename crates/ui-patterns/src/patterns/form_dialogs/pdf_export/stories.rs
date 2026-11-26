//! Stories of `PdfExportDialog`.

use super::{HostedPdfExport, PdfArtboards, PdfExportDialog, PdfExportMode, PresetDraft};
use crate::buttons::Button;
use crate::contract::{Show as _, clicked};
use crate::dialog::stories::hosted_with;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_pdf::settings::{PdfSettings, builtin_preset};
use egui::Ui;

/// The artboards of the story's document.
const ARTBOARDS: usize = 5;

/// The built-in presets by the name `builtin_preset` takes.
const PRESETS: [(&str, &str); 4] =
    [("Default", "default"), ("Press", "Press Quality"), ("PdfX1a", "PDF/X-1a:2001"), ("Smallest", "Smallest File Size")];

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let preset = *data.choice("preset", "default", &PRESETS);
    let (((settings, applied), pdf_at), (open, open_at)) = data.pair_at(("pdf", (PdfSettings::default(), "default")), ("open", true));
    if *applied != preset {
        if let Some(found) = builtin_preset(preset, "Story") {
            *settings = found.settings;
        }
        *applied = preset;
    }
    PdfExportDialog::new("Story", pdf_at.bind(settings)).opened(open_at.bind(open)).artboard_count(ARTBOARDS).show(ui).into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_art_bounds(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((settings, open), at) = data.value_at("pdf-art-bounds", (PdfSettings::default(), true));
    PdfExportDialog::new("Story", at.bind(settings)).opened(at.bind(open)).artboards(PdfArtboards::ArtBounds).show(ui).into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_preset_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((settings, open, draft), at) = data.value_at("pdf-editor", (PdfSettings::default(), true, PresetDraft::default()));
    PdfExportDialog::new("Story", at.bind(settings))
        .opened(at.bind(open))
        .mode(PdfExportMode::PresetEditor)
        .preset_draft(at.bind(draft))
        .show(ui)
        .into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_custom_flattener(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ((settings, open), at) = data.value_at("pdf-custom", (PdfSettings::default(), true));
    PdfExportDialog::new("Story", at.bind(settings)).opened(at.bind(open)).artboard_count(ARTBOARDS).custom_flattener(true).show(ui).into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn hosted_pdf_export(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // The dialog starts closed; the button opens the real one.
    if clicked(&Button::new("Save PDF…").show(ui)) {
        *data.value("open", false) = true;
    }
    hosted_with(ui, data, (), || HostedPdfExport::new("Story", PdfSettings::default()).artboard_count(ARTBOARDS), |_| {})
}
