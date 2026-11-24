//! Filter › Camera Raw Filter: a float develop pipeline over RGB pixels.
//!
//! Stages, in order (each skipped when its controls are neutral, so defaults are an identity):
//!
//! 1. **White balance and exposure** in linear light (the document's values decoded with the sRGB
//!    curve as a perceptual working encoding; encoding is undone exactly at the end).
//! 2. **Dehaze**: K. He, J. Sun, X. Tang, *Single Image Haze Removal Using Dark Channel Prior*,
//!    CVPR 2009 (dark channel, atmospheric light from its brightest 0.1 %, transmission refined
//!    with the guided filter); negative amounts add haze toward the atmospheric light.
//! 3. **Tone** on luminance with colour ratios kept: Highlights / Shadows as exposure changes
//!    weighted by an edge-preserving base layer (guided filter: K. He, J. Sun, X. Tang, *Guided
//!    Image Filtering*, ECCV 2010), so local contrast survives; Contrast is an S-curve about
//!    middle grey; Clarity (large radius, midtone weighted) and Texture (small radius) add
//!    band-pass local contrast. Whites / Blacks follow as Photoshop's Camera Raw applies them:
//!    image-adaptive global curves measured on Photoshop 25.4 (see `whites_blacks`).
//! 4. **Presence**: Vibrance (weighted toward less saturated colours) and Saturation.
//! 5. **Tone Curve**: parametric regions (Highlights, Lights, Darks, Shadows with movable splits)
//!    then point curves (master and per channel), monotone cubic interpolation (F. Fritsch,
//!    R. Carlson, *Monotone Piecewise Cubic Interpolation*, SIAM J. Numer. Anal. 1980).
//! 6. **Color Mixer (HSL)**: eight hue bands (red, orange, yellow, green, aqua, blue, purple,
//!    magenta) with hat-function weights on the hue circle.
//! 7. **Color Grading**: shadows / midtones / highlights / global wheels (hue, saturation,
//!    luminance) with Blending and Balance.
//! 8. **Detail**: sharpening on luminance (unsharp mask with Detail as halo control and Masking as
//!    an edge mask) and noise reduction (luminance by guided filter, colour by chroma blur).
//! 9. **Effects**: post-crop vignette (amount, midpoint, roundness, feather, highlights,
//!    style) and grain (amount, size, roughness; position-hashed, deterministic).

#![allow(clippy::needless_range_loop)] // index loops mirror the maths (pixels × channels)
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// The settings are the framework's: one model, with its validation and identity rules.
mod types;
mod util;

pub use types::{GradeWheel as Wheel, MAX_CURVE_POINTS, RawDevelopSettings as CameraRaw, VignetteStyle as StoredVignetteStyle, validate_curve};

// PLAN: `develop`, `per_pixel` and `white_black_points` now take `&DevelopInputs`; their callers outside this set's files need it:
// crates/gpu/tests/it/camera_raw.rs and examples/bench_camera_raw.rs (spatial(p) is a free function now), dialogs-w51's
// camera_raw_gpu.rs `CameraRawGpu::develop` (takes the inputs beside `p`), crates/io/src/camera_raw_xmp.rs (`vignette_style` is the
// framework's `VignetteStyle` enum, not a string). Owner: tools-w28 / dialogs-w51 / the io set.
/// What one develop reads besides the settings. PhotoCraft sets these per job (the proxy's scale, the document's
/// transfer curve), so they are not part of the settings the workspace edits and the commands store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DevelopInputs {
    /// Seeds the grain's noise.
    pub seed: u32,
    /// Pixel-size multiplier for previews on downsampled proxies (radii in px scale by it).
    pub pixel_scale: f32,
    /// The document's transfer curve: 0 for sRGB's, else the gamma of a power-law space (1.8 for ProPhoto). Whites and
    /// Blacks act on light.
    pub encoding_gamma: f32,
}

impl Default for DevelopInputs {
    fn default() -> Self {
        DevelopInputs { seed: 0, pixel_scale: 1.0, encoding_gamma: 0.0 }
    }
}

use util::{hash01, linear_to_srgb, par_rows, srgb_to_linear};

pub mod local_tone;
mod per_pixel;
pub mod spatial;
mod whites_blacks;

pub use local_tone::{LocalTone, ToneKind, TonePass};
pub use per_pixel::{
    CURVE_LUT, Grade, Grain, PerPixel, Vignette, VignetteStyle, WB_STEPS, WhitesBlacks, WhitesBlacksModel, per_pixel, spatial, spatial_tone,
    white_black_points, whites_blacks_model,
};
pub use spatial::{Clarity, Dehaze, Detail, Texture};

/// How the pixels encode light: the sRGB curve (gamma 0) or a pure power law (ProPhoto's 1.8,
/// Adobe RGB's 2.2). The tone controls work on light, so the document's own curve decodes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Encoding(f32);

impl Encoding {
    pub(crate) fn of(gamma: f32) -> Self {
        Encoding(if gamma.is_finite() && gamma > 0.0 { gamma.clamp(0.5, 4.0) } else { 0.0 })
    }
    /// A value in this encoding as linear light.
    pub(crate) fn decode(self, v: f32) -> f32 {
        if self.0 > 0.0 { v.max(0.0).powf(self.0) } else { srgb_to_linear(v) }
    }
    /// Linear light back into this encoding, the inverse of [`Encoding::decode`].
    pub(crate) fn encode(self, v: f32) -> f32 {
        if self.0 > 0.0 { v.max(0.0).powf(1.0 / self.0) } else { linear_to_srgb(v) }
    }
}

fn luma_linear(c: [f32; 3]) -> f32 {
    luma(c)
}

/// Hue band centres of the Color Mixer (degrees).
pub const HSL_BANDS: [f32; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0];

/// How far a primary turns at Hue ±100 (degrees), as the Color Mixer's hue sliders do.
const CALIBRATION_TURN: f32 = 30.0;

/// How much of each primary (red, green, blue) a hue is made of: triangular weights around
/// 0°, 120° and 240° that sum to one, so a colour between two primaries follows both.
pub fn calibration_weights(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0);
    let mut w = [0.0f32; 3];
    for (i, centre) in [0.0f32, 120.0, 240.0].iter().enumerate() {
        let d = (h - centre).abs().min(360.0 - (h - centre).abs());
        w[i] = (1.0 - d / 120.0).max(0.0);
    }
    let sum: f32 = w.iter().sum();
    if sum > 1e-6 { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0] }
}

/// The Calibration panel's primaries on one linear-light pixel: its hue turns by the weighted
/// Hue of the primaries it is made of (±100 is ±30°, positive towards the next primary) and its
/// saturation scales by their weighted Saturation (−100 greys it); the luminance is kept, so
/// neutrals are untouched.
fn calibrate(c: [f32; 3], hue: [f32; 3], sat: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = rgb_to_hsv(c.map(|x| x.max(0.0)));
    if s <= 1e-5 || v <= 1e-6 {
        return c;
    }
    let w = calibration_weights(h);
    let (mut dh, mut ds) = (0.0f32, 0.0f32);
    for i in 0..3 {
        dh += w[i] * hue[i].clamp(-100.0, 100.0);
        ds += w[i] * sat[i].clamp(-100.0, 100.0);
    }
    let nh = h + dh / 100.0 * CALIBRATION_TURN;
    let ns = (s * (1.0 + ds / 100.0)).clamp(0.0, 1.0);
    let out = hsv_to_rgb(nh, ns, v);
    let (l0, l1) = (luma(c), luma(out));
    if l1 > 1e-6 { out.map(|x| x * l0 / l1) } else { out }
}

/// Shadows › Tint on one linear-light pixel: `tint` −1 (green) … 1 (magenta), weighted by how
/// dark the pixel is; the luminance is kept.
fn shadow_tint(c: [f32; 3], tint: f32) -> [f32; 3] {
    let l = luma(c).max(0.0);
    // Linear 0.18 is middle grey: full strength below it, fading out towards white.
    let weight = (1.0 - (l / 0.18).sqrt().min(1.0)).powi(2) * 0.5 + (1.0 - l.min(1.0)).powi(4) * 0.5;
    let k = tint * 0.12 * weight * (l + 0.02);
    let o = [c[0] + k, c[1] - k, c[2] + k].map(|v| v.max(0.0));
    let l2 = luma(o);
    if l2 > 1e-6 { o.map(|v| v * l / l2) } else { o }
}

