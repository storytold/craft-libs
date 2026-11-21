//! Stories of `Divider`.

use super::Divider;
use crate::contract::Show as _;
use crate::contract::ShowContainer as _;
use crate::display::Text;
use crate::layout_components::{Row, Stack};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Divider, covers = [Default], drive = Drive::Never)]
fn divider(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Divider::horizontal().show(ui).into()
}

/// A vertical line given its length beside a taller stack and a shorter text: it keeps its length, neither filling the
/// row nor setting its height.
#[story(Divider, covers = [], drive = Drive::Never, title = "Given length")]
fn divider_length(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new()
        .show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for line in ["First", "Second", "Third"] {
                    Text::new(line).show(ui);
                }
            });
            Divider::vertical().length(24.0).show(ui);
            Text::new("Right").show(ui);
        })
        .into()
}

#[story(Divider, covers = [], drive = Drive::Never)]
fn divider_vertical(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.horizontal(|ui| {
        Text::new("Left").show(ui);
        Divider::vertical().show(ui)
    })
    .inner
    .into()
}
