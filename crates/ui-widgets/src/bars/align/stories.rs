//! Stories of `AlignBar`.

use super::AlignBar;
use crate::bars::{Bar, BarKind};
use crate::contract::ShowContainer as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(AlignBar, covers = [], drive = Drive::Never)]
fn align_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            AlignBar::new().show(ui);
        })
        .output
        .into()
}
