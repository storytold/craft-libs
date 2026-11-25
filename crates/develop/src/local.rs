//! Scene-linear preparation: white balance + exposure, and the spatial planes the per-pixel stage
//! needs (edge-aware base layer for highlights/shadows, clarity/texture bands, dehaze veil).

use craft_color_transform::cct::{temp_tint_to_xy, wb_matrix as wb_matrix_checked, xy_to_temp_tint};
use craft_color_transform::{BRADFORD, Mat3, REC2020, RgbSpace, Xy};
use craft_develop_model::{DevelopSettings, LocalAdjustments, Mask, Process, WbMode, WbScale};
use std::sync::Arc;

use crate::colorops::luminance_2020;
use crate::raster::blur::{gauss_taps, gaussian};
use crate::raster::{Plane, Rgb32f, par_join, par_rows as for_rows};

/// Legacy (V1..V9) unchecked Bradford white-balance matrix.
fn wb_matrix_legacy(space: &RgbSpace, src: Xy) -> Mat3 {
    let (Some(s_xyz), Some(d_xyz), Some(inv)) = (src.to_xyz(), space.white.to_xyz(), BRADFORD.inverse()) else {
        return Mat3::IDENTITY;
    };
    let (s, d) = (BRADFORD.apply(s_xyz), BRADFORD.apply(d_xyz));
    let scale = Mat3::diag(d[0] / s[0], d[1] / s[1], d[2] / s[2]);
    let adapt = inv.mul(&scale).mul(&BRADFORD);
    space.from_xyz().mul(&adapt).mul(&space.to_xyz())
}

/// Render quality for spatial planes (`Draft` caps the Highlights/Shadows base radius).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Full,
    Draft,
}

/// Source white-balance metadata needed to interpret [`DevelopSettings::wb`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WbSource {
    pub raw: bool,
    pub as_shot_temp: f64,
    pub as_shot_tint: f64,
    pub relative_wb: bool,
    pub legacy_relative_wb: bool,
}

impl Default for WbSource {
    fn default() -> Self {
        Self { raw: false, as_shot_temp: 6500.0, as_shot_tint: 0.0, relative_wb: false, legacy_relative_wb: false }
    }
}

/// White balance (relative to the source's as-shot white) and exposure, in place.
pub fn scene_linear_pre(img: &mut Rgb32f, info: &WbSource, s: &DevelopSettings) {
    wb_gain(img, info, s, 2f32.powf(s.light.exposure as f32));
}

/// White balance only (exposure is applied by the per-pixel stage).
pub fn white_balance(img: &mut Rgb32f, info: &WbSource, s: &DevelopSettings) {
    wb_gain(img, info, s, 1.0);
}

/// Whether `s` uses the checked Bradford white-balance adaptation (`Process::V10` on) instead
/// of the legacy matrix (V1 to V9, which stay frozen).
pub fn checked_wb(s: &DevelopSettings) -> bool {
    s.process.0 >= 10
}

/// The white-balance matrix (linear Rec.2020) for the settings, or `None` when the as-shot white
/// is kept.
pub fn wb_matrix_for(info: &WbSource, s: &DevelopSettings) -> Option<[[f32; 3]; 3]> {
    wb_matrix_for_with(info, s, None)
}

/// [`wb_matrix_for`] with an optional camera-space white-balance callback (`rebalance(target_xy)`).
pub fn wb_matrix_for_with(info: &WbSource, s: &DevelopSettings, rebalance: Option<&dyn Fn(Xy) -> Option<Mat3>>) -> Option<[[f32; 3]; 3]> {
    let (t, tint) = effective_wb(info, s);
    if (t - info.as_shot_temp).abs() < 1e-6 && (tint - info.as_shot_tint).abs() < 1e-6 {
        return None;
    }
    if let Some(m) = rebalance.filter(|_| info.raw && !info.relative_wb).and_then(|f| f(temp_tint_to_xy(t, tint))) {
        return Some(m.to_f32());
    }
    let checked = checked_wb(s);
    let set = if checked { wb_matrix_checked(&REC2020, effective_white(info, s)) } else { wb_matrix_legacy(&REC2020, effective_white(info, s)) };
    let shot = if checked {
        wb_matrix_checked(&REC2020, temp_tint_to_xy(info.as_shot_temp, info.as_shot_tint))
    } else {
        wb_matrix_legacy(&REC2020, temp_tint_to_xy(info.as_shot_temp, info.as_shot_tint))
    };
    let m = set.mul(&shot.inverse().unwrap_or(Mat3::IDENTITY));
    // Normalize so neutral luminance is preserved (WB shouldn't change exposure).
    let g = m.apply([1.0, 1.0, 1.0]);
    let y = g[0] * 0.2627 + g[1] * 0.6780 + g[2] * 0.0593;
    Some(m.mul(&Mat3::diag(1.0 / y, 1.0 / y, 1.0 / y)).to_f32())
}

fn wb_gain(img: &mut Rgb32f, info: &WbSource, s: &DevelopSettings, gain: f32) {
    let m = wb_matrix_for(info, s);
    let w = img.width;
    for_rows(&mut img.data, w, |_, row| {
        for p in row.iter_mut() {
            let mut c = *p;
            if let Some(m) = &m {
                c = [
                    m[0][0] * c[0] + m[0][1] * c[1] + m[0][2] * c[2],
                    m[1][0] * c[0] + m[1][1] * c[1] + m[1][2] * c[2],
                    m[2][0] * c[0] + m[2][1] * c[1] + m[2][2] * c[2],
                ];
            }
            *p = [(c[0] * gain).max(0.0), (c[1] * gain).max(0.0), (c[2] * gain).max(0.0)];
        }
    });
}

/// The white balance actually in effect (presets resolve to their Kelvin values for raw files; settings written on
/// the relative scale before a raw's camera matrices reached its white balance give the same colour change on the
/// Kelvin scale, see [`legacy_to_kelvin`]).
pub fn effective_wb(info: &WbSource, s: &DevelopSettings) -> (f64, f64) {
    match s.wb.mode {
        WbMode::AsShot => (info.as_shot_temp, info.as_shot_tint),
        _ if legacy_wb(info, s) => legacy_to_kelvin(s.wb.temp, s.wb.tint, (info.as_shot_temp, info.as_shot_tint)),
        m if info.raw && !info.relative_wb => m.preset().unwrap_or((s.wb.temp, s.wb.tint)),
        _ => (s.wb.temp, s.wb.tint),
    }
}

/// The source white the render adapts from the as-shot white to: [`effective_wb`] as a chromaticity, except that a
/// white balance still on the relative scale is composed exactly ([`legacy_white`]) rather than read back through
/// its rounded Kelvin / tint.
fn effective_white(info: &WbSource, s: &DevelopSettings) -> Xy {
    if legacy_wb(info, s)
        && s.wb.mode != WbMode::AsShot
        && let Some(xy) = legacy_white(s.wb.temp, s.wb.tint, (info.as_shot_temp, info.as_shot_tint))
    {
        return xy;
    }
    let (t, tint) = effective_wb(info, s);
    temp_tint_to_xy(t, tint)
}

/// Whether `s` holds a white balance written on the relative scale for a source that is edited in Kelvin today.
fn legacy_wb(info: &WbSource, s: &DevelopSettings) -> bool {
    info.raw && !info.relative_wb && info.legacy_relative_wb && s.wb.scale == WbScale::Legacy
}

/// The white that renders exactly as `(temp, tint)` did on the relative scale, where 6500 K / 0 meant *as shot* and
/// the source's real as-shot white is `as_shot`.
///
/// The relative render adapted the picture from 6500 K / 0 to `(temp, tint)` (Bradford, [`wb_matrix_for`]); the
/// Kelvin render adapts it from `as_shot` to the result. Bradford adaptations are von Kries scalings in one cone
/// space, so they compose exactly: the new white's cone response is the old target's times the as-shot white's
/// over the old reference's, and the two renders give the same matrix (up to the scale the luminance
/// normalisation removes). `None` when the arithmetic has nowhere to go (a non-finite input, a white outside the
/// chromaticity plane).
pub fn legacy_white(temp: f64, tint: f64, as_shot: (f64, f64)) -> Option<Xy> {
    if !(temp.is_finite() && tint.is_finite() && as_shot.0.is_finite() && as_shot.1.is_finite()) {
        return None;
    }
    let cones = |t: f64, tint: f64| temp_tint_to_xy(t, tint).to_xyz().map(|xyz| BRADFORD.apply(xyz));
    let (old, reference, shot) = (cones(temp, tint)?, cones(6500.0, 0.0)?, cones(as_shot.0, as_shot.1)?);
    let inverse = BRADFORD.inverse()?;
    let mut new = [0.0; 3];
    for i in 0..3 {
        if reference[i].abs() < 1e-12 {
            return None;
        }
        new[i] = old[i] * shot[i] / reference[i];
    }
    let xy = Xy::from_xyz(inverse.apply(new))?;
    (xy.x.is_finite() && xy.y.is_finite() && xy.y > 0.0).then_some(xy)
}

