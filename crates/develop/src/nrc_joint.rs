//! Linear colour NR with a fixed, denoised joint guide. Strength controls the spatial scale;
//! noise-normalized guide distances control which neighbours can exchange colour. Both chroma
//! components use the same weights. Symmetric fluxes preserve the grid's channel sums, including
//! at its boundaries; no nonnegative clamp or square-root bias correction is involved.

use crate::colorops::luminance_2020;
use crate::local::{NrcNoise, NrcParams, fit_nonneg_line};
use crate::raster::{Rgb32f, blur::gaussian, par_rows as for_rows, resample::Filter};

/// A pixel sub-rectangle of an image (`x`, `y`, `w`, `h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelWindow {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct JointNoise {
    pub model: NrcNoise,
    /// Do not extrapolate beyond bands that contain usable noise measurements.
    pub ymax: [f32; 3],
    pub tolerance: f32,
}

impl JointNoise {
    pub fn var(&self, i: usize, y: f32) -> f32 {
        self.model.var(i, y.min(self.ymax[i]))
    }
}

fn yuv(c: [f32; 3]) -> [f32; 3] {
    let y = luminance_2020(c);
    [y, c[0] - y, c[2] - y]
}

/// Noise of Y, R-Y and B-Y measured from small high-pass residuals, not the colour variance
/// of a wide smoothing window. The median-square calibration is for Gaussian residuals.
/// Luminance bands with predominantly texture may only lower their weight in the fit: they
/// cannot force a shadow scene's extrapolated highlight noise arbitrarily high.
fn noise_model(img: &Rgb32f) -> JointNoise {
    if img.width < 3 || img.height < 3 {
        return JointNoise { model: NrcNoise { alpha: [0.0; 3], beta: [1e-14; 3] }, ymax: [0.0; 3], tolerance: 1.0 };
    }
    let mut samples = Vec::new();
    push_samples(img, noise_sample_stride(img.len()), &mut samples);
    noise_from_samples(samples)
}

/// The noise samples of `img` (Y, R-Y, B-Y) every `stride` pixels, in pixel order (see [`noise_model`]).
fn push_samples(img: &Rgb32f, stride: usize, samples: &mut Vec<NoiseSample>) {
    if img.width < 3 || img.height < 3 {
        return;
    }
    for index in (0..img.len()).step_by(stride) {
        let (x, y) = (index % img.width, index / img.width);
        if x == 0 || y == 0 || x + 1 == img.width || y + 1 == img.height {
            continue;
        }
        let Some(c) = img.data.get(index) else { continue };
        let mut residual = [[0.0f32; 3]; 3];
        let mut brightness = 0.0;
        let mut valid = true;
        for (scale, delta) in [1usize, 2, 4].into_iter().enumerate() {
            // Tiny images use the nearest supported scale rather than extending an edge
            // into the noise samples.
            let delta = delta.min(x).min(y).min(img.width - x - 1).min(img.height - y - 1);
            let neighbours = [index - delta, index + delta, index - delta * img.width, index + delta * img.width];
            let mut m = [0.0f32; 3];
            for n in neighbours {
                if let Some(p) = img.data.get(n) {
                    m = std::array::from_fn(|i| m[i] + 0.25 * p[i]);
                }
            }
            residual[scale] = std::array::from_fn(|i| (c[i] - m[i]).powi(2));
            brightness += m[0] / 3.0;
            valid &= c.iter().chain(m.iter()).all(|v| v.is_finite());
        }
        if valid {
            samples.push((brightness.max(0.0), residual));
        }
    }
}

/// Side of the tiles a frame's noise is measured on (px).
const TILE: usize = 128;
/// Tiles per row and column: many small tiles follow a mostly dark frame's few bright parts better
/// than a few large ones.
const GRID: usize = 12;

/// The windows of a `full_w × full_h` frame its noise is measured on: the whole frame up to the
/// area of `GRID²` tiles, else `GRID × GRID` tiles spread evenly over it. A windowed render sees
/// only part of the frame; measured on these, every window of it gets the same noise model.
pub fn tiles(full_w: usize, full_h: usize) -> Vec<PixelWindow> {
    if full_w * full_h <= GRID * GRID * TILE * TILE || full_w < 2 * TILE || full_h < 2 * TILE {
        return vec![PixelWindow { x: 0, y: 0, w: full_w, h: full_h }];
    }
    let at = |n: usize, k: usize| ((n - TILE) as f64 * (k as f64 + 0.5) / GRID as f64).round() as usize;
    (0..GRID * GRID).map(|k| PixelWindow { x: at(full_w, k % GRID), y: at(full_h, k / GRID), w: TILE, h: TILE }).collect()
}

/// The noise model of `tiles` of a frame (linear RGB at output scale, see [`tiles`]): the same
/// measurement as for a whole image, with the samples of all tiles together.
pub fn noise_of_tiles(tiles: &[Rgb32f]) -> JointNoise {
    let transformed: Vec<Rgb32f> = tiles.iter().map(|t| t.map(yuv)).collect();
    let stride = noise_sample_stride(transformed.iter().map(Rgb32f::len).sum());
    let mut samples = Vec::new();
    for t in &transformed {
        push_samples(t, stride, &mut samples);
    }
    if samples.is_empty() {
        return JointNoise { model: NrcNoise { alpha: [0.0; 3], beta: [1e-14; 3] }, ymax: [0.0; 3], tolerance: 1.0 };
    }
    noise_from_samples(samples)
}

/// Distance between the noise samples of a `len`-pixel plane (CPU and GPU take the same pixels).
pub fn noise_sample_stride(len: usize) -> usize {
    len.div_ceil(crate::local::NRC_SAMPLES).max(1) | 1
}

/// One noise sample: the mean brightness of its neighbourhoods and the squared high-pass residual of
/// Y, R−Y, B−Y at spacings 1, 2 and 4.
pub type NoiseSample = (f32, [[f32; 3]; 3]);

