//! The `Layout::CommandLists` body: Word's Customize Keyboard.

use craft_ui::contract::BodyCx;
use egui::Ui;

use crate::shortcuts::state::{DialogState, Env};

/// One frame of the command-lists body: a search field, the Categories list beside the Commands
/// list, then the command's keys, the New Shortcut recorder, the holder line and the Assign and
/// Remove buttons.
#[allow(dead_code)] // P3(dialogs-w11): dg.F29
pub(super) fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let _ = (ui, cx, env, state);
    todo!("dg.F29")
}
