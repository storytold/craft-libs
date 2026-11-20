//! Pure-Rust finishing-stage helpers: sharpening masks/limits, vignette parameters, value film
//! grain, gamut mapping, and curve saturation refinement.

use craft_develop_model::{DevelopSettings, VignetteStyle};

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the sharpening a pixel gets (`0..1`) under Masking `masking` (`0..1`), from its
/// fine detail `det` (log luminance minus its texture blur): `1` everywhere at `0`, only stronger
/// edges as it rises.
#[inline]
pub fn sharpen_mask_weight(masking: f32, det: f32) -> f32 {
    if masking > 0.0 { smooth(masking * 0.25, masking * 0.25 + 0.15, det.abs()) } else { 1.0 }
}

/// Masking weight for the V3 pixel-scale sharpening band, also used by its Alt-drag preview.
#[inline]
pub fn pixel_sharpen_mask_weight(masking: f32, det: f32) -> f32 {
    if masking > 0.0 { smooth(masking * 0.1, masking * 0.1 + 0.06, det.abs()) } else { 1.0 }
}

/// Gain of the sharpening detail term (per unit of `Amount / 150`), in stops of luminance per stop of detail.
pub const SHARPEN_GAIN: f32 = 7.0;

/// Soft-clip limit (stops) of the sharpening detail term for Detail `0..=100`: `0.03` stops (large
/// edge detail is flattened so halos stay small) up to `1.5` stops (practically linear).
pub fn sharpen_limit(detail: f64) -> f32 {
    let d = (detail / 100.0).clamp(0.0, 1.0) as f32;
    0.03 * 50f32.powf(d)
}

/// The sharpening boost (stops) for pixel-scale detail `det` (log luminance minus its blur at the
/// sharpen radius): linear for small detail, soft-clipped at `limit`, and gated by the Masking edge
/// mask (`mask` = Masking / 100) so flat areas and noise can be left alone.
#[inline]
pub fn sharpen_term(det: f32, limit: f32, mask: f32) -> f32 {
    let m = pixel_sharpen_mask_weight(mask, det);
    SHARPEN_GAIN * m * det / (1.0 + det.abs() / limit)
}

/// Vignette (post-crop) parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vig {
    pub amount: f32,
    pub start: f32,
    pub width: f32,
    pub aspect_mix: f32,
    pub power: f32,
    pub highlights: f32,
    pub style: VignetteStyle,
}

/// Resolve post-crop vignette parameters from `s`, returning `None` when `s.vignette.amount == 0.0`.
pub fn vignette(s: &DevelopSettings) -> Option<Vig> {
    let v = &s.vignette;
    (v.amount != 0.0).then(|| {
        let r = (v.roundness / 100.0) as f32;
        Vig {
            amount: (v.amount / 100.0) as f32,
            start: 0.15 + (v.midpoint / 100.0) as f32 * 0.95,
            width: 0.05 + (v.feather / 100.0) as f32 * 1.1,
            aspect_mix: ((r + 1.0) / 2.0).clamp(0.0, 1.0),
            power: if r >= 0.0 { 2.0 } else { 2.0 + (-r) * 6.0 },
            highlights: (v.highlights / 100.0) as f32,
            style: v.style,
        }
    })
}

/// Hash constants of [`grain_noise`] (shared with the GPU kernel).
pub const GRAIN_HASH: [u32; 4] = [0x8da6_b343, 0xd816_3841, 0xcb1a_b31f, 0x5bd1_e995];

/// The grain of processes V1 to V6: two octaves of value noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueGrain {
    /// Amplitude (display-encoded).
    pub amt: f32,
    /// Noise cell size (output pixels).
    pub cell: f32,
    /// Roughness `0..1`: weight of the finer octave.
    pub rough: f32,
    pub seed: u32,
}

/// Resolve [`ValueGrain`] parameters from `s` at `px_per_long` output pixels per long edge,
/// returning `None` when `s.grain.amount <= 0.0`.
pub fn value_grain(s: &DevelopSettings, px_per_long: f64) -> Option<ValueGrain> {
    let g = &s.grain;
    (g.amount > 0.0).then(|| {
        let cell = (0.0006 + (g.size / 100.0) as f32 * 0.0024) * px_per_long as f32;
        ValueGrain { amt: (g.amount / 100.0) as f32 * 0.13, cell: cell.max(0.6), rough: (g.roughness / 100.0) as f32, seed: g.seed }
    })
}

/// Smooth value noise in `-1..1` at `(x, y)` for `seed`. Non-finite coordinates return `0.0`.
#[inline]
pub fn grain_noise(x: f32, y: f32, seed: u32) -> f32 {
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let h = |i: i32, j: i32| {
        let mut v = (i as u32).wrapping_mul(GRAIN_HASH[0]) ^ (j as u32).wrapping_mul(GRAIN_HASH[1]) ^ seed.wrapping_mul(GRAIN_HASH[2]);
        v ^= v >> 13;
        v = v.wrapping_mul(GRAIN_HASH[3]);
        v ^= v >> 15;
        (v & 0xffff) as f32 / 32768.0 - 1.0
    };
    let (i, j) = (x0 as i32, y0 as i32);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * u;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * u;
    a + (b - a) * v
}

/// A colour needing more desaturation than this (scale < `GAMUT_WARN`) to fit is out of gamut for
/// the gamut warnings (tolerates rounding at the gamut boundary).
pub const GAMUT_WARN: f32 = 0.995;

