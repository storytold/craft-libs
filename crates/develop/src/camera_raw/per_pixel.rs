//! Develop's stages resolved for one image: every constant and table the per-pixel ones use, so
//! another implementation of them (the GPU preview, `photocraft_gpu::CameraRawDevelop`) runs the
//! same maths. [`develop`](super::develop) builds its own constants with the same functions.
//! The spatial stages come as their parameters: Dehaze, Clarity, Texture and Detail (see
//! [`spatial`](super::spatial)), Shadows and Highlights as a [`LocalTone`].

use super::local_tone::{LocalTone, WORK_SIDE};
use super::spatial::{Clarity, Dehaze, Detail, Texture};
use super::types::VignetteStyle as StoredStyle;
use super::{
    CameraRaw, DevelopInputs, Encoding, Wheel, calibration_neutral, curve_lut, grade_neutral, hsl_neutral, hsv_to_rgb, is_linear, luma,
    parametric_lut, white_balance_gains, whites_blacks,
};

pub use whites_blacks::Model as WhitesBlacksModel;

/// Entries of the tone curve tables (parametric and point curves).
pub const CURVE_LUT: usize = 1024;
/// Samples of a Whites or Blacks curve row: grey levels 0, 1/32, …, 1.
pub const WB_STEPS: usize = whites_blacks::STEPS;

/// White balance and exposure: channel gains in linear light, when the stage is on.
pub(crate) fn gains(p: &CameraRaw) -> Option<[f32; 3]> {
    if p.temperature == 0.0 && p.tint == 0.0 && p.exposure == 0.0 {
        return None;
    }
    let g = white_balance_gains(p.temperature, p.tint);
    let norm = luma(g);
    Some(g.map(|v| v / norm * 2f32.powf(p.exposure)))
}

/// Contrast's strength, when Contrast is set (with or without Texture and Clarity).
fn contrast(p: &CameraRaw) -> Option<f32> {
    (p.contrast != 0.0).then_some(p.contrast / 100.0 * 0.6)
}

/// Whites and Blacks' tables, for an implementation that measures the image's points itself.
pub fn whites_blacks_model() -> WhitesBlacksModel {
    whites_blacks::model()
}

/// Whites and Blacks as two curve rows for the image's amounts.
#[derive(Clone, Debug, PartialEq)]
pub struct WhitesBlacks {
    /// The Whites curve over grey levels 0, 1/32, …, 1, when Whites is set.
    pub whites: Option<[f32; WB_STEPS]>,
    /// The Blacks curve, applied after Whites.
    pub blacks: Option<[f32; WB_STEPS]>,
    /// The clipping directions (Whites up, Blacks down) curve each channel; the others scale the
    /// pixel by the change of its luminance.
    pub per_channel: bool,
}

/// Color Grading's wheels resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Grade {
    /// Colour offsets of the shadows, midtones, highlights and global wheels.
    pub tints: [[f32; 3]; 4],
    /// Luminance of the same wheels (slider units).
    pub lums: [f32; 4],
    pub blend: f32,
    pub balance: f32,
}

impl Grade {
    pub(crate) fn new(p: &CameraRaw) -> Self {
        let tint = |wh: &Wheel| -> [f32; 3] {
            let c = hsv_to_rgb(wh.hue, 1.0, 1.0);
            let l = luma(c);
            [c[0] - l, c[1] - l, c[2] - l].map(|v| v * wh.sat / 100.0 * 0.3)
        };
        Grade {
            tints: [tint(&p.grade_shadows), tint(&p.grade_midtones), tint(&p.grade_highlights), tint(&p.grade_global)],
            lums: [p.grade_shadows.lum, p.grade_midtones.lum, p.grade_highlights.lum, p.grade_global.lum],
            blend: 0.15 + p.grade_blending.clamp(0.0, 100.0) / 100.0 * 0.35,
            balance: p.grade_balance.clamp(-100.0, 100.0) / 100.0 * 0.25,
        }
    }
}

/// The vignette's `vignette_style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VignetteStyle {
    HighlightPriority,
    ColorPriority,
    PaintOverlay,
}

/// The post-crop vignette's shape on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Vignette {
    /// −1 … 1.
    pub amount: f32,
    /// 0 … 1.
    pub highlights: f32,
    pub style: VignetteStyle,
    /// Frame centre (pixels).
    pub centre: [f32; 2],
    /// Half axes of the shape (pixels).
    pub axes: [f32; 2],
    /// Exponent of the superellipse.
    pub power: f32,
    /// The mask's smoothstep edges in normalised radius.
    pub edges: [f32; 2],
}

