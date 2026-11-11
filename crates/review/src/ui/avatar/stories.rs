//! Stories of `Avatar`.

use super::Avatar;
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Avatar, covers = [Default], drive = Drive::Never)]
fn avatar_accent(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Avatar::new("Ana Reyes").show(ui).into()
}

#[story(Avatar, covers = [], drive = Drive::Never)]
fn avatar_filled(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Avatar::new("Bo Chen").fill([232, 160, 40]).show(ui).into()
}

#[story(Avatar, covers = [], drive = Drive::Never)]
fn avatar_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Avatar::new("Cy Dahl").fill([60, 130, 230]).pressable(true).selected(true).show(ui).into()
}
