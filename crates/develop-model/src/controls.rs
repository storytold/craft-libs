//! Control specs: one table describes every numeric develop slider — id, label, section, range,
//! default, step, display precision and track style. The UI builds panels from it, MCP exposes it as
//! a schema, and `get`/`set` address settings by id (e.g. `light.exposure`, `mixer.red.hue`).
//!
//! Settings that come in lists (Point Color samples) are *indexed* controls: one template spec in
//! [`INDEXED`] (`pointColor.hueShift`) addresses every element as `pointColor.<i>.hueShift`.

use serde::{Deserialize, Serialize};

use crate::settings::DevelopSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    Light,
    Curve,
    Color,
    Mixer,
    BwMix,
    Grading,
    Effects,
    Vignette,
    Grain,
    Detail,
    Optics,
    Geometry,
    Profile,
    Calibration,
    PointColor,
    RedEye,
    LensBlur,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Light => "Light",
            Section::Curve => "Tone Curve",
            Section::Color => "Color",
            Section::Mixer => "Color Mixer",
            Section::BwMix => "B&W Mixer",
            Section::Grading => "Color Grading",
            Section::Effects => "Effects",
            Section::Vignette => "Vignette",
            Section::Grain => "Grain",
            Section::Detail => "Detail",
            Section::Optics => "Optics",
            Section::Geometry => "Geometry",
            Section::Profile => "Profile",
            Section::Calibration => "Calibration",
            Section::PointColor => "Point Color",
            Section::RedEye => "Red Eye",
            Section::LensBlur => "Lens Blur",
        }
    }
}

/// How the slider track is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Track {
    /// Plain grey track, fill from the minimum.
    Plain,
    /// Grey track, zero in the middle.
    Centered,
    /// Blue → yellow (white balance temperature).
    Temp,
    /// Green → magenta (tint).
    Tint,
    /// A two-colour gradient (sRGB hex strings).
    Gradient { from: &'static str, to: &'static str },
    /// Hue shift for a mixer band: gradient from the neighbouring hues through the band colour.
    Hue { band: u8 },
    /// Saturation for a band: grey → band colour.
    Sat { band: u8 },
    /// Luminance for a band: dark → light band colour.
    Lum { band: u8 },
    /// Full hue rainbow.
    Rainbow,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ControlSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub section: Section,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub step: f64,
    /// Decimal places shown.
    pub decimals: u8,
    pub track: Track,
}

impl ControlSpec {
    pub fn clamp(&self, v: f64) -> f64 {
        if v.is_finite() { v.clamp(self.min, self.max) } else { self.default }
    }
    pub fn format(&self, v: f64) -> String {
        let s = format!("{:.*}", self.decimals as usize, v);
        if v.is_finite() && !s.bytes().any(|b| matches!(b, b'1'..=b'9')) {
            // Rounds to zero: no sign.
            return s.trim_start_matches('-').to_string();
        }
        if v > 0.0 && self.min < 0.0 { format!("+{s}") } else { s }
    }
    /// Where `v` sits on the slider track, from 0 (left end) to 1 (right end). Linear, except for the Kelvin
    /// temperature, which follows Lightroom's scale: 2000–7000 K fill the left half evenly (100 K per 1 %), and
    /// the right half grows logarithmically up to 50000 K, so everyday temperatures aren't crowded into the
    /// left sixth of the track. The relative temperature of rendered files (−100…+100, already in mireds) stays
    /// linear.
    pub fn position(&self, v: f64) -> f64 {
        let v = self.clamp(v);
        let p = match self.kelvin_scale() {
            Some((centre, right)) if v > centre => 0.5 + 0.5 * (1.0 + (v - centre) / KELVIN_KNEE).ln() / right,
            Some((centre, _)) => 0.5 * (v - self.min) / (centre - self.min),
            None => (v - self.min) / (self.max - self.min).max(1e-9),
        };
        if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 }
    }
    /// The value at track position `p` (0…1): the inverse of [`ControlSpec::position`], not yet on the steps.
    pub fn value_at(&self, p: f64) -> f64 {
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
        let v = match self.kelvin_scale() {
            Some(_) if p >= 1.0 => self.max,
            Some((centre, right)) if p > 0.5 => centre + KELVIN_KNEE * (((2.0 * p - 1.0) * right).exp() - 1.0),
            Some((centre, _)) => self.min + 2.0 * p * (centre - self.min),
            None => self.min + p * (self.max - self.min),
        };
        self.clamp(v)
    }
    /// The Kelvin temperature's track centre and `ln` of its right half's span (see [`ControlSpec::position`]);
    /// `None` for every other control, or a range the centre isn't inside.
    fn kelvin_scale(&self) -> Option<(f64, f64)> {
        let centre = KELVIN_CENTRE;
        (self.id == "wb.temp" && self.min < centre && centre < self.max).then(|| (centre, (1.0 + (self.max - centre) / KELVIN_KNEE).ln()))
    }
}

