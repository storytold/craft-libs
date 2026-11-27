//! Stories of `ColorPanel`.

use super::{ColorPanel, ColorPanelLayout, ColorPanelMode, ColorPanelRamp, ColorPanelShown, ColorPanelState, TintRow};
use crate::ui::Gamut;
use craft_theme::Srgba;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A foreground that is not a web colour and not printable, over a white background.
const COLORS: [Srgba; 2] = [Srgba::rgb(0.07, 0.65, 0.58), Srgba::rgb(1.0, 1.0, 1.0)];

/// Colours the Recent Colors slot shows.
static RECENT: [Srgba; 8] = [
    Srgba::rgb(0.90, 0.28, 0.30),
    Srgba::rgb(0.97, 0.42, 0.08),
    Srgba::rgb(0.19, 0.64, 0.42),
    Srgba::rgb(0.07, 0.65, 0.58),
    Srgba::rgb(0.24, 0.39, 0.87),
    Srgba::rgb(0.56, 0.31, 0.78),
    Srgba::rgb(0.0, 0.0, 0.0),
    Srgba::rgb(1.0, 1.0, 1.0),
];

fn panel(ui: &mut Ui, data: &mut StoryData, state: ColorPanelState) -> StoryOutput {
    ColorPanel::new(data.bind("state", state), COLORS).show(ui).into()
}

/// The FillStroke panel on `state` showing `shown`, set up further by `tweak`.
fn fill_stroke(
    ui: &mut Ui,
    data: &mut StoryData,
    state: ColorPanelState,
    shown: ColorPanelShown,
    tweak: impl FnOnce(ColorPanel<'_>) -> ColorPanel<'_>,
) -> StoryOutput {
    let panel = ColorPanel::new(data.bind("state", state), COLORS).layout(ColorPanelLayout::FillStroke).shown(shown);
    tweak(panel).show(ui).into()
}

/// The FillStroke panel showing the rows of `mode`.
fn fill_stroke_mode(ui: &mut Ui, data: &mut StoryData, mode: ColorPanelMode) -> StoryOutput {
    let state = ColorPanelState { mode, ..ColorPanelState::default() };
    fill_stroke(ui, data, state, ColorPanelShown::Mode(mode), |panel| panel)
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

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_rgb(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::RgbSliders)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_hsb(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::HsbSliders)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_cmyk(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::CmykSliders)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_grayscale(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::GrayscaleSlider)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_web(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::WebColorSliders)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_lab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke_mode(ui, data, ColorPanelMode::LabSliders)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_tint(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut tint = TintRow::new("Brand orange", Srgba::rgb(0.95, 0.45, 0.10), 60.0);
    tint.global = true;
    tint.spot = true;
    fill_stroke(ui, data, ColorPanelState::default(), ColorPanelShown::Tint(tint), |panel| panel)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_none(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke(ui, data, ColorPanelState::default(), ColorPanelShown::Empty, |panel| panel.edited(None))
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = ColorPanelState { mode: ColorPanelMode::RgbSliders, ..ColorPanelState::default() };
    fill_stroke(ui, data, state, ColorPanelShown::Mode(ColorPanelMode::RgbSliders), |panel| panel.edited(None))
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_hidden_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = ColorPanelState { hide_options: true, ..ColorPanelState::default() };
    fill_stroke(ui, data, state, ColorPanelShown::Mode(ColorPanelMode::RgbSliders), |panel| panel)
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_out_of_gamut(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = ColorPanelShown::Mode(ColorPanelMode::RgbSliders);
    // A gamut that pulls green and blue in, so the foreground is outside it.
    fill_stroke(ui, data, ColorPanelState::default(), shown, |panel| {
        panel.gamut(Gamut::new(|color| Srgba::new(color.r, color.g * 0.8, color.b * 0.8, color.a)))
    })
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_recent_colors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    fill_stroke(ui, data, ColorPanelState::default(), ColorPanelShown::Mode(ColorPanelMode::RgbSliders), |panel| panel.recent(&RECENT))
}

#[story(ColorPanel<'static>, covers = [], drive = Drive::Never)]
fn color_panel_fill_stroke_rgb_percent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = ColorPanelState { rgb_percent: true, ..ColorPanelState::default() };
    fill_stroke(ui, data, state, ColorPanelShown::Mode(ColorPanelMode::RgbSliders), |panel| panel)
}