/// The Kelvin / tint of [`legacy_white`]: what a white balance written on the relative scale reads as, and becomes
/// when it is next edited ([`normalize_wb`]). Exact wherever the white has a Kelvin value (1000–50000 K after the
/// composition; the relative slider reached 2000–13550 K around 6500 K, so only a far-blue as-shot white pushed
/// to the slider's blue end lands beyond, and clamps), to the precision of the Kelvin / tint parametrisation
/// (better than 0.01 % away from its 4000 K seam). Unchanged when the arithmetic has nowhere to go.
pub fn legacy_to_kelvin(temp: f64, tint: f64, as_shot: (f64, f64)) -> (f64, f64) {
    if temp == 6500.0 && tint == 0.0 && as_shot.0.is_finite() && as_shot.1.is_finite() {
        // the relative scale's as-shot reference, exactly (a Custom value left where As Shot was stays as shot)
        return as_shot;
    }
    match legacy_white(temp, tint, as_shot).map(xy_to_temp_tint) {
        Some((t, tint)) if t.is_finite() && tint.is_finite() => (t, tint),
        _ => (temp, tint),
    }
}

/// The relative-scale `(temp, tint)` that [`legacy_to_kelvin`] would read as the Kelvin `(temp, tint)` on a source
/// whose real as-shot white is `as_shot`: the inverse of [`legacy_white`]'s composition. It runs the Bradford step
/// backwards — the old target's cone response times the old reference's over the as-shot white's — and reads the
/// result back through [`xy_to_temp_tint`]. `(6500, 0)` is the relative scale's as-shot reference, so the Kelvin
/// white of the source itself maps exactly to it. `None` when the arithmetic has nowhere to go (a non-finite input,
/// a white outside the chromaticity plane, or an as-shot cone response of zero, where the forward step discarded it).
pub fn kelvin_to_legacy(temp: f64, tint: f64, as_shot: (f64, f64)) -> Option<(f64, f64)> {
    if !(temp.is_finite() && tint.is_finite() && as_shot.0.is_finite() && as_shot.1.is_finite()) {
        return None;
    }
    if (temp - as_shot.0).abs() < 1e-9 && (tint - as_shot.1).abs() < 1e-9 {
        // the source's own white: the relative scale's as-shot reference, exactly
        return Some((6500.0, 0.0));
    }
    let cones = |t: f64, tint: f64| temp_tint_to_xy(t, tint).to_xyz().map(|xyz| BRADFORD.apply(xyz));
    let (target, reference, shot) = (cones(temp, tint)?, cones(6500.0, 0.0)?, cones(as_shot.0, as_shot.1)?);
    let inverse = BRADFORD.inverse()?;
    let mut old = [0.0; 3];
    for i in 0..3 {
        if shot[i].abs() < 1e-12 {
            return None;
        }
        old[i] = target[i] * reference[i] / shot[i];
    }
    let xy = Xy::from_xyz(inverse.apply(old))?;
    if !(xy.x.is_finite() && xy.y.is_finite() && xy.y > 0.0) {
        return None;
    }
    let (t, tint) = xy_to_temp_tint(xy);
    (t.is_finite() && tint.is_finite()).then_some((t, tint))
}

/// Bring the white balance of `d` onto the scale `info` is edited on, before a new value is written into it: on a
/// raw with a Kelvin scale the values are marked Kelvin, and ones still on the relative scale become the Custom
/// Kelvin / tint that renders the same ([`legacy_to_kelvin`]; a preset mode's look is kept as that custom
/// value, since the preset's Kelvin value would be a different look). Elsewhere the scale is cleared, so values
/// edited on a rendered photo or a raw without matrices never pass for Kelvin when pasted onto one that has them.
pub fn normalize_wb(d: &mut DevelopSettings, info: &WbSource) {
    if !info.raw || info.relative_wb {
        d.wb.scale = WbScale::Legacy;
        return;
    }
    if legacy_wb(info, d) && d.wb.mode != WbMode::AsShot {
        let (t, tint) = legacy_to_kelvin(d.wb.temp, d.wb.tint, (info.as_shot_temp, info.as_shot_tint));
        d.wb.temp = t;
        d.wb.tint = tint;
        d.wb.mode = WbMode::Custom;
    }
    d.wb.scale = WbScale::Kelvin;
}

/// Self-guided filter (He et al.) on a single plane with Gaussian windows of `sigma` px.
pub fn guided(p: &Plane, sigma: f32, eps: f32) -> Plane {
    let (ma, mb) = guided_coeffs(p, sigma, eps);
    guided_apply(p, &ma, &mb)
}

/// Blurred guided-filter coefficients (mean a, mean b) of `p`; every pass row-parallel, and the
/// two independent blurs of each step run side by side.
fn guided_coeffs(p: &Plane, sigma: f32, eps: f32) -> (Plane, Plane) {
    let (mean, corr) = par_join(|| gaussian(p, sigma), || gaussian(&p.map(|v| v * v), sigma));
    let a = corr.zip_map(&mean, |c, m| {
        let var = (c - m * m).max(0.0);
        var / (var + eps)
    });
    let b = mean.zip_map(&a, |m, a| m - a * m);
    par_join(|| gaussian(&a, sigma), || gaussian(&b, sigma))
}

/// `q = a·p + b` per pixel. Reduced coefficients are sampled with the same separable
/// weights and accumulation order as a bilinear resize, without full-size coefficient planes.
fn guided_apply(p: &Plane, a: &Plane, b: &Plane) -> Plane {
    let mut q = Plane::new(p.width, p.height);
    let w = p.width;
    if (a.width, a.height) != (p.width, p.height) {
        use crate::raster::resample::{Filter, weights};
        let wx = weights(a.width, w, Filter::Bilinear);
        let wy = weights(a.height, p.height, Filter::Bilinear);
        // Bilinear upsampling has at most four taps (including zero-weight boundary taps).
        // Keep only those horizontally sampled rows per band, reusing them for neighbouring
        // output rows. Scratch scales with width and active bands, not image area.
        const BAND: usize = 32;
        for_rows(&mut q.data, w.saturating_mul(BAND), |band, rows| {
            let mut cache: [(usize, Vec<f32>, Vec<f32>); 4] = std::array::from_fn(|_| (usize::MAX, vec![0.0; w], vec![0.0; w]));
            let mut bias = vec![0.0; w];
            for (dy, row) in rows.chunks_mut(w).enumerate() {
                let y = band * BAND + dy;
                let (top, ys) = &wy[y];
                bias.fill(0.0);
                for (dy, yw) in ys.iter().enumerate() {
                    let cy = top + dy;
                    let (cached_y, aa, bb) = &mut cache[cy % 4];
                    if *cached_y != cy {
                        let (ar, br) = (a.row(cy), b.row(cy));
                        for ((av, bv), (left, xs)) in aa.iter_mut().zip(bb.iter_mut()).zip(&wx) {
                            *av = 0.0;
                            *bv = 0.0;
                            for (dx, xw) in xs.iter().enumerate() {
                                *av += ar[left + dx] * xw;
                                *bv += br[left + dx] * xw;
                            }
                        }
                        *cached_y = cy;
                    }
                    for (av, ax) in row.iter_mut().zip(aa) {
                        *av += *ax * yw;
                    }
                    for (bv, bx) in bias.iter_mut().zip(bb) {
                        *bv += *bx * yw;
                    }
                }
                for ((v, pv), bv) in row.iter_mut().zip(p.row(y)).zip(&bias) {
                    *v = *v * pv + bv;
                }
            }
        });
        return q;
    }
    for_rows(&mut q.data, w, |y, row| {
        for ((v, pv), (av, bv)) in row.iter_mut().zip(p.row(y)).zip(a.row(y).iter().zip(b.row(y))) {
            *v = av * pv + bv;
        }
    });
    q
}

/// Fast guided filter (He & Sun 2015): coefficients computed on a `s×` subsampled plane and
/// sampled bilinearly at full resolution; the output keeps full-resolution edges. Equivalent to [`guided`] for
/// large windows at a fraction of the cost.
pub fn guided_fast(p: &Plane, sigma: f32, eps: f32) -> Plane {
    use crate::raster::resample::{Filter, resize};
    let s = guided_fast_step(sigma);
    if s <= 1 {
        return guided(p, sigma, eps);
    }
    let (lw, lh) = (p.width.div_ceil(s).max(1), p.height.div_ceil(s).max(1));
    let lo = resize(p, lw, lh, Filter::Box);
    let (ma, mb) = guided_coeffs(&lo, sigma / s as f32, eps);
    guided_apply(p, &ma, &mb)
}