/// Camera Raw's Auto for the Light panel: Exposure, Contrast, Highlights, Shadows, Whites and
/// Blacks from the image's luminance histogram (`px` in the document's encoding). A well
/// exposed image gets near-zero settings; a dark one more exposure and lifted shadows; one
/// with a crushed or blown end gets Blacks or Whites back from the edge. The other panels are
/// left alone, as Camera Raw's Auto leaves them.
pub fn auto_tone(px: &[[f32; 4]], p: &mut CameraRaw) {
    if px.is_empty() {
        return;
    }
    let mut lum: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]]).clamp(0.0, 1.0)).collect();
    lum.sort_by(|a, b| a.total_cmp(b));
    let at = |f: f32| lum[((lum.len() - 1) as f32 * f).round() as usize];
    let (black, shadow, median, light, white) = (at(0.001), at(0.1), at(0.5), at(0.9), at(0.999));
    // Exposure: the median towards middle grey (sRGB 0.46), damped so flat scenes aren't
    // forced to it.
    let median_lin = srgb_to_linear(median).max(1e-4);
    p.exposure = (0.6 * (0.18f32 / median_lin).log2()).clamp(-2.0, 2.0);
    // Blacks and Whites bring the ends to the edges without clipping more than Camera Raw does.
    p.blacks = ((0.04 - black) * 400.0).clamp(-40.0, 40.0);
    p.whites = ((0.96 - white) * 300.0).clamp(-40.0, 40.0);
    // Shadows up when the darker tenth is crushed, highlights down when the brighter tenth is
    // near white.
    p.shadows = ((0.25 - shadow) * 200.0).clamp(0.0, 50.0);
    p.highlights = ((light - 0.85) * -300.0).clamp(-50.0, 0.0);
    // A little contrast for a flat histogram.
    let spread = (light - shadow).max(0.0);
    p.contrast = ((0.6 - spread) * 60.0).clamp(0.0, 25.0);
    // Shown as Camera Raw shows them: Exposure to a hundredth of a stop, the others whole.
    p.exposure = (p.exposure * 100.0).round() / 100.0;
    for v in [&mut p.blacks, &mut p.whites, &mut p.shadows, &mut p.highlights, &mut p.contrast] {
        *v = v.round();
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Box mean of a single-channel buffer (radius `r`, edge clamped): rows in parallel, then
/// columns in parallel bands with running column sums (cache-friendly, no transposes).
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return v.to_vec();
    }
    let n = (2 * r + 1) as f64;
    let mut tmp = vec![0.0f32; w * h];
    par_rows(&mut tmp, w, 1, |y, row| {
        let src = &v[y * w..(y + 1) * w];
        let at = |k: i64| src[k.clamp(0, w as i64 - 1) as usize] as f64;
        let mut acc: f64 = (-(r as i64)..=r as i64).map(at).sum();
        for (x, o) in row.iter_mut().enumerate() {
            *o = (acc / n) as f32;
            acc += at(x as i64 + r as i64 + 1) - at(x as i64 - r as i64);
        }
    });
    const BAND: usize = 256;
    let bands = util::par_map(w.div_ceil(BAND), |b| {
        let (x0, x1) = (b * BAND, ((b + 1) * BAND).min(w));
        let bw = x1 - x0;
        let row = |y: i64| &tmp[y.clamp(0, h as i64 - 1) as usize * w + x0..y.clamp(0, h as i64 - 1) as usize * w + x1];
        let mut acc = vec![0.0f64; bw];
        for y in -(r as i64)..=r as i64 {
            for (a, v) in acc.iter_mut().zip(row(y)) {
                *a += *v as f64;
            }
        }
        let mut out = vec![0.0f32; bw * h];
        for y in 0..h {
            for (o, a) in out[y * bw..(y + 1) * bw].iter_mut().zip(&acc) {
                *o = (*a / n) as f32;
            }
            let (add, sub) = (row(y as i64 + r as i64 + 1), row(y as i64 - r as i64));
            for ((a, p), m) in acc.iter_mut().zip(add).zip(sub) {
                *a += (*p - *m) as f64;
            }
        }
        out
    });
    let mut out = vec![0.0f32; w * h];
    for (b, band) in bands.iter().enumerate() {
        let (x0, x1) = (b * BAND, ((b + 1) * BAND).min(w));
        let bw = x1 - x0;
        for y in 0..h {
            out[y * w + x0..y * w + x1].copy_from_slice(&band[y * bw..(y + 1) * bw]);
        }
    }
    out
}

/// Approximate Gaussian (three box passes).
#[cfg(test)]
fn gauss(v: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    box_means(v, w, h, &spatial::gauss_radii(sigma))
}

/// Box means of radii `radii` in turn.
fn box_means(v: &[f32], w: usize, h: usize, radii: &[usize]) -> Vec<f32> {
    let mut out = v.to_vec();
    for &r in radii {
        out = box_mean(&out, w, h, r);
    }
    out
}

/// Fast guided filter (K. He, J. Sun, *Fast Guided Filter*, arXiv 2015): the linear
/// coefficients are computed on a `s×` subsampled copy and upsampled bilinearly, which is
/// visually equivalent for the large radii used by tone controls at a fraction of the cost.
fn guided_fast(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let (s, rs) = spatial::fast_guided_steps(r);
    if s == 1 {
        return guided(i, p, w, h, r, eps);
    }
    let (sw, sh) = (w.div_ceil(s), h.div_ceil(s));
    let down = |v: &[f32]| -> Vec<f32> {
        let mut o = vec![0.0f32; sw * sh];
        let mut c = vec![0.0f32; sw * sh];
        for y in 0..h {
            for x in 0..w {
                let k = (y / s) * sw + x / s;
                o[k] += v[y * w + x];
                c[k] += 1.0;
            }
        }
        o.iter().zip(&c).map(|(a, b)| a / b.max(1.0)).collect()
    };
    let (si, sp) = (down(i), down(p));
    let mi = box_mean(&si, sw, sh, rs);
    let mp = box_mean(&sp, sw, sh, rs);
    let ip: Vec<f32> = si.iter().zip(&sp).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = si.iter().map(|a| a * a).collect();
    let mip = box_mean(&ip, sw, sh, rs);
    let mii = box_mean(&ii, sw, sh, rs);
    let a: Vec<f32> = (0..sw * sh).map(|k| (mip[k] - mi[k] * mp[k]) / (mii[k] - mi[k] * mi[k] + eps)).collect();
    let b: Vec<f32> = (0..sw * sh).map(|k| mp[k] - a[k] * mi[k]).collect();
    let (ma, mb) = (box_mean(&a, sw, sh, rs), box_mean(&b, sw, sh, rs));
    let mut out = vec![0.0f32; w * h];
    let sf = s as f32;
    par_rows(&mut out, w, 1, |y, row| {
        let fy = (y as f32 + 0.5) / sf - 0.5;
        for (x, o) in row.iter_mut().enumerate() {
            let fx = (x as f32 + 0.5) / sf - 0.5;
            let av = util::bilinear(&ma, sw, sh, 1, 0, fx, fy);
            let bv = util::bilinear(&mb, sw, sh, 1, 0, fx, fy);
            *o = av * i[y * w + x] + bv;
        }
    });
    out
}

/// Guided filter of `p` guided by `i` (He et al. 2010).
fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mi = box_mean(i, w, h, r);
    let mp = box_mean(p, w, h, r);
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let ii: Vec<f32> = i.iter().map(|a| a * a).collect();
    let mip = box_mean(&ip, w, h, r);
    let mii = box_mean(&ii, w, h, r);
    let a: Vec<f32> = (0..w * h).map(|k| (mip[k] - mi[k] * mp[k]) / (mii[k] - mi[k] * mi[k] + eps)).collect();
    let b: Vec<f32> = (0..w * h).map(|k| mp[k] - a[k] * mi[k]).collect();
    let ma = box_mean(&a, w, h, r);
    let mb = box_mean(&b, w, h, r);
    (0..w * h).map(|k| ma[k] * i[k] + mb[k]).collect()
}

