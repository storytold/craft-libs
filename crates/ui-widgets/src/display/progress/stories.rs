//! Stories of `Progress`.

use super::Progress;
use crate::contract::{Props as _, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Progress, covers = [Default], drive = Drive::Never)]
fn progress(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Progress::new(0.4).label("Export").show(ui).into()
}

#[story(Progress, covers = [], drive = Drive::Never)]
fn progress_indeterminate(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Progress::indeterminate().label("Working").show(ui).into()
}