/// The fitted noise model from the samples, in pixel order (see `noise_model`).
pub fn noise_from_samples(samples: Vec<NoiseSample>) -> JointNoise {
    // Moving a slider re-renders the same image: the last fit is reused for identical samples (64-bit
    // hash of their bits and count; the samples alone determine the fit).
    static LAST: std::sync::Mutex<Option<(u64, JointNoise)>> = std::sync::Mutex::new(None);
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ samples.len() as u64;
    for s in &samples {
        for v in std::iter::once(s.0).chain(s.1.iter().flatten().copied()) {
            hash = (hash ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
        }
    }
    if let Some((h, noise)) = *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        && h == hash
    {
        return noise;
    }
    let noise = fit_noise(samples);
    *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((hash, noise));
    noise
}

fn fit_noise(samples: Vec<NoiseSample>) -> JointNoise {
    // brightness order, ties in pixel order (as a stable sort): sort compact keys (the bits `total_cmp`
    // compares by, then the index), then gather
    let mut keys: Vec<(i32, u32)> = samples
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let bits = s.0.to_bits() as i32;
            (bits ^ (((bits >> 31) as u32) >> 1) as i32, k as u32)
        })
        .collect();
    keys.sort_unstable();
    let samples: Vec<NoiseSample> = keys.iter().filter_map(|&(_, k)| samples.get(k as usize).copied()).collect();
    const BANDS: usize = 16;
    let per = samples.len().div_ceil(BANDS).max(32);
    let mut pts: [Vec<(f64, f64, f64)>; 3] = std::array::from_fn(|_| Vec::new());
    let mut r: Vec<f32> = Vec::with_capacity(per);
    for band in samples.chunks(per).filter(|b| b.len() >= 32) {
        let y = band.iter().map(|p| p.0 as f64).sum::<f64>() / band.len() as f64;
        for (i, pts) in pts.iter_mut().enumerate() {
            let mut variance = 1e-14f64;
            for scale in 0..3 {
                r.clear();
                r.extend(band.iter().map(|p| p.1[scale][i]));
                let middle = r.len() / 2;
                let (_, median, _) = r.select_nth_unstable_by(middle, f32::total_cmp);
                // Each kernel has norm squared 1.25. Larger spacings recover noise
                // hidden by the positive spatial correlations of demosaiced RGB.
                variance = variance.max(*median as f64 / (0.454_936_4 * 1.25));
            }
            pts.push((y, variance, band.len() as f64));
        }
    }
    let mut model = NrcNoise { alpha: [0.0; 3], beta: [1e-14; 3] };
    let mut ymax = [0.0; 3];
    for (i, pts) in pts.iter().enumerate() {
        if pts.is_empty() {
            continue;
        }
        let (a, b, top) = robust_line(pts);
        ymax[i] = top;
        model.alpha[i] = a as f32;
        model.beta[i] = (b as f32).max(1e-14);
    }
    JointNoise { model, ymax, tolerance: 1.0 }
}

/// The noise line `v = a·y + b` through the bands `(y, variance, weight)` and the brightest band it
/// rests on. Seeded from the darker half, then all bands with a bounded influence for unexpectedly
/// high residuals (foliage etc.); refit rather than clamp the intercept.
pub(crate) fn robust_line(pts: &[(f64, f64, f64)]) -> (f64, f64, f32) {
    let (mut a, mut b) = fit_nonneg_line(&pts[..pts.len().div_ceil(2)]);
    let mut ymax = 0.0;
    for _ in 0..4 {
        // A band's weight fades to 0 between 4 and 5 times the fitted noise instead of dropping at
        // 4: a band at the bound must not flip in or out with the last bits of the samples (the
        // GPU's differ from the CPU's there), which moved the fit by a few percent.
        let weighted: Vec<_> = pts
            .iter()
            .filter_map(|&(y, v, w)| {
                let excess = v / (a * y + b).max(1e-14);
                let fade = (5.0 - excess).clamp(0.0, 1.0);
                (fade > 0.0).then_some((y, v, w * (2.0 / excess.max(2.0)).powi(4) * fade))
            })
            .collect();
        if !weighted.is_empty() {
            (a, b) = fit_nonneg_line(&weighted);
            ymax = weighted.iter().map(|p| p.0 as f32).fold(0.0, f32::max);
        }
    }
    (a, b, ymax)
}

/// Squared distance beyond the expected noise in a difference of two guide samples. A small
/// relative luminance allowance lets a smooth surface exchange colour without crossing a
/// significant luminance or isoluminant colour boundary. The guide is fixed across all scales.
/// (The filter runs `distance_run`, the same operations over planes; this per-pair form is its test reference.)
#[cfg(test)]
fn distance(a: [f32; 3], b: [f32; 3], noise: &JointNoise) -> f32 {
    let y = 0.5 * (a[0] + b[0]);
    let allowance = (0.025 * y.max(0.0)).powi(2);
    let terms: [f32; 3] = std::array::from_fn(|i| {
        let v = noise.var(i, y);
        let excess = ((a[i] - b[i]).powi(2) - v).max(0.0);
        // The 3x3 pilot blur reduces uncorrelated noise to 1/9. Keep a conservative
        // allowance for correlations left by demosaicing and grid resampling.
        // (A difference within the noise adds exactly 0: no division needed.)
        if excess == 0.0 { 0.0 } else { excess / (3.0 * noise.tolerance * v + allowance) }
    });
    terms.into_iter().sum::<f32>()
}

/// The guide as three planes (Y, R−Y, B−Y): `distance_run` over them vectorises.
struct Planes {
    c: [Vec<f32>; 3],
    w: usize,
}

impl Planes {
    /// `len` pixels of row `y` from column `from`.
    fn run(&self, y: usize, from: usize, len: usize) -> Option<[&[f32]; 3]> {
        let start = y.checked_mul(self.w)?.checked_add(from)?;
        let end = start.checked_add(len)?;
        let [a, b, c] = &self.c;
        Some([a.get(start..end)?, b.get(start..end)?, c.get(start..end)?])
    }
}

/// `distance` of each pair of guide pixels `(a[k], b[k])` into `out[k]`, the same operations in the same
/// order (bit for bit), without branches so the compiler can work on several pixels at once.
fn distance_run(a: [&[f32]; 3], b: [&[f32]; 3], noise: &JointNoise, out: &mut [f32]) {
    let ([ay, au, av], [by, bu, bv]) = (a, b);
    let (alpha, beta, ymax, tolerance) = (noise.model.alpha, noise.model.beta, noise.ymax, noise.tolerance);
    for ((((((o, &ay), &au), &av), &by), &bu), &bv) in out.iter_mut().zip(ay).zip(au).zip(av).zip(by).zip(bu).zip(bv) {
        let y = 0.5 * (ay + by);
        let allowance = (0.025 * y.max(0.0)).powi(2);
        // `JointNoise::var`; an excess of 0 divides to exactly 0, as the branch in `distance` gives
        let term = |i: usize, d: f32| {
            let v = (alpha[i] * y.min(ymax[i]).max(0.0) + beta[i]).max(1e-14);
            let excess = (d.powi(2) - v).max(0.0);
            excess / (3.0 * tolerance * v + allowance)
        };
        *o = term(0, ay - by) + term(1, au - bu) + term(2, av - bv);
    }
}

/// The exchange weight `exp(−max(distance, path))`; exactly 1 when both are 0 (no `exp` needed).
#[inline]
fn weight(distance: f32, path: f32) -> f32 {
    let m = distance.max(path);
    if m == 0.0 { 1.0 } else { (-m).exp() }
}

/// `weight(distance, path)` with the path given as exp(−path): min(exp(−distance), that), and no `exp`
/// for a distance of 0.
#[inline]
fn path_weight(distance: f32, path: f32) -> f32 {
    if distance == 0.0 { path } else { (-distance).exp().min(path) }
}