/// Monotone cubic (Fritsch–Carlson) through `pts` (x ascending, 0..=1), evaluated into a LUT via
/// [`crate::color::spline::MonotoneCurve`].
pub fn curve_lut(pts: &[[f32; 2]], n: usize) -> Vec<f32> {
    // Keep the legacy f32 knot selection; normalizing in f64 can drop a saved knot.
    let mut p: Vec<[f32; 2]> = pts.iter().map(|q| [q[0] / 255.0, q[1] / 255.0]).collect();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
    if p.len() < 2 || n < 2 {
        return (0..n).map(|k| if n <= 1 { 0.0 } else { k as f32 / (n - 1) as f32 }).collect();
    }
    let p: Vec<_> = p.into_iter().map(|[x, y]| (f64::from(x), f64::from(y))).collect();
    crate::color::spline::MonotoneCurve::new(&p).to_lut(n).v.into_iter().map(|v| v.clamp(0.0, 1.0)).collect()
}

/// Migrate a PhotoCraft Camera Raw / `filter.cameraRaw` settings object to shared
/// [`craft_develop_model::DevelopSettings`], stripping PhotoCraft-only transport keys
/// (`layer`, `encodingGamma`, `__cameraRawPsd`) before delegating to
/// [`craft_develop_model::migration::migrate_camera_raw`].
pub fn migrate_to_develop_settings(
    base: &craft_develop_model::DevelopSettings,
    fields: &serde_json::Value,
) -> Result<craft_develop_model::migration::Migration, craft_develop_model::migration::MigrationError> {
    let Some(obj) = fields.as_object() else {
        return craft_develop_model::migration::migrate_camera_raw(base, fields);
    };
    let cleaned: serde_json::Map<String, serde_json::Value> = obj
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "layer" | "encodingGamma" | "__cameraRawPsd"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    craft_develop_model::migration::migrate_camera_raw(base, &serde_json::Value::Object(cleaned))
}

fn lut_eval(lut: &[f32], v: f32) -> f32 {
    if !(0.0..=1.0).contains(&v) {
        // Extend linearly outside (float documents).
        return if v < 0.0 { v + lut[0] } else { lut[lut.len() - 1] + (v - 1.0) };
    }
    let x = v * (lut.len() - 1) as f32;
    let i = (x as usize).min(lut.len() - 2);
    let t = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * t
}

/// Parametric tone curve as a LUT (regions split at `splits`, each slider ±100 moves its region
/// by up to a quarter of the range, smoothly).
fn parametric_lut(p: &CameraRaw, n: usize) -> Vec<f32> {
    let s = p.curve_splits.map(|v| v.clamp(5.0, 95.0) / 100.0);
    let centres = [s[0] / 2.0, (s[0] + s[1]) / 2.0, (s[1] + s[2]) / 2.0, (s[2] + 1.0) / 2.0];
    let amounts = [p.curve_shadows, p.curve_darks, p.curve_lights, p.curve_highlights];
    let widths = [s[0].max(0.1), (s[1] - s[0]).max(0.1), (s[2] - s[1]).max(0.1), (1.0 - s[2]).max(0.1)];
    let mut lut: Vec<f32> = (0..n)
        .map(|i| {
            let x = i as f32 / (n - 1) as f32;
            let mut y = x;
            for k in 0..4 {
                let d = (x - centres[k]) / (widths[k] * 1.2);
                let bump = (-d * d * 2.0).exp() * x * (1.0 - x) * 4.0;
                y += amounts[k] / 100.0 * 0.25 * bump;
            }
            y.clamp(0.0, 1.0)
        })
        .collect();
    // Keep it monotone.
    for i in 1..n {
        lut[i] = lut[i].max(lut[i - 1]);
    }
    lut
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 1e-9 {
        0.0
    } else if mx == c[0] {
        60.0 * (((c[1] - c[2]) / d).rem_euclid(6.0))
    } else if mx == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, if mx > 1e-9 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// Weights of the eight HSL bands at hue `h` (hat functions between neighbouring centres).
pub fn band_weights(h: f32) -> [f32; 8] {
    let mut w = [0.0f32; 8];
    for k in 0..8 {
        let (c, prev, next) = (HSL_BANDS[k], HSL_BANDS[(k + 7) % 8], HSL_BANDS[(k + 1) % 8]);
        let diff = (h - c + 540.0).rem_euclid(360.0) - 180.0;
        let span = if diff >= 0.0 { (next - c).rem_euclid(360.0) } else { (c - prev).rem_euclid(360.0) };
        w[k] = (1.0 - diff.abs() / span.max(1e-3)).max(0.0);
    }
    w
}

#[cfg(test)]
#[test]
fn band_weights_wrap_equivalent_hues_in_both_directions() {
    let weights = band_weights(320.0);
    assert_eq!(band_weights(-400.0), weights);
    assert_eq!(band_weights(680.0), weights);
}

/// Contrast, Highlights, Shadows, Texture and Clarity (the luminance stage) at rest.
fn local_tone_neutral(p: &CameraRaw) -> bool {
    [p.contrast, p.highlights, p.shadows, p.texture, p.clarity].iter().all(|v| *v == 0.0)
}

/// The Color Mixer at rest.
pub(crate) fn hsl_neutral(p: &CameraRaw) -> bool {
    p.hsl_hue.iter().chain(&p.hsl_sat).chain(&p.hsl_lum).all(|v| *v == 0.0)
}

/// Color Grading at rest.
pub(crate) fn grade_neutral(p: &CameraRaw) -> bool {
    [&p.grade_shadows, &p.grade_midtones, &p.grade_highlights, &p.grade_global].iter().all(|w| w.sat == 0.0 && w.lum == 0.0)
}

/// The Calibration panel at rest.
pub(crate) fn calibration_neutral(p: &CameraRaw) -> bool {
    p.shadow_tint == 0.0 && p.calibration_hue.iter().chain(&p.calibration_sat).all(|v| *v == 0.0)
}

/// The Tone Curve panel at rest.
fn curves_neutral(p: &CameraRaw) -> bool {
    [p.curve_highlights, p.curve_lights, p.curve_darks, p.curve_shadows].iter().all(|v| *v == 0.0)
        && [&p.point_curve, &p.point_curve_red, &p.point_curve_green, &p.point_curve_blue].iter().all(|c| is_linear(c))
}

fn is_linear(c: &[[f32; 2]]) -> bool {
    c.len() < 2 || c.iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
}

/// Camera Raw's white-balance channel gains for Temperature / Tint (−100…100), relative to the
/// as-shot balance: warmer raises red and lowers blue (±35 % at the ends); a negative tint
/// raises green, a positive one lowers it toward magenta (±25 %). An observed approximation of
/// Adobe Camera Raw's sliders, not a colour-temperature model; shared by the filter (in linear
/// RGB) and the raw open (on the camera's white-balance multipliers) so both agree.
pub fn white_balance_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let t = temperature / 100.0;
    let tn = tint / 100.0;
    [1.0 + 0.35 * t, 1.0 - 0.25 * tn, 1.0 - 0.35 * t]
}