/// Separable median (`2r+1` px along each row, then each column; clamped edges). It keeps the
/// steps between regions where they are and removes features thinner than about `r` px.
pub fn median_hv(p: &Plane, r: usize) -> Plane {
    if r == 0 || p.data.is_empty() {
        return p.clone();
    }
    let rows = |src: &Plane| {
        let mut out = Plane::new(src.width, src.height);
        for_rows(&mut out.data, src.width, |y, row| median_row(src.row(y), row, r));
        out
    };
    transpose(&rows(&transpose(&rows(p))))
}

/// Running median of `src` over `2r+1` taps (clamped edges) into `out`: a sorted window that
/// drops the leaving value and inserts the entering one.
fn median_row(src: &[f32], out: &mut [f32], r: usize) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let at = |i: isize| total_key(src[i.clamp(0, n as isize - 1) as usize]);
    let r_ = r as isize;
    let mut win: Vec<i32> = (-r_..=r_).map(at).collect();
    win.sort_unstable();
    for (x, o) in out.iter_mut().enumerate() {
        *o = f32::from_bits(total_key_inv(win[r]));
        let (old, new) = (at(x as isize - r_), at(x as isize + r_ + 1));
        if old != new {
            // Ranks by counting the smaller keys in one branch-free pass the compiler vectorizes.
            // Binary searches here mispredicted on most steps and, with `remove` + `insert`, cost
            // 2.4 s per 24 MP at r = 9 on one core.
            let (mut i, mut j) = (0u32, 0u32);
            for &v in &win {
                i += u32::from(v < old);
                j += u32::from(v < new);
            }
            let (i, j) = (i as usize, j as usize);
            // `old` sits at `i`; shift the keys between the two ranks by one over it
            if j > i {
                win.copy_within(i + 1..j, i);
                win[j - 1] = new;
            } else {
                win.copy_within(j..i, j + 1);
                win[j] = new;
            }
        }
    }
}

/// `v`'s bits as an `i32` that orders like [`f32::total_cmp`] (the same transform it applies), so
/// equal keys are equal bits.
fn total_key(v: f32) -> i32 {
    let b = v.to_bits() as i32;
    b ^ (((b >> 31) as u32) >> 1) as i32
}

/// The bits [`total_key`] came from (the transform is its own inverse: it keeps the sign bit).
fn total_key_inv(k: i32) -> u32 {
    (k ^ (((k >> 31) as u32) >> 1) as i32) as u32
}

fn transpose(p: &Plane) -> Plane {
    let (w, h) = (p.width, p.height);
    let mut t = Plane::new(h, w);
    for_rows(&mut t.data, h, |x, row| {
        for (y, v) in row.iter_mut().enumerate() {
            *v = p.data[y * w + x];
        }
    });
    t
}

/// Detail scale of the highlights/shadows base from process V2, as a fraction of the long edge:
/// the base is the edge-aware filter of log luminance without its detail at this radius, so
/// twigs, lines and fine texture brighter or darker than their surroundings take their
/// surroundings' shift (and keep their contrast, as with Exposure) while region boundaries stay
/// sharp (no halos). V1's base is the edge-aware filter of log luminance itself (issue #632).
pub const BASE_DETAIL: f64 = 0.0015;
/// Largest [`base_detail_radius`] (px).
pub const BASE_DETAIL_MAX: usize = 24;

/// Median radius (px) of the highlights/shadows base at `px_per_long` (see [`BASE_DETAIL`]).
pub fn base_detail_radius(px_per_long: f64) -> usize {
    ((BASE_DETAIL * px_per_long).round().max(0.0) as usize).min(BASE_DETAIL_MAX)
}

/// Median radius (px) of the highlights/shadows base of `s`'s rendering process at `px_per_long`:
/// 0 (no median) under V1, [`base_detail_radius`] from V2 (later processes keep V2's base).
pub fn tone_base_detail(s: &DevelopSettings, px_per_long: f64) -> usize {
    match s.process.process() {
        Process::V1 => 0,
        Process::V2 | Process::V3 | Process::V4 | Process::V5 | Process::V6 | Process::V7 | Process::V8 => base_detail_radius(px_per_long),
    }
}

/// The highlights/shadows base layer of log luminance `l`: the fast guided filter ([`BASE_EPS`])
/// at `detail` px ([`tone_base_detail`]) of its [`detail_free`] plane when `keeps_gaps`
/// ([`base_keeps_gaps`], V8 on), else of its [`median_hv`] (V2 to V7); at 0, of `l` itself (V1).
pub fn tone_base(l: &Plane, sigma: f32, detail: usize, keeps_gaps: bool) -> Plane {
    if detail == 0 {
        return guided_fast(l, sigma, BASE_EPS);
    }
    let m = if keeps_gaps { detail_free(l, detail) } else { median_hv(l, detail) };
    guided_fast(&m, sigma, BASE_EPS)
}

/// Downscale factor of the coarse median in [`detail_free`]: its radius at full size is this many
/// times the fine one's.
pub const BASE_COARSE_STEP: usize = 3;

/// Log luminance `l` without detail finer than `r` px (the highlights/shadows base of process V8
/// on takes its edge-aware filter): each value moves toward its [`median_hv`] at `r` only as far as the
/// median of a [`BASE_COARSE_STEP`]× wider neighbourhood agrees (`l` clamped between the two).
/// A twig, line or speck in an even surround has both medians on the surround's side and takes
/// its tone, as #632 wants. The gap between two letter strokes, which a median at `r` fills with the
/// strokes' tone, sits in a wider neighbourhood mostly of the panel's tone: the two medians
/// disagree and the gap keeps its own. The coarse median is taken at `r` of a box-downscaled copy and
/// enlarged bilinearly, like [`guided_fast`]'s coefficients.
pub fn detail_free(l: &Plane, r: usize) -> Plane {
    use crate::raster::resample::{Filter, resize};
    if r == 0 || l.data.is_empty() {
        return l.clone();
    }
    let (w, h) = (l.width, l.height);
    let (lw, lh) = (w.div_ceil(BASE_COARSE_STEP).max(1), h.div_ceil(BASE_COARSE_STEP).max(1));
    if (lw, lh) == (w, h) {
        // nothing to downscale: both medians are the same
        return median_hv(l, r);
    }
    let lo = median_hv(&resize(l, lw, lh, Filter::Box), r);
    // The bilinear enlargement of `lo` (`resize`'s horizontal pass, then its vertical pass taken
    // row by row with the same sums) is clamped into `out` as it is made, not stored.
    let wide = resize(&lo, w, lh, Filter::Bilinear);
    let wy = crate::raster::resample::weights(lh, h, Filter::Bilinear);
    let mut out = median_hv(l, r);
    for_rows(&mut out.data, w, |y, row| {
        let Some((y0, ws)) = wy.get(y) else { return };
        let mut coarse = vec![0.0f32; w];
        for (k, &wt) in ws.iter().enumerate() {
            let Some(t) = wide.data.get((y0 + k) * w..(y0 + k + 1) * w) else { continue };
            for (c, &v) in coarse.iter_mut().zip(t) {
                *c += v * wt;
            }
        }
        for ((o, &v), &c) in row.iter_mut().zip(l.row(y)).zip(&coarse) {
            *o = clamp_between(v, *o, c);
        }
    });
    out
}

/// `v` clamped between `a` and `b` (in either order); a NaN bound is ignored, not a panic.
fn clamp_between(v: f32, a: f32, b: f32) -> f32 {
    let (lo, hi) = (a.min(b), a.max(b));
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Subsampling step of [`guided_fast`] for `sigma` (1 = no subsampling: the plain guided filter).
pub fn guided_fast_step(sigma: f32) -> usize {
    (sigma / 3.0).floor().clamp(1.0, 16.0) as usize
}

/// Luminance noise reduction: guided filter of log luminance (`sigma` px, `eps`), blended by `k`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NrLum {
    pub sigma: f32,
    pub eps: f32,
    pub k: f32,
}

/// Colour noise reduction: chromaticity blurred by `sigma` px, mixed in by `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NrColor {
    pub sigma: f32,
    pub t: f32,
}

/// Output pixels per pixel of the original photo for an output of `px_per_long` pixels per unit
/// of the long edge of an original `native_long` pixels long, at most 1 (`crate::Plan::detail_scale`).
pub fn detail_scale(px_per_long: f64, native_long: usize) -> f32 {
    let k = (px_per_long / native_long.max(1) as f64) as f32;
    if k.is_finite() { k.clamp(0.0, 1.0) } else { 1.0 }
}

/// Smallest radius (px) noise reduction filters a preview with: [`gaussian`] makes no blur below
/// about 0.58 px and one 3-px box from there ([`lightcraft_raster::blur::box_sigma`]).
const NR_MIN_SIGMA: f32 = 1.0;

/// Below this fade a preview's noise reduction is not drawn at all (less than 1 % of a 1-px blur).
const NR_MIN_BLEND: f32 = 0.01;