impl Vignette {
    pub(crate) fn new(w: usize, h: usize, p: &CameraRaw) -> Self {
        let mid = p.vignette_midpoint.clamp(0.0, 100.0) / 100.0;
        let feather = p.vignette_feather.clamp(0.0, 100.0) / 100.0;
        let round = p.vignette_roundness.clamp(-100.0, 100.0) / 100.0;
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        // Roundness: 0 follows the frame's aspect, +1 a circle, −1 a rounded rectangle.
        let (ax, ay) = if round >= 0.0 {
            let r = cx.min(cy) + (cx.max(cy) - cx.min(cy)) * (1.0 - round);
            (if cx >= cy { r } else { cx }, if cy > cx { r } else { cy })
        } else {
            (cx, cy)
        };
        let inner = 1.0 - mid * 0.8;
        Vignette {
            amount: p.vignette_amount / 100.0,
            highlights: p.vignette_highlights.clamp(0.0, 100.0) / 100.0,
            style: match p.vignette_style {
                StoredStyle::PaintOverlay => VignetteStyle::PaintOverlay,
                StoredStyle::ColorPriority => VignetteStyle::ColorPriority,
                _ => VignetteStyle::HighlightPriority,
            },
            centre: [cx, cy],
            axes: [ax, ay],
            power: 2.0 + (-round).max(0.0) * 6.0,
            edges: [inner - feather * 0.5 * inner, inner + feather * 0.5 * (1.4 - inner) + 0.02],
        }
    }
}

/// Grain: value noise sampled in full-resolution coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Grain {
    /// Noise cell size (full-resolution pixels).
    pub cell: f32,
    /// Weight of the finer octave.
    pub roughness: f32,
    pub amount: f32,
    pub seed: u64,
    /// Full-resolution pixels per pixel (`1 / pixel_scale`).
    pub scale: f32,
}

impl Grain {
    pub(crate) fn new(p: &CameraRaw, inputs: &DevelopInputs) -> Self {
        Grain {
            cell: 1.0 + p.grain_size / 25.0,
            roughness: p.grain_roughness / 100.0,
            amount: p.grain_amount / 100.0 * 0.12,
            seed: u64::from(inputs.seed),
            scale: 1.0 / inputs.pixel_scale.clamp(0.01, 1.0),
        }
    }
}

/// Develop's per-pixel stages for one image and settings, in order. `None` stages are off.
#[derive(Clone, Debug, PartialEq)]
pub struct PerPixel {
    /// 1. White balance and exposure gains in linear light (sRGB-decoded values).
    pub gains: Option<[f32; 3]>,
    /// 2. Dehaze.
    pub dehaze: Option<Dehaze>,
    /// 3. Shadows and Highlights: a local Laplacian filter of the whole frame, then each pixel's
    ///    light scaled by the change of its log luminance.
    pub local_tone: Option<LocalTone>,
    /// 3. Clarity and Texture on the luminance clipped to 1, before Contrast.
    pub clarity: Option<Clarity>,
    pub texture: Option<Texture>,
    /// 3. Contrast's strength.
    pub contrast: Option<f32>,
    /// 3b. Whites and Blacks, in the document's encoding.
    pub whites_blacks: Option<WhitesBlacks>,
    /// 3b. Whites and Blacks to measure the image's points for (the sliders, −100 … 100), when a
    ///     spatial stage before them is set (any but Detail): the points then depend on its output, so whatever runs it
    ///     measures them and picks the curves from [`whites_blacks_model`]. `whites_blacks` is
    ///     then `None`.
    pub measure_whites_blacks: Option<[f32; 2]>,
    /// The document's transfer curve: 0 for sRGB's, else a power-law gamma.
    pub encoding: f32,
    /// 4. Vibrance and Saturation (slider / 100).
    pub presence: Option<[f32; 2]>,
    /// 5. The parametric tone curve, [`CURVE_LUT`] entries over 0 … 1.
    pub parametric: Option<Vec<f32>>,
    /// 5. Point curves: master, red, green, blue.
    pub point: [Option<Vec<f32>>; 4],
    /// 6. Color Mixer hue, saturation and luminance per band.
    pub hsl: Option<[[f32; 8]; 3]>,
    /// 7. Color Grading.
    pub grade: Option<Grade>,
    /// 8. Detail: noise reduction and sharpening.
    pub detail: Option<Detail>,
    /// 9. Vignette.
    pub vignette: Option<Vignette>,
    /// 9. Grain.
    pub grain: Option<Grain>,
    /// Clip RGB to 0 … 1 at the end (all but 32-bit documents).
    pub clamp: bool,
}

/// Whether developing needs a stage that reads neighbouring pixels: Dehaze, Shadows and
/// Highlights, Texture, Clarity, sharpening or noise reduction.
pub fn spatial(p: &CameraRaw) -> bool {
    spatial_tone(p) || p.sharpen_amount > 0.0 || p.noise_luminance > 0.0 || p.noise_color > 0.0
}