/// The Kelvin temperature at the middle of its track, as in Lightroom.
const KELVIN_CENTRE: f64 = 7000.0;
/// Where the right half of the Kelvin track turns logarithmic: 1000 K gives it close to the left half's slope
/// just past the centre (76 K against 100 K per 1 % of the track), so the thumb moves evenly across it.
const KELVIN_KNEE: f64 = 1000.0;

macro_rules! controls {
    ($( $id:literal => $($field:ident).+ , $label:literal, $section:ident, $min:expr, $max:expr, $def:expr, $step:expr, $dec:expr, $track:expr; )*) => {
        /// All numeric develop controls, in panel order.
        pub static CONTROLS: &[ControlSpec] = &[
            $( ControlSpec { id: $id, label: $label, section: Section::$section, min: $min as f64, max: $max as f64, default: $def as f64, step: $step as f64, decimals: $dec, track: $track }, )*
        ];

        /// Read a control value by id.
        pub fn get(s: &DevelopSettings, id: &str) -> Option<f64> {
            match id {
                $( $id => Some(s.$($field).+ as f64), )*
                _ => indexed_get(s, id),
            }
        }

        /// Write a control value by id (clamped to the spec range). Returns false for unknown ids.
        pub fn set(s: &mut DevelopSettings, id: &str, v: f64) -> bool {
            let Some(spec) = find(id) else { return false };
            let v = spec.clamp(v);
            match id {
                $( $id => { s.$($field).+ = v as _; true } )*
                _ => indexed_set(s, id, v),
            }
        }
    };
}

use Track::*;

