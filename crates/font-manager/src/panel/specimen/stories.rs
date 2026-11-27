//! Stories of `FontSample`.

use std::sync::Arc;

use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{FontSample, SampleFace};
use crate::craft_font_catalog::{Limits, Os};
use crate::panel::faces::Owner;
use crate::panel::fixtures::{VARIABLE, story_catalog};
use crate::panel::samples::PANGRAM;
use crate::panel::subsets::SubsetFont;
use crate::{Catalog, FontEnv, Source};

/// The sample size of the stories: the panel's default sample size in points.
const POINTS: f32 = 24.0;
/// A sample a wide face shows whole in a story's room: where an ellipsis cuts moves with the display's scale.
const SHORT: &str = "Quick brown fox";

#[story(FontSample<'static>, covers = [Default], drive = Drive::Never)]
fn ready(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let catalog = story_catalog();
    let faces = catalog.styles("Hack");
    let face = match faces.first() {
        Some(face) => SampleFace::Face { catalog: &catalog, owner: Owner::Installed, face },
        None => SampleFace::Missing,
    };
    FontSample::new("Hack: sample", PANGRAM, face, POINTS).show(ui).into()
}

/// A variable face at a coordinate it is not registered at: its boldest weight.
#[story(FontSample<'static>, covers = [], drive = Drive::Never)]
fn at_coordinates(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    let faces = catalog.add_bytes(VARIABLE.to_vec(), Source::Bundled, &Limits::default()).unwrap_or_default();
    let face = match faces.first() {
        Some(face) => SampleFace::Face { catalog: &catalog, owner: Owner::Installed, face },
        None => SampleFace::Missing,
    };
    FontSample::new("Orbitron: sample", SHORT, face, POINTS).coords(&[(*b"wght", 900.0)]).show(ui).into()
}

/// A subset from the preview service.
#[story(FontSample<'static>, covers = [], drive = Drive::Never)]
fn subset(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let font = SubsetFont { key: "story/orbitron".to_owned(), bytes: VARIABLE.into() };
    FontSample::new("Orbitron: sample", SHORT, SampleFace::Subset(&font), POINTS).show(ui).into()
}

#[story(FontSample<'static>, covers = [], drive = Drive::Never)]
fn unavailable(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FontSample::new("Hack: sample", PANGRAM, SampleFace::Missing, POINTS).show(ui).into()
}