/// Runs the Camera Raw pipeline on straight RGBA pixels (`w × h`, display-encoded RGB).
/// `float` keeps values above 1 (32-bit documents). `inputs` are the job's scale, grain seed and encoding.
pub fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, inputs: &DevelopInputs, float: bool) {
    if px.is_empty() || p.is_identity() {
        return;
    }
    // 1. White balance + exposure in linear light.
    if let Some(g) = per_pixel::gains(p) {
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                *q = balance(*q, g);
            }
        });
    }
    // 1b. Calibration: the primaries turned and purer or greyer, and the shadows tinted, in
    // linear light (the camera's primaries are what Camera Raw's panel calibrates).
    if !calibration_neutral(p) {
        let primaries = p.calibration_hue.iter().chain(&p.calibration_sat).any(|v| *v != 0.0);
        let tint = p.shadow_tint.clamp(-100.0, 100.0) / 100.0;
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                let c = [srgb_to_linear(q[0]), srgb_to_linear(q[1]), srgb_to_linear(q[2])];
                let mut o = if primaries { calibrate(c, p.calibration_hue, p.calibration_sat) } else { c };
                if tint != 0.0 {
                    o = shadow_tint(o, tint);
                }
                for k in 0..3 {
                    q[k] = linear_to_srgb(o[k].max(0.0));
                }
            }
        });
    }
    // 2. Dehaze.
    if let Some(d) = Dehaze::new(p, w, h) {
        dehaze(px, w, h, &d);
    }
    // 3. Tone. Highlights and Shadows: a local Laplacian filter (see `local_tone`).
    if p.highlights != 0.0 || p.shadows != 0.0 {
        local_tone::apply(px, w, h, p.shadows, p.highlights, Encoding::of(inputs.encoding_gamma));
    }
    if !local_tone_neutral(p) && [p.contrast, p.texture, p.clarity].iter().any(|v| *v != 0.0) {
        let l: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]]).max(0.0)).collect();
        let lc: Vec<f32> = l.iter().map(|v| v.min(1.0)).collect();
        let clarity = Clarity::new(p, w, h);
        let texture = Texture::new(p, inputs, w, h);
        let clar = clarity.as_ref().map(|c| guided_fast(&lc, &lc, w, h, c.radius, spatial::CLARITY_EPS));
        let tex = texture.as_ref().map(|t| box_means(&lc, w, h, &t.radii));
        let k_con = p.contrast / 100.0 * 0.6;
        par_rows(px, w, 1, |y, row| {
            for (x, q) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let l0 = l[i];
                let mut lv = lc[i];
                if let (Some(c), Some(k)) = (&clar, &clarity) {
                    let mw = 1.0 - (2.0 * lv.clamp(0.0, 1.0) - 1.0).powi(2);
                    lv += k.amount * (lc[i] - c[i]) * mw;
                }
                if let (Some(t), Some(k)) = (&tex, &texture) {
                    lv += k.amount * (lc[i] - t[i]).clamp(-spatial::TEXTURE_LIMIT, spatial::TEXTURE_LIMIT);
                }
                *q = tone(*q, l0, lc[i], lv, k_con);
            }
        });
    }
    // 3b. Whites and Blacks: global curves adapted to the image's black and white points.
    if p.whites != 0.0 || p.blacks != 0.0 {
        let enc = Encoding::of(inputs.encoding_gamma);
        let (bp, wp) = luminance_points(px, w, h, enc, |q| q);
        let (xw, xb) = whites_blacks::amounts(p.whites.clamp(-100.0, 100.0), p.blacks.clamp(-100.0, 100.0), bp, wp);
        whites_blacks::apply(px, w, xw, xb, enc);
    }
    // 4–7: per-pixel colour stages.
    let need_color = p.vibrance != 0.0 || p.saturation != 0.0 || !curves_neutral(p) || !hsl_neutral(p) || !grade_neutral(p);
    if need_color {
        let par = (!(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0))
            .then(|| parametric_lut(p, 1024));
        let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, 1024));
        let (master, cr, cg, cb) = (pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue));
        let hsl = !hsl_neutral(p);
        let grade = !grade_neutral(p);
        let Grade { tints, lums, blend, balance: bal } = Grade::new(p);
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                let mut c = [q[0], q[1], q[2]];
                if p.vibrance != 0.0 || p.saturation != 0.0 {
                    let l = luma(c);
                    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                    let sat = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
                    // Vibrance protects skin-ish hues (oranges) a little, like ACR.
                    let hue = rgb_to_hsv(c)[0];
                    let skin = (1.0 - ((hue - 25.0).abs() / 25.0)).clamp(0.0, 1.0) * 0.5;
                    let k = 1.0 + p.saturation / 100.0 + p.vibrance / 100.0 * (1.0 - sat) * (1.0 - skin);
                    c = c.map(|v| l + (v - l) * k.max(0.0));
                }
                if let Some(lut) = &par {
                    c = c.map(|v| lut_eval(lut, v));
                }
                if let Some(lut) = &master {
                    c = c.map(|v| lut_eval(lut, v));
                }
                for (ch, lut) in [&cr, &cg, &cb].iter().enumerate() {
                    if let Some(lut) = lut {
                        c[ch] = lut_eval(lut, c[ch]);
                    }
                }
                if hsl {
                    let [hh, s, v] = rgb_to_hsv(c.map(|x| x.max(0.0)));
                    if s > 1e-4 {
                        let wts = band_weights(hh);
                        let (mut dh, mut ds, mut dl) = (0.0f32, 0.0f32, 0.0f32);
                        for k in 0..8 {
                            dh += wts[k] * p.hsl_hue[k];
                            ds += wts[k] * p.hsl_sat[k];
                            dl += wts[k] * p.hsl_lum[k];
                        }
                        let nh = hh + dh / 100.0 * 30.0;
                        let ns = (s * (1.0 + ds / 100.0)).clamp(0.0, 1.0);
                        let mut out = hsv_to_rgb(nh, ns, v);
                        // Keep the luminance, then apply the luminance shift weighted by saturation.
                        let (l0, l1) = (luma(c), luma(out));
                        if l1 > 1e-6 {
                            out = out.map(|x| x * l0 / l1);
                        }
                        let k = 2f32.powf(dl / 100.0 * 1.0 * s);
                        c = out.map(|x| x * k);
                    }
                }
                if grade {
                    let l = luma(c).clamp(0.0, 1.0);
                    let sh = 1.0 - smoothstep(0.0, 0.5 + bal + blend, l);
                    let hi = smoothstep(0.5 + bal - blend, 1.0, l);
                    let mid = (1.0 - sh - hi).max(0.0);
                    let masks = [sh, mid, hi, 1.0];
                    for k in 0..4 {
                        for ch in 0..3 {
                            c[ch] += masks[k] * tints[k][ch];
                        }
                        if lums[k] != 0.0 {
                            let f = 2f32.powf(masks[k] * lums[k] / 100.0 * 0.6);
                            c = c.map(|x| x * f);
                        }
                    }
                }
                q[0] = c[0];
                q[1] = c[1];
                q[2] = c[2];
            }
        });
    }
    // 8. Detail.
    if let Some(d) = Detail::new(p, inputs) {
        detail(px, w, h, &d);
    }
    // 9. Effects.
    if p.vignette_amount != 0.0 {
        vignette(px, w, h, p);
    }
    if p.grain_amount > 0.0 {
        let Grain { cell, roughness: rough, amount: amt, seed, scale: pscale } = Grain::new(p, inputs);
        par_rows(px, w, 1, |y, row| {
            for (x, q) in row.iter_mut().enumerate() {
                // Sample in full-resolution coordinates so previews match the result.
                let (fx, fy) = (x as f32 * pscale / cell, y as f32 * pscale / cell);
                let n = value_noise(fx, fy, seed) * (1.0 - rough) + (value_noise(fx * 2.3, fy * 2.3, seed + 7) - 0.0) * rough;
                let l = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
                let g = (n - 0.5) * 2.0 * amt * (0.4 + 2.4 * l * (1.0 - l));
                for c in 0..3 {
                    q[c] += g;
                }
            }
        });
    }
    if !float {
        for q in px.iter_mut() {
            for c in 0..3 {
                q[c] = q[c].clamp(0.0, 1.0);
            }
        }
    }
}

/// The end of stage 3 on one pixel: Contrast (strength `k_con`) on the luminance `lv` that
/// Clarity and Texture left, then the pixel rescaled to it. `l0` is its luminance, `lc` that
/// clipped to 1.
fn tone(mut q: [f32; 4], l0: f32, lc: f32, mut lv: f32, k_con: f32) -> [f32; 4] {
    if k_con != 0.0 {
        let tau = std::f32::consts::TAU;
        let xx = lv.clamp(0.0, 1.0);
        lv += -k_con * (tau * xx).sin() / tau;
    }
    let lv = lv.max(0.0) + (l0 - lc);
    let ratio = if l0 > 1e-5 { lv / l0 } else { 1.0 };
    for c in 0..3 {
        q[c] = if l0 > 1e-5 { q[c] * ratio } else { q[c] + (lv - l0) };
    }
    q
}

/// Stage 3 on one pixel when Contrast is its only control.
fn contrast(q: [f32; 4], k_con: f32) -> [f32; 4] {
    let l0 = luma([q[0], q[1], q[2]]).max(0.0);
    let lc = l0.min(1.0);
    tone(q, l0, lc, lc, k_con)
}

/// Stage 1 on one pixel: white balance and exposure `gains` in linear light.
fn balance(mut q: [f32; 4], g: [f32; 3]) -> [f32; 4] {
    for c in 0..3 {
        q[c] = linear_to_srgb((srgb_to_linear(q[c]) * g[c]).max(0.0));
    }
    q
}

/// The index of the `q` quantile (0 … 1) among `len` sorted values, as every percentile in the
/// develop picks it.
pub fn quantile_rank(len: usize, q: f32) -> usize {
    let last = len.saturating_sub(1);
    ((last as f32 * q).round() as usize).min(last)
}

#[cfg(test)]
#[test]
fn quantile_rank_keeps_the_upper_endpoint_inside_the_sample() {
    // This last index rounds up when cast to f32.
    let len = 16_777_220;
    assert_eq!(quantile_rank(len, 1.0), len - 1);
    assert_eq!(quantile_rank(4, 0.5), 2);
    assert_eq!(quantile_rank(0, 1.0), 0);
}

