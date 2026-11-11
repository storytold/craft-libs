//! Stories of `FloatingGroup`.

use super::FloatingGroup;
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::menu::MenuItem;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Pos2, Rect, Ui};

const START: Rect = Rect { min: Pos2::new(40.0, 40.0), max: Pos2::new(280.0, 220.0) };

#[story(FloatingGroup<'static>, covers = [Default], drive = Drive::Never)]
fn floating_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = *data.value("rect", START);
    let shown = FloatingGroup::new("Layers", rect)
        .menu(|ui, _| {
            MenuItem::new("New Layer").show(ui);
        })
        .show_group(ui, |ui, _| {
            Button::new("Add layer").show(ui);
        });
    if let Some(change) = shown.output.placed {
        *data.value("rect", START) = change.rect;
    }
    shown.output.into()
}
