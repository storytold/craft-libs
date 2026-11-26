//! Colour models a picker edits a colour in: HSB, RGB, CIE Lab and OKLCH, and the trait hosts
//! implement for their own (CMYK through their working space).
//!
//! Model values are in display units (degrees, percent, 0..=255), so a channel's range, unit and
//! decimals describe what its field shows.

use crate::color::{Hsb, Lab, Oklch, Srgba};

/// The most channels a model has (CMYK).
pub const MAX_CHANNELS: usize = 4;

/// One channel of a [`ColorModel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    /// The full name, English: the key a host translates ("Hue"); screen readers read it.
    pub name: &'static str,
    /// The name beside another model's channel of the same name, English: "HSB hue". A picker
    /// showing both uses it in place of `name`, as a key of its own.
    pub qualified: &'static str,
    /// The short label, English: "H".
    pub label: &'static str,
    /// The lowest value of the field.
    pub min: f32,
    /// The highest value of the field.
    pub max: f32,
    /// The unit drawn inside the field ("°", "%"), empty for none.
    pub unit: &'static str,
    /// Decimals the field shows.
    pub decimals: u8,
    /// One step of the arrow keys, in display units.
    pub step: f32,
    /// Colour stops to sample the channel's track at, evenly from `min` to `max`: two where the
    /// track is a straight blend, more for hue and for perceptual spaces.
    pub track_stops: usize,
}

/// A colour as one model's channel values. It records the model, so a host can tell which model
/// an edit was made in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelValues {
    /// The [`ColorModel::id`] of the model.
    pub model: &'static str,
    /// Channel values in display units; those past the model's channel count are 0.
    pub values: [f32; MAX_CHANNELS],
}

impl ModelValues {
    /// Values for `model`, from the first [`MAX_CHANNELS`] of `values`.
    pub fn new(model: &'static str, values: &[f32]) -> ModelValues {
        let mut out = [0.0; MAX_CHANNELS];
        for (slot, v) in out.iter_mut().zip(values) {
            *slot = *v;
        }
        ModelValues { model, values: out }
    }
}

/// Where a model is offered without the host asking for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultOn {
    /// In the Full picker's channel grid.
    pub full: bool,
    /// In the Compact picker's model menu.
    pub compact: bool,
}

/// A colour model: channels and the conversions to and from encoded sRGB.
///
/// Colour management belongs to the host: a model converts to and from encoded sRGB, and values
/// outside sRGB map into it as the model decides (Lab clamps, OKLCH reduces chroma).
#[allow(clippy::wrong_self_convention)]
pub trait ColorModel {
    /// A stable key, such as `"hsb"`; [`ModelValues::model`] holds it.
    fn id(&self) -> &'static str;
    /// The name for the model menu, English: "HSB".
    fn name(&self) -> &'static str;
    /// The channels, in field order.
    fn channels(&self) -> &[Channel];
    /// The model's values for `color` (alpha ignored).
    fn from_srgb(&self, color: Srgba) -> ModelValues;
    /// The opaque colour for `values` (their [`ModelValues::model`] is not checked).
    fn to_srgb(&self, values: &[f32; MAX_CHANNELS]) -> Srgba;
    /// Where the model is offered unless the host chooses.
    fn default_on(&self) -> DefaultOn;

    /// The colour stops of channel `channel`'s track with the other channels held at `values`.
    fn track(&self, channel: usize, values: &ModelValues) -> Vec<Srgba> {
        let ch = self.channels()[channel];
        let n = ch.track_stops.max(2);
        (0..n)
            .map(|i| {
                let mut v = values.values;
                v[channel] = ch.min + (ch.max - ch.min) * i as f32 / (n - 1) as f32;
                self.to_srgb(&v)
            })
            .collect()
    }
}

/// HSB on encoded sRGB, as Photoshop and PhotoCraft.
#[derive(Clone, Copy, Debug)]
pub struct HsbModel;
/// RGB, 0..=255.
#[derive(Clone, Copy, Debug)]
pub struct RgbModel;
/// CIE Lab, D50, Bradford-adapted.
#[derive(Clone, Copy, Debug)]
pub struct LabModel;
/// OKLCH: lightness in percent, chroma 0..=0.4, hue in degrees. A grey reads as chroma 0 and
/// hue 0 ([`ACHROMATIC`](crate::color::ACHROMATIC)).
#[derive(Clone, Copy, Debug)]
pub struct OklchModel;

const fn channel(
    (name, qualified): (&'static str, &'static str),
    label: &'static str,
    (min, max): (f32, f32),
    unit: &'static str,
    decimals: u8,
    step: f32,
    stops: usize,
) -> Channel {
    Channel { name, qualified, label, min, max, unit, decimals, step, track_stops: stops }
}

