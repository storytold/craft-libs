//! Stories of `ModifierKeysPanel`.

use super::ModifierKeysPanel;
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_keymap::Mods;
use egui::Ui;

#[story(ModifierKeysPanel<'static>, covers = [], drive = Drive::Never, title = "Shift latched")]
fn modifier_keys_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let latched = data.value("latched", Mods::SHIFT);
    ModifierKeysPanel::new(Bind::bound(&mut *latched)).show(ui);
    let readout = format!("{latched:?}");
    StoryOutput::default().readout(readout)
}