/// Denoise the pilot along a clearly smoother axis at strong edges. An isotropic blur
/// dilutes a two-pixel isoluminant line before the edge decision even sees it. Weak or
/// ambiguous structure uses the isotropic pilot, so noise alone does not protect itself.
/// Written straight into planes (see `Planes`), one row at a time without branches per pixel.
fn pilot(original: &Rgb32f, noise: &JointNoise) -> Planes {
    let blurred = gaussian(original, 0.8);
    let (w, h) = (original.width, original.height);
    let mut planes = Planes { c: std::array::from_fn(|_| vec![0.0; original.len()]), w };
    let [py, pu, pv] = &mut planes.c;
    let process_row = |out: &mut Vec<[f32; 3]>, y: usize, ry: &mut [f32], ru: &mut [f32], rv: &mut [f32]| {
        let Some(blur) = blurred.data.get(y * w..(y + 1) * w) else { return };
        out.clear();
        out.extend_from_slice(blur);
        if y > 0
            && y + 1 < h
            && let (Some(up), Some(mid), Some(down)) =
                (original.data.get((y - 1) * w..y * w), original.data.get(y * w..(y + 1) * w), original.data.get((y + 1) * w..(y + 2) * w))
        {
            // columns 1 .. w − 1: the pixel, its left/right (axis 0) and upper/lower (axis 1) neighbours
            for (((o, win), &u), &d) in out.iter_mut().skip(1).zip(mid.windows(3)).zip(up.iter().skip(1)).zip(down.iter().skip(1)) {
                let &[l, p, r] = win else { continue };
                let var: [f32; 3] = std::array::from_fn(|i| 9.0 * noise.var(i, p[0]));
                let axis = |a: [f32; 3], b: [f32; 3]| {
                    let m: [f32; 3] = std::array::from_fn(|i| (a[i] + p[i] + b[i]) / 3.0);
                    let t = |i: usize| ((a[i] - m[i]).powi(2) + (p[i] - m[i]).powi(2) + (b[i] - m[i]).powi(2)) / var[i];
                    (m, t(0) + t(1) + t(2))
                };
                let (mh, vh) = axis(l, r);
                let (mv, vvar) = axis(u, d);
                *o = pilot_pick(*o, mh, vh, mv, vvar);
            }
        }
        for (((c, y), u), v) in out.iter().zip(ry.iter_mut()).zip(ru.iter_mut()).zip(rv.iter_mut()) {
            (*y, *u, *v) = (c[0], c[1], c[2]);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        py.par_chunks_mut(w.max(1))
            .zip(pu.par_chunks_mut(w.max(1)))
            .zip(pv.par_chunks_mut(w.max(1)))
            .enumerate()
            .for_each_init(Vec::new, |out, (y, ((ry, ru), rv))| process_row(out, y, ry, ru, rv));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut out = Vec::new();
        for (y, ((ry, ru), rv)) in py.chunks_mut(w.max(1)).zip(pu.chunks_mut(w.max(1))).zip(pv.chunks_mut(w.max(1))).enumerate() {
            process_row(&mut out, y, ry, ru, rv);
        }
    }
    planes
}

/// The pilot at one pixel from its isotropic blur `o`, the horizontal mean `mh` and noise-normalised
/// variance `vh`, and the vertical ones `mv`, `vvar`: the mean along the clearly smoother axis where
/// the other is more than 4 times as rough (and above 4), the blur where neither is. The choice fades
/// in between 0.8 and 1.25 times that bound instead of switching at it, so a pixel near the bound
/// takes nearly the same pilot when the noise model moves by a hair.
pub(crate) fn pilot_pick(o: [f32; 3], mh: [f32; 3], vh: f32, mv: [f32; 3], vvar: f32) -> [f32; 3] {
    let sv = smoothstep(0.8, 1.25, vh / 4.0f32.max(4.0 * vvar));
    let sh = smoothstep(0.8, 1.25, vvar / 4.0f32.max(4.0 * vh));
    if sv >= 1.0 {
        mv
    } else if sh >= 1.0 {
        mh
    } else {
        std::array::from_fn(|i| o[i] + (mv[i] - o[i]) * sv + (mh[i] - o[i]) * sh)
    }
}

/// The two chroma components `R − Y`, `B − Y`, or a pair of per-axis values (horizontal, vertical).
type Pair = [f32; 2];
type PairImage = crate::raster::Image<Pair>;

fn rgb2(y: f32, uv: Pair) -> [f32; 3] {
    let r = y + uv[0];
    let b = y + uv[1];
    [r, (y - 0.2627 * r - 0.0593 * b) / 0.6780, b]
}

/// One conservative horizontal/vertical diffusion step. Pair weights are symmetric; the
/// guide is never replaced by the filtered result. Long taps also account for the intervening
/// guide edges, so a wide pass cannot jump over a thin differently coloured object.
///
/// Per pixel: `out = p`, then the lower neighbour's exchange, then the upper one's, in that order
/// (the order the result depends on, bit for bit).
fn diffuse(src: &PairImage, dst: &mut PairImage, links: &PairImage, delta: usize, vertical: bool, rate: f32) {
    if vertical { diffuse_v(src, dst, links, delta, rate) } else { diffuse_h(src, dst, links, delta, rate) }
}

#[inline(always)]
fn exchange(out: &mut Pair, p: Pair, q: Pair, weight: f32) {
    out[0] += weight * (q[0] - p[0]);
    out[1] += weight * (q[1] - p[1]);
}

fn diffuse_h(src: &PairImage, dst: &mut PairImage, links: &PairImage, delta: usize, rate: f32) {
    let w = src.width;
    for_rows(&mut dst.data, w, |y, out| {
        let (Some(s), Some(l)) = (src.data.get(y * w..(y + 1) * w), links.data.get(y * w..(y + 1) * w)) else { return };
        for (x, o) in out.iter_mut().enumerate() {
            let p = s[x];
            *o = p;
            if x >= delta {
                exchange(o, p, s[x - delta], rate * l[x - delta][0]);
            }
            if x + delta < w {
                exchange(o, p, s[x + delta], rate * l[x][0]);
            }
        }
    });
}

fn diffuse_v(src: &PairImage, dst: &mut PairImage, links: &PairImage, delta: usize, rate: f32) {
    let (w, h) = (src.width, src.height);
    for_rows(&mut dst.data, w, |y, out| {
        let Some(s) = src.data.get(y * w..(y + 1) * w) else { return };
        out.copy_from_slice(s);
        if y >= delta
            && let (Some(up), Some(lu)) = (src.data.get((y - delta) * w..(y - delta + 1) * w), links.data.get((y - delta) * w..(y - delta + 1) * w))
        {
            for (((o, &p), &q), lk) in out.iter_mut().zip(s).zip(up).zip(lu) {
                exchange(o, p, q, rate * lk[1]);
            }
        }
        if y + delta < h
            && let (Some(down), Some(ld)) = (src.data.get((y + delta) * w..(y + delta + 1) * w), links.data.get(y * w..(y + 1) * w))
        {
            for (((o, &p), &q), lk) in out.iter_mut().zip(s).zip(down).zip(ld) {
                exchange(o, p, q, rate * lk[1]);
            }
        }
    });
}

/// The diffusion scales for a final spatial `sigma` (in grid pixels) on a `w × h` grid: each step's
/// neighbour distance and rate, in order. CPU and GPU run exactly these.
pub fn scale_plan(sigma: f32, w: usize, h: usize) -> Vec<(usize, f32)> {
    let mut plan = Vec::new();
    let mut remaining = sigma * sigma;
    let mut delta = 1usize;
    while remaining > 0.01 && delta <= w.max(h) {
        let rate = (remaining / (4.0 * (delta as f32).powi(2))).min(0.25);
        plan.push((delta, rate));
        remaining -= 4.0 * rate * (delta as f32).powi(2);
        delta = delta.saturating_mul(2);
    }
    plan
}

/// Share of the filtered colour at Color `amount` (0..=1): a ramp up to the default 25.
pub fn strength(amount: f32) -> f32 {
    (4.0 * amount).clamp(0.0, 1.0)
}

/// Edge tolerance at Color Detail `detail` (0..=1).
pub fn tolerance(detail: f32) -> f32 {
    (1.5 - detail).clamp(0.5, 1.5)
}

/// The speck brake at Color `amount` (0..=1): the chroma excess (in noise variances of the two chroma
/// components together) from which a pixel starts to count as a colour speck (fully at twice that), and
/// the largest share of its colour that is replaced. Weak at the default 25 (only extreme specks, at most
/// half), stronger up to 100. Below 25 the final blend (`strength`) fades it out with the rest.
pub fn speck_brake(amount: f32) -> SpeckBrake {
    let t = ((amount - 0.25) / 0.75).clamp(0.0, 1.0);
    SpeckBrake { threshold: 64.0 - 39.0 * t, share: 0.5 + 0.5 * t }
}

/// See [`speck_brake`]; `share` 0 turns the brake off.
#[derive(Clone, Copy, Debug)]
pub struct SpeckBrake {
    pub threshold: f32,
    pub share: f32,
}

impl SpeckBrake {
    pub const OFF: SpeckBrake = SpeckBrake { threshold: f32::INFINITY, share: 0.0 };
}

/// The median (mean of the middle two) of 8 values, by an optimal 19-comparator sorting network
/// (the GPU kernel uses the same one).
fn median8(v: [f32; 8]) -> f32 {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = v;
    let cmp = |x: &mut f32, y: &mut f32| {
        let (lo, hi) = (x.min(*y), x.max(*y));
        (*x, *y) = (lo, hi);
    };
    cmp(&mut a, &mut c);
    cmp(&mut b, &mut d);
    cmp(&mut e, &mut g);
    cmp(&mut f, &mut h);
    cmp(&mut a, &mut e);
    cmp(&mut b, &mut f);
    cmp(&mut c, &mut g);
    cmp(&mut d, &mut h);
    cmp(&mut a, &mut b);
    cmp(&mut c, &mut d);
    cmp(&mut e, &mut f);
    cmp(&mut g, &mut h);
    cmp(&mut c, &mut e);
    cmp(&mut d, &mut f);
    cmp(&mut b, &mut e);
    cmp(&mut d, &mut g);
    cmp(&mut b, &mut c);
    cmp(&mut d, &mut e);
    cmp(&mut f, &mut g);
    let _ = (a, b, c, f, g, h);
    0.5 * (d + e)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a pixel's chroma the speck brake replaces by the median of the ring around it, and that
/// median (Y, R−Y, B−Y), from the pixel `p`, the ring `ring` and the rest of the 5 × 5 border `rest`. A
/// speck stands out in colour far beyond the noise, but not in luminance (a star, a lit point or any real
/// small detail almost always has a luminance step), and it is compact: the colour shift does not reach
/// the border (a line or an edge carries it on). Demosaicing spreads high-ISO specks over 2 × 2 to
/// 3 × 3 pixels, which all lie inside. The noise is taken at the ring's mean luminance (a median of it
/// is only needed for the few pixels past the threshold).
fn speck_share(p: [f32; 3], ring: &[[f32; 3]; 8], rest: impl Iterator<Item = [f32; 3]>, noise: &JointNoise, brake: SpeckBrake) -> (f32, [f32; 3]) {
    let y = ring.iter().map(|q| q[0]).sum::<f32>() / 8.0;
    let (vu, vv) = (noise.var(1, y), noise.var(2, y));
    // The medians lie between the ring's least and largest value, so the farther of the two bounds the
    // excess from above (rounding keeps that order): most pixels are decided without the medians.
    let reach = |c: usize| {
        let (lo, hi) = ring.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), q| (lo.min(q[c]), hi.max(q[c])));
        (p[c] - lo).abs().max((p[c] - hi).abs())
    };
    let (bu, bv) = (reach(1), reach(2));
    if bu * bu / vu + bv * bv / vv <= brake.threshold {
        return (0.0, [0.0; 3]);
    }
    let m = [0.0, median8(ring.map(|q| q[1])), median8(ring.map(|q| q[2]))];
    let (du, dv) = (p[1] - m[1], p[2] - m[2]);
    let excess = du * du / vu + dv * dv / vv;
    if excess <= brake.threshold {
        return (0.0, m);
    }
    let m = [median8(ring.map(|q| q[0])), m[1], m[2]];
    // luminance: full brake up to half the chroma excess, none from 1.5 times it
    let dy = p[0] - m[0];
    let luminance = dy * dy / noise.var(0, y);
    let alone = ((1.5 * excess - luminance) / excess).clamp(0.0, 1.0);
    if alone <= 0.0 {
        return (0.0, m);
    }
    // compactness: the second largest share of the pixel's colour shift that a border pixel also has (a
    // line or an edge through the pixel crosses the border at least twice; one noisy border pixel that
    // happens to lean the same way does not protect a speck)
    let (_, second) = ring
        .iter()
        .copied()
        .chain(rest)
        .map(|q| ((q[1] - m[1]) * du / vu + (q[2] - m[2]) * dv / vv) / excess)
        .fold((0.0f32, 0.0f32), |(a, b), s| if s > a { (s, a) } else { (a, b.max(s)) });
    let compact = ((0.8 - second) / 0.3).clamp(0.0, 1.0);
    (brake.share * smoothstep(brake.threshold, 2.0 * brake.threshold, excess) * alone * compact, m)
}