const HSB_CHANNELS: [Channel; 3] = [
    channel(("Hue", "HSB hue"), "H", (0.0, 360.0), "°", 0, 1.0, 13),
    channel(("Saturation", "HSB saturation"), "S", (0.0, 100.0), "%", 0, 1.0, 2),
    channel(("Brightness", "HSB brightness"), "B", (0.0, 100.0), "%", 0, 1.0, 2),
];
const RGB_CHANNELS: [Channel; 3] = [
    channel(("Red", "RGB red"), "R", (0.0, 255.0), "", 0, 1.0, 2),
    channel(("Green", "RGB green"), "G", (0.0, 255.0), "", 0, 1.0, 2),
    channel(("Blue", "RGB blue"), "B", (0.0, 255.0), "", 0, 1.0, 2),
];
const LAB_CHANNELS: [Channel; 3] = [
    channel(("Lightness", "Lab lightness"), "L", (0.0, 100.0), "", 0, 1.0, 2),
    channel(("Green-red", "Lab green-red"), "a", (-128.0, 127.0), "", 0, 1.0, 9),
    channel(("Blue-yellow", "Lab blue-yellow"), "b", (-128.0, 127.0), "", 0, 1.0, 9),
];
const OKLCH_CHANNELS: [Channel; 3] = [
    channel(("Lightness", "OKLCH lightness"), "L", (0.0, 100.0), "%", 0, 1.0, 9),
    channel(("Chroma", "OKLCH chroma"), "C", (0.0, 0.4), "", 3, 0.001, 9),
    channel(("Hue", "OKLCH hue"), "H", (0.0, 360.0), "°", 0, 1.0, 13),
];

impl ColorModel for HsbModel {
    fn id(&self) -> &'static str {
        "hsb"
    }
    fn name(&self) -> &'static str {
        "HSB"
    }
    fn channels(&self) -> &[Channel] {
        &HSB_CHANNELS
    }
    fn from_srgb(&self, color: Srgba) -> ModelValues {
        let Hsb { h, s, b } = color.to_hsb();
        ModelValues::new(self.id(), &[h, s * 100.0, b * 100.0])
    }
    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        Srgba::from_hsb(Hsb { h: v[0], s: v[1] / 100.0, b: v[2] / 100.0 })
    }
    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: true, compact: true }
    }
}

impl ColorModel for RgbModel {
    fn id(&self) -> &'static str {
        "rgb"
    }
    fn name(&self) -> &'static str {
        "RGB"
    }
    fn channels(&self) -> &[Channel] {
        &RGB_CHANNELS
    }
    fn from_srgb(&self, c: Srgba) -> ModelValues {
        ModelValues::new(self.id(), &[c.r * 255.0, c.g * 255.0, c.b * 255.0])
    }
    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        Srgba::rgb(v[0] / 255.0, v[1] / 255.0, v[2] / 255.0)
    }
    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: true, compact: true }
    }
}

impl ColorModel for LabModel {
    fn id(&self) -> &'static str {
        "lab"
    }
    fn name(&self) -> &'static str {
        "Lab"
    }
    fn channels(&self) -> &[Channel] {
        &LAB_CHANNELS
    }
    fn from_srgb(&self, color: Srgba) -> ModelValues {
        let Lab { l, a, b } = color.to_lab();
        ModelValues::new(self.id(), &[l, a, b])
    }
    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        Srgba::from_lab(Lab { l: v[0], a: v[1], b: v[2] })
    }
    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: true, compact: false }
    }
}

impl ColorModel for OklchModel {
    fn id(&self) -> &'static str {
        "oklch"
    }
    fn name(&self) -> &'static str {
        "OKLCH"
    }
    fn channels(&self) -> &[Channel] {
        &OKLCH_CHANNELS
    }
    fn from_srgb(&self, color: Srgba) -> ModelValues {
        let Oklch { l, c, h } = color.to_oklch();
        ModelValues::new(self.id(), &[(l * 100.0) as f32, c as f32, h as f32])
    }
    fn to_srgb(&self, v: &[f32; MAX_CHANNELS]) -> Srgba {
        Srgba::from_oklch(Oklch { l: f64::from(v[0]) / 100.0, c: f64::from(v[1]), h: f64::from(v[2]) })
    }
    /// An entry of the Compact picker's model menu; off in the Full picker's grid, which keeps
    /// Photoshop's layout unless the host turns it on.
    fn default_on(&self) -> DefaultOn {
        DefaultOn { full: false, compact: true }
    }
}

/// The four built-in models: HSB, RGB, Lab, OKLCH.
pub fn builtin_models() -> [&'static dyn ColorModel; 4] {
    [&HsbModel, &RgbModel, &LabModel, &OklchModel]
}

/// The values a picker shows for `color` in `model`: the `kept` values while they are of this
/// model and still give the same 8-bit sRGB as `color`, otherwise the model's values for `color`.
///
/// Greys keep their hue, black keeps hue and saturation, and typed Lab or host-model values
/// outside sRGB stay as typed, none of which sRGB alone can say (PhotoCraft's `Keep`).
pub fn kept_or_converted(model: &dyn ColorModel, color: Srgba, kept: Option<&ModelValues>) -> ModelValues {
    match kept {
        Some(k) if k.model == model.id() && model.to_srgb(&k.values).same_rgb8(color) => *k,
        _ => model.from_srgb(color),
    }
}
