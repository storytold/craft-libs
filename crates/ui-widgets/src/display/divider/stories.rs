//! Stories of `Divider`.

use super::Divider;
use crate::contract::Show as _;
use crate::display::Text;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Divider, covers = [Default], drive = Drive::Never)]
fn divider(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Divider::horizontal().show(ui).into()
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
