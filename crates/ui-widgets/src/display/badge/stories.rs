//! Stories of `Badge`.

use super::Badge;
use crate::contract::{Props as _, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::{
    Variant,
    variant::{Emphasis, Tone},
};
use egui::Ui;

#[story(Badge, covers = [Default], drive = Drive::Never)]
fn badge(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Badge::new("Needs review").variant(Variant::look(Emphasis::Soft, Tone::Warn)).show(ui).into()
}

#[story(Badge, covers = [], drive = Drive::Never)]
fn badge_count(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Badge::count(1234).variant(Variant::look(Emphasis::Solid, Tone::Bad)).show(ui).into()
}