/// Desaturate linear `r` towards its luminance (weights `luma`) until every channel is in `0..=1`;
/// returns the mapped colour and the chroma scale used (`1.0` = already in gamut).
#[inline]
pub fn gamut_map(r: [f32; 3], luma: [f32; 3]) -> ([f32; 3], f32) {
    gamut_map_to(r, luma, 1.0)
}

/// [`gamut_map`] into `0..=peak` (an HDR render's range; `peak = 1.0` is [`gamut_map`]).
#[inline]
pub fn gamut_map_to(r: [f32; 3], luma: [f32; 3], peak: f32) -> ([f32; 3], f32) {
    let yy = (luma[0] * r[0] + luma[1] * r[1] + luma[2] * r[2]).clamp(0.0, peak);
    let mut t = 1.0f32;
    for c in r {
        if c < 0.0 {
            t = t.min(yy / (yy - c).max(1e-9));
        } else if c > peak {
            t = t.min((peak - yy) / (c - yy).max(1e-9));
        }
    }
    if t < 1.0 { (r.map(|c| yy + (c - yy) * t), t) } else { (r, 1.0) }
}

/// Refine Saturation: scale the curved colour's chroma (around its luma, encoded values) so its
/// saturation (chroma / luma) moves from the curve's towards the pre-curve one: the ratio is
/// `(s_curve / s_before)^refine`, so `1` keeps the curve and `0` restores the original saturation.
#[inline]
pub fn refine_saturation(before: [f32; 3], after: [f32; 3], refine: f32) -> [f32; 3] {
    let luma = |e: [f32; 3]| 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
    let chroma = |e: [f32; 3]| e[0].max(e[1]).max(e[2]) - e[0].min(e[1]).min(e[2]);
    let (y0, y1) = (luma(before), luma(after));
    let s0 = chroma(before) / y0.max(1e-4);
    let s1 = chroma(after) / y1.max(1e-4);
    if s1 <= 1e-6 || s0 <= 1e-6 {
        return after;
    }
    let k = (s0 / s1).powf(1.0 - refine).clamp(0.0, 4.0);
    after.map(|v| y1 + (v - y1) * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharpen_helpers_behave_smoothly_and_bound_detail() {
        assert_eq!(sharpen_mask_weight(0.0, 0.0), 1.0);
        assert_eq!(pixel_sharpen_mask_weight(0.0, 0.0), 1.0);
        assert_eq!(sharpen_mask_weight(0.8, 0.0), 0.0);
        assert!(sharpen_mask_weight(0.8, 1.0) == 1.0);
        assert!((sharpen_limit(0.0) - 0.03).abs() < 1e-6);
        assert!((sharpen_limit(100.0) - 1.5).abs() < 1e-4);
        let lim = sharpen_limit(50.0);
        assert!(sharpen_term(0.0, lim, 0.0) == 0.0);
        assert!(sharpen_term(10.0, lim, 0.0) < SHARPEN_GAIN * lim);
        assert!(sharpen_term(0.1, lim, 0.0) > 0.0);
    }

    #[test]
    fn vignette_resolves_only_when_nonzero() {
        let mut s = DevelopSettings::default();
        assert!(vignette(&s).is_none());
        s.vignette.amount = -50.0;
        s.vignette.roundness = -100.0;
        let v = vignette(&s).unwrap();
        assert!((v.amount - -0.5).abs() < 1e-6);
        assert!((v.power - 8.0).abs() < 1e-6);
        assert!((v.aspect_mix - 0.0).abs() < 1e-6);
    }

    #[test]
    fn grain_is_bounded_and_handles_non_finite_coords() {
        let mut s = DevelopSettings::default();
        assert!(value_grain(&s, 2000.0).is_none());
        s.grain.amount = 50.0;
        s.grain.size = 25.0;
        s.grain.roughness = 50.0;
        let g = value_grain(&s, 2000.0).unwrap();
        assert!(g.amt > 0.0 && g.cell >= 0.6);
        assert_eq!(grain_noise(f32::NAN, 1.0, 7), 0.0);
        assert_eq!(grain_noise(1.0, f32::INFINITY, 7), 0.0);
        for i in 0..20 {
            let n = grain_noise(i as f32 * 0.37, i as f32 * 0.71, g.seed);
            assert!((-1.0..=1.0).contains(&n), "{n}");
        }
    }

    #[test]
    fn gamut_map_clamps_out_of_gamut_towards_luma() {
        let luma = [0.2126, 0.7152, 0.0722];
        let (in_gamut, t0) = gamut_map([0.2, 0.5, 0.8], luma);
        assert_eq!(in_gamut, [0.2, 0.5, 0.8]);
        assert_eq!(t0, 1.0);

        let (mapped, t1) = gamut_map([1.5, -0.2, 0.3], luma);
        assert!(t1 < GAMUT_WARN);
        assert!(mapped.iter().all(|&c| (-1e-6..=1.0 + 1e-6).contains(&c)), "{mapped:?}");

        let (hdr, t2) = gamut_map_to([2.5, 0.5, 0.5], luma, 4.0);
        assert_eq!(hdr, [2.5, 0.5, 0.5]);
        assert_eq!(t2, 1.0);
    }

    #[test]
    fn refine_saturation_interpolates_between_original_and_curved() {
        let before = [0.4, 0.2, 0.1];
        let after = [0.6, 0.3, 0.15];
        assert_eq!(refine_saturation(before, after, 1.0), after);
        let neutral = refine_saturation([0.3, 0.3, 0.3], [0.5, 0.5, 0.5], 0.5);
        assert_eq!(neutral, [0.5, 0.5, 0.5]);
    }
}