controls! {
    "profile.amount" => profile.amount, "Amount", Profile, 0, 200, 100, 1, 0, Plain;
    "wb.temp" => wb.temp, "Temp", Color, 2000, 50000, 6500, 50, 0, Temp;
    "wb.tint" => wb.tint, "Tint", Color, -150, 150, 0, 1, 0, Tint;
    "light.exposure" => light.exposure, "Exposure", Light, -5, 5, 0, 0.01, 2, Centered;
    "light.contrast" => light.contrast, "Contrast", Light, -100, 100, 0, 1, 0, Centered;
    "light.highlights" => light.highlights, "Highlights", Light, -100, 100, 0, 1, 0, Centered;
    "light.shadows" => light.shadows, "Shadows", Light, -100, 100, 0, 1, 0, Centered;
    "light.whites" => light.whites, "Whites", Light, -100, 100, 0, 1, 0, Centered;
    "light.blacks" => light.blacks, "Blacks", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.maxEv" => hdr.max_ev, "HDR Headroom", Light, 0, 5, 3, 0.1, 1, Plain;
    "hdr.sdrBrightness" => hdr.sdr_brightness, "SDR Brightness", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.sdrContrast" => hdr.sdr_contrast, "SDR Contrast", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.sdrHighlights" => hdr.sdr_highlights, "SDR Highlights", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.sdrShadows" => hdr.sdr_shadows, "SDR Shadows", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.sdrWhites" => hdr.sdr_whites, "SDR Whites", Light, -100, 100, 0, 1, 0, Centered;
    "hdr.sdrClarity" => hdr.sdr_clarity, "SDR Clarity", Light, -100, 100, 0, 1, 0, Centered;
    "curve.highlights" => curve.highlights, "Highlights", Curve, -100, 100, 0, 1, 0, Centered;
    "curve.lights" => curve.lights, "Lights", Curve, -100, 100, 0, 1, 0, Centered;
    "curve.darks" => curve.darks, "Darks", Curve, -100, 100, 0, 1, 0, Centered;
    "curve.shadows" => curve.shadows, "Shadows", Curve, -100, 100, 0, 1, 0, Centered;
    "curve.splitShadows" => curve.split_shadows, "Shadows split", Curve, 10, 70, 25, 1, 0, Plain;
    "curve.splitMid" => curve.split_mid, "Midtones split", Curve, 20, 80, 50, 1, 0, Plain;
    "curve.splitHighlights" => curve.split_highlights, "Highlights split", Curve, 30, 90, 75, 1, 0, Plain;
    "curve.refineSaturation" => curve.refine_saturation, "Refine Saturation", Curve, 0, 100, 100, 1, 0, Gradient { from: "#7a7a7a", to: "#d85a3a" };
    "color.vibrance" => color.vibrance, "Vibrance", Color, -100, 100, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#d8406a" };
    "color.saturation" => color.saturation, "Saturation", Color, -100, 100, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#e04a3a" };
    "mixer.red.hue" => mixer.red.hue, "Red Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 0 };
    "mixer.red.sat" => mixer.red.sat, "Red Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 0 };
    "mixer.red.lum" => mixer.red.lum, "Red Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 0 };
    "mixer.orange.hue" => mixer.orange.hue, "Orange Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 1 };
    "mixer.orange.sat" => mixer.orange.sat, "Orange Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 1 };
    "mixer.orange.lum" => mixer.orange.lum, "Orange Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 1 };
    "mixer.yellow.hue" => mixer.yellow.hue, "Yellow Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 2 };
    "mixer.yellow.sat" => mixer.yellow.sat, "Yellow Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 2 };
    "mixer.yellow.lum" => mixer.yellow.lum, "Yellow Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 2 };
    "mixer.green.hue" => mixer.green.hue, "Green Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 3 };
    "mixer.green.sat" => mixer.green.sat, "Green Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 3 };
    "mixer.green.lum" => mixer.green.lum, "Green Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 3 };
    "mixer.aqua.hue" => mixer.aqua.hue, "Aqua Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 4 };
    "mixer.aqua.sat" => mixer.aqua.sat, "Aqua Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 4 };
    "mixer.aqua.lum" => mixer.aqua.lum, "Aqua Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 4 };
    "mixer.blue.hue" => mixer.blue.hue, "Blue Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 5 };
    "mixer.blue.sat" => mixer.blue.sat, "Blue Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 5 };
    "mixer.blue.lum" => mixer.blue.lum, "Blue Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 5 };
    "mixer.purple.hue" => mixer.purple.hue, "Purple Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 6 };
    "mixer.purple.sat" => mixer.purple.sat, "Purple Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 6 };
    "mixer.purple.lum" => mixer.purple.lum, "Purple Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 6 };
    "mixer.magenta.hue" => mixer.magenta.hue, "Magenta Hue", Mixer, -100, 100, 0, 1, 0, Hue { band: 7 };
    "mixer.magenta.sat" => mixer.magenta.sat, "Magenta Saturation", Mixer, -100, 100, 0, 1, 0, Sat { band: 7 };
    "mixer.magenta.lum" => mixer.magenta.lum, "Magenta Luminance", Mixer, -100, 100, 0, 1, 0, Lum { band: 7 };
    "bw.red" => bw_mix.red, "Red", BwMix, -100, 100, 0, 1, 0, Lum { band: 0 };
    "bw.orange" => bw_mix.orange, "Orange", BwMix, -100, 100, 0, 1, 0, Lum { band: 1 };
    "bw.yellow" => bw_mix.yellow, "Yellow", BwMix, -100, 100, 0, 1, 0, Lum { band: 2 };
    "bw.green" => bw_mix.green, "Green", BwMix, -100, 100, 0, 1, 0, Lum { band: 3 };
    "bw.aqua" => bw_mix.aqua, "Aqua", BwMix, -100, 100, 0, 1, 0, Lum { band: 4 };
    "bw.blue" => bw_mix.blue, "Blue", BwMix, -100, 100, 0, 1, 0, Lum { band: 5 };
    "bw.purple" => bw_mix.purple, "Purple", BwMix, -100, 100, 0, 1, 0, Lum { band: 6 };
    "bw.magenta" => bw_mix.magenta, "Magenta", BwMix, -100, 100, 0, 1, 0, Lum { band: 7 };
    "grading.shadows.hue" => grading.shadows.hue, "Shadows Hue", Grading, 0, 360, 0, 1, 0, Rainbow;
    "grading.shadows.sat" => grading.shadows.sat, "Shadows Saturation", Grading, 0, 100, 0, 1, 0, Plain;
    "grading.shadows.lum" => grading.shadows.lum, "Shadows Luminance", Grading, -100, 100, 0, 1, 0, Centered;
    "grading.midtones.hue" => grading.midtones.hue, "Midtones Hue", Grading, 0, 360, 0, 1, 0, Rainbow;
    "grading.midtones.sat" => grading.midtones.sat, "Midtones Saturation", Grading, 0, 100, 0, 1, 0, Plain;
    "grading.midtones.lum" => grading.midtones.lum, "Midtones Luminance", Grading, -100, 100, 0, 1, 0, Centered;
    "grading.highlights.hue" => grading.highlights.hue, "Highlights Hue", Grading, 0, 360, 0, 1, 0, Rainbow;
    "grading.highlights.sat" => grading.highlights.sat, "Highlights Saturation", Grading, 0, 100, 0, 1, 0, Plain;
    "grading.highlights.lum" => grading.highlights.lum, "Highlights Luminance", Grading, -100, 100, 0, 1, 0, Centered;
    "grading.global.hue" => grading.global.hue, "Global Hue", Grading, 0, 360, 0, 1, 0, Rainbow;
    "grading.global.sat" => grading.global.sat, "Global Saturation", Grading, 0, 100, 0, 1, 0, Plain;
    "grading.global.lum" => grading.global.lum, "Global Luminance", Grading, -100, 100, 0, 1, 0, Centered;
    "grading.blending" => grading.blending, "Blending", Grading, 0, 100, 50, 1, 0, Plain;
    "grading.balance" => grading.balance, "Balance", Grading, -100, 100, 0, 1, 0, Centered;
    "effects.texture" => effects.texture, "Texture", Effects, -100, 100, 0, 1, 0, Centered;
    "effects.clarity" => effects.clarity, "Clarity", Effects, -100, 100, 0, 1, 0, Centered;
    "effects.dehaze" => effects.dehaze, "Dehaze", Effects, -100, 100, 0, 1, 0, Centered;
    "vignette.amount" => vignette.amount, "Vignette", Vignette, -100, 100, 0, 1, 0, Gradient { from: "#101010", to: "#f0f0f0" };
    "vignette.midpoint" => vignette.midpoint, "Midpoint", Vignette, 0, 100, 50, 1, 0, Plain;
    "vignette.roundness" => vignette.roundness, "Roundness", Vignette, -100, 100, 0, 1, 0, Centered;
    "vignette.feather" => vignette.feather, "Feather", Vignette, 0, 100, 50, 1, 0, Plain;
    "vignette.highlights" => vignette.highlights, "Highlights", Vignette, 0, 100, 0, 1, 0, Plain;
    "grain.amount" => grain.amount, "Grain", Grain, 0, 100, 0, 1, 0, Plain;
    "grain.size" => grain.size, "Size", Grain, 0, 100, 25, 1, 0, Plain;
    "grain.roughness" => grain.roughness, "Roughness", Grain, 0, 100, 50, 1, 0, Plain;
    "enhance.denoise" => enhance.denoise, "Amount", Detail, 0, 100, 0, 1, 0, Plain;
    "detail.sharpenAmount" => detail.sharpen_amount, "Sharpening", Detail, 0, 150, 0, 1, 0, Plain;
    "detail.sharpenRadius" => detail.sharpen_radius, "Radius", Detail, 0.5, 3, 1, 0.1, 1, Plain;
    "detail.sharpenDetail" => detail.sharpen_detail, "Detail", Detail, 0, 100, 25, 1, 0, Plain;
    "detail.sharpenMasking" => detail.sharpen_masking, "Masking", Detail, 0, 100, 0, 1, 0, Plain;
    "detail.nrLuminance" => detail.nr_luminance, "Noise Reduction", Detail, 0, 100, 0, 1, 0, Plain;
    "detail.nrDetail" => detail.nr_detail, "Detail", Detail, 0, 100, 50, 1, 0, Plain;
    "detail.nrContrast" => detail.nr_contrast, "Contrast", Detail, 0, 100, 0, 1, 0, Plain;
    "detail.nrColor" => detail.nr_color, "Color Noise Reduction", Detail, 0, 100, 0, 1, 0, Plain;
    "detail.nrColorDetail" => detail.nr_color_detail, "Detail", Detail, 0, 100, 50, 1, 0, Plain;
    "detail.nrColorSmoothness" => detail.nr_color_smoothness, "Smoothness", Detail, 0, 100, 50, 1, 0, Plain;
    "optics.distortion" => optics.distortion, "Distortion", Optics, -100, 100, 0, 1, 0, Centered;
    "optics.vignetting" => optics.vignetting, "Vignetting", Optics, -100, 100, 0, 1, 0, Centered;
    "optics.vignettingMidpoint" => optics.vignetting_midpoint, "Midpoint", Optics, 0, 100, 50, 1, 0, Plain;
    "optics.profileDistortion" => optics.profile_distortion, "Profile Distortion", Optics, 0, 200, 100, 1, 0, Plain;
    "optics.profileVignetting" => optics.profile_vignetting, "Profile Vignetting", Optics, 0, 200, 100, 1, 0, Plain;
    "optics.defringePurple" => optics.defringe_purple_amount, "Purple Amount", Optics, 0, 20, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#a040d0" };
    "optics.defringePurpleHueLo" => optics.defringe_purple_hue_lo, "Purple Hue Low", Optics, 0, 90, 30, 1, 0, Gradient { from: "#6040e0", to: "#d040b0" };
    "optics.defringePurpleHueHi" => optics.defringe_purple_hue_hi, "Purple Hue High", Optics, 10, 100, 70, 1, 0, Gradient { from: "#6040e0", to: "#d040b0" };
    "optics.defringeGreen" => optics.defringe_green_amount, "Green Amount", Optics, 0, 20, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#40c040" };
    "optics.defringeGreenHueLo" => optics.defringe_green_hue_lo, "Green Hue Low", Optics, 0, 90, 40, 1, 0, Gradient { from: "#c0d030", to: "#30c0b0" };
    "optics.defringeGreenHueHi" => optics.defringe_green_hue_hi, "Green Hue High", Optics, 10, 100, 60, 1, 0, Gradient { from: "#c0d030", to: "#30c0b0" };
    "optics.caRed" => optics.ca_red, "Red/Cyan Fringe", Optics, -100, 100, 0, 1, 0, Gradient { from: "#30c0c0", to: "#e04040" };
    "optics.caBlue" => optics.ca_blue, "Blue/Yellow Fringe", Optics, -100, 100, 0, 1, 0, Gradient { from: "#d0c030", to: "#4060e0" };
    "geometry.vertical" => geometry.vertical, "Vertical", Geometry, -100, 100, 0, 1, 0, Centered;
    "geometry.horizontal" => geometry.horizontal, "Horizontal", Geometry, -100, 100, 0, 1, 0, Centered;
    "geometry.rotate" => geometry.rotate, "Rotate", Geometry, -10, 10, 0, 0.1, 1, Centered;
    "geometry.aspect" => geometry.aspect, "Aspect", Geometry, -100, 100, 0, 1, 0, Centered;
    "geometry.scale" => geometry.scale, "Scale", Geometry, 50, 150, 100, 1, 0, Plain;
    "geometry.offsetX" => geometry.offset_x, "Offset X", Geometry, -100, 100, 0, 0.1, 1, Centered;
    "geometry.offsetY" => geometry.offset_y, "Offset Y", Geometry, -100, 100, 0, 0.1, 1, Centered;
    "lensBlur.amount" => lens_blur.amount, "Blur Amount", LensBlur, 0, 100, 0, 1, 0, Plain;
    "lensBlur.boost" => lens_blur.boost, "Boost", LensBlur, 0, 100, 0, 1, 0, Plain;
    "lensBlur.transition" => lens_blur.transition, "Transition", LensBlur, 0, 100, 50, 1, 0, Plain;
    "calibration.shadowsTint" => calibration.shadows_tint, "Shadows Tint", Calibration, -100, 100, 0, 1, 0, Tint;
    "calibration.redHue" => calibration.red_hue, "Red Hue", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#e0306a", to: "#e08a30" };
    "calibration.redSat" => calibration.red_sat, "Red Saturation", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#e03a3a" };
    "calibration.greenHue" => calibration.green_hue, "Green Hue", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#b0c030", to: "#30c08a" };
    "calibration.greenSat" => calibration.green_sat, "Green Saturation", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#40b050" };
    "calibration.blueHue" => calibration.blue_hue, "Blue Hue", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#30a0d0", to: "#8040d0" };
    "calibration.blueSat" => calibration.blue_sat, "Blue Saturation", Calibration, -100, 100, 0, 1, 0, Gradient { from: "#7a7a7a", to: "#3a60e0" };
    "crop.angle" => crop.geometry.angle, "Straighten", Geometry, -45, 45, 0, 0.01, 2, Centered;
}

