//! Stories of Brightness/Contrast, Threshold, Color Balance and Photo Filter.

use super::{
    BrightnessContrast, BrightnessContrastEditor, ColorBalance, ColorBalanceEditor, PhotoFilter, PhotoFilterEditor, PhotoFilterPreset,
    ThresholdEditor,
};
use crate::ui::curve::stories::story_bins;
use crate::ui::histogram::{ChannelColor, HistogramData};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(BrightnessContrastEditor<'static>, covers = [Default], drive = Drive::Never)]
fn brightness_contrast(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: false });
    BrightnessContrastEditor::new("Brightness/Contrast", Bind::bound(value)).show(ui).into()
}

/// The image's histogram with the marker at the level.
#[story(ThresholdEditor<'static>, covers = [Default], drive = Drive::Never)]
fn threshold(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let histogram = HistogramData::tone(&story_bins(0.0), ChannelColor::Neutral);
    let level = data.value("level", 90.0);
    ThresholdEditor::new("Threshold", Bind::bound(level)).histogram(&histogram).show(ui).into()
}

#[story(ColorBalanceEditor<'static>, covers = [Default], drive = Drive::Never)]
fn color_balance(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", ColorBalance { midtones: [20.0, -10.0, 5.0], ..ColorBalance::default() });
    ColorBalanceEditor::new("Color Balance", Bind::bound(value)).show(ui).into()
}

/// A filter preset: the Filter row is on.
#[story(PhotoFilterEditor<'static>, covers = [Default], drive = Drive::Never)]
fn photo_filter_preset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("preset", PhotoFilter { color: PhotoFilterPreset::Cooling80.color(), density: 25.0, preserve_luminosity: true });
    PhotoFilterEditor::new("Photo Filter", Bind::bound(value)).show(ui).into()
}

/// A colour of no preset: the Color row is on, and the select offers Custom.
#[story(PhotoFilterEditor<'static>, covers = [], drive = Drive::Never)]
fn photo_filter_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("color", PhotoFilter { color: Srgba::rgb(0.4, 0.2, 0.6), density: 60.0, preserve_luminosity: false });
    PhotoFilterEditor::new("Photo Filter", Bind::bound(value)).show(ui).into()
}
