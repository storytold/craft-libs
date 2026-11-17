//! Stories of `DropperButton`.

use super::{DropperButton, DropperKind};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn button(ui: &mut Ui, data: &mut StoryData, kind: DropperKind, armed: bool) -> StoryOutput {
    let armed = data.value("armed", armed);
    StoryOutput::from(DropperButton::new("Sample in image", Bind::bound(armed)).kind(kind).show(ui))
}

#[story(DropperButton<'static>, covers = [], drive = Drive::Never)]
fn sample(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    button(ui, data, DropperKind::Sample, false)
}

#[story(DropperButton<'static>, covers = [], drive = Drive::Never)]
fn black_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    button(ui, data, DropperKind::Black, false)
}

#[story(DropperButton<'static>, covers = [], drive = Drive::Never)]
fn gray_point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    button(ui, data, DropperKind::Gray, false)
}

#[story(DropperButton<'static>, covers = [], drive = Drive::Never)]
fn white_point_armed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    button(ui, data, DropperKind::White, true)
}