/// Template specs of the indexed controls (`<family>.<field>`; addressed as `<family>.<i>.<field>`).
pub static INDEXED: &[ControlSpec] = &[
    ControlSpec {
        id: "pointColor.hueShift",
        label: "Hue",
        section: Section::PointColor,
        min: -100.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Rainbow,
    },
    ControlSpec {
        id: "pointColor.satShift",
        label: "Saturation",
        section: Section::PointColor,
        min: -100.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Gradient { from: "#7a7a7a", to: "#e04a3a" },
    },
    ControlSpec {
        id: "pointColor.lumShift",
        label: "Luminance",
        section: Section::PointColor,
        min: -100.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Gradient { from: "#202020", to: "#f0f0f0" },
    },
    ControlSpec {
        id: "pointColor.variance",
        label: "Variance",
        section: Section::PointColor,
        min: -100.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Centered,
    },
    ControlSpec {
        id: "pointColor.range",
        label: "Range",
        section: Section::PointColor,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Plain,
    },
    ControlSpec {
        id: "pointColor.hueRange",
        label: "Hue Range",
        section: Section::PointColor,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Rainbow,
    },
    ControlSpec {
        id: "pointColor.satRange",
        label: "Saturation Range",
        section: Section::PointColor,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Gradient { from: "#7a7a7a", to: "#e04a3a" },
    },
    ControlSpec {
        id: "pointColor.lumRange",
        label: "Luminance Range",
        section: Section::PointColor,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Gradient { from: "#202020", to: "#f0f0f0" },
    },
    ControlSpec {
        id: "redEye.pupilSize",
        label: "Pupil Size",
        section: Section::RedEye,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Plain,
    },
    ControlSpec {
        id: "redEye.darken",
        label: "Darken",
        section: Section::RedEye,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Gradient { from: "#d0d0d0", to: "#101010" },
    },
];