/// The image's black and white points as sRGB-encoded grey levels: the 3rd percentile of the
/// luminance and the 99.5th of the brightest channel, on a copy at most 256 pixels across (so a
/// preview proxy and the full image agree). `map` is applied to each pixel first.
fn luminance_points(px: &[[f32; 4]], w: usize, h: usize, enc: Encoding, map: impl Fn([f32; 4]) -> [f32; 4] + Sync) -> (f32, f32) {
    let f = w.max(h).div_ceil(whites_blacks::POINTS_SIDE).max(1);
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    // Each band of `f` rows sums into its own row of cells, in the same order as one pass would.
    let bands = util::par_map(sh, |sy| {
        let mut sum = vec![[0.0f64; 4]; sw];
        for y in sy * f..((sy + 1) * f).min(h) {
            for x in 0..w {
                let q = map(px[y * w + x]);
                let s = &mut sum[x / f];
                for c in 0..3 {
                    s[c] += f64::from(enc.decode(q[c]).clamp(0.0, 1.0));
                }
                s[3] += 1.0;
            }
        }
        sum
    });
    let cells: Vec<[f32; 3]> = bands.iter().flatten().filter(|s| s[3] > 0.0).map(|s| [0, 1, 2].map(|c| (s[c] / s[3]) as f32)).collect();
    if cells.is_empty() {
        return (0.0, 1.0);
    }
    let pct = |mut v: Vec<f32>, q: f32| {
        v.sort_by(f32::total_cmp);
        v[quantile_rank(v.len(), q)]
    };
    let bp = pct(cells.iter().map(|c| luma(*c)).collect(), whites_blacks::POINT_QUANTILES[0]);
    let wp = pct(cells.iter().map(|c| c[0].max(c[1]).max(c[2])).collect(), whites_blacks::POINT_QUANTILES[1]);
    (linear_to_srgb(bp), linear_to_srgb(wp))
}

fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |dx: i64, dy: i64| hash01(xi as i64 + dx, yi as i64 + dy, seed);
    let top = h(0, 0) + (h(1, 0) - h(0, 0)) * sx;
    let bot = h(0, 1) + (h(1, 1) - h(0, 1)) * sx;
    top + (bot - top) * sy
}

/// Dehaze's copy of `px` (`w × h`): each pixel mapped by `map`, clipped to 0 … 1 and averaged over
/// `d.factor × d.factor` blocks, each block summed in raster order.
fn haze_cells(px: &[[f32; 4]], w: usize, h: usize, d: &Dehaze, map: impl Fn([f32; 4]) -> [f32; 4] + Sync) -> Vec<[f32; 3]> {
    let (k, [sw, sh]) = (d.factor, d.size);
    let bands = util::par_map(sh, |sy| {
        let mut small = vec![[0.0f32; 3]; sw];
        let mut cnt = vec![0f32; sw];
        for y in sy * k..((sy + 1) * k).min(h) {
            for x in 0..w {
                let q = map(px[y * w + x]);
                let i = x / k;
                for c in 0..3 {
                    small[i][c] += q[c].clamp(0.0, 1.0);
                }
                cnt[i] += 1.0;
            }
        }
        small.iter().zip(&cnt).map(|(s, c)| s.map(|v| v / c.max(1.0))).collect::<Vec<_>>()
    });
    bands.into_iter().flatten().collect()
}

fn dehaze(px: &mut [[f32; 4]], w: usize, h: usize, d: &Dehaze) {
    // Transmission on a ≤ DEHAZE_SIDE px proxy, upsampled.
    let (k, [sw, sh], r, amount) = (d.factor, d.size, d.radius, d.amount);
    let small = haze_cells(px, w, h, d, |q| q);
    // The minimum (or, with `f32::max`, maximum) over each (2r + 1)² window.
    let rank_filter = |v: &[f32], start: f32, pick: fn(f32, f32) -> f32| -> Vec<f32> {
        let mut a = vec![0.0f32; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let (x0, x1) = (x.saturating_sub(r), (x + r).min(sw - 1));
                a[y * sw + x] = (x0..=x1).map(|xx| v[y * sw + xx]).fold(start, pick);
            }
        }
        let mut b = vec![0.0f32; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let (y0, y1) = (y.saturating_sub(r), (y + r).min(sh - 1));
                b[y * sw + x] = (y0..=y1).map(|yy| a[yy * sw + x]).fold(start, pick);
            }
        }
        b
    };
    let min_filter = |v: &[f32]| rank_filter(v, f32::MAX, f32::min);
    let dark = min_filter(&small.iter().map(|c| c[0].min(c[1]).min(c[2])).collect::<Vec<_>>());
    // Atmospheric light: mean colour of the brightest 0.1 % of the dark channel (ties in the
    // order of the cells).
    let mut idx: Vec<usize> = (0..sw * sh).collect();
    idx.sort_by(|&a, &b| dark[b].total_cmp(&dark[a]));
    let top = d.top;
    let mut air = [0.0f32; 3];
    for &i in &idx[..top] {
        for c in 0..3 {
            air[c] += small[i][c] / top as f32;
        }
    }
    let air = air.map(|v| v.max(spatial::DEHAZE_AIR_FLOOR));
    let norm_dark = min_filter(&small.iter().map(|c| (c[0] / air[0]).min(c[1] / air[1]).min(c[2] / air[2])).collect::<Vec<_>>());
    // Opened (a maximum filter of the same radius after the minimum): the minimum grows every
    // dark object by `r`, so sky beside a dark building took its dark channel, counted as
    // haze-free and kept a bright halo (#9273). Opening shrinks dark regions back to their
    // outline and is still a lower bound of the dark channel over the window. It raises the dark
    // channel on average, so the ω weight is scaled to keep the slider's overall strength.
    let opened = rank_filter(&norm_dark, f32::MIN, f32::max);
    // Summed in f64: an f32 running sum over the copy's few hundred thousand cells drifts by about
    // 1e-5, which the low transmission beside haze multiplies up to a visible fraction of a level.
    let mean = |v: &[f32]| (v.iter().map(|x| f64::from(*x)).sum::<f64>() / v.len().max(1) as f64) as f32;
    let weight = spatial::DEHAZE_OMEGA * mean(&norm_dark) / mean(&opened).max(1e-6);
    let t_raw: Vec<f32> = opened.iter().map(|d| 1.0 - weight * d).collect();
    let guide: Vec<f32> = small.iter().map(|c| luma(*c)).collect();
    let t = guided(&guide, &t_raw, sw, sh, d.guide_radius, spatial::DEHAZE_EPS);
    let ks = k as f32;
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let tv = util::bilinear(&t, sw, sh, 1, 0, (x as f32 + 0.5) / ks - 0.5, (y as f32 + 0.5) / ks - 0.5).clamp(spatial::DEHAZE_T_FLOOR, 1.0);
            for c in 0..3 {
                let i = q[c];
                let j = (i - air[c]) / tv + air[c];
                q[c] = if amount >= 0.0 { i + (j - i) * amount } else { i + (air[c] - i) * (-amount) * 0.5 * (1.0 - tv * 0.5) };
            }
        }
    });
}

fn sharpening_detail(detail: f32) -> f32 {
    let detail = if detail.is_nan() { CameraRaw::default().sharpen_detail } else { detail.clamp(0.0, 100.0) };
    detail / 100.0
}