/// Radius and blend weight that stand in for a blur of `sigma` px: below [`NR_MIN_SIGMA`] the
/// smallest blur the filter makes, blended in by the ratio of their variances, which keeps the
/// smoothing's second moment (how much it softens the finest detail) that of `sigma`.
fn nr_radius(sigma: f32) -> (f32, f32) {
    if sigma >= NR_MIN_SIGMA {
        return (sigma, 1.0);
    }
    let made = crate::raster::blur::box_sigma(NR_MIN_SIGMA);
    (NR_MIN_SIGMA, (sigma / made).powi(2).min(1.0))
}

/// Noise-reduction parameters at `scale` output pixels per pixel of the original
/// (`crate::Plan::detail_scale`; see [`denoise`]). Noise lives at the original's pixels, so the
/// radii are the full-size radii times `scale`, and the guided filter's `eps` (the variance it
/// treats as noise) follows the noise variance, which averaging `1/scale × 1/scale` pixels into one
/// multiplies by `scale²`. A radius below `NR_MIN_SIGMA` fades instead (`nr_radius`): previews look like
/// the full-size render scaled down (issue #960). At `scale` 1 these are the full-size parameters.
pub fn nr_params(s: &DevelopSettings, scale: f32) -> (Option<NrLum>, Option<NrColor>) {
    let lum = (s.detail.nr_luminance / 100.0) as f32;
    let col = (s.detail.nr_color / 100.0) as f32;
    let scale = if scale.is_finite() { scale.clamp(0.0, 1.0) } else { 1.0 };
    let l = (lum > 0.0).then(|| nr_radius((1.0 + 2.5 * lum) * scale)).filter(|&(_, w)| w >= NR_MIN_BLEND).map(|(sigma, w)| {
        let detail = (s.detail.nr_detail / 100.0) as f32;
        let eps = ((0.002 + lum * lum * 0.25 * (1.0 - 0.7 * detail)) * scale * scale).max(1e-7);
        NrLum { sigma, eps, k: lum.sqrt() * w }
    });
    let smooth = 1.0 + (s.detail.nr_color_smoothness / 100.0) as f32;
    let c = (col > 0.0).then(|| nr_radius((1.5 + 6.0 * col) * scale * smooth)).filter(|&(_, w)| w >= NR_MIN_BLEND).map(|(sigma, w)| {
        let keep = (s.detail.nr_color_detail / 100.0) as f32 * 0.5;
        NrColor { sigma, t: col * (1.0 - keep) * w }
    });
    (l, c)
}

/// Noise reduction at output resolution: luminance via an edge-aware self-guided filter on
/// log-luminance, colour by blurring chromaticity (rgb / Y) and re-applying the original luminance.
/// `scale`: output pixels per pixel of the original ([`nr_params`]). `colour_noise`: the whole
/// frame's noise model for the joint colour NR (`crate::colour_noise`; `None`: measured on `img`).
pub fn denoise(img: &mut Rgb32f, s: &DevelopSettings, scale: f32, colour_noise: Option<&crate::nrc_joint::JointNoise>) {
    let (lum, col) = nr_params(s, scale);
    if col.is_some() && joint_colour_nr(s) {
        apply_nr(img, lum, None);
        if let Some(p) = nrc_params(s, scale) {
            crate::nrc_joint::denoise(img, &p, (s.detail.nr_color_detail / 100.0) as f32, (s.detail.nr_color / 100.0) as f32, colour_noise);
        }
        return;
    }
    apply_nr(img, lum, col);
}

/// Noise reduction with explicit parameters (see [`denoise`]).
pub fn apply_nr(img: &mut Rgb32f, lum: Option<NrLum>, col: Option<NrColor>) {
    let w = img.width;
    if let Some(nr) = lum {
        let l = img.map(log_lum);
        let f = guided(&l, nr.sigma, nr.eps);
        for_rows(&mut img.data, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let d = (f.data[i] - l.data[i]) * nr.k;
                let g = d.exp2();
                *p = p.map(|v| v * g);
            }
        });
    }
    if let Some(nr) = col {
        let chroma = img.map(|c| {
            let y = luminance_2020(c).max(1e-6);
            [c[0] / y, c[1] / y, c[2] / y]
        });
        let b = gaussian(&chroma, nr.sigma);
        for_rows(&mut img.data, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let yl = luminance_2020(*p);
                let c0 = chroma.data[i];
                let cb = b.data[i];
                *p = [0, 1, 2].map(|k| ((c0[k] + (cb[k] - c0[k]) * nr.t) * yl).max(0.0));
            }
        });
    }
}

/// Most samples the noise level of the new colour noise reduction is measured from. The CPU and
/// the GPU sample the same pixels (see `nrc_joint::noise_sample_stride`).
pub const NRC_SAMPLES: usize = 131_072;

/// One self-guided pass of the new colour noise reduction: Gaussian windows of `sigma` px, the
/// coefficients computed on a plane subsampled by `step`, `eps = k ×` the `pct`-quantile of the
/// local variance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NrcPass {
    pub sigma: f32,
    pub step: usize,
    pub k: f32,
    pub pct: f32,
}

/// Parameters of the new colour noise reduction: the offset of the square roots and the passes
/// (fine, then coarse).
#[derive(Clone, Debug, PartialEq)]
pub struct NrcParams {
    pub off: f32,
    pub passes: Vec<NrcPass>,
}

/// The new colour noise reduction's parameters at `scale` output pixels per pixel of the original
/// (`crate::Plan::detail_scale`), `None` when Color is 0.
pub fn nrc_params(s: &DevelopSettings, scale: f32) -> Option<NrcParams> {
    let amount = (s.detail.nr_color / 100.0) as f32;
    if amount <= 0.0 {
        return None;
    }
    let detail = (s.detail.nr_color_detail / 100.0) as f32;
    let smooth = 1.0 + (s.detail.nr_color_smoothness / 100.0) as f32;
    let scale = if scale.is_finite() { scale.clamp(0.05, 1.0) } else { 1.0 };
    let sigma = (2.0 + 10.0 * amount) * smooth * scale;
    let k = 64.0 * amount * (1.5 - detail);
    // the coefficients vary over about sigma: a grid of sigma / 2 holds them
    let pass = |sigma: f32| NrcPass { sigma, step: (sigma / 2.0).floor().clamp(1.0, 16.0) as usize, k, pct: 0.2 };
    // fine, then coarse (three times wider); the joint method takes the coarse one's reach
    Some(NrcParams { off: 0.003, passes: vec![pass(sigma), pass(sigma * 3.0)] })
}

/// Chroma noise variance per channel as a function of the local luminance: `α · Y + β` (shot plus read noise).
#[derive(Clone, Copy, Debug)]
pub struct NrcNoise {
    pub alpha: [f32; 3],
    pub beta: [f32; 3],
}

impl NrcNoise {
    #[inline]
    pub fn var(&self, i: usize, y: f32) -> f32 {
        (self.alpha[i] * y.max(0.0) + self.beta[i]).max(1e-14)
    }
}

/// Weighted least squares `v = a · l + b` over `(l, v, weight)` with `a, b ≥ 0`: the free fit if both come out
/// non-negative, else the better of the line through the origin (`b = 0`) and the constant (`a = 0`).
pub fn fit_nonneg_line(pts: &[(f64, f64, f64)]) -> (f64, f64) {
    let sw: f64 = pts.iter().map(|p| p.2).sum::<f64>().max(1e-300);
    let (ml, mv) = (pts.iter().map(|p| p.2 * p.0).sum::<f64>() / sw, pts.iter().map(|p| p.2 * p.1).sum::<f64>() / sw);
    let (cov, vl) = pts.iter().fold((0.0, 0.0), |(c, v), p| (c + p.2 * (p.0 - ml) * (p.1 - mv), v + p.2 * (p.0 - ml) * (p.0 - ml)));
    if vl > 0.0 {
        let a = cov / vl;
        let b = mv - a * ml;
        if a >= 0.0 && b >= 0.0 {
            return (a, b);
        }
    }
    let sse = |a: f64, b: f64| pts.iter().map(|p| p.2 * (p.1 - a * p.0 - b).powi(2)).sum::<f64>();
    let ll: f64 = pts.iter().map(|p| p.2 * p.0 * p.0).sum();
    let origin = if ll > 0.0 { (pts.iter().map(|p| p.2 * p.0 * p.1).sum::<f64>() / ll).max(0.0) } else { 0.0 };
    let constant = mv.max(0.0);
    if sse(origin, 0.0) < sse(0.0, constant) { (origin, 0.0) } else { (0.0, constant) }
}

/// Whether `s` gets the joint colour noise reduction (`nrc_joint`, process V6 on), else main's
/// blurred chromaticity.
pub fn joint_colour_nr(s: &DevelopSettings) -> bool {
    s.process.process() >= Process::V6
}

pub fn log_lum(c: [f32; 3]) -> f32 {
    (luminance_2020(c).max(1e-7) / crate::tone::GREY).log2()
}