/// `pointColor.3.hueShift` → (`pointColor`, 3, `hueShift`).
fn split_indexed(id: &str) -> Option<(&str, usize, &str)> {
    let mut it = id.splitn(3, '.');
    let (fam, i, field) = (it.next()?, it.next()?, it.next()?);
    if i.is_empty() || !i.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((fam, i.parse().ok()?, field))
}

fn indexed_field<'a>(s: &'a mut DevelopSettings, id: &str) -> Option<&'a mut f64> {
    let (fam, i, field) = split_indexed(id)?;
    match fam {
        "pointColor" => {
            let p = s.point_colors.get_mut(i)?;
            Some(match field {
                "hueShift" => &mut p.hue_shift,
                "satShift" => &mut p.sat_shift,
                "lumShift" => &mut p.lum_shift,
                "variance" => &mut p.variance,
                "range" => &mut p.range,
                "hueRange" => &mut p.hue_range,
                "satRange" => &mut p.sat_range,
                "lumRange" => &mut p.lum_range,
                _ => return None,
            })
        }
        "redEye" => {
            let e = s.red_eye.get_mut(i)?;
            Some(match field {
                "pupilSize" => &mut e.pupil_size,
                "darken" => &mut e.darken,
                _ => return None,
            })
        }
        _ => None,
    }
}

