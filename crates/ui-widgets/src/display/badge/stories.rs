//! Stories of `Badge`.

use super::Badge;
use crate::contract::{Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Row;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::{
    Variant,
    variant::{Emphasis, Tone},
};
use egui::Ui;

/// Every tone with its name, in the order the tones' badges show.
const TONES: [(Tone, &str); 6] =
    [(Tone::Neutral, "neutral"), (Tone::Accent, "accent"), (Tone::Good, "good"), (Tone::Warn, "warn"), (Tone::Bad, "bad"), (Tone::Info, "info")];

/// The texts the tones' badges cycle through.
const SAMPLES: [&str; 3] = ["Beta", "Synced", "Needs review"];

/// A badge of every tone in `emphasis`, each its sample text and tone name.
fn tones(ui: &mut Ui, emphasis: Emphasis) -> StoryOutput {
    Row::new()
        .wrap(true)
        .show(ui, |ui, _| {
            for ((tone, name), text) in TONES.into_iter().zip(SAMPLES.into_iter().cycle()) {
                Badge::new(format!("{text} {name}")).variant(Variant::look(emphasis, tone)).show(ui);
            }
        })
        .into()
}

#[story(Badge, covers = [Default], drive = Drive::Never, group = "Tones", title = "Soft")]
fn badge(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tones(ui, Emphasis::Soft)
}

#[story(Badge, covers = [], drive = Drive::Never, group = "Tones", title = "Solid")]
fn badge_solid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tones(ui, Emphasis::Solid)
}

/// Counts below, at and above the cap, then the first count as a solid bad badge.
#[story(Badge, covers = [], drive = Drive::Never, group = "Counts", title = "Counts")]
fn badge_count(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let first = *data.value("count", 7u32);
    Row::new()
        .wrap(true)
        .show(ui, |ui, _| {
            for n in [first, 1, 9, 10, 99, 100, 1234] {
                Badge::count(n).show(ui);
            }
            Badge::count(first).variant(Variant::look(Emphasis::Solid, Tone::Bad)).show(ui);
        })
        .into()
}