/// Spatial planes of one (pre-exposure) image, each tagged with the radius it was computed at.
#[derive(Clone, Default)]
pub struct Planes {
    pub key: u64,
    /// The image's key without its noise reduction (`Plan::pre_nr_key`).
    pub pre_nr: u64,
    pub log_l: Option<Arc<Plane>>,
    /// Keyed by its radius, process and median radius ([`BaseKey`]).
    pub base: Option<(BaseKey, Arc<Plane>)>,
    pub clarity: Option<(u32, Arc<Plane>)>,
    pub texture: Option<(u32, Arc<Plane>)>,
    /// Sharpening blur (exact Gaussian at the sharpen radius).
    pub sharpen: Option<(SharpenKey, Arc<Plane>)>,
    /// Dark channel and its airlight.
    pub dark: Option<(u32, Arc<Plane>, f32)>,
    /// Blurred chromaticity (local Moiré / Noise).
    pub chroma: Option<(u32, Arc<Rgb32f>)>,
}

impl Planes {
    /// The planes that still fit an image differing from this one only in noise reduction, close
    /// enough for a draft: the wide blurs (base, clarity, dehaze, chromaticity). Its `key` matches
    /// no image, so only another draft takes them.
    pub fn for_other_nr(self) -> Planes {
        Planes { key: 0, pre_nr: self.pre_nr, log_l: None, texture: None, sharpen: None, ..self }
    }
}

/// What the highlights/shadows base plane was computed with: its radius (`f32` bits), the
/// rendering process and the median radius ([`tone_base_detail`]). The base differs between
/// processes, so a photo moved to another one (Update to Current Process, undo) recomputes it.
pub type BaseKey = (u32, Process, usize);

pub type SharpenKey = (u32, Process);

pub fn sharpen_key(s: &DevelopSettings, sigma: f32) -> SharpenKey {
    (sigma.to_bits(), s.process.process())
}

/// Whether `s`'s highlights/shadows base takes [`detail_free`] (process V8 on) instead of V2's plain
/// [`median_hv`] (V2 to V7, which shipped with it and stay frozen).
pub fn base_keeps_gaps(s: &DevelopSettings) -> bool {
    s.process.process() >= Process::V8
}

pub fn pixel_sharpen(s: &DevelopSettings) -> bool {
    s.process.process() >= Process::V3
}

/// The [`BaseKey`] of a base at `sigma` for `s` (`detail`: [`PlaneSigmas::base_detail`]).
pub fn base_key(s: &DevelopSettings, sigma: f32, detail: usize) -> BaseKey {
    (sigma.to_bits(), s.process.process(), detail)
}

/// Reuse `slot` if it was computed with `key` (a radius's `f32` bits, or a [`BaseKey`]), else
/// compute and store it.
pub fn plane_at<K: Copy + PartialEq, P>(slot: &mut Option<(K, Arc<P>)>, key: K, f: impl FnOnce() -> P) -> Arc<P> {
    match slot {
        Some((k, p)) if *k == key => p.clone(),
        _ => {
            let p = Arc::new(f());
            *slot = Some((key, p.clone()));
            p
        }
    }
}

/// Guided-filter epsilons (EV²) of the highlights/shadows base and the clarity band.
pub const BASE_EPS: f32 = 0.35;
pub const CLARITY_EPS: f32 = 0.8;

/// Dark channel of a pixel (dehaze).
#[inline]
pub fn dark_of(c: [f32; 3]) -> f32 {
    c[0].min(c[1]).min(c[2])
}

/// Radius of the local Moiré / colour-noise chromaticity blur, as a fraction of the long edge.
pub const CHROMA_SIGMA: f32 = 0.004;

/// Radii (px) of the spatial planes the settings need (`None` = not needed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaneSigmas {
    /// Edge-aware base for highlights/shadows ([`tone_base`]: fast guided filter, [`BASE_EPS`]; from
    /// process V2 of a median at [`BASE_DETAIL`], from V8 of a detail-free plane).
    pub base: Option<f32>,
    /// Median radius (px) of that base ([`tone_base_detail`]: 0 under V1).
    pub base_detail: usize,
    /// Clarity band (fast guided filter, [`CLARITY_EPS`]).
    pub clarity: Option<f32>,
    /// Texture / sharpening band (Gaussian).
    pub texture: Option<f32>,
    /// Sharpening blur (exact Gaussian, see [`sharpen_sigma`]).
    pub sharpen: Option<f32>,
    /// Dehaze dark channel (Gaussian).
    pub dark: Option<f32>,
    /// Chromaticity blur for local Moiré / colour noise (Gaussian).
    pub chroma: Option<f32>,
}

/// Gaussian sigma (pixels) of the texture / sharpening band at `px_per_long`.
pub fn texture_sigma(px_per_long: f64) -> f32 {
    (0.0018 * px_per_long as f32).max(0.6)
}

/// Sharpening blur radius in output pixels before clamping: Radius (source pixels) times the
/// output pixels per original pixel (`native_long` = the original photo's long edge), so a preview and
/// an export show the same detail at the same displayed scale.
pub fn sharpen_sigma_raw(s: &DevelopSettings, px_per_long: f64, native_long: f64) -> f32 {
    let r = s.detail.sharpen_radius;
    let radius = if r.is_finite() { r.clamp(0.1, 10.0) } else { 1.0 };
    (radius * px_per_long / native_long.max(1.0)) as f32
}

/// Smallest and largest sigma (px) of the sharpening blur. The largest is the most the Gaussian taps
/// support (CPU and GPU share them), so an enlarging export keeps every Radius step distinct.
pub const SHARPEN_SIGMA_MIN: f32 = 0.5;
pub const SHARPEN_SIGMA_MAX: f32 = crate::raster::blur::GAUSS_SIGMA_MAX;

/// The sharpening blur's sigma for a raw value of [`sharpen_sigma_raw`].
pub fn sharpen_sigma(raw: f32) -> f32 {
    raw.clamp(SHARPEN_SIGMA_MIN, SHARPEN_SIGMA_MAX)
}

/// Match the continuous Gaussian's Nyquist response below half a pixel, using a representable
/// FIR blur. Blend the correction out between half and one pixel: no jump at the floor, and
/// full-pixel sharpening (including Radius 1 at native size) is unchanged.
pub fn sharpen_gain(raw: f32) -> f32 {
    if !raw.is_finite() || raw <= 0.0 {
        return 0.0;
    }
    if raw >= 1.0 {
        return 1.0;
    }
    let taps = gauss_taps(sharpen_sigma(raw));
    let radius = taps.len() / 2;
    let response = taps.iter().enumerate().map(|(i, &t)| if (i + radius).is_multiple_of(2) { t } else { -t }).sum::<f32>();
    let desired = -(-0.5 * std::f32::consts::PI.powi(2) * raw * raw).exp_m1();
    let ratio = desired / (1.0 - response).max(1e-6);
    let blend = ((1.0 - raw) / 0.5).clamp(0.0, 1.0);
    1.0 + (ratio - 1.0) * blend
}

pub fn plane_sigmas(s: &DevelopSettings, px_per_long: f64, native_long: f64, q: Quality) -> PlaneSigmas {
    let ppl = px_per_long as f32;
    // A hidden mask, or one with no components, does not change the picture. Its sliders must
    // not pick the tone base Clarity reads (#1214).
    let live = |m: &&Mask| m.visible && !m.components.is_empty();
    let local_any = |f: fn(&LocalAdjustments) -> f64| s.masks.iter().filter(live).any(|m| f(&m.adjust) != 0.0);
    let tone_active = s.light.highlights != 0.0
        || s.light.shadows != 0.0
        || s.masks.iter().filter(live).any(|m| m.adjust.highlights != 0.0 || m.adjust.shadows != 0.0);
    // Edge-aware base at ~1.5% of the long edge (EV² epsilon: edges of > ~0.6 EV are preserved)
    // From V2 it is taken of the median at 0.15%: lines and texture finer than that are detail,
    // not tone (#632).
    let base = tone_active.then(|| {
        let sigma = (0.015 * ppl).max(1.0);
        if q == Quality::Draft { sigma.min(24.0) } else { sigma }
    });
    let clarity = (s.effects.clarity != 0.0 || local_any(|a| a.clarity)).then(|| (0.012 * ppl).max(1.0));
    // local Noise and Defringe read the fine detail band too
    let texture = (s.effects.texture != 0.0
        || local_any(|a| a.texture)
        || local_any(|a| a.noise)
        || (!pixel_sharpen(s) && (s.detail.sharpen_amount != 0.0 || local_any(|a| a.sharpness)))
        || s.masks.iter().filter(live).any(|m| m.adjust.defringe > 0.0))
    .then(|| texture_sigma(px_per_long));
    let dark = (s.effects.dehaze != 0.0 || local_any(|a| a.dehaze)).then(|| (0.02 * ppl).max(1.0));
    let chroma = (local_any(|a| a.moire) || s.masks.iter().filter(live).any(|m| m.adjust.noise > 0.0)).then(|| (CHROMA_SIGMA * ppl).max(1.0));
    let base_detail = if base.is_some() { tone_base_detail(s, px_per_long) } else { 0 };
    // V1/V2 keep their texture-band sharpening; only V3 requests the pixel-scale plane.
    let sharpen_raw = sharpen_sigma_raw(s, px_per_long, native_long);
    let sharpen = (pixel_sharpen(s) && sharpen_gain(sharpen_raw) > 0.0 && (s.detail.sharpen_amount != 0.0 || local_any(|a| a.sharpness)))
        .then(|| sharpen_sigma(sharpen_raw));
    PlaneSigmas { base, base_detail, clarity, texture, sharpen, dark, chroma }
}

