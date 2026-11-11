//! Stories of `ViewportWindow`. Each window shows while the story's `open` value is on; closing it turns the value off.

use super::ViewportWindow;
use crate::contract::Show as _;
use crate::display::Text;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(ViewportWindow, covers = [], drive = Drive::Never)]
fn viewport_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = data.value("open", true);
    if !*open {
        return StoryOutput::default();
    }
    let shown = ViewportWindow::new("Export Log").show(ui, |ui, _| {
        let _ = Text::new("The export finished.").show(ui);
    });
    *open = shown.close_requested.is_none();
    shown.into()
}

#[story(ViewportWindow, covers = [], drive = Drive::Never)]
fn viewport_window_without_app_modal(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = data.value("open", true);
    if !*open {
        return StoryOutput::default();
    }
    let shown = ViewportWindow::new("Tool Window").app_modal(false).show(ui, |ui, _| {
        let _ = Text::new("Other windows keep taking input.").show(ui);
    });
    *open = shown.close_requested.is_none();
    shown.into()
}