fn indexed_get(s: &DevelopSettings, id: &str) -> Option<f64> {
    // one accessor for get and set: a copy of the one element read (never of the lists, which a
    // read of every indexed control would copy once per control)
    let (fam, i, field) = split_indexed(id)?;
    let mut c = DevelopSettings::default();
    match fam {
        "pointColor" => c.point_colors.push(*s.point_colors.get(i)?),
        "redEye" => c.red_eye.push(*s.red_eye.get(i)?),
        _ => return None,
    }
    indexed_field(&mut c, &format!("{fam}.0.{field}")).map(|v| *v)
}

fn indexed_set(s: &mut DevelopSettings, id: &str, v: f64) -> bool {
    match indexed_field(s, id) {
        Some(f) => {
            *f = v;
            true
        }
        None => false,
    }
}

/// The spec of a control id, including indexed ids (`pointColor.0.hueShift` → the
/// `pointColor.hueShift` template).
pub fn find(id: &str) -> Option<&'static ControlSpec> {
    if let Some(c) = CONTROLS.iter().find(|c| c.id == id) {
        return Some(c);
    }
    let (fam, _, field) = split_indexed(id)?;
    INDEXED.iter().find(|c| c.id.strip_prefix(fam).and_then(|r| r.strip_prefix('.')) == Some(field))
}