/// The airlight is estimated from every `AIRLIGHT_STEP`-th value of the dark channel.
pub const AIRLIGHT_STEP: usize = 7;

/// Airlight estimate: bright end of the dark channel.
pub fn airlight(dark: &Plane) -> f32 {
    airlight_of(dark.data.iter().step_by(AIRLIGHT_STEP).copied().collect())
}

/// Airlight from the sampled dark-channel values (see [`airlight`]): their 99.5th percentile.
pub fn airlight_of(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 1.0;
    }
    let k = ((v.len() as f32) * 0.995) as usize;
    let k = k.min(v.len() - 1);
    v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    v[k].max(0.05)
}

#[cfg(test)]
mod tests {
    /// Edits saved under V1 to V5 keep main's colour noise reduction (blurred chromaticity); V6 gets
    /// the joint method of `nrc_joint`.
    #[test]
    fn colour_nr_follows_the_process() {
        use craft_develop_model::{DevelopSettings, Process};
        let (w, h) = (96usize, 64usize);
        let img = Rgb32f::from_fn(w, h, |x, y| {
            let n = |c: u32| {
                let mut v = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b) ^ c.wrapping_mul(0xc2b2_ae35) | 1;
                v ^= v << 13;
                v ^= v >> 17;
                v ^= v << 5;
                (v as f32 / u32::MAX as f32 - 0.5) * 0.02
            };
            let base = if x < w / 2 { [0.06, 0.03, 0.02] } else { [0.02, 0.04, 0.06] };
            [base[0] + n(1), base[1] + n(2), base[2] + n(3)]
        });
        let run = |p: Process| {
            let mut s = DevelopSettings { process: p.version(), ..Default::default() };
            s.detail.nr_color = 50.0;
            let mut o = img.clone();
            denoise(&mut o, &s, 1.0, None);
            o
        };
        let (v1, v2, v5, v6) = (run(Process::V1), run(Process::V2), run(Process::V5), run(Process::V6));
        assert!(v1.data == v2.data && v2.data == v5.data, "V1 to V5 share main's colour NR");
        // main's colour NR clamps at 0 and keeps each pixel's luminance; the joint method keeps it too
        assert!(v5.data != v6.data, "V6 takes the joint colour NR");
        let mut s = DevelopSettings { process: Process::V6.version(), ..Default::default() };
        s.detail.nr_color = 50.0;
        let mut joint = img.clone();
        let p = nrc_params(&s, 1.0).expect("Color 50 has parameters");
        crate::nrc_joint::denoise(&mut joint, &p, (s.detail.nr_color_detail / 100.0) as f32, 0.5, None);
        assert!(joint.data == v6.data, "V6 is the joint method");
    }

    /// Float mean of a flat, noisy dark patch over three noise realisations; linear modes assert drift bounds.
    #[test]
    #[ignore]
    fn probe_nrc_mean_drift() {
        use craft_develop_model::DevelopSettings;
        let (w, h) = (768usize, 768usize);
        let mut worst = 0.0f64;
        for seed0 in [0x2545_f491_4f6c_dd1du64, 0x9e37_79b9_7f4a_7c15, 0x1234_5678_9abc_def1] {
            let mut seed = seed0;
            let mut gauss = || {
                (0..4)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
                    })
                    .sum::<f32>()
                    * 1.732
            };
            for (name, base, sigma) in [
                ("dark warm, blue near 0", [0.006f32, 0.004, 0.0008], [0.002f32, 0.0015, 0.003]),
                ("cat-like, red near 0", [0.0015, 0.004, 0.009], [0.004, 0.003, 0.005]),
            ] {
                let mut img = Rgb32f::new(w, h);
                for p in img.data.iter_mut() {
                    *p = [0, 1, 2].map(|i| base[i] + sigma[i] * gauss());
                }
                let mean = |im: &Rgb32f| {
                    let n = im.data.len() as f64;
                    [0, 1, 2].map(|i| im.data.iter().map(|p| p[i] as f64).sum::<f64>() / n)
                };
                let m0 = mean(&img);
                for nr in [25.0, 100.0] {
                    let mut s = DevelopSettings::default();
                    s.detail.nr_color = nr;
                    let mut a = img.clone();
                    denoise(&mut a, &s, 1.0, None);
                    let m = mean(&a);
                    let clipped0 = mean(&img.map(|p| p.map(|v| v.max(0.0))));
                    let sd = |im: &Rgb32f, mm: [f64; 3]| {
                        [0, 1, 2].map(|i| (im.data.iter().map(|p| (p[i] as f64 - mm[i]).powi(2)).sum::<f64>() / im.data.len() as f64).sqrt())
                    };
                    worst = worst.max([0, 1, 2].iter().map(|&i| (m[i] / m0[i] - 1.0).abs()).fold(0.0, f64::max));
                    println!(
                        "{name} NR {nr}: true mean {m0:.5?}, clipped noisy mean {clipped0:.5?}, after v2 {m:.5?} → {:+.3?} % vs true; noise sd {:.5?} → {:.5?}",
                        [0, 1, 2].map(|i| 100.0 * (m[i] / m0[i] - 1.0)),
                        sd(&img, m0),
                        sd(&a, m)
                    );
                }
            }
        }
        println!("worst mean deviation {:.4} %", 100.0 * worst);
        assert!(worst < 0.0005, "the joint colour NR moved a channel's mean by {:.3} %", 100.0 * worst);
    }

    /// PROBE (prints, never fails): what colour NR v2 keeps of real colour. A clean synthetic scene (an
    /// equal-luminance red/green edge, a 2 px red line on grey, a saturated blue patch near black, a flat grey
    /// patch) plus noise; each measure compares the output with the clean scene.
    #[test]
    #[ignore]
    fn probe_nrc_real_colour() {
        use craft_develop_model::DevelopSettings;
        let (w, h) = (768usize, 512usize);
        let y_of = |c: [f32; 3]| luminance_2020(c);
        // two colours of the same luminance 0.03
        let red = [0.08f32, 0.0155, 0.02];
        let green = {
            let g = [0.012f32, 0.04, 0.015];
            let k = y_of(red) / y_of(g);
            g.map(|v| v * k)
        };
        let grey = [0.03f32; 3];
        let blue_dark = [0.001f32, 0.0015, 0.02];
        // a bright saturated green next to black (a lit leaf against a dark background)
        let leaf = [0.02f32, 0.12, 0.01];
        let clean = |x: usize, y: usize| -> [f32; 3] {
            if y < 256 {
                if x < 384 { red } else { green }
            } else if x < 384 {
                if (190..192).contains(&x) { red } else { grey }
            } else if x < 576 {
                blue_dark
            } else if y < 384 {
                grey
            } else if x < 672 {
                leaf
            } else {
                [0.0005f32; 3]
            }
        };
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut gauss = || {
            (0..4)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
                })
                .sum::<f32>()
                * 1.732
        };
        let mut img = Rgb32f::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let c = clean(x, y);
                // shot + read noise per channel
                img.data[y * w + x] = c.map(|v| v + (0.00002 + 0.004 * v.max(0.0)).sqrt() * 0.6 * gauss());
            }
        }
        let chroma = |c: [f32; 3]| {
            let y = y_of(c);
            c.map(|v| v - y)
        };
        let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        for nr in [25.0, 100.0] {
            let mut s = DevelopSettings::default();
            s.detail.nr_color = nr;
            let mut o = img.clone();
            denoise(&mut o, &s, 1.0, None);
            let px = |x: usize, y: usize| o.data[y * w + x];
            // edge: chroma error 1..8 px either side, relative to the step between the two colours
            let step = dist(chroma(red), chroma(green));
            let mut e = 0.0;
            let mut n = 0.0;
            for y in 40..216 {
                for d in 1..=8 {
                    for x in [384 - d, 383 + d] {
                        e += dist(chroma(px(x, y)), chroma(clean(x, y)));
                        n += 1.0;
                    }
                }
            }
            // line: share of the line's chroma kept (mean over its rows)
            let (mut kept, mut m) = (0.0, 0.0);
            for y in 300..480 {
                for x in 190..192 {
                    let c = chroma(px(x, y));
                    let r = chroma(red);
                    let rg = chroma(grey);
                    let full = [0, 1, 2].map(|i| r[i] - rg[i]);
                    let got = [0, 1, 2].map(|i| c[i] - rg[i]);
                    kept += (got[0] * full[0] + got[1] * full[1] + got[2] * full[2]) / (full[0] * full[0] + full[1] * full[1] + full[2] * full[2]);
                    m += 1.0;
                }
            }
            // saturated blue near black: chroma of the patch's mean kept
            let mut acc = [0.0f32; 3];
            let mut cnt = 0.0;
            for y in 300..480 {
                for x in 420..540 {
                    acc = [0, 1, 2].map(|i| acc[i] + px(x, y)[i]);
                    cnt += 1.0;
                }
            }
            let mean = acc.map(|v| v / cnt);
            let sat_kept = dist(chroma(mean), [0.0; 3]) / dist(chroma(blue_dark), [0.0; 3]);
            // grey: residual chroma noise
            let mut sd = 0.0;
            let mut k = 0.0;
            for y in 270..370 {
                for x in 600..760 {
                    sd += dist(chroma(px(x, y)), [0.0; 3]).powi(2);
                    k += 1.0;
                }
            }
            let mut sd0 = 0.0;
            for y in 270..370 {
                for x in 600..760 {
                    sd0 += dist(chroma(img.data[y * w + x]), [0.0; 3]).powi(2);
                }
            }
            // leaf next to black: chroma kept in the leaf's 12 px next to the black (mean over rows)
            let (mut lk, mut ln) = (0.0f32, 0.0f32);
            for y in 400..500 {
                for x in 660..672 {
                    let c = chroma(px(x, y));
                    let r = chroma(leaf);
                    lk += (c[0] * r[0] + c[1] * r[1] + c[2] * r[2]) / (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]);
                    ln += 1.0;
                }
            }
            println!("NR {nr}: leaf next to black keeps {:.0} % of its chroma (12 px next to the black)", 100.0 * lk / ln);
            println!(
                "NR {nr}: edge error {:.1} % of the step | red line kept {:.0} % | dark blue chroma kept {:.0} % | grey chroma noise {:.5} → {:.5}",
                100.0 * e / n / step,
                100.0 * kept / m,
                100.0 * sat_kept,
                (sd0 / k).sqrt(),
                (sd / k).sqrt()
            );
        }
    }

    #[test]
    fn nrc_noise_fit_is_non_negative() {
        // a falling cloud would give a negative slope: the constant fits instead
        let (a, b) = fit_nonneg_line(&[(0.0, 4.0, 1.0), (1.0, 3.0, 1.0), (2.0, 2.0, 1.0)]);
        assert!(a == 0.0 && (b - 3.0).abs() < 1e-9);
        // a line through a negative intercept: refitted through the origin
        let (a, b) = fit_nonneg_line(&[(1.0, 1.0, 1.0), (2.0, 3.0, 1.0), (3.0, 5.0, 1.0)]);
        assert!(a > 0.0 && b == 0.0);
        // an exact shot + read noise line is found as it is
        let (a, b) = fit_nonneg_line(&[(0.0, 2.0, 1.0), (1.0, 5.0, 1.0), (2.0, 8.0, 1.0)]);
        assert!((a - 3.0).abs() < 1e-9 && (b - 2.0).abs() < 1e-9);
    }
    use super::*;
    use craft_color_transform::cct::xy_to_uv;

    #[test]
    fn guided_preserves_step_edges_and_flattens_texture() {
        let p = Plane::from_fn(80, 20, |x, y| if x < 40 { -2.0 } else { 2.0 } + if (x + y) % 2 == 0 { 0.05 } else { -0.05 });
        let q = guided(&p, 4.0, 0.3);
        // edge kept
        assert!(q.get(35, 10) < -1.5 && q.get(45, 10) > 1.5);
        // checkerboard texture removed away from the edge
        assert!((q.get(10, 10) - q.get(11, 10)).abs() < 0.02);
    }

    #[test]
    fn wb_as_shot_is_identity_and_warming_adds_red() {
        let s = DevelopSettings::default();
        let info = WbSource::default();
        let mut img = Rgb32f::filled(4, 4, [0.3, 0.3, 0.3]);
        scene_linear_pre(&mut img, &info, &s);
        assert!((img.get(0, 0)[0] - 0.3).abs() < 1e-6);
        let mut warm = s.clone();
        warm.wb.mode = WbMode::Custom;
        warm.wb.temp = 9000.0;
        let mut img2 = Rgb32f::filled(4, 4, [0.3, 0.3, 0.3]);
        scene_linear_pre(&mut img2, &info, &warm);
        let c = img2.get(0, 0);
        assert!(c[0] > c[2], "{c:?}");
        // luminance preserved
        assert!((luminance_2020(c) - 0.3).abs() < 0.01);
    }

    #[test]
    fn process_v10_uses_checked_wb() {
        let mut s = DevelopSettings::default();
        assert!(!checked_wb(&s));
        s.process = craft_develop_model::ProcessVersion(10);
        assert!(checked_wb(&s));
    }

    #[test]
    fn a_hidden_or_empty_mask_does_not_request_the_tone_base() {
        use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
        use craft_geom::Point;
        let shape = MaskShape::Linear { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0) };
        let component = MaskComponent { name: None, op: MaskOp::Add, invert: false, shape };
        let mut s = DevelopSettings::default();
        s.masks.push(Mask {
            visible: false,
            components: vec![component],
            adjust: craft_develop_model::LocalAdjustments { highlights: 40.0, ..Default::default() },
            ..Default::default()
        });
        assert!(plane_sigmas(&s, 2000.0, 2000.0, Quality::Full).base.is_none());
        s.masks[0].visible = true;
        s.masks[0].components.clear();
        assert!(plane_sigmas(&s, 2000.0, 2000.0, Quality::Full).base.is_none());
        s.masks[0].components.push(MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Linear { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0) },
        });
        assert!(plane_sigmas(&s, 2000.0, 2000.0, Quality::Full).base.is_some());
    }

    #[test]
    fn exposure_doubles() {
        let mut s = DevelopSettings::default();
        s.light.exposure = 1.0;
        let mut img = Rgb32f::filled(2, 2, [0.1, 0.2, 0.3]);
        scene_linear_pre(&mut img, &WbSource::default(), &s);
        assert!((img.get(1, 1)[2] - 0.6).abs() < 1e-5);
    }

    /// [`kelvin_to_legacy`] inverts [`legacy_to_kelvin`] over the whole relative slider range and back: the
    /// round trip returns the start, in both directions, to the precision of the Kelvin / tint parametrisation.
    #[test]
    fn kelvin_to_legacy_inverts_legacy_to_kelvin() {
        for as_shot in [(4986.0, -2.0), (6500.0, 0.0), (3200.0, 14.0), (9500.0, -12.0)] {
            // the relative slider's whole reach: temp 0..100 (2000–13550 K around 6500), tint −150..150
            for pct in [0.0, 12.5, 25.0, 50.0, 75.0, 100.0] {
                for tint in [-150.0, -40.0, 0.0, 33.0, 150.0] {
                    let (lt, ltint) = controls_relative(pct, tint);
                    let (kt, ktint) = legacy_to_kelvin(lt, ltint, as_shot);
                    // past the Kelvin parametrisation's own reach (its search runs 1000–50000 K) the forward
                    // conversion has already clamped, so the pair is not expected to come back
                    if !(1050.0..=49_000.0).contains(&kt) {
                        continue;
                    }
                    let back = kelvin_to_legacy(kt, ktint, as_shot).unwrap();
                    let (x1, y1) = xy_to_uv(temp_tint_to_xy(lt, ltint));
                    let (x2, y2) = xy_to_uv(temp_tint_to_xy(back.0, back.1));
                    // the pair may differ by more than the slider's own step at the extreme corners (the Kelvin
                    // parametrisation is read back through a search), but never by much, and the white it renders
                    // is the same
                    assert!(
                        (back.0 - lt).abs() <= lt.abs() * 0.01 && (back.1 - ltint).abs() < 0.5,
                        "as shot {as_shot:?}: relative ({lt}, {ltint}) -> kelvin ({kt}, {ktint}) -> {back:?}"
                    );
                    assert!((x1 - x2).hypot(y1 - y2) < 1e-3, "as shot {as_shot:?}: relative ({lt}, {ltint}) -> kelvin ({kt}, {ktint}) -> {back:?}");
                }
            }
            // the other direction: every white the relative scale can hold maps back to a pair that renders the same
            for kt in [as_shot.0, 2500.0, 4200.0, 6500.0, 12000.0] {
                for ktint in [-60.0, 0.0, 45.0] {
                    let Some((lt, ltint)) = kelvin_to_legacy(kt, ktint, as_shot) else { continue };
                    // outside the slider's reach the relative scale has no value for this white: the forward
                    // conversion would clamp, so the round trip is not expected to close there
                    if !(2000.0..=13550.0).contains(&lt) || ltint.abs() > 150.0 {
                        continue;
                    }
                    let (kt2, ktint2) = legacy_to_kelvin(lt, ltint, as_shot);
                    let (x1, y1) = xy_to_uv(temp_tint_to_xy(kt, ktint));
                    let (x2, y2) = xy_to_uv(temp_tint_to_xy(kt2, ktint2));
                    assert!(
                        (x1 - x2).hypot(y1 - y2) < 1e-3,
                        "as shot {as_shot:?}: kelvin ({kt}, {ktint}) -> relative ({lt}, {ltint}) -> ({kt2}, {ktint2})"
                    );
                }
            }
            // the source's own white is the relative scale's as-shot reference, exactly
            assert_eq!(kelvin_to_legacy(as_shot.0, as_shot.1, as_shot), Some((6500.0, 0.0)));
        }
        // the exact inverse of the special case inside legacy_to_kelvin
        assert_eq!(kelvin_to_legacy(4986.0, -2.0, (4986.0, -2.0)), Some((6500.0, 0.0)));
        assert_eq!(legacy_to_kelvin(6500.0, 0.0, (4986.0, -2.0)), (4986.0, -2.0));
        // hostile numbers answer None, never a panic or a plausible lie
        assert_eq!(kelvin_to_legacy(f64::NAN, 0.0, (5000.0, 0.0)), None);
        assert_eq!(kelvin_to_legacy(5000.0, f64::INFINITY, (5000.0, 0.0)), None);
        assert_eq!(kelvin_to_legacy(5000.0, 0.0, (f64::NAN, 0.0)), None);
    }

    /// The relative scale's `(temp, tint)` for a slider temperature at `pct` % (2000–7000 K fill the left half) and tint.
    fn controls_relative(pct: f64, tint: f64) -> (f64, f64) {
        let t = if pct <= 50.0 { 2000.0 + pct * 100.0 } else { 7000.0 + (pct - 50.0) * 131.0 };
        (t, tint)
    }
}

