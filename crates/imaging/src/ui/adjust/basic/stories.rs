//! Stories of Brightness/Contrast, Threshold, Color Balance and Photo Filter.

use super::{
    BrightnessContrast, BrightnessContrastEditor, ColorBalance, ColorBalanceEditor, ColorBalanceTone, PhotoFilter, PhotoFilterEditor,
    PhotoFilterPreset, ThresholdEditor,
};
use crate::ui::adjust::HostedAdjustDialog;
use crate::ui::curve::stories::story_bins;
use crate::ui::histogram::{ChannelColor, HistogramData};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::dialog::stories::hosted;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(BrightnessContrastEditor<'static>, covers = [Default], drive = Drive::Never)]
fn brightness_contrast(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: false });
    BrightnessContrastEditor::new("Brightness/Contrast", Bind::bound(value)).show(ui).into()
}

/// The editor in its Image › Adjustments dialog, opened from a button.
#[story(BrightnessContrastEditor<'static>, covers = [], drive = Drive::Never, title = "In its dialog")]
fn brightness_contrast_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: false };
    hosted(ui, data, || {
        HostedAdjustDialog::new("image.adjustments.brightnessContrast", "Brightness/Contrast", value, |ui, value, cx| {
            BrightnessContrastEditor::new("Brightness/Contrast", value).context(cx).show(ui);
            None::<()>
        })
    })
}

/// The image's histogram with the marker at the level.
#[story(ThresholdEditor<'static>, covers = [Default], drive = Drive::Never)]
fn threshold(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let histogram = HistogramData::tone(&story_bins(0.0), ChannelColor::Neutral);
    let level = data.value("level", 90.0);
    ThresholdEditor::new("Threshold", Bind::bound(level)).histogram(&histogram).show(ui).into()
}

fn color_balance(ui: &mut Ui, data: &mut StoryData, start: ColorBalanceTone) -> StoryOutput {
    let shifts =
        ColorBalance { shadows: [-20.0, 0.0, 30.0], midtones: [10.0, -15.0, 0.0], highlights: [25.0, 10.0, -20.0], ..ColorBalance::default() };
    // The host keeps the value and the tone shown.
    let (value, tone) = data.value("value", (shifts, start));
    let shown = ColorBalanceEditor::new("Color Balance", Bind::bound(value)).tone(*tone).show(ui);
    *tone = shown.tone;
    shown.into()
}

#[story(ColorBalanceEditor<'static>, covers = [], drive = Drive::Never, group = "Tones", title = "Shadows")]
fn color_balance_shadows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    color_balance(ui, data, ColorBalanceTone::Shadows)
}

#[story(ColorBalanceEditor<'static>, covers = [Default], drive = Drive::Never, group = "Tones", title = "Midtones")]
fn color_balance_midtones(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    color_balance(ui, data, ColorBalanceTone::Midtones)
}

#[story(ColorBalanceEditor<'static>, covers = [], drive = Drive::Never, group = "Tones", title = "Highlights")]
fn color_balance_highlights(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    color_balance(ui, data, ColorBalanceTone::Highlights)
}

/// A filter preset: the Filter row is on.
#[story(PhotoFilterEditor<'static>, covers = [Default], drive = Drive::Never, group = "Sources", title = "A filter preset")]
fn photo_filter_preset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("preset", PhotoFilter { color: PhotoFilterPreset::Cooling80.color(), density: 25.0, preserve_luminosity: true });
    PhotoFilterEditor::new("Photo Filter", Bind::bound(value)).show(ui).into()
}

/// A colour of no preset: the Color row is on, and the select offers Custom.
#[story(PhotoFilterEditor<'static>, covers = [], drive = Drive::Never, group = "Sources", title = "A custom color")]
fn photo_filter_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("color", PhotoFilter { color: Srgba::rgb(0.4, 0.2, 0.6), density: 60.0, preserve_luminosity: false });
    PhotoFilterEditor::new("Photo Filter", Bind::bound(value)).show(ui).into()
}
