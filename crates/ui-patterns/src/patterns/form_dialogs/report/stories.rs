//! Stories of `ReportDialog`.

use super::{ReportDialog, ReportNote};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::variant::Tone;
use egui::Ui;

#[story(ReportDialog<'static>, covers = [], drive = Drive::Never)]
fn report_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let count = *data.ranged("notes", 3_usize, 0.0..=40.0);
    let notes: Vec<ReportNote> = (0..count)
        .map(|n| match n % 3 {
            0 => ReportNote::new(Tone::Warn, format!("Font on page {} was substituted", n + 1)),
            1 => ReportNote::new(Tone::Info, format!("Page {} has no bleed", n + 1)),
            _ => ReportNote::new(Tone::Bad, format!("Image on page {} could not be read", n + 1)),
        })
        .collect();
    ReportDialog::new("Import Report", "Imported 12 pages.", notes).opened(Bind::bound(data.value("open", true))).show(ui).into()
}