#[cfg(test)]
mod fast_tests {
    use super::*;

    /// [`detail_free`] enlarges the coarse median as it clamps; the result is the plain
    /// composition of `resize` and [`median_hv`], bit for bit.
    #[test]
    fn detail_free_matches_its_reference() {
        use crate::raster::resample::{Filter, resize};
        for (w, h) in [(1, 1), (2, 1), (3, 3), (4, 7), (61, 47), (200, 31)] {
            let p = Plane::from_fn(w, h, |x, y| ((x * 37 + y * 101) % 23) as f32 * 0.4 - 4.0 + if (x / 5 + y / 7) % 2 == 0 { 2.0 } else { 0.0 });
            for r in [1, 2, 5, 10] {
                let fine = median_hv(&p, r);
                let (lw, lh) = (w.div_ceil(BASE_COARSE_STEP), h.div_ceil(BASE_COARSE_STEP));
                let coarse = resize(&median_hv(&resize(&p, lw, lh, Filter::Box), r), w, h, Filter::Bilinear);
                let want: Vec<u32> = (0..w * h).map(|i| clamp_between(p.data[i], fine.data[i], coarse.data[i]).to_bits()).collect();
                let got: Vec<u32> = detail_free(&p, r).data.iter().map(|v| v.to_bits()).collect();
                assert_eq!(got, want, "{w}×{h} r={r}");
            }
        }
    }