fn detail(px: &mut [[f32; 4]], w: usize, h: usize, d: &Detail) {
    // Work in Y + chroma (BT.709 luma, B−Y, R−Y).
    let mut y: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]])).collect();
    let mut cb: Vec<f32> = px.iter().zip(&y).map(|(q, l)| q[2] - l).collect();
    let mut cr: Vec<f32> = px.iter().zip(&y).map(|(q, l)| q[0] - l).collect();
    if let Some(n) = &d.luminance {
        let smooth = guided(&y, &y, w, h, n.radius, n.eps);
        for i in 0..w * h {
            y[i] = smooth[i] + (y[i] - smooth[i]) * n.keep;
        }
    }
    if let Some(c) = &d.color {
        let (bcb, bcr) = (box_means(&cb, w, h, &c.radii), box_means(&cr, w, h, &c.radii));
        for i in 0..w * h {
            cb[i] = bcb[i] + (cb[i] - bcb[i]) * c.keep;
            cr[i] = bcr[i] + (cr[i] - bcr[i]) * c.keep;
        }
    }
    if let Some(s) = &d.sharpen {
        let blur = box_means(&y, w, h, &s.radii);
        // Masking: an edge mask from the gradient of a smoothed luminance.
        let mask: Option<Vec<f32>> = s.mask.as_ref().map(|(radii, [e0, e1])| {
            let s = box_means(&y, w, h, radii);
            util::par_map(w * h, |i| {
                let (x, yy) = (i % w, i / w);
                let gx = s[yy * w + (x + 1).min(w - 1)] - s[yy * w + x.saturating_sub(1)];
                let gy = s[(yy + 1).min(h - 1) * w + x] - s[yy.saturating_sub(1) * w + x];
                smoothstep(*e0, *e1, gx.hypot(gy))
            })
        });
        for i in 0..w * h {
            // Low Detail damps large (halo-producing) differences.
            let hp = (y[i] - blur[i]).clamp(-s.limit, s.limit) * s.scale;
            let m = mask.as_ref().map_or(1.0, |m| m[i]);
            y[i] += s.amount * hp * m;
        }
    }
    for i in 0..w * h {
        let (l, b, r) = (y[i], cb[i], cr[i]);
        let (rr, bb) = (r + l, b + l);
        let g = (l - 0.2126 * rr - 0.0722 * bb) / 0.7152;
        px[i][0] = rr;
        px[i][1] = g;
        px[i][2] = bb;
    }
}

