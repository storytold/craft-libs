//! The story of the full-window dialog shell.

use super::FullWindowDialog;
use crate::contract::{Bind, ShowContainer as _};
use crate::dialog::stories::{body, close_box};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(FullWindowDialog<'static>, covers = [Default], drive = Drive::Custom(close_box))]
fn full_window_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = FullWindowDialog::new("Grid options")
        .opened(Bind::bound(data.value("open", true)))
        .closable(true)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}
