//! Stories of `ColorPopover`.

use super::ColorPopover;
use crate::ui::PickedColor;
use craft_theme::Srgba;
use craft_ui::buttons::Button;
use craft_ui::contract::ShowContainer as _;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn picked() -> PickedColor {
    PickedColor { color: Srgba::rgb(0.2, 0.5, 0.8), model: None }
}

#[story(ColorPopover<'static>, covers = [Default], drive = Drive::Never)]
fn closed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    StoryOutput::from(ColorPopover::new("Fill colour", Bind::bound(data.value("color", picked()))).show(ui))
}

#[story(ColorPopover<'static>, covers = [], drive = Drive::Never)]
fn open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = data.value("color", picked());
    let shown = Stack::new().show(ui, |ui, _| {
        let opener = Button::new("Stop").show(ui);
        ColorPopover::new("Stop colour", Bind::bound(&mut *color)).anchor(&opener, Bind::given(true)).show(ui)
    });
    match shown.inner {
        Some(output) => StoryOutput::from(output),
        None => StoryOutput::from(shown.output),
    }
}