/// Every indexed control that exists in `s` (id, template spec), e.g. `pointColor.0.hueShift`.
pub fn indexed_instances(s: &DevelopSettings) -> Vec<(String, &'static ControlSpec)> {
    let mut out = Vec::new();
    for spec in INDEXED {
        let Some((fam, field)) = spec.id.split_once('.') else { continue };
        let n = match fam {
            "pointColor" => s.point_colors.len(),
            "redEye" => s.red_eye.len(),
            _ => 0,
        };
        for i in 0..n {
            out.push((format!("{fam}.{i}.{field}"), spec));
        }
    }
    out
}

pub fn in_section(section: Section) -> impl Iterator<Item = &'static ControlSpec> {
    CONTROLS.iter().filter(move |c| c.section == section)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_shows_no_sign_on_a_value_that_rounds_to_zero() {
        let e = find("light.exposure").unwrap();
        assert_eq!(e.format(0.004), "0.00");
        assert_eq!(e.format(-0.004), "0.00");
        assert_eq!(e.format(0.5), "+0.50");
        assert_eq!(e.format(-0.5), "-0.50");
        assert_eq!(e.format(f64::INFINITY), "+inf");
        assert_eq!(e.format(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn every_control_reads_its_default() {
        let s = DevelopSettings::default();
        for c in CONTROLS {
            let v = get(&s, c.id).unwrap_or_else(|| panic!("no getter for {}", c.id));
            assert_eq!(v, c.default, "{}", c.id);
            assert!(c.min <= c.default && c.default <= c.max, "{}", c.id);
        }
    }

    #[test]
    fn ids_unique_and_settable() {
        let mut seen = std::collections::HashSet::new();
        let mut s = DevelopSettings::default();
        for c in CONTROLS {
            assert!(seen.insert(c.id), "duplicate {}", c.id);
            assert!(set(&mut s, c.id, c.max + 1000.0));
            assert_eq!(get(&s, c.id), Some(c.max), "{}", c.id);
            assert!(set(&mut s, c.id, f64::NAN));
            assert_eq!(get(&s, c.id), Some(c.default));
        }
        assert!(!set(&mut s, "nope", 1.0));
        assert_eq!(get(&s, "nope"), None);
    }

    #[test]
    fn indexed_controls_address_list_elements() {
        let mut s = DevelopSettings::default();
        assert_eq!(get(&s, "pointColor.0.hueShift"), None);
        assert!(!set(&mut s, "pointColor.0.hueShift", 10.0), "no sample yet");
        s.point_colors.push(Default::default());
        s.point_colors.push(Default::default());
        assert!(set(&mut s, "pointColor.1.hueShift", 250.0));
        assert_eq!(s.point_colors[1].hue_shift, 100.0, "clamped by the template spec");
        assert_eq!(get(&s, "pointColor.1.range"), Some(50.0));
        assert_eq!(find("pointColor.7.lumRange").map(|c| c.id), Some("pointColor.lumRange"));
        assert!(find("pointColor.x.lumRange").is_none() && find("pointColor.0.nope").is_none());
        let ids: Vec<String> = indexed_instances(&s).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 2 * INDEXED.iter().filter(|c| c.id.starts_with("pointColor.")).count());
        for (id, spec) in indexed_instances(&s) {
            assert!(get(&s, &id).is_some(), "{id}");
            assert!(spec.min <= spec.default && spec.default <= spec.max);
        }
    }

    #[test]
    fn signed_indexes_are_unknown_ids() {
        let mut s = DevelopSettings::default();
        s.point_colors.push(Default::default());
        let before = s.point_colors[0].hue_shift;
        assert!(!set(&mut s, "pointColor.+0.hueShift", 10.0), "signed index");
        assert_eq!(s.point_colors[0].hue_shift, before);
        assert_eq!(get(&s, "pointColor.+0.hueShift"), None);
        assert!(find("pointColor.+0.hueShift").is_none());
        assert!(find("redEye.+0.darken").is_none());
    }

    #[test]
    fn formatting() {
        let e = find("light.exposure").unwrap();
        assert_eq!(e.format(0.5), "+0.50");
        assert_eq!(e.format(-1.0), "-1.00");
        assert_eq!(find("wb.temp").unwrap().format(5500.0), "5500");
    }

    /// The Kelvin temperature's track follows Lightroom's (positions measured in Lightroom Classic against
    /// the linear Tint slider; the model is within 2 % of them); every other control stays linear.
    #[test]
    fn the_kelvin_track_is_lightrooms_and_others_stay_linear() {
        let temp = find("wb.temp").unwrap();
        for (k, lightroom, model) in [
            (2000.0, 0.0, 0.0),
            (3650.0, 0.167, 0.165),
            (5400.0, 0.333, 0.34),
            (7000.0, 0.5, 0.5),
            (15000.0, 0.79, 0.79),
            (20000.0, 0.853, 0.849),
            (30000.0, 0.90, 0.92),
            (50000.0, 1.0, 1.0),
        ] {
            let p = temp.position(k);
            assert!((p - model).abs() < 0.001, "{k} K at {p}, the model says {model}");
            assert!((p - lightroom).abs() <= 0.021, "{k} K at {p}, Lightroom at {lightroom}");
        }
        // every 50 K step survives value → position → value, and the track never runs backwards
        let mut last = -1.0;
        for k in (2000..=50000).step_by(50).map(f64::from) {
            let p = temp.position(k);
            assert!(p > last, "{k} K");
            last = p;
            let back = (temp.value_at(p) / temp.step).round() * temp.step;
            assert_eq!(back, k, "{k} K → {p} → {back}");
        }
        assert_eq!((temp.value_at(-1.0), temp.value_at(2.0), temp.value_at(f64::NAN)), (2000.0, 50000.0, 2000.0));
        assert_eq!((temp.position(1.0), temp.position(1e9), temp.position(f64::NAN)), (0.0, 1.0, temp.position(temp.default)));
        let exposure = find("light.exposure").unwrap();
        for (v, p) in [(-5.0, 0.0), (-2.5, 0.25), (0.0, 0.5), (1.0, 0.6), (5.0, 1.0)] {
            assert!((exposure.position(v) - p).abs() < 1e-12 && (exposure.value_at(p) - v).abs() < 1e-12, "{v}");
        }
        // the relative temperature of rendered files (its own spec, centred on 0) isn't a Kelvin scale
        let relative = ControlSpec { id: "wb.tempRel", min: -100.0, max: 100.0, default: 0.0, step: 1.0, ..*temp };
        assert_eq!((relative.position(-50.0), relative.value_at(0.75)), (0.25, 50.0));
        // a Kelvin range that doesn't contain the centre falls back to linear rather than dividing by zero
        let narrow = ControlSpec { min: 7000.0, max: 9000.0, ..*temp };
        assert_eq!((narrow.position(8000.0), narrow.value_at(0.5)), (0.5, 8000.0));
    }
}
