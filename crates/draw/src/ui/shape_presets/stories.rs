//! Stories of the shape menu: every preset, and the presets of a host with no connectors.

use super::ShapeMenu;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(ShapeMenu<'static>, covers = [], drive = Drive::Never)]
fn shape_menu(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let _ = ShapeMenu::new(|_| true).show(ui);
    StoryOutput::default()
}

#[story(ShapeMenu<'static>, covers = [], drive = Drive::Never)]
fn shape_menu_without_connectors(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let _ = ShapeMenu::new(|preset| !preset.is_connector()).show(ui);
    StoryOutput::default()
}
