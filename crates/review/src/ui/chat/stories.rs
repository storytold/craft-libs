//! Stories of the cursor chat box.

use super::CursorChatBox;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Ui, pos2};

#[story(CursorChatBox<'static>, covers = [Default], drive = Drive::Never)]
fn cursor_chat_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = true;
    let mut text = data.value("text", String::from("Looks good")).clone();
    CursorChatBox::new(&mut text, &mut open).show(ui, Some(pos2(60.0, 60.0))).into()
}