/// Replace the chroma of compact colour specks by that of their surroundings (see `speck_share`), in the
/// transformed image (Y, R−Y, B−Y); luminance is never touched. Runs before the guide is built, so a speck
/// is neither kept by the edge-aware weights nor spread by them.
fn despeck(img: &mut Rgb32f, noise: &JointNoise, brake: SpeckBrake) {
    if brake.share <= 0.0 || img.width < 5 || img.height < 5 {
        return;
    }
    let (w, h) = (img.width, img.height);
    let source = &*img;
    let process_row = |y: usize| {
        let mut changed = Vec::new();
        let rows: [Option<&[[f32; 3]]>; 5] = std::array::from_fn(|k| source.data.get((y + k - 2) * w..(y + k - 1) * w));
        let [Some(r0), Some(r1), Some(r2), Some(r3), Some(r4)] = rows else { return changed };
        for (x, ((((w0, w1), w2), w3), w4)) in r0.windows(5).zip(r1.windows(5)).zip(r2.windows(5)).zip(r3.windows(5)).zip(r4.windows(5)).enumerate() {
            let (&[a0, a1, a2, a3, a4], &[b0, _, _, _, b4], &[c0, _, p, _, c4], &[d0, _, _, _, d4], &[e0, e1, e2, e3, e4]) = (w0, w1, w2, w3, w4)
            else {
                continue;
            };
            // the ring: corners and edge centres of the 5 × 5 square, far enough that a speck spread over
            // its direct neighbours by demosaicing does not reach them; the rest of the border: with the
            // ring, every thin line or edge through the pixel crosses at least one of the 16 at any angle
            let ring = [a0, a2, a4, c0, c4, e0, e2, e4];
            let rest = [a1, a3, b0, b4, d0, d4, e1, e3];
            let (k, m) = speck_share(p, &ring, rest.into_iter(), noise, brake);
            if k > 0.0 {
                changed.push((y * w + x + 2, [p[1] + k * (m[1] - p[1]), p[2] + k * (m[2] - p[2])]));
            }
        }
        changed
    };
    #[cfg(not(target_arch = "wasm32"))]
    let changes: Vec<Vec<(usize, [f32; 2])>> = {
        use rayon::prelude::*;
        (2..h - 2).into_par_iter().map(process_row).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let changes: Vec<Vec<(usize, [f32; 2])>> = (2..h - 2).map(process_row).collect();
    for (index, uv) in changes.into_iter().flatten() {
        if let Some(o) = img.data.get_mut(index) {
            (o[1], o[2]) = (uv[0], uv[1]);
        }
    }
}

/// `noise`: the frame's noise model (see [`noise_of_tiles`]); `None` measures it on `img`.
pub(crate) fn denoise(img: &mut Rgb32f, params: &NrcParams, detail: f32, amount: f32, noise: Option<&JointNoise>) {
    // Continuous ramp from zero; from the ordinary default Color=25 onward the spatial
    // scale controls the extra smoothing. Noise calibration and edge decisions stay fixed.
    denoise_with(img, params, 1, detail, strength(amount), speck_brake(amount), noise);
}

#[cfg(test)]
fn denoise_grid(img: &mut Rgb32f, params: &NrcParams, step: usize, detail: f32, strength: f32, brake: SpeckBrake) {
    denoise_with(img, params, step, detail, strength, brake, None);
}

fn denoise_with(img: &mut Rgb32f, params: &NrcParams, step: usize, detail: f32, strength: f32, brake: SpeckBrake, noise: Option<&JointNoise>) {
    if img.is_empty() || strength <= 0.0 {
        return;
    }
    // A maximum 2x grid keeps colour boundaries visible even at high strength. The old grid
    // grew to 16x with the radius, making the colour loss inseparable from slider strength.
    let step = if img.width.is_multiple_of(step) && img.height.is_multiple_of(step) { step } else { 1 };
    let (w, h) = (img.width.div_ceil(step), img.height.div_ceil(step));
    // Transform before resampling; negative linear differences are valid throughout.
    let mut original = Rgb32f::new(w, h);
    if step == 1 {
        // the generic loop below with one sample per pixel, same operations (0 + c, / 1)
        for_rows(&mut original.data, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                if let Some(&c) = img.data.get(y * w + x) {
                    let c = yuv(c);
                    *p = c.map(|v| (0.0f32 + v) / 1.0f32);
                }
            }
        });
    } else {
        for_rows(&mut original.data, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let mut count = 0.0f32;
                for sy in y * step..((y + 1) * step).min(img.height) {
                    for sx in x * step..((x + 1) * step).min(img.width) {
                        if let Some(&c) = img.data.get(sy * img.width + sx) {
                            let c = yuv(c);
                            *p = std::array::from_fn(|i| p[i] + c[i]);
                            count += 1.0;
                        }
                    }
                }
                *p = p.map(|v| v / count.max(1.0));
            }
        });
    }
    let mut noise = match noise {
        Some(n) => *n,
        None => noise_model(&original),
    };
    noise.tolerance = tolerance(detail);
    despeck(&mut original, &noise, brake);
    let planes = pilot(&original, &noise);
    // Working buffers carry the two chroma components only (Y is restored from the image at the end).
    let mut filtered: PairImage = original.map(|p| [p[1], p[2]]);
    // the full-resolution reconstruction reads Y from `img`: the transformed copy is no longer needed
    let original = (step != 1).then_some(original);
    let mut scratch = PairImage::new(w, h);
    let mut paths = PairImage::new(w, h);
    let mut links = PairImage::new(w, h);
    // Direct neighbours: the path over one step is the pair's own distance, and its weight is exp(−distance).
    // `paths` holds exp(−path), the largest distance along a path, exponentiated: exp(−max(a, b)) is
    // min(exp(−a), exp(−b)) bit for bit (exp is monotonic), so longer paths take minima and a weight with
    // a new distance of 0 needs no `exp`; a path of 0 (no pair, at the border) is 1.
    // the distances of row `y` to the pixels `offset` to the right and `offset` rows down
    let row_distances = |y: usize, offset: usize, dh: &mut Vec<f32>, dv: &mut Vec<f32>| {
        let n = w.saturating_sub(offset);
        dh.resize(w, 0.0);
        dv.resize(w, 0.0);
        if let (Some(a), Some(b), Some(out)) = (planes.run(y, 0, n), planes.run(y, offset, n), dh.get_mut(..n)) {
            distance_run(a, b, &noise, out);
        }
        if y + offset < h
            && let (Some(a), Some(b)) = (planes.run(y, 0, w), planes.run(y + offset, 0, w))
        {
            distance_run(a, b, &noise, dv);
        }
    };
    let init_row = |(dh, dv): &mut (Vec<f32>, Vec<f32>), y: usize, prow: &mut [[f32; 2]], lrow: &mut [[f32; 2]]| {
        row_distances(y, 1, dh, dv);
        for (x, (((out, link), &d0), &d1)) in prow.iter_mut().zip(lrow.iter_mut()).zip(dh.iter()).zip(dv.iter()).enumerate() {
            *link = [0.0; 2];
            *out = [1.0; 2];
            if x + 1 < w {
                link[0] = weight(d0, d0);
                out[0] = link[0];
            }
            if y + 1 < h {
                link[1] = weight(d1, d1);
                out[1] = link[1];
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        paths
            .data
            .par_chunks_mut(w)
            .zip(links.data.par_chunks_mut(w))
            .enumerate()
            .for_each_init(|| (Vec::new(), Vec::new()), |bufs, (y, (prow, lrow))| init_row(bufs, y, prow, lrow));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut bufs = (Vec::new(), Vec::new());
        for (y, (prow, lrow)) in paths.data.chunks_mut(w).zip(links.data.chunks_mut(w)).enumerate() {
            init_row(&mut bufs, y, prow, lrow);
        }
    }
    // Scale is independent of the edge decision. K has no role in deciding whether a leaf
    // is noise. Two dyadic sweeps provide broad smoothing with a fixed number of scales.
    let sigma = params.passes.last().map_or(0.0, |p| p.sigma / step as f32);
    let plan = scale_plan(sigma, w, h);
    for (k, &(delta, rate)) in plan.iter().enumerate() {
        for _ in 0..2 {
            diffuse(&filtered, &mut scratch, &links, delta, false, rate);
            std::mem::swap(&mut filtered, &mut scratch);
            diffuse(&filtered, &mut scratch, &links, delta, true, rate);
            std::mem::swap(&mut filtered, &mut scratch);
        }
        let Some(&(next, _)) = plan.get(k + 1) else { break };
        // In one pass: paths of length 2·delta (from the two adjacent delta-length paths, read from
        // `paths`, written to `scratch`) and the next scale's pair weights from them.
        let old = &paths;
        let step_row = |(dh, dv): &mut (Vec<f32>, Vec<f32>), y: usize, prow: &mut [[f32; 2]], lrow: &mut [[f32; 2]]| {
            row_distances(y, next, dh, dv);
            let (Some(here), below) = (old.data.get(y * w..(y + 1) * w), old.data.get((y + delta) * w..(y + delta + 1) * w)) else { return };
            for (x, (((out, link), &d0), &d1)) in prow.iter_mut().zip(lrow.iter_mut()).zip(dh.iter()).zip(dv.iter()).enumerate() {
                let Some(&p) = here.get(x) else { continue };
                *out = p;
                if x + delta < w
                    && let Some(q) = here.get(x + delta)
                {
                    out[0] = out[0].min(q[0]);
                }
                if y + delta < h
                    && let Some(q) = below.and_then(|b| b.get(x))
                {
                    out[1] = out[1].min(q[1]);
                }
                *link = [0.0; 2];
                if x + next < w {
                    link[0] = path_weight(d0, out[0]);
                }
                if y + next < h {
                    link[1] = path_weight(d1, out[1]);
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            scratch
                .data
                .par_chunks_mut(w)
                .zip(links.data.par_chunks_mut(w))
                .enumerate()
                .for_each_init(|| (Vec::new(), Vec::new()), |bufs, (y, (prow, lrow))| step_row(bufs, y, prow, lrow));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut bufs = (Vec::new(), Vec::new());
            for (y, (prow, lrow)) in scratch.data.chunks_mut(w).zip(links.data.chunks_mut(w)).enumerate() {
                step_row(&mut bufs, y, prow, lrow);
            }
        }
        std::mem::swap(&mut paths, &mut scratch);
    }
    drop(scratch);
    drop(paths);
    drop(links);
    match &original {
        None => {
            let fw = img.width;
            for_rows(&mut img.data, fw, |y, row| {
                for (x, p) in row.iter_mut().enumerate() {
                    let Some(&q) = filtered.data.get(y * w + x) else { continue };
                    let result = rgb2(luminance_2020(*p), q);
                    *p = std::array::from_fn(|i| p[i] + strength * (result[i] - p[i]));
                }
            });
        }
        Some(original) => {
            // Reconstruct only rows. Keep the fine residual with a shared, noise-subtracted
            // Wiener share; a flat noisy patch receives no automatic protection from noisy Y.
            let mean = gaussian(original, 1.0);
            let corr = gaussian(&original.map(|p| p.map(|v| v * v)), 1.0);
            let share = corr.zip_map(&mean, |c, m| {
                let snr = (1..3).map(|i| ((c[i] - m[i] * m[i]).max(0.0) / noise.var(i, m[0]) - 1.0).max(0.0)).sum::<f32>() * 0.5;
                [snr / (snr + 8.0); 3]
            });
            let share = gaussian(&share, 0.8);
            // Resize one output row at a time using the raster resampler's exact pixel centres.
            use crate::raster::resample::weights;
            let ux = weights(w, img.width, Filter::Bilinear);
            let uy = weights(h, img.height, Filter::Bilinear);
            let ow = img.width;
            for_rows(&mut img.data, ow, |y, row| {
                let Some((y0, wy)) = uy.get(y) else { return };
                for (x, p) in row.iter_mut().enumerate() {
                    let Some((x0, wx)) = ux.get(x) else { continue };
                    let (mut f, mut o, mut a) = ([0.0; 2], [0.0; 3], 0.0);
                    for (j, &dy) in wy.iter().enumerate() {
                        for (i, &dx) in wx.iter().enumerate() {
                            let index = (y0 + j) * w + x0 + i;
                            if let (Some(q), Some(r), Some(s)) = (filtered.data.get(index), original.data.get(index), share.data.get(index)) {
                                let weight = dx * dy;
                                f = std::array::from_fn(|c| f[c] + weight * q[c]);
                                o = std::array::from_fn(|c| o[c] + weight * r[c]);
                                a += weight * s[0];
                            }
                        }
                    }
                    let c = yuv(*p);
                    let q = [f[0] + a * (c[1] - o[1]), f[1] + a * (c[2] - o[2])];
                    let result = rgb2(c[0], q);
                    *p = std::array::from_fn(|i| p[i] + strength * (result[i] - p[i]));
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::NrcPass;

    fn params(step: usize) -> NrcParams {
        NrcParams { off: 0.003, passes: vec![NrcPass { sigma: 4.0, step, k: 64.0, pct: 0.2 }, NrcPass { sigma: 12.0, step, k: 64.0, pct: 0.2 }] }
    }

    fn noisy(clean: &Rgb32f, seed: u64, sigma: f32) -> Rgb32f {
        let mut result = clean.clone();
        let mut seed = seed;
        for p in &mut result.data {
            for v in p {
                let mut n = 0.0;
                for _ in 0..4 {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    n += (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5;
                }
                *v += sigma * 1.732 * n;
            }
        }
        result
    }

    /// The optimised filter reproduces the reference implementation (8b89526d) bit for bit: same scales,
    /// weights, paths and reconstruction, only fewer passes and smaller buffers.
    #[test]
    fn joint_matches_the_reference_bit_for_bit() {
        let mut scene = Rgb32f::new(97, 83);
        for (i, p) in scene.data.iter_mut().enumerate() {
            let (x, y) = (i % 97, i / 97);
            // edges, a thin line, a dark patch and a gradient
            *p = if x < 30 {
                [0.08, 0.016, 0.02]
            } else if x < 32 {
                [0.01, 0.05, 0.01]
            } else if y < 40 {
                [0.0015, 0.004, 0.009]
            } else {
                [0.02 + 0.001 * x as f32, 0.03, 0.01 + 0.0005 * y as f32]
            };
        }
        for (w, h) in [(97usize, 83usize), (96, 82), (5, 3), (1, 7)] {
            let mut clean = Rgb32f::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    clean.data[y * w + x] = scene.data[y * 97 + x];
                }
            }
            for seed in [0x2545_f491_4f6c_dd1d, 0x9e37_79b9_7f4a_7c15] {
                let input = noisy(&clean, seed, 0.004);
                for step in [1, 2] {
                    for (detail, strength, sigma) in [(0.5, 1.0, 12.0), (0.25, 0.6, 4.0), (0.9, 1.0, 27.0)] {
                        let p = NrcParams { off: 0.003, passes: vec![NrcPass { sigma, step, k: 64.0, pct: 0.2 }] };
                        let (mut a, mut b) = (input.clone(), input.clone());
                        denoise_grid(&mut a, &p, step, detail, strength, SpeckBrake::OFF);
                        crate::nrc_joint_reference::denoise_grid(&mut b, &p, step, detail, strength);
                        for (i, (x, y)) in a.data.iter().zip(&b.data).enumerate() {
                            for c in 0..3 {
                                assert!(
                                    x[c].to_bits() == y[c].to_bits(),
                                    "{w}x{h} step {step} sigma {sigma}: pixel {i} channel {c}: {} vs {}",
                                    x[c],
                                    y[c]
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// BENCH (ignored, prints): the optimised filter against the reference on the same 24 MP image,
    /// interleaved so both see the same machine load. Median of 3 each.
    #[test]
    #[ignore]
    fn bench_joint_against_reference() {
        let (w, h) = (6000usize, 4000usize);
        let mut clean = Rgb32f::new(w, h);
        for (i, p) in clean.data.iter_mut().enumerate() {
            let (x, y) = (i % w, i / w);
            *p = if (x / 300 + y / 300) % 2 == 0 { [0.03, 0.02, 0.01] } else { [0.004, 0.006, 0.012] };
        }
        let input = noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.004);
        let p = NrcParams { off: 0.003, passes: vec![NrcPass { sigma: 27.0, step: 1, k: 64.0, pct: 0.2 }] };
        let (mut new, mut old) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            let mut a = input.clone();
            let t = std::time::Instant::now();
            denoise_grid(&mut a, &p, 1, 0.5, 1.0, SpeckBrake::OFF);
            new.push(t.elapsed().as_secs_f64());
            let mut b = input.clone();
            let t = std::time::Instant::now();
            crate::nrc_joint_reference::denoise_grid(&mut b, &p, 1, 0.5, 1.0);
            old.push(t.elapsed().as_secs_f64());
        }
        new.sort_by(f64::total_cmp);
        old.sort_by(f64::total_cmp);
        println!("joint 24 MP: optimised {:.2} s, reference {:.2} s (medians of {new:.2?} / {old:.2?})", new[1], old[1]);
    }

    /// BENCH (ignored, prints): fitting the noise model to 130 000 samples (a 33 MP image's), median of 5.
    #[test]
    #[ignore]
    fn bench_noise_fit() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        let samples: Vec<NoiseSample> = (0..130_000)
            .map(|_| {
                let y = rnd() * 0.3;
                (y, std::array::from_fn(|_| std::array::from_fn(|_| (1e-5 + 1e-4 * y) * rnd() * 2.0)))
            })
            .collect();
        let mut times: Vec<f64> = (0..5)
            .map(|_| {
                let s = samples.clone();
                let t = std::time::Instant::now();
                let model = fit_noise(s);
                assert!(model.model.beta[0] > 0.0);
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(f64::total_cmp);
        println!("noise fit, 130 000 samples: {:.1} ms (of {times:.1?})", times[2]);
    }

    /// BENCH (ignored, prints): the speck brake alone against the whole joint filter on one 24 MP image,
    /// interleaved. Median of 3 each.
    #[test]
    #[ignore]
    fn bench_speck_brake() {
        let (w, h) = (6000usize, 4000usize);
        let clean = Rgb32f::from_fn(w, h, |x, y| if (x / 300 + y / 300) % 2 == 0 { [0.03, 0.02, 0.01] } else { [0.004, 0.006, 0.012] });
        let input = noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.004);
        let p = NrcParams { off: 0.003, passes: vec![NrcPass { sigma: 27.0, step: 1, k: 64.0, pct: 0.2 }] };
        let transformed = input.map(yuv);
        let noise = noise_model(&transformed);
        let (mut brake, mut joint) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            let mut out = transformed.clone();
            let t = std::time::Instant::now();
            despeck(&mut out, &noise, speck_brake(1.0));
            brake.push(t.elapsed().as_secs_f64());
            let mut a = input.clone();
            let t = std::time::Instant::now();
            denoise_grid(&mut a, &p, 1, 0.5, 1.0, SpeckBrake::OFF);
            joint.push(t.elapsed().as_secs_f64());
        }
        brake.sort_by(f64::total_cmp);
        joint.sort_by(f64::total_cmp);
        println!("24 MP: speck brake {:.3} s, joint without it {:.2} s (medians of {brake:.3?} / {joint:.2?})", brake[1], joint[1]);
    }

    #[test]
    fn joint_keeps_negative_means_luminance_and_reduces_chroma_noise() {
        for (w, h) in [(128, 96), (127, 95), (1, 19), (19, 1)] {
            let clean = Rgb32f::filled(w, h, [0.006, 0.004, 0.0008]);
            for seed in [0x2545_f491_4f6c_dd1d, 0x1234_5678_9abc_def1] {
                let input = noisy(&clean, seed, 0.003);
                for step in [1, 2] {
                    let mut output = input.clone();
                    denoise_grid(&mut output, &params(step), step, 0.5, 1.0, speck_brake(1.0));
                    assert!(output.data.iter().flatten().all(|v| v.is_finite()));
                    for (a, b) in input.data.iter().zip(&output.data) {
                        assert!((luminance_2020(*a) - luminance_2020(*b)).abs() < 1e-7);
                    }
                    // Thin inputs cannot support a spatial noise estimate; finite/luminance
                    // checks above still apply, but their sample mean is too noisy for this bound.
                    if w < 3 || h < 3 {
                        continue;
                    }
                    for c in 0..3 {
                        let difference = input.data.iter().zip(&output.data).map(|(a, b)| (b[c] - a[c]) as f64).sum::<f64>() / input.len() as f64;
                        assert!(difference.abs() < 2e-6, "step {step}, {w}x{h}, mean error {difference}");
                    }
                    let energy = |im: &Rgb32f| {
                        im.data
                            .iter()
                            .map(|&p| {
                                let p = yuv(p);
                                let c = yuv(clean.data[0]);
                                (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)
                            })
                            .sum::<f32>()
                            / im.len() as f32
                    };
                    assert!(energy(&output) < 0.08 * energy(&input));
                    assert!(output.data.iter().any(|p| p.iter().any(|&v| v < 0.0)), "negative values were clipped");
                }
            }
        }
    }

    #[test]
    fn joint_protects_leaf_and_isoluminant_edges_without_chroma_halos() {
        let leaf = [0.02, 0.12, 0.01];
        let black = [0.0005; 3];
        let red = [0.08, 0.0155, 0.02];
        let mut green = [0.012, 0.04, 0.015];
        let gain = luminance_2020(red) / luminance_2020(green);
        green = green.map(|v| v * gain);
        for (left, right) in [(leaf, black), (red, green)] {
            let clean = Rgb32f::from_fn(192, 128, |x, _| if x < 96 { left } else { right });
            for seed in [0x2545_f491_4f6c_dd1d, 0x1234_5678_9abc_def1] {
                let input = noisy(&clean, seed, 0.003);
                for step in [1, 2] {
                    let mut output = input.clone();
                    denoise_grid(&mut output, &params(step), step, 0.5, 1.0, speck_brake(1.0));
                    let step_uv = [yuv(left)[1] - yuv(right)[1], yuv(left)[2] - yuv(right)[2]];
                    let length = step_uv.iter().map(|v| v * v).sum::<f32>().sqrt();
                    for x in [92, 93, 98, 99] {
                        let mut mean = [0.0; 3];
                        for y in 16..112 {
                            let p = yuv(output.get(x, y));
                            mean = std::array::from_fn(|i| mean[i] + p[i] / 96.0);
                        }
                        let expected = yuv(if x < 96 { left } else { right });
                        let error = ((mean[1] - expected[1]).powi(2) + (mean[2] - expected[2]).powi(2)).sqrt() / length;
                        assert!(error < 0.08, "step {step}, x {x}, colour error {}%", 100.0 * error);
                    }
                }
            }
        }
    }

    #[test]
    fn joint_noise_fit_does_not_treat_bright_texture_as_shot_noise() {
        let clean = Rgb32f::from_fn(256, 192, |x, y| {
            let l = if x < 192 { 0.003 + x as f32 * 0.00002 } else { 0.15 + if (x + y) % 2 == 0 { 0.08 } else { -0.08 } };
            [l; 3]
        });
        let input = noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.002).map(yuv);
        let model = noise_model(&input);
        for i in 0..3 {
            assert!(model.var(i, 0.15) < 5.0 * model.var(i, 0.005), "texture dominated noise fit: {model:?}");
        }
    }

    #[test]
    fn joint_keeps_a_two_pixel_isoluminant_line() {
        let grey = [0.03; 3];
        let mut red = [0.08, 0.0155, 0.02];
        let gain = luminance_2020(grey) / luminance_2020(red);
        red = red.map(|v| v * gain);
        let clean = Rgb32f::from_fn(192, 128, |x, _| if (94..96).contains(&x) { red } else { grey });
        let mut output = noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.006);
        denoise_grid(&mut output, &params(1), 1, 0.5, 1.0, speck_brake(1.0));
        let reference = yuv(red);
        let mut mean = [0.0; 3];
        for y in 16..112 {
            for x in 94..96 {
                let p = yuv(output.get(x, y));
                mean = std::array::from_fn(|i| mean[i] + p[i] / 192.0);
            }
        }
        let kept = (mean[1] * reference[1] + mean[2] * reference[2]) / (reference[1].powi(2) + reference[2].powi(2));
        assert!(kept > 0.85, "thin line keeps only {}%", 100.0 * kept);
    }

    #[test]
    fn joint_estimates_correlated_noise_at_a_matching_scale() {
        let clean = Rgb32f::filled(256, 192, [0.03; 3]);
        let correlated = gaussian(&noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.012), 0.8).map(yuv);
        let model = noise_model(&correlated);
        for i in 0..3 {
            let mean = correlated.data.iter().map(|p| p[i] as f64).sum::<f64>() / correlated.len() as f64;
            let variance = correlated.data.iter().map(|p| (p[i] as f64 - mean).powi(2)).sum::<f64>() / correlated.len() as f64;
            let predicted = model.var(i, 0.03) as f64;
            assert!(predicted > 0.6 * variance && predicted < 1.6 * variance, "channel {i}: estimated {predicted}, actual {variance}");
        }
    }

    /// The speck brake takes the colour out of a compact, isoluminant 2 × 2 speck (as demosaicing leaves
    /// them at high ISO), more at Color 100 than at 25, but leaves a coloured star (a luminance peak) alone.
    #[test]
    fn noise_line_does_not_jump_when_a_band_crosses_the_outlier_bound() {
        // The luminance bands of a 640-px preview of demo scene 0 after luminance NR. Its band at
        // 0.593 sat at 4 times the first fit: the CPU's samples put it just above, the GPU's just
        // below, and the fits came out 2.6 % apart (CPU and GPU renders then differed by 10 levels).
        let mut pts: Vec<(f64, f64, f64)> = [
            (0.009, 5.028e-7),
            (0.022, 1.554e-4),
            (0.063, 1.459e-4),
            (0.147, 9.728e-6),
            (0.213, 2.701e-5),
            (0.272, 5.077e-6),
            (0.315, 3.236e-5),
            (0.356, 2.531e-5),
            (0.390, 5.329e-6),
            (0.420, 4.924e-6),
            (0.467, 5.453e-5),
            (0.530, 1.823e-5),
            (0.593, 0.0),
            (0.648, 1.501e-8),
            (0.667, 1.898e-8),
            (0.996, 3.490e-7),
        ]
        .map(|(y, v)| (y, v, 5649.0))
        .to_vec();
        let (a, b) = fit_nonneg_line(&pts[..pts.len().div_ceil(2)]);
        let bound = 4.0 * (a * 0.593 + b);
        let mut fit = |k: f64| {
            pts[12].1 = bound * k;
            robust_line(&pts)
        };
        let (below, above) = (fit(1.0 - 1e-6), fit(1.0 + 1e-6));
        assert!((below.1 / above.1 - 1.0).abs() < 1e-4, "{below:?} vs {above:?}");
        assert!((below.0 - above.0).abs() <= 1e-4 * below.0.abs().max(1e-12), "{below:?} vs {above:?}");
    }

    #[test]
    fn speck_brake_removes_compact_colour_specks_but_not_stars() {
        let dark = [0.004f32; 3];
        let mut magenta = [0.024, 0.0, 0.024];
        magenta[1] = (luminance_2020(dark) - 0.2627 * magenta[0] - 0.0593 * magenta[2]) / 0.6780;
        let star = [0.3, 0.2, 0.1];
        let clean = Rgb32f::from_fn(128, 96, |x, y| match (x, y) {
            (40..42, 40..42) => magenta,
            (90..92, 40..42) => star,
            _ => dark,
        });
        let input = noisy(&clean, 0x2545_f491_4f6c_dd1d, 0.001);
        // share of the object's chroma (against the background) left in its 2 × 2 pixels
        let kept = |out: &Rgb32f, x0: usize, colour: [f32; 3]| {
            let (r, b) = (yuv(colour), yuv(dark));
            let reference = [r[1] - b[1], r[2] - b[2]];
            let mut mean = [0.0f32; 2];
            for (x, y) in [(x0, 40), (x0 + 1, 40), (x0, 41), (x0 + 1, 41)] {
                let p = yuv(out.get(x, y));
                mean = [mean[0] + 0.25 * (p[1] - b[1]), mean[1] + 0.25 * (p[2] - b[2])];
            }
            (mean[0] * reference[0] + mean[1] * reference[1]) / (reference[0].powi(2) + reference[1].powi(2))
        };
        let run = |brake| {
            let mut out = input.clone();
            denoise_grid(&mut out, &params(1), 1, 0.5, 1.0, brake);
            (kept(&out, 40, magenta), kept(&out, 90, star))
        };
        let (off, star_off) = run(SpeckBrake::OFF);
        let (weak, _) = run(speck_brake(0.25));
        let (strong, star_strong) = run(speck_brake(1.0));
        assert!(off > 0.5, "without the brake the speck keeps {off}");
        assert!(weak < 0.8 * off && strong < 0.2, "speck kept: off {off}, at 25 {weak}, at 100 {strong}");
        assert!(strong < weak, "the brake is stronger at 100 ({strong}) than at 25 ({weak})");
        assert!((star_strong - star_off).abs() < 0.02, "star colour {star_off} -> {star_strong}");
    }

    /// `distance_run` (planes, no branches) gives `distance` bit for bit.
    #[test]
    fn distance_run_matches_distance() {
        let clean = Rgb32f::from_fn(64, 3, |x, _| [0.002 * x as f32, 0.01, 0.03 - 0.0004 * x as f32]);
        let img = noisy(&clean, 0x9e37_79b9_7f4a_7c15, 0.01).map(yuv);
        let mut noise = noise_model(&noisy(&Rgb32f::filled(64, 64, [0.02; 3]), 7, 0.004).map(yuv));
        noise.tolerance = 0.7;
        let planes = Planes { c: std::array::from_fn(|i| img.data.iter().map(|p| p[i]).collect()), w: 64 };
        let mut out = vec![0.0; 64];
        distance_run(planes.run(0, 0, 64).unwrap(), planes.run(2, 0, 64).unwrap(), &noise, &mut out);
        for (x, d) in out.iter().enumerate() {
            assert_eq!(d.to_bits(), distance(img.get(x, 0), img.get(x, 2), &noise).to_bits(), "pixel {x}");
        }
    }
}
