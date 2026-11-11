//! Stories of `SelectableText`.

use super::SelectableText;
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

const LONG: &str = "A name that runs on well past the end of the field, so the field scrolls its text under the caret";

#[story(SelectableText<'static>, covers = [Default], drive = Drive::Never)]
fn selectable_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SelectableText::new("Path", "/projects/poster.indd").show(ui).into()
}

#[story(SelectableText<'static>, covers = [LongText], drive = Drive::Never)]
fn selectable_text_long(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SelectableText::new("Path", LONG).show(ui).into()
}
