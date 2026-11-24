//! Stories of Hue/Saturation: Master, a colour range with its thumbs, Colorize, and edited ranges on their swatches.

use super::{HueRangeValue, HueSaturation, HueSaturationEditor};
use craft_theme::{Hsb, Srgba};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// What `color` becomes under `value`: a stand-in for the host's engine. Colorize tints the colour with the master hue
/// and saturation; otherwise the master and the ranges whose full span holds the colour's hue shift that hue, scale
/// its saturation and move its brightness.
fn adjusted(value: &HueSaturation, color: Srgba) -> Srgba {
    let Hsb { h, s, b } = color.to_hsb();
    if value.colorize {
        let b = (f64::from(b) * (1.0 + value.lightness / 100.0)).clamp(0.0, 1.0);
        return Srgba::from_hsb(Hsb { h: value.hue.rem_euclid(360.0) as f32, s: (value.saturation / 100.0) as f32, b: b as f32 });
    }
    let (mut hue, mut saturation, mut lightness) = (value.hue, value.saturation, value.lightness);
    for range in &value.ranges {
        let bounds = HueRangeValue::canonical_bounds(range.bounds);
        let inside = (f64::from(h) - bounds[1]).rem_euclid(360.0) <= bounds[2] - bounds[1];
        if inside {
            (hue, saturation, lightness) = (hue + range.hue, saturation + range.saturation, lightness + range.lightness);
        }
    }
    let s = (f64::from(s) * (1.0 + saturation / 100.0)).clamp(0.0, 1.0);
    let b = (f64::from(b) * (1.0 + lightness / 100.0)).clamp(0.0, 1.0);
    Srgba::from_hsb(Hsb { h: (f64::from(h) + hue).rem_euclid(360.0) as f32, s: s as f32, b: b as f32 })
}

#[story(HueSaturationEditor<'static>, covers = [Default], drive = Drive::Never)]
fn hue_saturation_master(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("master", HueSaturation { hue: 40.0, saturation: 20.0, ..HueSaturation::default() });
    HueSaturationEditor::new("Hue/Saturation", value_at.bind(value)).adjusted(&adjusted).show(ui).into()
}

/// The Reds range with its four thumbs and the readout, its Hue moved: the split swatch, its dot and "0° / 15°".
#[story(HueSaturationEditor<'static>, covers = [], drive = Drive::Never)]
fn hue_saturation_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = HueSaturation::default();
    start.ranges[0] = HueRangeValue { hue: 15.0, ..HueRangeValue::neutral(0) };
    let (value, value_at) = data.value_at("range", start);
    HueSaturationEditor::new("Hue/Saturation", value_at.bind(value)).range(Some(0)).adjusted(&adjusted).show(ui).into()
}

/// Colorize on: one read-only swatch of the colorize colour, and the Hue runs 0 to 360.
#[story(HueSaturationEditor<'static>, covers = [], drive = Drive::Never)]
fn hue_saturation_colorize(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, value_at) = data.value_at("colorize", HueSaturation { hue: 210.0, saturation: 40.0, colorize: true, ..HueSaturation::default() });
    HueSaturationEditor::new("Hue/Saturation", value_at.bind(value)).adjusted(&adjusted).show(ui).into()
}

/// Master shown with the Reds' Hue at +30 and the Blues' Saturation at −80: their swatches split before and after,
/// each with its dot.
#[story(HueSaturationEditor<'static>, covers = [], drive = Drive::Never)]
fn hue_saturation_edited_ranges(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = HueSaturation::default();
    start.ranges[0] = HueRangeValue { hue: 30.0, ..HueRangeValue::neutral(0) };
    start.ranges[4] = HueRangeValue { saturation: -80.0, ..HueRangeValue::neutral(4) };
    let (value, value_at) = data.value_at("edited", start);
    HueSaturationEditor::new("Hue/Saturation", value_at.bind(value)).adjusted(&adjusted).show(ui).into()
}
