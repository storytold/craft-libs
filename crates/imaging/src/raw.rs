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
    /// Colour priority.
    ColorPriority,
    /// Paint overlay.
    PaintOverlay,
    /// Highlight priority; a stored name outside the three reads as this one (serde's fallback is the last variant).
    #[default]
    #[serde(other)]
    HighlightPriority,
}

/// Most points a point curve accepts from commands and the control channel.
pub const MAX_CURVE_POINTS: usize = 16;

/// The demosaic algorithm a raw is opened with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Demosaic {
    /// Adaptive homogeneity-directed.
    Ahd,
    /// Modified high-contrast.
    Mhc,
    /// Bilinear.
    Bilinear,
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
    // Calibration
    /// Shadows › Tint: −100 (green) … 100 (magenta), strongest in the darkest tones.
    pub shadow_tint: f32,
    /// Red, Green and Blue Primary › Hue: −100 … 100, the primary turned towards its neighbours.
    pub calibration_hue: [f32; 3],
    /// Red, Green and Blue Primary › Saturation: −100 (grey) … 100 (purer).
    pub calibration_sat: [f32; 3],
}

impl Default for RawDevelopSettings {
    fn default() -> Self {
        RawDevelopSettings {
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            curve_highlights: 0.0,
            curve_lights: 0.0,
            curve_darks: 0.0,
            curve_shadows: 0.0,
            curve_splits: [25.0, 50.0, 75.0],
            point_curve: Vec::new(),
            point_curve_red: Vec::new(),
            point_curve_green: Vec::new(),
            point_curve_blue: Vec::new(),
            hsl_hue: [0.0; 8],
            hsl_sat: [0.0; 8],
            hsl_lum: [0.0; 8],
            grade_shadows: GradeWheel::default(),
            grade_midtones: GradeWheel::default(),
            grade_highlights: GradeWheel::default(),
            grade_global: GradeWheel::default(),
            grade_blending: 50.0,
            grade_balance: 0.0,
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            noise_luminance: 0.0,
            noise_luminance_detail: 50.0,
            noise_color: 0.0,
            noise_color_detail: 50.0,
            grain_amount: 0.0,
            grain_size: 25.0,
            grain_roughness: 50.0,
            vignette_amount: 0.0,
            vignette_midpoint: 50.0,
            vignette_roundness: 0.0,
            vignette_feather: 50.0,
            vignette_highlights: 0.0,
            vignette_style: VignetteStyle::HighlightPriority,
            shadow_tint: 0.0,
            calibration_hue: [0.0; 3],
            calibration_sat: [0.0; 3],
        }
    }
}

/// Checks a point curve from untrusted input: empty (linear) or 2..=16 finite points in 0..=255 with inputs
/// increasing by at least one level. Deserialization stays lenient: earlier editors saved curves this rejects, and the
/// develop sorts and de-duplicates them, so stored Smart Filters keep rendering.
pub fn validate_curve(points: &[[f32; 2]]) -> Result<(), String> {
    if points.len() == 1 {
        return Err("a nonempty curve needs at least two points".into());
    }
    if points.len() > MAX_CURVE_POINTS {
        return Err(format!("at most {MAX_CURVE_POINTS} curve points"));
    }
    if !points.iter().flatten().all(|v| v.is_finite() && (0.0..=255.0).contains(v)) {
        return Err("curve coordinates must be finite and in 0..=255".into());
    }
    if points.windows(2).any(|w| w[1][0] - w[0][0] < 1.0) {
        return Err("curve inputs must increase by at least one level".into());
    }
    Ok(())
}

impl GradeWheel {
    fn neutral(&self) -> bool {
        self.sat == 0.0 && self.lum == 0.0
    }
}

fn is_linear(curve: &[[f32; 2]]) -> bool {
    curve.len() < 2 || curve.iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
}

impl RawDevelopSettings {
    /// Refuses malformed point curves, naming the first bad one.
    pub fn validate(&self) -> Result<(), String> {
        for (name, curve) in [
            ("pointCurve", &self.point_curve),
            ("pointCurveRed", &self.point_curve_red),
            ("pointCurveGreen", &self.point_curve_green),
            ("pointCurveBlue", &self.point_curve_blue),
        ] {
            validate_curve(curve).map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(())
    }

    /// [`validate`](Self::validate) for an edit of `stored` settings: a curve left as stored is accepted, so editing
    /// another control of an older Smart Filter still works.
    pub fn validate_changes(&self, stored: &Self) -> Result<(), String> {
        for (name, curve, kept) in [
            ("pointCurve", &self.point_curve, &stored.point_curve),
            ("pointCurveRed", &self.point_curve_red, &stored.point_curve_red),
            ("pointCurveGreen", &self.point_curve_green, &stored.point_curve_green),
            ("pointCurveBlue", &self.point_curve_blue, &stored.point_curve_blue),
        ] {
            if curve != kept {
                validate_curve(curve).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        Ok(())
    }

    fn wb_neutral(&self) -> bool {
        self.temperature == 0.0 && self.tint == 0.0 && self.exposure == 0.0
    }

    fn tone_neutral(&self) -> bool {
        [self.contrast, self.highlights, self.shadows, self.texture, self.clarity, self.whites, self.blacks].iter().all(|v| *v == 0.0)
    }

    fn hsl_neutral(&self) -> bool {
        self.hsl_hue.iter().chain(&self.hsl_sat).chain(&self.hsl_lum).all(|v| *v == 0.0)
    }

    fn grade_neutral(&self) -> bool {
        self.grade_shadows.neutral() && self.grade_midtones.neutral() && self.grade_highlights.neutral() && self.grade_global.neutral()
    }

    fn calibration_neutral(&self) -> bool {
        self.shadow_tint == 0.0 && self.calibration_hue.iter().chain(&self.calibration_sat).all(|v| *v == 0.0)
    }

    fn curves_neutral(&self) -> bool {
        [self.curve_highlights, self.curve_lights, self.curve_darks, self.curve_shadows].iter().all(|v| *v == 0.0)
            && [&self.point_curve, &self.point_curve_red, &self.point_curve_green, &self.point_curve_blue].iter().all(|c| is_linear(c))
    }

    /// Whether every control is neutral, so the settings leave the picture as it is.
    pub fn is_identity(&self) -> bool {
        self.wb_neutral()
            && self.calibration_neutral()
            && self.tone_neutral()
            && self.dehaze == 0.0
            && self.vibrance == 0.0
            && self.saturation == 0.0
            && self.curves_neutral()
            && self.hsl_neutral()
            && self.grade_neutral()
            && self.sharpen_amount == 0.0
            && self.noise_luminance == 0.0
            && self.noise_color == 0.0
            && self.grain_amount == 0.0
            && self.vignette_amount == 0.0
    }
}
