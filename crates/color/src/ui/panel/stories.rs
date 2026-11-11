//! Stories of `ColorPanel`.

use super::{ColorPanel, ColorPanelMode, ColorPanelRamp, ColorPanelState};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A foreground that is not a web colour and not printable, over a white background.
const COLORS: [Srgba; 2] = [Srgba::rgb(0.07, 0.65, 0.58), Srgba::rgb(1.0, 1.0, 1.0)];

fn panel(ui: &mut Ui, data: &mut StoryData, state: ColorPanelState) -> StoryOutput {
    ColorPanel::new(Bind::bound(data.value("state", state)), COLORS).show(ui).into()
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, ColorPanelState::default())
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_sliders(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = ColorPanelState { mode: ColorPanelMode::WebColorSliders, ramp: ColorPanelRamp::RgbSpectrum, ..ColorPanelState::default() };
    panel(ui, data, state)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_wheel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, ColorPanelState { mode: ColorPanelMode::ColorWheel, ..ColorPanelState::default() })
}
