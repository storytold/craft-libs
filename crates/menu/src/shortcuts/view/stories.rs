//! Stories of the command table and its row: the table over a small keymap, whose rows show the row component.

use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::table::{self, CommandRow, ShortcutTable};
use crate::shortcuts::Options;
use crate::shortcuts::state::{DialogState, Env};
use crate::shortcuts::stories::keymap;
use crate::shortcuts::words::Say;

/// The table over the stories' keymap for one pass, listing the commands whose name matches `query`.
fn scene(ui: &mut Ui, query: &str) -> StoryOutput {
    let mut keymap = keymap();
    let options = Options::default();
    let say = Say::of(ui);
    let mut state = DialogState::new();
    let mut env = Env { keymap: &mut keymap, sets: None, options: &options, tabs: None, text: &say };
    state.begin(&mut env);
    state.set_query(query.to_owned());
    table::show(ui, 0.0, &mut env, &mut state);
    StoryOutput::default()
}

#[story(ShortcutTable<'static>, covers = [Default, Many], drive = Drive::Never)]
fn shortcut_table(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, "")
}

#[story(ShortcutTable<'static>, covers = [One], drive = Drive::Never)]
fn shortcut_table_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, "Loop")
}

#[story(ShortcutTable<'static>, covers = [Empty], drive = Drive::Never)]
fn shortcut_table_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, "no such command")
}

#[story(CommandRow<'static>, covers = [Default], drive = Drive::Never)]
fn command_row(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, "")
}
