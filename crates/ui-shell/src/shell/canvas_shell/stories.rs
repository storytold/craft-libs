//! Stories of `CanvasPanel`.

use super::CanvasPanel;
use crate::contract::Show as _;
use crate::display::Text;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(CanvasPanel<'static>, covers = [Default], drive = Drive::Never)]
fn canvas_panel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    CanvasPanel::new()
        .rail("Panels", |ui, _| {
            Text::new("Rail").show(ui);
        })
        .tabs(|ui, _| {
            Text::new("Tabs").show(ui);
        })
        .show(ui, |ui, _| {
            Text::new("Document").show(ui);
        })
        .into()
}
