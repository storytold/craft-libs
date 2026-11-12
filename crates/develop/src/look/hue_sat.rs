use super::mono::to_prophoto;
use super::pairs::{displayed, luma};
use super::{CameraLook, CameraTone, HsvTable, Mat3, ToneMap, luminance_2020};
#[allow(unused_imports)]
use crate::sensor::prelude::*;

/// [`HueSat::apply`] restores the luminance in full only if the table kept at least this share of it. A colour
/// the table darkens to [`NO_LUMINANCE_KEPT`] or less (only near-black noise, whose channels go negative) keeps
/// its colour; in between the two are blended, so noisy shadows show no step.
pub(super) const MIN_LUMINANCE_KEPT: f32 = 0.25;

pub(super) const NO_LUMINANCE_KEPT: f32 = 0.15;

/// How much of the table's colour a pixel gets when the table kept `share` of its luminance: 0 up to
/// [`NO_LUMINANCE_KEPT`], 1 from [`MIN_LUMINANCE_KEPT`], a smoothstep between.
pub(super) fn restore_weight(share: f32) -> f32 {
    if !share.is_finite() {
        return 0.0;
    }
    let t = ((share - NO_LUMINANCE_KEPT) / (MIN_LUMINANCE_KEPT - NO_LUMINANCE_KEPT)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A fitted [`CameraLook::hue_sat`] table, ready to apply to linear Rec.2020 pixels. It changes
/// hue and saturation only: luminance is restored, the camera tone curve owns it.
pub struct HueSat<'a> {
    table: &'a HsvTable,
    pub(super) to: [[f32; 3]; 3],
    pub(super) from: [[f32; 3]; 3],
}

impl<'a> HueSat<'a> {
    pub fn new(table: &'a HsvTable) -> Option<HueSat<'a>> {
        let to = to_prophoto();
        Some(HueSat { table, to: to.to_f32(), from: to.inverse()?.to_f32() })
    }

    #[inline]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mul = |m: &[[f32; 3]; 3], v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) };
        let out = mul(&self.from, self.table.apply(mul(&self.to, rgb)));
        let (before, after) = (luminance_2020(rgb), luminance_2020(out));
        // restoring the luminance divides by the table's: near black, where noise makes some channels
        // negative, the table can bring it close to 0, and the gain would turn the pixel white (storytold/lightcraft#820)
        if before <= 0.0 || after <= 0.0 || !out.iter().all(|v| v.is_finite()) {
            return rgb;
        }
        let w = restore_weight(after / before);
        if w >= 1.0 {
            return out.map(|v| v * before / after);
        }
        if w <= 0.0 {
            return rgb;
        }
        std::array::from_fn(|i| rgb[i] + (out[i] * before / after - rgb[i]) * w)
    }

    /// [`Self::apply`] to `N` pixels at once (`rgb[channel][lane]`), bit for bit, through
    /// [`HsvTable::apply_lanes`], with the guards as selects.
    #[inline]
    pub fn apply_lanes<const N: usize>(&self, rgb: [[f32; N]; 3]) -> [[f32; N]; 3] {
        let mul = |m: &[[f32; 3]; 3], v: &[[f32; N]; 3]| {
            let mut out = [[0.0f32; N]; 3];
            for (row, out) in m.iter().zip(&mut out) {
                for l in 0..N {
                    out[l] = row[0] * v[0][l] + row[1] * v[1][l] + row[2] * v[2][l];
                }
            }
            out
        };
        let out = mul(&self.from, &self.table.apply_lanes(mul(&self.to, &rgb)));
        let mut restored = rgb;
        for l in 0..N {
            let (input, out) = ([rgb[0][l], rgb[1][l], rgb[2][l]], [out[0][l], out[1][l], out[2][l]]);
            let (before, after) = (luminance_2020(input), luminance_2020(out));
            let untouched = (before <= 0.0) | (after <= 0.0) | !(out[0].is_finite() & out[1].is_finite() & out[2].is_finite());
            let w = restore_weight(after / before);
            for (c, restored) in restored.iter_mut().enumerate() {
                let full = out[c] * before / after;
                let blend = input[c] + (full - input[c]) * w;
                // (`w >= 1` and `w <= 0` exclude each other)
                restored[l] = if untouched | (w <= 0.0) {
                    input[c]
                } else if w >= 1.0 {
                    full
                } else {
                    blend
                };
            }
        }
        restored
    }
}

/// HSV hue (degrees) and saturation; `None` for black or non-finite colours.
pub(super) fn hue_saturation(p: [f64; 3]) -> Option<(f64, f64)> {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    if !(max.is_finite() && min.is_finite()) || max <= 0.0 {
        return None;
    }
    let d = max - min;
    if d <= 0.0 {
        return Some((0.0, 0.0));
    }
    let h = if max == p[0] {
        ((p[1] - p[2]) / d).rem_euclid(6.0)
    } else if max == p[1] {
        (p[2] - p[0]) / d + 2.0
    } else {
        (p[0] - p[1]) / d + 4.0
    };
    Some((h * 60.0, d / max))
}

/// Table resolution: 5° hue steps (a coarser grid blurred the lime shirt into neighbouring browns
/// that want the opposite shift), saturation 0, 0.25 … 1. Value is not an axis: tone is fitted separately.
const TABLE_HUES: usize = 72;

const TABLE_SATS: usize = 5;

/// Value levels (sRGB-encoded, as DNG tables with encoding 1): cameras turn dark yellows toward
/// orange but bright yellow-greens toward green, which one shift per hue averages away.
const TABLE_VALS: usize = 5;

/// Kernel widths around each table node (hue in degrees, saturation, encoded value).
const KERNEL_HUE: f64 = 2.5;

const KERNEL_SAT: f64 = 0.15;

const KERNEL_VAL: f64 = 0.12;

/// Kernel weight at which a node keeps half of its estimate; sparse nodes shrink to identity.
const SHRINK_WEIGHT: f64 = 5.0;

/// Fit hue shifts and saturation scales of the training pairs left after `matrix`, per (hue,
/// saturation) node, kernel-weighted and shrunk toward identity where the photo has few samples.
/// Saturation 0 stays identity, so neutrals are never tinted.
#[allow(clippy::type_complexity)] // the type is spelled out where it is used
pub(super) fn fit_hue_sat(pairs: &[([f64; 3], [f64; 3])], matrix: &Mat3) -> Option<HsvTable> {
    let to = to_prophoto();
    let samples: Vec<(f64, f64, f64, f64, f64)> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .filter_map(|(_, (x, y))| {
            let p = to.apply(matrix.apply(*x));
            let (hp, sp) = hue_saturation(p)?;
            let value = f64::from(crate::color::transfer::linear_to_srgb(p[0].max(p[1]).max(p[2]).min(1.0) as f32));
            let (ht, st) = hue_saturation(to.apply(*y))?;
            // hue is meaningless near neutral
            if sp < 0.08 || st < 0.02 {
                return None;
            }
            let shift = ((ht - hp + 180.0).rem_euclid(360.0) - 180.0).clamp(-30.0, 30.0);
            let log_scale = (st / sp).ln().clamp(-0.7, 0.7);
            Some((hp, sp.min(1.0), value, shift, log_scale))
        })
        .collect();
    if samples.len() < 64 {
        return None;
    }
    // Samples by table hue step: a node only looks at hues within its kernel's reach.
    let step = 360.0 / TABLE_HUES as f64;
    let mut by_hue: Vec<Vec<(f64, f64, f64, f64, f64)>> = vec![Vec::new(); TABLE_HUES];
    for sample in samples {
        if let Some(bin) = by_hue.get_mut(((sample.0.rem_euclid(360.0) / step) as usize).min(TABLE_HUES - 1)) {
            bin.push(sample);
        }
    }
    let reach = (4.0 * KERNEL_HUE / step).ceil() as usize + 1;
    use rayon::prelude::*;
    let data: Vec<[f32; 3]> = (0..TABLE_VALS * TABLE_HUES * TABLE_SATS)
        .into_par_iter()
        .map(|index| {
            let (v, h, s) = (index / (TABLE_HUES * TABLE_SATS), index / TABLE_SATS % TABLE_HUES, index % TABLE_SATS);
            if s == 0 {
                return [0.0, 1.0, 1.0];
            }
            let (val, hue, sat) = (v as f64 / (TABLE_VALS - 1) as f64, h as f64 * step, s as f64 / (TABLE_SATS - 1) as f64);
            let (mut weight, mut shift, mut log_scale) = (0.0, 0.0, 0.0);
            for offset in 0..=2 * reach {
                let Some(bin) = by_hue.get((h + TABLE_HUES * 2 + offset - reach) % TABLE_HUES) else { continue };
                for &(hp, sp, vp, dh, ls) in bin {
                    let dhue = (hp - hue + 180.0).rem_euclid(360.0) - 180.0;
                    if dhue.abs() > 4.0 * KERNEL_HUE {
                        continue;
                    }
                    let d2 = (dhue / KERNEL_HUE).powi(2) + ((sp - sat) / KERNEL_SAT).powi(2) + ((vp - val) / KERNEL_VAL).powi(2);
                    let k = (-0.5 * d2).exp() * sp;
                    weight += k;
                    shift += k * dh;
                    log_scale += k * ls;
                }
            }
            if weight <= 0.0 {
                return [0.0, 1.0, 1.0];
            }
            let shrink = 1.0 / (weight + SHRINK_WEIGHT);
            [(shift * shrink) as f32, (log_scale * shrink).exp() as f32, 1.0]
        })
        .collect();
    data.iter().all(|e| e.iter().all(|v| v.is_finite())).then_some(HsvTable {
        hue_divisions: TABLE_HUES,
        sat_divisions: TABLE_SATS,
        val_divisions: TABLE_VALS,
        data,
        srgb_value: true,
    })
}

/// Kernel width of the chroma curve's nodes (display luminance) and the weight at which a node
/// keeps half of its estimate.
const CHROMA_KERNEL: f64 = 0.08;

const CHROMA_SHRINK: f64 = 2.0;

/// The look's tone curve with a chroma-by-display-luminance curve fitted to `pairs` (two thirds
/// train, one third held out), when that lowers the held-out error. Colourfulness is compared as
/// the distance from neutral of luminance-normalised RGB.
pub(super) fn fit_chroma(pairs: &[([f64; 3], [f64; 3])], look: &CameraLook) -> Option<CameraTone> {
    let correction = look.hue_sat.as_ref().and_then(HueSat::new);
    let scene = |x: &[f64; 3]| {
        let p = look.matrix.apply(*x);
        correction.as_ref().map_or(p, |c| c.apply(p.map(|v| v as f32)).map(f64::from))
    };
    let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    let predict = |x: &[f64; 3]| displayed(scene(x), &tone);
    let chroma = |p: [f64; 3]| {
        let y = luma(p);
        (y > 0.0).then(|| p.iter().map(|v| (v / y - 1.0).powi(2)).sum::<f64>().sqrt())
    };
    let samples: Vec<(f64, f64, f64)> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .filter_map(|(_, (x, y))| {
            let p = predict(x);
            let (cp, cy) = (chroma(p)?, chroma(*y)?);
            (cp > 0.05).then(|| (luma(p), cp, (cy.max(1e-3) / cp).ln().clamp(0.05f64.ln(), 2.5f64.ln())))
        })
        .collect();
    if samples.len() < 64 {
        return None;
    }
    let mut curve = [1.0f32; crate::tone::CHROMA_N];
    for (j, node) in curve.iter_mut().enumerate() {
        let at = j as f64 / (crate::tone::CHROMA_N - 1) as f64;
        let (mut weight, mut sum) = (0.0, 0.0);
        for &(o, cp, log_ratio) in &samples {
            let k = (-0.5 * ((o - at) / CHROMA_KERNEL).powi(2)).exp() * cp;
            weight += k;
            sum += k * log_ratio;
        }
        *node = (sum / (weight + CHROMA_SHRINK)).exp() as f32;
    }
    let fitted = look.tone.with_chroma(curve)?;
    let with = ToneMap::camera(&fitted, 0.0, 0.0, 0.0);
    let error = |map: &ToneMap| -> f64 {
        pairs
            .iter()
            .step_by(3)
            .map(|(x, y)| {
                let p = displayed(scene(x), map);
                (0..3).map(|c| (p[c] - y[c]).powi(2)).sum::<f64>()
            })
            .sum()
    };
    let (before, after) = (error(&tone), error(&with));
    if crate::profiling() {
        eprintln!("[profile] ARW chroma curve {curve:?}: held-out error {before:.4} -> {after:.4}");
    }
    (after.is_finite() && after < before).then_some(fitted)
}
