//! The RAW Develop settings: the typed session of the Camera Raw filter, without egui.

use serde::{Deserialize, Serialize};

/// One colour grading wheel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradeWheel {
    /// Degrees 0..360.
    pub hue: f32,
    /// 0..=100.
    pub sat: f32,
    /// -100..=100.
    pub lum: f32,
}

/// What the vignette protects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VignetteStyle {
    /// Highlight priority.
    #[default]
    HighlightPriority,
    /// Colour priority.
    ColorPriority,
    /// Paint overlay.
    PaintOverlay,
}

/// The settings the RAW Develop workspace edits and the develop reads: the field set and serde names of PhotoCraft's
/// `CameraRaw`, without its three develop inputs (`seed`, `pixel_scale`, `encoding_gamma`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct RawDevelopSettings {
    // Basic
    /// The temperature.
    pub temperature: f32,
    /// The tint.
    pub tint: f32,
    /// The exposure.
    pub exposure: f32,
    /// The contrast.
    pub contrast: f32,
    /// The highlights.
    pub highlights: f32,
    /// The shadows.
    pub shadows: f32,
    /// The whites.
    pub whites: f32,
    /// The blacks.
    pub blacks: f32,
    /// The texture.
    pub texture: f32,
    /// The clarity.
    pub clarity: f32,
    /// The dehaze.
    pub dehaze: f32,
    /// The vibrance.
    pub vibrance: f32,
    /// The saturation.
    pub saturation: f32,
    // Tone curve
    /// The curve highlights.
    pub curve_highlights: f32,
    /// The curve lights.
    pub curve_lights: f32,
    /// The curve darks.
    pub curve_darks: f32,
    /// The curve shadows.
    pub curve_shadows: f32,
    /// Region splits (percent).
    pub curve_splits: [f32; 3],
    /// Point curves as `[input, output]` in 0..=255 (empty = linear).
    pub point_curve: Vec<[f32; 2]>,
    /// The point curve red.
    pub point_curve_red: Vec<[f32; 2]>,
    /// The point curve green.
    pub point_curve_green: Vec<[f32; 2]>,
    /// The point curve blue.
    pub point_curve_blue: Vec<[f32; 2]>,
    // Color mixer: red, orange, yellow, green, aqua, blue, purple, magenta.
    /// The hsl hue.
    pub hsl_hue: [f32; 8],
    /// The hsl sat.
    pub hsl_sat: [f32; 8],
    /// The hsl lum.
    pub hsl_lum: [f32; 8],
    // Color grading
    /// The grade shadows.
    pub grade_shadows: GradeWheel,
    /// The grade midtones.
    pub grade_midtones: GradeWheel,
    /// The grade highlights.
    pub grade_highlights: GradeWheel,
    /// The grade global.
    pub grade_global: GradeWheel,
    /// The grade blending.
    pub grade_blending: f32,
    /// The grade balance.
    pub grade_balance: f32,
    // Detail
    /// The sharpen amount.
    pub sharpen_amount: f32,
    /// The sharpen radius.
    pub sharpen_radius: f32,
    /// The sharpen detail.
    pub sharpen_detail: f32,
    /// The sharpen masking.
    pub sharpen_masking: f32,
    /// The noise luminance.
    pub noise_luminance: f32,
    /// The noise luminance detail.
    pub noise_luminance_detail: f32,
    /// The noise color.
    pub noise_color: f32,
    /// The noise color detail.
    pub noise_color_detail: f32,
    // Effects
    /// The grain amount.
    pub grain_amount: f32,
    /// The grain size.
    pub grain_size: f32,
    /// The grain roughness.
    pub grain_roughness: f32,
    /// The vignette amount.
    pub vignette_amount: f32,
    /// The vignette midpoint.
    pub vignette_midpoint: f32,
    /// The vignette roundness.
    pub vignette_roundness: f32,
    /// The vignette feather.
    pub vignette_feather: f32,
    /// The vignette highlights.
    pub vignette_highlights: f32,
    /// The vignette style.
    pub vignette_style: VignetteStyle,
}

impl Default for RawDevelopSettings {
    fn default() -> Self {
        todo!("dl.K66")
    }
}

// P3(dialogs-w17): dl.K66, moved from algo camera_raw.rs:596-665 with `validate_curve` and `wb_neutral`
impl RawDevelopSettings {
    /// Refuses settings outside their ranges, naming the first bad field.
    pub fn validate(&self) -> Result<(), String> {
        todo!("dl.K66")
    }

    /// [`validate`](Self::validate) for the fields that differ from `stored`.
    pub fn validate_changes(&self, stored: &Self) -> Result<(), String> {
        let _ = stored;
        todo!("dl.K66")
    }

    /// Whether the settings leave the picture as it is.
    pub fn is_identity(&self) -> bool {
        todo!("dl.K66")
    }
}
