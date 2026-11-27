//! Stories of `ReportDialog`.

use super::{ReportDialog, ReportGroup, ReportNote};
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::variant::Tone;
use egui::Ui;

#[story(ReportDialog<'static>, covers = [], drive = Drive::Never)]
fn report_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let count = *data.ranged("notes", 3_usize, 0.0..=40.0);
    let plain = *data.choice("body", false, &[("Notes", false), ("Lines", true)]);
    let notes: Vec<ReportNote> = (0..count)
        .map(|n| match n % 3 {
            0 => ReportNote::new(Tone::Warn, format!("Font on page {} was substituted", n + 1)),
            1 => ReportNote::new(Tone::Info, format!("Page {} has no bleed", n + 1)),
            _ => ReportNote::new(Tone::Bad, format!("Image on page {} could not be read", n + 1)),
        })
        .collect();
    let mut dialog = ReportDialog::new("Import Report", "Imported 12 pages.", notes);
    if plain {
        dialog = dialog.lines((0..count).map(|n| Label::from(format!("page-{:03}.png: 2 layers merged", n + 1))).collect());
    }
    dialog.opened(data.bind("open", true)).show(ui).into()
}

/// Two groups under headings: notes with a detail and a note in the mono role.
#[story(ReportDialog<'static>, covers = [], drive = Drive::Never)]
fn report_dialog_grouped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = vec![
        ReportGroup::new(
            "Fonts",
            vec![ReportNote::new(Tone::Warn, "Arial was substituted").detail("page 2"), ReportNote::new(Tone::Info, "Inter Variable").code()],
        ),
        ReportGroup::new("Images", vec![ReportNote::new(Tone::Bad, "hero.png could not be read").detail("3 uses")]),
    ];
    ReportDialog::new("Check Script Report", "Checked 2 scripts.", Vec::new()).groups(groups).opened(data.bind("open", true)).show(ui).into()
}
