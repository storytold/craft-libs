//! Stories of `PdfExportDialog`.

use super::{HostedPdfExport, PdfArtboards, PdfExportDialog, PdfExportMode, PresetDraft};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, clicked};
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
    let (settings, open, applied) = data.value("pdf", (PdfSettings::default(), true, "default"));
    if *applied != preset {
        if let Some(found) = builtin_preset(preset, "Story") {
            *settings = found.settings;
        }
        *applied = preset;
    }
    PdfExportDialog::new("Story", Bind::bound(settings)).opened(Bind::bound(open)).artboard_count(ARTBOARDS).show(ui).into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_art_bounds(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, open) = data.value("pdf-art-bounds", (PdfSettings::default(), true));
    PdfExportDialog::new("Story", Bind::bound(settings)).opened(Bind::bound(open)).artboards(PdfArtboards::ArtBounds).show(ui).into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_preset_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, open, draft) = data.value("pdf-editor", (PdfSettings::default(), true, PresetDraft::default()));
    PdfExportDialog::new("Story", Bind::bound(settings))
        .opened(Bind::bound(open))
        .mode(PdfExportMode::PresetEditor)
        .preset_draft(Bind::bound(draft))
        .show(ui)
        .into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn pdf_export_custom_flattener(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, open) = data.value("pdf-custom", (PdfSettings::default(), true));
    PdfExportDialog::new("Story", Bind::bound(settings))
        .opened(Bind::bound(open))
        .artboard_count(ARTBOARDS)
        .custom_flattener(true)
        .show(ui)
        .into()
}

#[story(PdfExportDialog<'static>, covers = [], drive = Drive::Never)]
fn hosted_pdf_export(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // The dialog starts closed; the button opens the real one.
    if clicked(&Button::new("Save PDF…").show(ui)) {
        *data.value("open", false) = true;
    }
    hosted_with(ui, data, (), || HostedPdfExport::new("Story", PdfSettings::default()).artboard_count(ARTBOARDS), |_| {})
}
