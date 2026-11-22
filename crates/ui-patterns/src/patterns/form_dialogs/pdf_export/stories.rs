//! Stories of `PdfExportDialog`.

use super::PdfExportDialog;
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