fn vignette(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw) {
    let Vignette { amount: amt, highlights: hl, style, centre: [cx, cy], axes: [ax, ay], power: pw, edges: [edge0, edge1] } = Vignette::new(w, h, p);
    par_rows(px, w, 1, |y, row| {
        for (x, q) in row.iter_mut().enumerate() {
            let (dx, dy) = (((x as f32 + 0.5 - cx) / ax).abs(), ((y as f32 + 0.5 - cy) / ay).abs());
            let r = (dx.powf(pw) + dy.powf(pw)).powf(1.0 / pw);
            let m = smoothstep(edge0, edge1, r);
            if m <= 0.0 {
                continue;
            }
            let l = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
            match style {
                VignetteStyle::PaintOverlay => {
                    let target = if amt < 0.0 { 0.0 } else { 1.0 };
                    let k = m * amt.abs();
                    for c in 0..3 {
                        q[c] += (target - q[c]) * k;
                    }
                }
                _ => {
                    let mut k = 2f32.powf(amt * 2.0 * m);
                    if amt < 0.0 && hl > 0.0 {
                        // Highlights: let bright areas punch through a darkening vignette.
                        let protect = smoothstep(0.6, 1.0, l) * hl;
                        k += (1.0 - k) * protect;
                    }
                    if style == VignetteStyle::ColorPriority {
                        let nl = l * k;
                        for c in 0..3 {
                            q[c] += nl - l;
                        }
                    } else {
                        for c in 0..3 {
                            q[c] *= k;
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CameraRaw` with the named fields set (the type is `#[non_exhaustive]`, so no struct expression), or a copy of a base with them set.
    macro_rules! cr {
        ($($f:ident : $v:expr),* $(,)?) => {{
            let mut s = CameraRaw::default();
            $(s.$f = $v;)*
            s
        }};
        ($base:expr => $($f:ident : $v:expr),* $(,)?) => {{
            let mut s: CameraRaw = $base;
            $(s.$f = $v;)*
            s
        }};
    }

    /// [`super::develop`] with the default inputs (full scale, sRGB encoding, seed 0).
    fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
        super::develop(px, w, h, p, &DevelopInputs::default(), float);
    }

    /// [`super::per_pixel`] with the default inputs.
    fn per_pixel(p: &CameraRaw, w: usize, h: usize, float: bool, points: Option<(f32, f32)>) -> Option<PerPixel> {
        super::per_pixel(p, &DevelopInputs::default(), w, h, float, points)
    }

    #[test]
    fn sharpening_detail_stays_in_range_at_the_shared_entry_point() {
        for value in [0.0_f32, 25.0, 37.5, 100.0] {
            assert_eq!(sharpening_detail(value).to_bits(), (value / 100.0).to_bits());
        }
        assert_eq!(sharpening_detail(-1.0), 0.0);
        assert_eq!(sharpening_detail(101.0), 1.0);
        assert_eq!(sharpening_detail(f32::NAN), sharpening_detail(CameraRaw::default().sharpen_detail));
        assert_eq!(sharpening_detail(f32::NEG_INFINITY), 0.0);
        assert_eq!(sharpening_detail(f32::INFINITY), 1.0);
    }

    /// #9273: Dehaze clears the haze evenly over a sky, right up to a dark building, instead of
    /// leaving a bright halo of uncorrected haze around it. Synthetic scene: a clear sky and a
    /// dark tower, hazed with `I = J·t + A·(1 − t)`.
    #[test]
    fn dehaze_leaves_no_halo_beside_dark_objects() {
        let (w, h) = (600usize, 400usize);
        let tower = |x: usize, y: usize| (250..350).contains(&x) && y >= 120;
        let (t, air) = (0.55f32, [0.9f32, 0.9, 0.9]);
        let mut px: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let j = if tower(x, y) { [0.06, 0.06, 0.07] } else { [0.35 + 0.1 * y as f32 / h as f32, 0.5, 0.8] };
                let c: [f32; 3] = std::array::from_fn(|k| j[k] * t + air[k] * (1.0 - t));
                [c[0], c[1], c[2], 1.0]
            })
            .collect();
        let d = Dehaze::new(&cr! { dehaze: 100.0}, w, h).unwrap();
        dehaze(&mut px, w, h, &d);
        let sky = |xs: std::ops::Range<usize>| {
            let v: Vec<f32> = (150..390)
                .flat_map(|y| xs.clone().map(move |x| (x, y)))
                .map(|(x, y)| luma([px[y * w + x][0], px[y * w + x][1], px[y * w + x][2]]))
                .collect();
            v.iter().sum::<f32>() / v.len() as f32
        };
        // Sky 1–8 px from the tower against open sky: the haze left beside it, in 8-bit levels.
        let halo = (sky(242..250) - sky(20..80)) * 255.0;
        // 6.4 levels before the dark channel was opened, under 1 after.
        assert!(halo < 3.0, "a {halo:.1}-level halo beside the tower");
        // The open sky is still cleared: darker than the hazy input.
        let hazy = luma(std::array::from_fn(|k| [0.4, 0.5, 0.8][k] * t + air[k] * (1.0 - t)));
        assert!(sky(20..80) < hazy - 0.02, "{} vs hazy {hazy}", sky(20..80));
    }

    #[test]
    fn new_curves_are_validated_but_saved_curves_still_load_and_render() {
        for field in ["pointCurve", "pointCurveRed", "pointCurveGreen", "pointCurveBlue"] {
            for points in [
                serde_json::json!([[128, 128]]),
                serde_json::json!([[255, 255], [0, 0]]),
                serde_json::json!([[0, 0], [0, 255]]),
                serde_json::json!([[0, -1], [255, 255]]),
                serde_json::json!([[0, 0], [256, 255]]),
                serde_json::json!([[0, 0], [1e30, 255]]),
                serde_json::json!((0..17).map(|i| [i * 15, i * 15]).collect::<Vec<_>>()),
            ] {
                let p: CameraRaw = serde_json::from_value(serde_json::json!({field: points})).unwrap();
                assert!(p.validate().is_err(), "{field}: {points}");
            }
            for points in [
                serde_json::json!([]),
                serde_json::json!([[0, 0], [255, 255]]),
                serde_json::json!((0..16).map(|i| [i * 17, i * 17]).collect::<Vec<_>>()),
            ] {
                let p: CameraRaw = serde_json::from_value(serde_json::json!({field: points})).unwrap();
                assert!(p.validate().is_ok(), "{field}: {points}");
            }
        }
        assert_eq!(serde_json::from_str::<CameraRaw>("{}").unwrap(), CameraRaw::default());
        // An older editor could stack two points on one input and exceed 16 points; such a
        // saved Smart Filter must still develop instead of silently disappearing.
        let mut legacy: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 12.0, i as f32 * 12.0]).collect();
        legacy.insert(5, [60.0, 200.0]);
        let p: CameraRaw = serde_json::from_value(serde_json::json!({"pointCurve": legacy})).unwrap();
        assert!(p.validate().is_err());
        let edited = cr! { p.clone() => exposure: 1.0};
        assert!(edited.validate_changes(&p).is_ok(), "an unchanged stored curve stays editable");
        let broken = cr! { p.clone() => point_curve: vec![[10.0, 10.0]]};
        assert!(broken.validate_changes(&p).is_err(), "a changed curve is validated");
        let mut px = img(4, 4);
        develop(&mut px, 4, 4, &p, false);
        assert!(px.iter().flatten().all(|v| v.is_finite()));
    }

    fn img(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [0.2 + 0.6 * x, 0.3 + 0.4 * y, 0.5 - 0.3 * x * y, 1.0]
            })
            .collect()
    }

    fn mean(px: &[[f32; 4]], c: usize) -> f32 {
        px.iter().map(|q| q[c]).sum::<f32>() / px.len() as f32
    }

    #[test]
    fn defaults_are_identity() {
        let mut a = img(40, 30);
        let b = a.clone();
        develop(&mut a, 40, 30, &CameraRaw::default(), false);
        assert_eq!(a, b);
    }

    #[test]
    fn basic_panel_moves_the_right_way() {
        let (w, h) = (64, 48);
        let base = img(w, h);
        let run = |p: CameraRaw| {
            let mut a = base.clone();
            develop(&mut a, w, h, &p, false);
            a
        };
        let warm = run(cr! { temperature: 50.0});
        assert!(mean(&warm, 0) > mean(&base, 0) && mean(&warm, 2) < mean(&base, 2));
        let bright = run(cr! { exposure: 1.0});
        assert!(mean(&bright, 1) > mean(&base, 1) + 0.1);
        let shadows = run(cr! { shadows: 100.0});
        let dark_px = |a: &[[f32; 4]]| a[0][1];
        assert!(dark_px(&shadows) > dark_px(&base));
        let hl = run(cr! { highlights: -100.0});
        assert!(hl[w * h - 1][0] < base[w * h - 1][0]);
        let con = run(cr! { contrast: 100.0});
        // More contrast: darker darks, brighter brights.
        assert!(con[0][1] < base[0][1] && con[w * h - 1][0] > base[w * h - 1][0]);
        let desat = run(cr! { saturation: -100.0});
        assert!(desat.iter().all(|q| (q[0] - q[1]).abs() < 1e-4 && (q[1] - q[2]).abs() < 1e-4));
        let vib = run(cr! { vibrance: 100.0});
        let spread = |q: [f32; 4]| q[0].max(q[1]).max(q[2]) - q[0].min(q[1]).min(q[2]);
        assert!(spread(vib[w * h / 2]) > spread(base[w * h / 2]));
        for p in [cr! { clarity: 80.0}, cr! { texture: 80.0}, cr! { dehaze: 60.0}, cr! { dehaze: -60.0}, cr! { whites: 50.0, blacks: -50.0}] {
            let out = run(p.clone());
            assert!(out.iter().all(|q| q[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))), "{p:?}");
            assert_ne!(out, base);
        }
    }

    #[test]
    fn camera_raw_curve_keeps_legacy_close_knots() {
        // The middle knots survive the original f32 normalization and make a sharp step.
        let x = 127.5_f32;
        let close = f32::from_bits(x.to_bits() + 33);
        let lut = curve_lut(&[[0.0, 0.0], [x, 0.0], [close, 255.0], [255.0, 255.0]], 1024);
        assert!(lut[256] < 0.01 && lut[767] > 0.99, "expected the legacy step, got {} and {}", lut[256], lut[767]);
    }

    #[test]
    fn curves_hsl_grading_detail_effects() {
        // Monotone cubic LUT through the points.
        let lut = curve_lut(&[[0.0, 0.0], [64.0, 40.0], [192.0, 220.0], [255.0, 255.0]], 256);
        assert!((lut[64] - 40.0 / 255.0).abs() < 0.01 && (lut[192] - 220.0 / 255.0).abs() < 0.01);
        assert!(lut.windows(2).all(|p| p[1] >= p[0]));
        // Band weights sum to one everywhere.
        for hh in (0..360).step_by(7) {
            let s: f32 = band_weights(hh as f32).iter().sum();
            assert!((s - 1.0).abs() < 1e-4, "{hh}: {s}");
        }
        let (w, h) = (48, 32);
        let reds: Vec<[f32; 4]> = vec![[0.8, 0.2, 0.2, 1.0]; w * h];
        let mut a = reds.clone();
        let mut hs = [0.0; 8];
        hs[0] = -100.0;
        develop(&mut a, w, h, &cr! { hsl_sat: hs}, false);
        assert!(a[0][0] - a[0][1] < 0.1, "reds desaturated: {:?}", a[0]);
        let blues: Vec<[f32; 4]> = vec![[0.2, 0.2, 0.8, 1.0]; w * h];
        let mut b = blues.clone();
        develop(&mut b, w, h, &cr! { hsl_sat: hs}, false);
        assert!((b[0][2] - 0.8).abs() < 1e-4, "blues untouched");
        // Grading: blue shadows tint dark pixels, leave bright ones nearly alone.
        let mut g: Vec<[f32; 4]> = vec![[0.1, 0.1, 0.1, 1.0], [0.95, 0.95, 0.95, 1.0]];
        develop(&mut g, 2, 1, &cr! { grade_shadows: Wheel { hue: 220.0, sat: 80.0, lum: 0.0 }}, false);
        assert!(g[0][2] > g[0][0] + 0.05 && (g[1][2] - g[1][0]).abs() < 0.02, "{g:?}");
        // Point curve + parametric.
        let mut c = img(w, h);
        develop(&mut c, w, h, &cr! { point_curve: vec![[0.0, 30.0], [255.0, 255.0]], curve_shadows: 50.0}, false);
        assert!(c.iter().all(|q| q[1] >= 30.0 / 255.0 - 1e-4));
        // Noise reduction reduces variance; sharpening increases it; grain is deterministic.
        let noisy: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let n = hash01(i as i64, 0, 3) * 0.2;
                [0.4 + n, 0.4 + n, 0.4 + n, 1.0]
            })
            .collect();
        let var = |a: &[[f32; 4]]| {
            let m = mean(a, 1);
            a.iter().map(|q| (q[1] - m).powi(2)).sum::<f32>() / a.len() as f32
        };
        let mut nr = noisy.clone();
        develop(&mut nr, w, h, &cr! { noise_luminance: 80.0}, false);
        assert!(var(&nr) < var(&noisy) * 0.5);
        // Out-of-range noise settings are held to their 0..=100 range, not turned into huge radii.
        let mut wild = noisy.clone();
        let hostile = cr! { noise_luminance: 1.0e30, noise_color: 1.0e30, noise_luminance_detail: 1.0e30, noise_color_detail: 1.0e30};
        develop(&mut wild, w, h, &hostile, false);
        assert!(wild.iter().all(|q| q[0].is_finite()));
        let mut sh = noisy.clone();
        develop(&mut sh, w, h, &cr! { sharpen_amount: 100.0}, false);
        assert!(var(&sh) > var(&noisy));
        let mut g1 = img(w, h);
        let mut g2 = img(w, h);
        let gp = cr! { grain_amount: 50.0};
        let seeded = DevelopInputs { seed: 4, ..Default::default() };
        super::develop(&mut g1, w, h, &gp, &seeded, false);
        super::develop(&mut g2, w, h, &gp, &seeded, false);
        assert_eq!(g1, g2);
        assert_ne!(g1, img(w, h));
        // Post-crop vignette darkens corners, leaves the centre.
        let flat = vec![[0.5f32, 0.5, 0.5, 1.0]; w * h];
        let mut v = flat.clone();
        develop(&mut v, w, h, &cr! { vignette_amount: -80.0}, false);
        assert!(v[0][0] < 0.35 && (v[(h / 2) * w + w / 2][0] - 0.5).abs() < 0.01, "{:?}", v[0]);
    }

    #[test]
    fn calibration_keeps_the_develop_off_the_per_pixel_gpu_stages() {
        // The per-pixel stages (the Camera Raw dialog's GPU preview) don't run Calibration, so
        // settings with it must fall back to the CPU develop instead of previewing without it.
        for p in [
            cr! { shadow_tint: 40.0},
            cr! { calibration_hue: [0.0, 0.0, 30.0], whites: 20.0},
            cr! { calibration_sat: [-50.0, 0.0, 0.0], exposure: 0.5},
        ] {
            assert!(!spatial(&p));
            assert!(per_pixel(&p, 2, 2, false, Some((0.0, 1.0))).is_none(), "{p:?}");
        }
        assert!(per_pixel(&cr! { exposure: 0.5}, 2, 2, false, None).is_some());
    }

    #[test]
    fn detail_reaches_the_per_pixel_stages_after_measured_whites_and_blacks() {
        // Detail runs after Whites and Blacks, so another implementation can take its parameters
        // and still measure the points on the per-pixel image.
        let sharpened = cr! { sharpen_amount: 60.0, noise_luminance: 30.0, whites: 20.0};
        let stages = per_pixel(&sharpened, 8, 8, false, Some((0.05, 0.9))).expect("Detail alone stays on the per-pixel stages");
        assert_eq!(stages.detail, Detail::new(&sharpened, &DevelopInputs::default()));
        assert!(stages.detail.is_some() && stages.whites_blacks.is_some() && stages.measure_whites_blacks.is_none());
        let px = vec![[0.4f32, 0.5, 0.6, 1.0]; 64];
        assert!(white_black_points(&px, 8, 8, &sharpened, &DevelopInputs::default()).is_some());
        // A stage before Whites and Blacks still leaves the measuring to whatever runs it.
        let clarified = cr!(sharpened.clone() => clarity: 30.0);
        assert!(white_black_points(&px, 8, 8, &clarified, &DevelopInputs::default()).is_none());
        assert!(per_pixel(&clarified, 8, 8, false, None).is_some_and(|s| s.measure_whites_blacks.is_some()));
        assert!(spatial(&sharpened) && !spatial_tone(&sharpened) && spatial_tone(&clarified));
    }

    #[test]
    fn calibration_turns_primaries_keeps_neutrals_and_tints_shadows() {
        let w = calibration_weights(0.0);
        assert!((w[0] - 1.0).abs() < 1e-6, "pure red is all red primary: {w:?}");
        let w = calibration_weights(60.0);
        assert!((w[0] - 0.5).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6, "yellow is half red, half green: {w:?}");
        let w = calibration_weights(300.0);
        assert!((w[2] - 0.5).abs() < 1e-6 && (w[0] - 0.5).abs() < 1e-6, "magenta wraps round: {w:?}");
        let (none, zero) = ([0.0f32; 3], [0.0f32; 3]);
        let grey = calibrate([0.3, 0.3, 0.3], [100.0, -100.0, 50.0], [100.0, -100.0, 100.0]);
        assert!(grey.iter().all(|v| (v - 0.3).abs() < 1e-5), "greys are untouched: {grey:?}");
        // Red turned towards green: it gains green, keeps its luminance.
        let red = calibrate([0.5, 0.0, 0.0], [100.0, 0.0, 0.0], zero);
        assert!(red[1] > 0.05 && (red[1] / red[0] - 0.5).abs() < 0.02 && red[2] < 1e-6, "red turns 30° towards green (orange): {red:?}");
        assert!((luma(red) - luma([0.5, 0.0, 0.0])).abs() < 1e-4, "luminance kept: {red:?}");
        // Blue at saturation −100 is grey; green at −50 is paler.
        let blue = calibrate([0.0, 0.0, 0.8], none, [0.0, 0.0, -100.0]);
        assert!((blue[0] - blue[1]).abs() < 1e-5 && (blue[1] - blue[2]).abs() < 1e-5, "blue at −100 saturation is grey: {blue:?}");
        let green = calibrate([0.2, 0.8, 0.2], none, [0.0, -50.0, 0.0]);
        assert!(green[0] > 0.2 && green[1] < 0.8, "green at −50 is paler: {green:?}");
        // A primary's sliders leave the other primaries alone.
        let unchanged = calibrate([0.0, 0.0, 0.8], [100.0, 100.0, 0.0], [100.0, 100.0, 0.0]);
        assert!(unchanged.iter().zip([0.0, 0.0, 0.8]).all(|(a, b)| (a - b).abs() < 1e-5), "{unchanged:?}");
        // Shadows tint: magenta lifts red and blue over green in a dark pixel, keeps luminance,
        // and leaves white alone.
        let dark = shadow_tint([0.02, 0.02, 0.02], 1.0);
        assert!(dark[0] > dark[1] && dark[2] > dark[1], "{dark:?}");
        assert!((luma(dark) - 0.02).abs() < 1e-4, "luminance kept: {dark:?}");
        let green = shadow_tint([0.02, 0.02, 0.02], -1.0);
        assert!(green[1] > green[0], "{green:?}");
        let white = shadow_tint([1.0, 1.0, 1.0], 1.0);
        assert!(white.iter().all(|v| (v - 1.0).abs() < 1e-3), "white is not tinted: {white:?}");
        // Through develop: neutral settings are an identity, a tint changes dark pixels only.
        let mut p = CameraRaw::default();
        assert!(p.is_identity());
        p.shadow_tint = 100.0;
        assert!(!p.is_identity());
        let mut px = vec![[0.1, 0.1, 0.1, 1.0], [0.95, 0.95, 0.95, 1.0]];
        develop(&mut px, 2, 1, &p, false);
        assert!(px[0][0] > px[0][1], "dark grey turns magenta: {:?}", px[0]);
        assert!((px[1][0] - px[1][1]).abs() < 0.01, "near-white stays neutral: {:?}", px[1]);
        p = CameraRaw::default();
        p.calibration_hue[2] = 100.0;
        let mut px = vec![[0.0, 0.0, 0.8, 1.0]];
        develop(&mut px, 1, 1, &p, false);
        assert!(px[0][0] > 0.05, "blue turned towards red: {:?}", px[0]);
    }

    #[test]
    fn auto_tone_brightens_dark_and_darkens_bright_images() {
        let mut p = CameraRaw::default();
        auto_tone(&[], &mut p);
        assert_eq!(p, CameraRaw::default(), "no pixels: nothing changes");
        let dark: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.05 + (i % 64) as f32 / 64.0 * 0.15;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&dark, &mut p);
        assert!(p.exposure > 0.5, "a dark image gets exposure: {}", p.exposure);
        assert!(p.shadows > 0.0, "and lifted shadows: {}", p.shadows);
        let mut p = CameraRaw::default();
        let bright: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.8 + (i % 64) as f32 / 64.0 * 0.19;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&bright, &mut p);
        assert!(p.exposure < -0.5, "a bright image loses exposure: {}", p.exposure);
        assert!(p.highlights < 0.0, "and its highlights come down: {}", p.highlights);
        let mut p = CameraRaw::default();
        let good: Vec<[f32; 4]> = (0..4096)
            .map(|i| {
                let v = 0.04 + (i % 64) as f32 / 63.0 * 0.92;
                [v, v, v, 1.0]
            })
            .collect();
        auto_tone(&good, &mut p);
        assert!(p.exposure.abs() < 0.3 && p.blacks.abs() < 10.0 && p.whites.abs() < 10.0, "a full-range image is left nearly alone: {p:?}");
        assert!(p.saturation == 0.0 && hsl_neutral(&p), "other panels untouched");
    }

    #[test]
    fn float_documents_keep_overrange() {
        let mut a = vec![[2.0f32, 1.5, 0.5, 1.0]; 16];
        develop(&mut a, 4, 4, &cr! { exposure: 0.5}, true);
        assert!(a[0][0] > 2.0);
    }

    /// A Gaussian narrower than 0.3 px has no box radii, so it returns its input unchanged, and
    /// Texture on a proxy that small changes nothing.
    #[test]
    fn a_gaussian_below_0_3_px_leaves_the_image_alone() {
        assert!(spatial::gauss_radii(0.29).is_empty());
        assert!(!spatial::gauss_radii(0.3).is_empty());
        let (w, h) = (64usize, 48usize);
        let v: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 101) as f32 / 100.0).collect();
        assert_eq!(gauss(&v, w, h, 0.29), v);
        assert_eq!(box_means(&v, w, h, &[]), v);
        assert_ne!(gauss(&v, w, h, 1.0), v);
        let src: Vec<[f32; 4]> = v.iter().map(|&x| [x, 1.0 - x, 0.5 * x, 1.0]).collect();
        let base = cr! { contrast: 10.0};
        let scaled = DevelopInputs { pixel_scale: 0.1, ..Default::default() };
        let (mut plain, mut textured) = (src.clone(), src);
        super::develop(&mut plain, w, h, &base, &scaled, false);
        super::develop(&mut textured, w, h, &cr! { base => texture: 80.0}, &scaled, false);
        assert_eq!(plain, textured);
    }
}
