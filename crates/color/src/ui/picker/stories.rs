//! Stories of `ColorPicker` and `ColorWarningChip`: Compact with alpha, Compact with Only Web Colors, Full with the
//! compare swatch and a warning, and Full with the host's CMYK model and OKLCH.

use craft_theme::{Channel, ColorModel, DefaultOn, HsbModel, LabModel, MAX_CHANNELS, ModelValues, OklchModel, RgbModel, Srgba};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::channel::{ChannelRow, ChannelRule, TrackRamp};
use super::screen_pick::ScreenPickButton;
use super::{ColorPicker, ColorPickerLayout, ColorWarning, ColorWarningChip, PickedColor};
use crate::ui::AreaMode;

/// A naive CMYK in percent, without a working space: a host's model for the stories. A real host converts through its
/// CMYK profile.
struct NaiveCmyk;

const CMYK: [Channel; 4] = [
    Channel { name: "Cyan", qualified: "CMYK cyan", label: "C", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Magenta", qualified: "CMYK magenta", label: "M", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Yellow", qualified: "CMYK yellow", label: "Y", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
    Channel { name: "Black", qualified: "CMYK black", label: "K", min: 0.0, max: 100.0, unit: "%", decimals: 0, step: 1.0, track_stops: 2 },
];

impl ColorModel for NaiveCmyk {
    fn id(&self) -> &'static str {
        "story-cmyk"
    }

    fn name(&self) -> &'static str {
        "CMYK"
    }

    fn channels(&self) -> &[Channel] {
        &CMYK
    }

    fn from_srgb(&self, c: Srgba) -> ModelValues {
        let k = 1.0 - c.r.max(c.g).max(c.b);
        let ink = |v: f32| if k >= 1.0 { 0.0 } else { 100.0 * (1.0 - v - k) / (1.0 - k) };
        ModelValues::new("story-cmyk", &[ink(c.r), ink(c.g), ink(c.b), 100.0 * k])
    }

    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        let k = 1.0 - v[3] / 100.0;
        Srgba::rgb((1.0 - v[0] / 100.0) * k, (1.0 - v[1] / 100.0) * k, (1.0 - v[2] / 100.0) * k)
    }

    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: true, compact: false }
    }
}

static CMYK_MODEL: NaiveCmyk = NaiveCmyk;

fn start() -> PickedColor {
    PickedColor::rgb(Srgba::rgb(0.2, 0.5, 0.8))
}

#[story(ColorPicker<'static>, covers = [Default], drive = Drive::Never)]
fn color_picker_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = data.value("color", start());
    ColorPicker::new("Foreground color", Bind::bound(color)).alpha(true).eyedropper(true).show(ui).into()
}

#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_web_colors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let web = *data.value("web", true);
    let mut web = web;
    let color = data.value("color", start());
    ColorPicker::new("Foreground color", Bind::bound(color)).web_colors(Bind::bound(&mut web)).show(ui).into()
}

#[story(ColorPicker<'static>, covers = [], drive = Drive::Never)]
fn color_picker_full(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut mode = *data.value("mode", AreaMode::default());
    let current = Srgba::rgb(0.9, 0.2, 0.1);
    let color = data.value("color", PickedColor::rgb(Srgba::rgb(1.0, 0.0, 1.0)));
    ColorPicker::new("Foreground color", Bind::bound(color))
        .layout(ColorPickerLayout::Full)
        .mode(Bind::bound(&mut mode))
        .compare(current)
        .warnings([ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.9, 0.1, 0.9))])
        .eyedropper(true)
        .show(ui)
        .into()
}

#[story(ColorPicker<'static>, covers = [Many], drive = Drive::Never)]
fn color_picker_full_models(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = data.value("color", start());
    let models: [&'static dyn ColorModel; 5] = [&HsbModel, &RgbModel, &LabModel, &CMYK_MODEL, &OklchModel];
    ColorPicker::new("Foreground color", Bind::bound(color)).layout(ColorPickerLayout::Full).models(models).alpha(true).show(ui).into()
}

#[story(ColorWarningChip<'static>, covers = [Default], drive = Drive::Never)]
fn color_warning_chip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    static WARNING: std::sync::LazyLock<ColorWarning> =
        std::sync::LazyLock::new(|| ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.9, 0.1, 0.9)));
    ColorWarningChip::new(&WARNING).show(ui).into()
}

#[story(ScreenPickButton, covers = [Default], drive = Drive::Never)]
fn screen_pick_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ScreenPickButton::new().label("Sample color from the screen").show(ui).into()
}

#[story(TrackRamp, covers = [Default], drive = Drive::Never)]
fn track_ramp(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    TrackRamp::new(vec![Srgba::rgb(0.0, 0.0, 0.0), Srgba::rgb(1.0, 1.0, 1.0)], false).show(ui).into()
}

#[story(ChannelRow<'static>, covers = [], drive = Drive::Never)]
fn channel_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", 40.0_f64);
    let channel = HsbModel.channels()[1];
    let row = ChannelRow::new("Saturation", Bind::bound(value), ChannelRule::of(&channel))
        .track(vec![Srgba::rgb(1.0, 1.0, 1.0), Srgba::rgb(1.0, 0.0, 0.0)]);
    match row.show(ui) {
        Some(edit) => StoryOutput::from(edit),
        None => StoryOutput::default(),
    }
}
