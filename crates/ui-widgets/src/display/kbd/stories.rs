//! Stories of `Kbd`.

use super::Kbd;
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Kbd, covers = [Default], drive = Drive::Never)]
fn kbd(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Kbd::new("Ctrl+Shift+S").show(ui).into()
}

#[story(Kbd, covers = [], drive = Drive::Never)]
fn kbd_keycap(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Kbd::new("Ctrl+Shift+S").keycap(true).show(ui).into()
}

#[story(Kbd, covers = [], drive = Drive::Never)]
fn kbd_chord(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Kbd::chord(craft_keymap::Chord::new(craft_keymap::Mods { cmd: true, shift: true, ..craft_keymap::Mods::NONE }, craft_keymap::Key::Delete))
        .keycap(true)
        .show(ui)
        .into()
}
