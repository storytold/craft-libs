//! The channel maths the colour area, strip, wheel and ramp share: a model channel's range and step, web-safe
//! quantizing and the spoken form of a component.

use craft_theme::{ColorModel, ModelValues, Srgba};
use std::ops::RangeInclusive;

/// The levels of a web colour's channels: 0x00, 0x33 ... 0xFF.
pub(crate) const WEB_LEVELS: usize = 6;

/// Cells per side of a quantised field, and rows of a quantised strip: fine enough that the level edges follow the
/// colour model, coarse enough to draw every frame.
pub(crate) const QUANTIZE_CELLS: usize = 64;

/// The range of `channel` of `model`, smallest first.
pub(crate) fn channel_range(model: &dyn ColorModel, channel: usize) -> RangeInclusive<f64> {
    let ch = model.channels()[channel];
    f64::from(ch.min)..=f64::from(ch.max)
}

/// The step of one arrow press on `channel` of `model`: the channel's own (1 degree, 1 percent, 1 of 255, 0.001 of
/// chroma).
pub(crate) fn channel_step(model: &dyn ColorModel, channel: usize) -> f64 {
    f64::from(model.channels()[channel].step)
}

/// `value` of `channel` kept in the channel's range and on its step.
pub(crate) fn settled(model: &dyn ColorModel, channel: usize, value: f32) -> f32 {
    let (range, step) = (channel_range(model, channel), channel_step(model, channel));
    let (min, max) = (*range.start(), *range.end());
    let value = f64::from(value).clamp(min, max);
    let on_step = if step > 0.0 { min + ((value - min) / step).round() * step } else { value };
    on_step.min(max) as f32
}

/// `values` with each red, green and blue channel of their colour on the nearest of `levels` evenly spaced levels (at
/// least two) from 0 to 1, in the model's values again (a web-safe colour has 6). The other components are what the
/// snapped colour gives.
pub(crate) fn quantized(model: &dyn ColorModel, values: &ModelValues, levels: usize) -> ModelValues {
    model.from_srgb(snapped(model.to_srgb(&values.values), levels))
}

/// `color` with each channel on the nearest of `levels` levels.
pub(crate) fn snapped(color: Srgba, levels: usize) -> Srgba {
    let top = (levels.max(2) - 1) as f32;
    let snap = |v: f32| (v.clamp(0.0, 1.0) * top).round() / top;
    Srgba::new(snap(color.r), snap(color.g), snap(color.b), color.a)
}

/// A component as a screen reader reads it: "Saturation 40 %", "Hue 120°", "Lightness 50".
pub(crate) fn spoken(name: &str, model: &dyn ColorModel, channel: usize, value: f32) -> String {
    let ch = model.channels()[channel];
    let factor = 10f32.powi(i32::from(ch.decimals));
    // Adding zero turns a rounded -0 into 0.
    let value = (value * factor).round() / factor + 0.0;
    let number = format!("{value:.*}", usize::from(ch.decimals));
    match ch.unit {
        "" => format!("{name} {number}"),
        "°" => format!("{name} {number}°"),
        unit => format!("{name} {number} {unit}"),
    }
}

/// `value` when it is a number, else 0.
pub(crate) fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// `values` with every non-finite channel set to 0, as PhotoCraft's model wrapper did: a NaN or infinity from
/// automation or a field must not reach the colour or the remembered values.
pub fn finite_values(mut values: ModelValues) -> ModelValues {
    for v in &mut values.values {
        *v = finite(*v);
    }
    values
}

/// Whether the press that `press` reports was made with the primary button: a drag with another button is the host's.
pub(crate) fn primary(press: &craft_behavior::PressState) -> bool {
    press.phase.start().is_some_and(|start| start.button == craft_behavior::PointerButton::Primary)
}
