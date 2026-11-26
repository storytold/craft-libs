//! Stories of `TargetMark`.

use super::TargetMark;
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(TargetMark, covers = [Default], drive = Drive::Never)]
fn target_mark(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let target = Button::new("Layers").show(ui);
    TargetMark::new(target.rect).show_mark(ui).into()
}
