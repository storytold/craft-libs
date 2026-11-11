//! Stories of `FontSample`.

use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{FontSample, SampleFace};
use crate::panel::faces::Owner;
use crate::panel::fixtures::story_catalog;
use crate::panel::samples::PANGRAM;

/// The sample size of the stories: the panel's default sample size in points.
const POINTS: f32 = 24.0;

#[story(FontSample<'static>, covers = [Default], drive = Drive::Never)]
fn ready(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let catalog = story_catalog();
    let faces = catalog.styles("Hack");
    let face = match faces.first() {
        Some(face) => SampleFace::Face { catalog: &catalog, owner: Owner::Installed, face },
        None => SampleFace::Interface,
    };
    FontSample::new("Hack: sample", PANGRAM, face, POINTS).show(ui).into()
}

#[story(FontSample<'static>, covers = [], drive = Drive::Never)]
fn unavailable(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FontSample::new("Hack: sample", PANGRAM, SampleFace::Interface, POINTS).show(ui).into()
}
