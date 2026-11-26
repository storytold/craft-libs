//! Stories of `FontPreviewBanner`.

use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::FontPreviewBanner;
use crate::panel::faces::Owner;
use crate::panel::fixtures::story_catalog;
use crate::panel::specimen::SampleFace;

/// The host's text in the family's face, under its name.
#[story(FontPreviewBanner<'static>, covers = [Default], drive = Drive::Never)]
fn host_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let catalog = story_catalog();
    let faces = catalog.styles("Hack");
    let face = match faces.first() {
        Some(face) => SampleFace::Face { catalog: &catalog, owner: Owner::Installed, face },
        None => SampleFace::Missing,
    };
    FontPreviewBanner::new("Font preview: Hack", "Hack", "The quick brown fox", face).show(ui).into()
}

/// A family the catalog lacks: the name over an empty sample band.
#[story(FontPreviewBanner<'static>, covers = [], drive = Drive::Never)]
fn unavailable(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FontPreviewBanner::new("Font preview: Gotham", "Gotham", "Sample", SampleFace::Missing).show(ui).into()
}
