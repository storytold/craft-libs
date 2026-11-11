//! Stories of Hue/Saturation: Master, a colour range with its thumbs, and Colorize.

use super::{HueRangeValue, HueSaturation, HueSaturationEditor};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// What a hue becomes when the image is shifted 40° towards yellow: a stand-in for the host's engine.
fn shifted(degrees: f64) -> Srgba {
    let turn = (degrees + 40.0).rem_euclid(360.0) / 60.0;
    let (r, g, b) = match turn as u8 {
        0 => (1.0, turn.fract(), 0.0),
        1 => (1.0 - turn.fract(), 1.0, 0.0),
        2 => (0.0, 1.0, turn.fract()),
        3 => (0.0, 1.0 - turn.fract(), 1.0),
        4 => (turn.fract(), 0.0, 1.0),
        _ => (1.0, 0.0, 1.0 - turn.fract()),
    };
    Srgba::rgb(r as f32, g as f32, b as f32)
}

#[story(HueSaturationEditor<'static>, covers = [Default], drive = Drive::Never)]
fn hue_saturation_master(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("master", HueSaturation { hue: 40.0, saturation: 20.0, ..HueSaturation::default() });
    HueSaturationEditor::new("Hue/Saturation", Bind::bound(value)).adjusted_spectrum(&shifted).show(ui).into()
}

/// The Reds range with its four thumbs and the readout.
#[story(HueSaturationEditor<'static>, covers = [], drive = Drive::Never)]
fn hue_saturation_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = HueSaturation::default();
    start.ranges[0] = HueRangeValue { hue: 15.0, ..HueRangeValue::neutral(0) };
    let value = data.value("range", start);
    HueSaturationEditor::new("Hue/Saturation", Bind::bound(value)).range(Some(0)).adjusted_spectrum(&shifted).show(ui).into()
}

/// Colorize on: the range select is off and the Hue runs 0 to 360.
#[story(HueSaturationEditor<'static>, covers = [], drive = Drive::Never)]
fn hue_saturation_colorize(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("colorize", HueSaturation { hue: 210.0, saturation: 40.0, colorize: true, ..HueSaturation::default() });
    HueSaturationEditor::new("Hue/Saturation", Bind::bound(value)).show(ui).into()
}
