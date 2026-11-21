//! Stories of `AxesButton`: the button closed, and its popover of sliders opened by the button.

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use super::AxesButton;
use crate::axes::{AxisValues, FontAxis, NamedInstance};

const WGHT: [u8; 4] = *b"wght";
const WDTH: [u8; 4] = *b"wdth";
const LABEL: &str = "Axes";

fn demo(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let axes = [FontAxis::new(WGHT, "Weight", 100.0, 400.0, 900.0), FontAxis::new(WDTH, "Width", 75.0, 100.0, 100.0)];
    let instance = |name: &str, weight: f32, width: f32| NamedInstance::new(name, vec![(WGHT, weight), (WDTH, width)]);
    let instances = [instance("Light", 300.0, 100.0), instance("Regular", 400.0, 100.0), instance("Bold Condensed", 700.0, 75.0)];
    let values: &mut AxisValues = data.value("values", vec![(WGHT, 400.0), (WDTH, 100.0)]);
    AxesButton::new(LABEL, &axes, Bind::bound(values)).instances(&instances).show(ui).into()
}

#[story(AxesButton<'static>, covers = [Default], drive = Drive::Never)]
fn axes_button_closed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    demo(ui, data)
}

/// Clicks the button, which opens the popover.
fn open_button(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_label(LABEL) {
        fixture.click(&button);
    }
    fixture.settle();
}

#[story(AxesButton<'static>, covers = [], drive = Drive::Custom(open_button))]
fn axes_button_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    demo(ui, data)
}
