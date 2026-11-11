//! Stories of `AlignBar`.

use super::{AlignBar, DistributeButtons};
use crate::bars::{Bar, BarKind};
use crate::contract::ShowContainer as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(AlignBar, covers = [], drive = Drive::Never)]
fn align_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            AlignBar::new().distribute(DistributeButtons::Row).show(ui);
        })
        .output
        .into()
}
