//! Stories of `DockRoot`.

use super::DockRoot;
use crate::contract::{Props as _, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(DockRoot<'static, ()>, covers = [Default], drive = Drive::Never)]
fn a_room_that_takes_no_drops(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    DockRoot::<()>::new(|_| None)
        .label("Workspace")
        .show(ui, |ui, _| {
            Text::new("Canvas").show(ui);
        })
        .output
        .into()
}