    /// CPU highlights/shadows base timings on the #3460 frame size:
    /// `cargo test --release -p craft-develop bench_tone_base_cpu -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_tone_base_cpu() {
        let (w, h) = (6960, 4640);
        let r = base_detail_radius(w as f64);
        let p = Plane::from_fn(w, h, |x, y| ((x * 7 + y * 13) % 1000) as f32 / 250.0 - 6.0 + if (x / 40 + y / 40) % 2 == 0 { 1.0 } else { 0.0 });
        let sigma = 0.015 * w as f32;
        let time = |name: &str, f: &dyn Fn() -> Plane| {
            let mut ts: Vec<f64> = (0..5)
                .map(|_| {
                    let t = std::time::Instant::now();
                    std::hint::black_box(f());
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            ts.sort_by(f64::total_cmp);
            eprintln!("{name:<20} min {:.1} ms  median {:.1} ms", ts[0], ts[2]);
        };
        time("median_hv", &|| median_hv(&p, r));
        time("detail_free", &|| detail_free(&p, r));
        time("guided_fast", &|| guided_fast(&p, sigma, BASE_EPS));
        time("tone_base", &|| tone_base(&p, sigma, r, true));
    }

    /// The previous fast path, including its two materialized coefficient resizes.
    fn resized_reference(p: &Plane, sigma: f32, eps: f32) -> Plane {
        use crate::raster::resample::{Filter, resize};
        let s = guided_fast_step(sigma);
        if s <= 1 {
            return guided(p, sigma, eps);
        }
        let lo = resize(p, p.width.div_ceil(s).max(1), p.height.div_ceil(s).max(1), Filter::Box);
        let (a, b) = guided_coeffs(&lo, sigma / s as f32, eps);
        let (a, b) = (resize(&a, p.width, p.height, Filter::Bilinear), resize(&b, p.width, p.height, Filter::Bilinear));
        Plane::from_fn(p.width, p.height, |x, y| a.get(x, y) * p.get(x, y) + b.get(x, y))
    }

    #[test]
    fn sampled_coefficients_match_materialized_resizes() {
        for (w, h) in [(0, 0), (1, 1), (1, 19), (23, 1), (2, 3), (17, 13), (64, 48), (161, 119)] {
            let planes = [
                Plane::filled(w, h, -3.25),
                Plane::from_fn(w, h, |x, y| ((x * 7919 + y * 104729) % 97) as f32 / 7.0 - 8.0),
                Plane::from_fn(w, h, |x, y| {
                    if x == w / 2 && y == h / 2 {
                        9.0
                    } else if x < w / 2 {
                        -5.0
                    } else {
                        2.0
                    }
                }),
            ];
            // Step 1 fallback, every small step, Draft's cap, and the maximum step (16).
            for sigma in [0.0, 2.99, 5.99, 6.0, 9.0, 12.0, 24.0, 48.0, 200.0] {
                for eps in [BASE_EPS, CLARITY_EPS, 0.0001] {
                    for p in &planes {
                        let expected = resized_reference(p, sigma, eps);
                        let actual = guided_fast(p, sigma, eps);
                        assert_eq!((actual.width, actual.height), (w, h));
                        assert_eq!(actual.data, expected.data, "{w}x{h}, sigma {sigma}, eps {eps}");
                    }
                }
            }
        }
    }

    #[test]
    fn sampled_tone_base_keeps_the_detail_free_plane_before_the_guided_filter() {
        let p = Plane::from_fn(73, 51, |x, y| if x % 9 == 0 { 3.0 } else { -4.0 + ((x + y) % 7) as f32 * 0.03 });
        for sigma in [6.0, 24.0, 120.0] {
            for detail in [0, 1, 3, BASE_DETAIL_MAX] {
                let free = detail_free(&p, detail);
                assert_eq!(tone_base(&p, sigma, detail, true).data, resized_reference(&free, sigma, BASE_EPS).data, "{sigma}, {detail}");
            }
        }
    }

    #[test]
    fn fast_guided_close_to_reference() {
        let p = Plane::from_fn(160, 120, |x, y| ((x as f32 * 0.13).sin() + (y as f32 * 0.07).cos()) * 2.0 + if x > 80 { 1.5 } else { 0.0 });
        let a = guided(&p, 12.0, 0.3);
        let b = guided_fast(&p, 12.0, 0.3);
        let err: f32 = a.data.iter().zip(&b.data).map(|(u, v)| (u - v).abs()).sum::<f32>() / a.len() as f32;
        assert!(err < 0.08, "{err}");
    }
}

#[cfg(test)]
mod nr_tests {
    use super::*;

    #[test]
    fn luminance_nr_reduces_noise_keeps_mean() {
        let mut img = Rgb32f::from_fn(64, 64, |x, y| {
            let n = (((x * 7919 + y * 104729) % 97) as f32 / 97.0 - 0.5) * 0.06;
            [0.2 + n; 3]
        });
        let var = |im: &Rgb32f| {
            let m = im.data.iter().map(|p| p[1]).sum::<f32>() / im.len() as f32;
            (im.data.iter().map(|p| (p[1] - m).powi(2)).sum::<f32>() / im.len() as f32, m)
        };
        let (v0, m0) = var(&img);
        let mut s = DevelopSettings::default();
        s.detail.nr_luminance = 80.0;
        denoise(&mut img, &s, 1.0, None);
        let (v1, m1) = var(&img);
        assert!(v1 < v0 * 0.5, "{v0} -> {v1}");
        assert!((m1 - m0).abs() < 0.01);
    }
}
