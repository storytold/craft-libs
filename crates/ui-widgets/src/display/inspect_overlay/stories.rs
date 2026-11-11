//! Stories of `InspectOverlay`.

use super::InspectOverlay;
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::runtime::inspect;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Id, Ui};

#[story(InspectOverlay, covers = [Default], drive = Drive::Never)]
fn inspect_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let key = Id::NULL.with("story.inspect_overlay");
    let (shown, recorded) = inspect::record(ui, key, |ui| {
        Stack::new().show(ui, |ui, _| {
            Button::new("Apply").show(ui);
            Checkbox::new(Bind::bound(data.value("snap", true))).label("Snap to grid").show(ui);
        })
    });
    let rect = shown.output.rect;
    match recorded {
        Some(recorded) => InspectOverlay::new(recorded).show_at(ui, rect).into(),
        None => shown.output.into(),
    }
}