/// Whether a stage before Whites and Blacks reads neighbouring pixels: Dehaze, Shadows and
/// Highlights, Texture or Clarity (all of [`spatial`] but Detail). The image's points then
/// depend on its output.
pub fn spatial_tone(p: &CameraRaw) -> bool {
    p.dehaze != 0.0 || [p.highlights, p.shadows, p.texture, p.clarity].iter().any(|v| *v != 0.0)
}

/// The image's black and white points that Whites and Blacks adapt to, for `px` (`w × h`,
/// display-encoded) developed per pixel with `p`: what [`develop`](super::develop) measures
/// after white balance and Contrast. `None` when Whites and Blacks are 0 or `p` needs a
/// spatial stage before them ([`spatial_tone`]; Detail comes after).
pub fn white_black_points(px: &[[f32; 4]], w: usize, h: usize, p: &CameraRaw, inputs: &DevelopInputs) -> Option<(f32, f32)> {
    if (p.whites == 0.0 && p.blacks == 0.0) || spatial_tone(p) || px.len() != w.saturating_mul(h) {
        return None;
    }
    let enc = Encoding::of(inputs.encoding_gamma);
    let (g, k_con) = (gains(p), contrast(p));
    Some(super::luminance_points(px, w, h, enc, |q| {
        let q = g.map_or(q, |g| super::balance(q, g));
        k_con.map_or(q, |k| super::contrast(q, k))
    }))
}

/// Develop's stages for settings without Calibration, on a `w × h` image; `points` are the image's
/// [`white_black_points`]. `None` when `p` needs Calibration (these stages don't run it), Shadows
/// or Highlights on a frame wider than the filter's working copy (the
/// develop then filters a reduced copy) or only 2 pixels on its short side, or Whites or Blacks
/// without a spatial stage and without `points`. The points themselves leave Calibration out, so a
/// caller may cache them across Calibration changes.
pub fn per_pixel(p: &CameraRaw, inputs: &DevelopInputs, w: usize, h: usize, float: bool, points: Option<(f32, f32)>) -> Option<PerPixel> {
    if !calibration_neutral(p) {
        return None;
    }
    // The develop leaves frames under 2 pixels on a side alone.
    let local_tone = if w >= 2 && h >= 2 { LocalTone::new(p.shadows, p.highlights) } else { None };
    // At 2 pixels on the short side the GPU pass's pyramid has one level, so the CPU develops it.
    if local_tone.is_some() && (w.max(h) > WORK_SIDE || w.min(h) <= 2) {
        return None;
    }
    let (dehaze, clarity, texture) = (Dehaze::new(p, w, h), Clarity::new(p, w, h), Texture::new(p, inputs, w, h));
    let spatial = local_tone.is_some() || dehaze.is_some() || clarity.is_some() || texture.is_some();
    let identity = p.is_identity();
    let wb_on = p.whites != 0.0 || p.blacks != 0.0;
    let measure_whites_blacks = (wb_on && spatial).then(|| [p.whites.clamp(-100.0, 100.0), p.blacks.clamp(-100.0, 100.0)]);
    let whites_blacks = if wb_on && measure_whites_blacks.is_none() {
        let (bp, wp) = points?;
        let (xw, xb) = whites_blacks::amounts(p.whites.clamp(-100.0, 100.0), p.blacks.clamp(-100.0, 100.0), bp, wp);
        (xw != 0.0 || xb != 0.0).then(|| WhitesBlacks {
            whites: (xw != 0.0).then(|| whites_blacks::whites_row(xw)),
            blacks: (xb != 0.0).then(|| whites_blacks::blacks_row(xb)),
            per_channel: xw > 0.0 || xb < 0.0,
        })
    } else {
        None
    };
    let parametric_on = !(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0);
    let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, CURVE_LUT));
    Some(PerPixel {
        gains: gains(p),
        dehaze,
        local_tone,
        clarity,
        texture,
        contrast: contrast(p),
        whites_blacks,
        measure_whites_blacks,
        encoding: Encoding::of(inputs.encoding_gamma).0,
        presence: (p.vibrance != 0.0 || p.saturation != 0.0).then_some([p.vibrance / 100.0, p.saturation / 100.0]),
        parametric: parametric_on.then(|| parametric_lut(p, CURVE_LUT)),
        point: [pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue)],
        hsl: (!hsl_neutral(p)).then_some([p.hsl_hue, p.hsl_sat, p.hsl_lum]),
        grade: (!grade_neutral(p)).then(|| Grade::new(p)),
        detail: Detail::new(p, inputs),
        vignette: (p.vignette_amount != 0.0).then(|| Vignette::new(w, h, p)),
        grain: (p.grain_amount > 0.0).then(|| Grain::new(p, inputs)),
        clamp: !float && !identity,
    })
}
