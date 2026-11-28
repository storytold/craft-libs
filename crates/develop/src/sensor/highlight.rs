//! Clip-aware highlight handling on demosaiced camera RGB (before white balance, white level = 1.0).
//!
//! - [`clip_neutral`]: white-balance-aware clipping so that sensor-clipped areas render neutral instead of
//!   magenta/cyan (each channel limited to the smallest white-balanced clip level).
//! - [`reconstruct`] / [`reconstruct_with`]: where only some channels are clipped, rebuild them from the
//!   unclipped channels using the chromaticity of reliable unclipped content nearby (bright, away from edges and
//!   from the clipped pixels themselves; diffused into the clipped region coarse to fine) unless both the pixel's
//!   own channels and the clipped surface's colour just below its clip (the rim) contradict it, and neutral where
//!   there is none within reach. Right past the rim, the rim's colour carries on. Fully clipped pixels become
//!   neutral at the brightest plausible level; partly clipped ones fade into it on their way from the first channel
//!   clipping to the last, pale ones from the start.
//! - [`reconstruct_masked`]: also uses clipping information from the original mosaic, for binned and full-size images.
//!
//! Why the care: the white-balance gains push clipped channels apart (green clips first, red and blue keep
//! rising), so whatever colour the rebuilt pixels carry is hidden at Neutral only by the tone map's roll-off to
//! white. Any darkening (Highlights, negative Exposure or Whites) brings it out. Three things used to leak colour
//! in: the chromaticity of whatever unclipped pixel happened to be nearest (dark foliage, purple and green
//! fringes along branches, pixels demosaiced from clipped neighbours); a colour model that does not render the
//! camera's white-balanced neutral as neutral (a look fitted to the camera's JPEG, a camera profile); and, in a
//! pale partly clipped sky, the colour of another reliable surface (bright foliage) and the sky's own faint
//! colour, which darkening and a camera profile turned teal.

use crate::raster::{Rgb32f, par_rows};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

/// Clip every channel of `wb ⊙ img` at `min_c(wb_c) · clip` — i.e. at the lowest channel's clip level after WB —
/// then divide the multipliers back out. `clip` is the sensor clip in normalised units (≈ 1.0).
pub fn clip_neutral(img: &mut Rgb32f, wb: [f32; 3], clip: f32) {
    let limit = wb.iter().cloned().fold(f32::MAX, f32::min) * clip;
    for p in &mut img.data {
        for c in 0..3 {
            p[c] = (p[c] * wb[c]).min(limit) / wb[c];
        }
    }
}

/// Unclipped pixels lend their chromaticity in proportion to how close they come to clipping (their largest
/// channel, relative to the clip level): from nothing below `SOURCE_LO` to fully above `SOURCE_HI`. Dark content
/// (foliage, branches, buildings against a clipped sky) says nothing about the sky's colour.
const SOURCE_LO: f32 = 0.25;
const SOURCE_HI: f32 = 0.6;
/// Sources on a strong edge (a 3 × 3 half-resolution neighbour brighter or darker by this many stops) are
/// left out, fading from `EDGE_LO` to `EDGE_HI`: edge pixels mix two surfaces and carry lens fringes.
const EDGE_LO: f32 = 0.15;
const EDGE_HI: f32 = 0.4;
/// Source cells within this many half-resolution cells of a clipped pixel are left out: their channels were
/// demosaiced partly from clipped samples. (At least 1: the same neighbourhood tells which cells clipped
/// pixels sample.)
const MARGIN: usize = 1;
/// Weight of the coarser estimate against a cell's own sources when the pyramid is pushed back down (in units
/// of source density: a cell half covered by full-weight sources counts 0.5).
const PRIOR: f32 = 0.01;
/// The rebuilt colour fades to neutral where reliable content covers less than `SUPPORT` of the surroundings at
/// every scale up to `REACH` of the long edge (so previews and exports agree), gradually, as the coverage of the
/// widest one falls: a clipped sky between bare branches is better white than the colour of a river far away.
const REACH: f32 = 1.0 / 4.0;
const SUPPORT: f32 = 0.05;
/// An unclipped channel above `FLOOR_LO` of its clip level counts more and more as the clip floor it is about to
/// become (see [`reconstruct_with`]).
const FLOOR_LO: f32 = 0.75;
/// A clipped pixel takes its estimated colour (reliable colour around, or the neutral guess) only as far as it is
/// brighter (or otherwise unlike) the unclipped pixels nearest its clip: the "rim", unclipped pixels above `RIM_LO`
/// of the clip level, weighted towards the clip, the clipped surface's own colour just below it. Up to `NEAR_RIM`
/// stops past the rim (the rim is an average, a little below the clip) it is rebuilt with the rim's colour, and
/// over the next `DEPTH` stops it goes over to the estimate, so a surface runs into clipping without a step
/// whatever its colour and whatever lent the estimate; well past it, or where the rim is another surface (its
/// colour differs; a clipped channel differs by at least as much as it lies above the rim's), the estimate applies.
const RIM_LO: f32 = 0.85;
const NEAR_RIM: f32 = 0.03;
const DEPTH: f32 = 0.2;
/// Reliable colour counts less and less as both the pixel's own channels (see [`contradiction`]) and the rim's
/// colour contradict it, by `AGREE.0` to `AGREE.1` stops: it is another surface's, like bright foliage below a pale
/// sky, whose green would turn the sky teal. (Either alone can be wrong: a pixel's own channels are noisy and
/// can't speak for the channels that clipped, and the rim can be another surface, like a white window frame
/// around a bright view.)
const AGREE: (f32, f32) = (0.15, 0.4);
/// On the way from the first channel clipping to the last, a partly clipped pixel's brightness goes over to the
/// clipped neutral's in the second half, and so does the colour of a strongly coloured one (saturation relative to
/// the white above `PALE.1`). A pale one's colour (below `PALE.0`) goes over from the start: the few per cent of
/// colour left in a pale sky once a channel clipped is as much the estimate's error as the sky's, and darkening
/// shows it.
const PALE: (f32, f32) = (0.2, 0.4);
/// Minimum brightness ratio against reconstruction by value. Its inverse limits brightening in saturated colours.
const CONTINUITY: f32 = 0.8;
/// Weights for comparing white-balanced brightness.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One level of a pyramid, `w × h` cells of `N - 1` values and a weight. Pulled: per cell the weighted sums of
/// the values and the weight, as densities (per pixel of the cell). Pushed: the estimated values, and still the
/// weight. The chromaticity pyramid holds red and blue chromaticity, the rim pyramid white-balanced RGB.
struct Level<const N: usize> {
    w: usize,
    h: usize,
    cells: Vec<[f32; N]>,
}

/// Bilinear sample of a level at the centre of pixel `(x, y)` of a grid `scale` times finer.
#[inline]
fn sample<const N: usize>(l: &Level<N>, scale: f32, x: usize, y: usize) -> [f32; N] {
    let (cv, cw, ch) = (&l.cells, l.w, l.h);
    let fx = ((x as f32 + 0.5) / scale - 0.5).clamp(0.0, cw.saturating_sub(1) as f32);
    let fy = ((y as f32 + 0.5) / scale - 0.5).clamp(0.0, ch.saturating_sub(1) as f32);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(cw.saturating_sub(1)), (y0 + 1).min(ch.saturating_sub(1)));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let g = |xx: usize, yy: usize| cv.get(yy * cw + xx).copied().unwrap_or([0.0; N]);
    let (a, b, c, d) = (g(x0, y0), g(x1, y0), g(x0, y1), g(x1, y1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        top + (bot - top) * ty
    })
}

/// The half-resolution source level of `img`: every `2 × 2` block's source-weighted chromaticity sums, with
/// blocks near clipped pixels or on strong edges left out. Also returns, per cell, whether a clipped pixel lies
/// in its row within [`MARGIN`] cells (the vertical half of that test is left to the readers).
fn sources(img: &Rgb32f, wb: [f32; 3], clip: f32, clipped: &impl Clipped) -> (Level<3>, Vec<bool>) {
    let (w, h) = (img.width, img.height);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    // per cell: [red sum, blue sum, weight] densities, luminance, whether it holds a clipped pixel
    let mut cells = vec![[0f32; 3]; cw * ch];
    let mut lum = vec![0f32; cw * ch];
    let mut near = vec![false; cw * ch];
    let process_row = |y: usize, crow: &mut [[f32; 3]], lrow: &mut [f32], nrow: &mut [bool]| {
        let mut touched = vec![false; cw];
        let row = |r: usize| img.data.get(r * w..(r + 1) * w).unwrap_or(&[]);
        let (r0, r1) = (row(2 * y), row(2 * y + 1));
        let lo = SOURCE_LO * clip;
        for x in 0..cw {
            let (mut a, mut sum, mut n, mut hit) = ([0f32; 3], 0f32, 0usize, false);
            let span = 2 * x..(2 * x + 2).min(w);
            let (i0, i1) = (2 * y * w + 2 * x, (2 * y + 1) * w + 2 * x);
            let pixels = r0.get(span.clone()).unwrap_or(&[]).iter().zip(i0..).chain(r1.get(span).unwrap_or(&[]).iter().zip(i1..));
            for (p, i) in pixels {
                let q = [p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]];
                let s = q[0].max(0.0) + q[1].max(0.0) + q[2].max(0.0);
                sum += s;
                n += 1;
                let top = p[0].max(p[1]).max(p[2]);
                if clipped(i, p).iter().any(|&c| c) {
                    hit = true;
                    continue;
                }
                if top > lo && s > 1e-4 {
                    let b = smoothstep(SOURCE_LO, SOURCE_HI, top / clip) / s;
                    a[0] += b * q[0].max(0.0);
                    a[1] += b * q[2].max(0.0);
                    a[2] += b * s;
                }
            }
            let k = 1.0 / n.max(1) as f32;
            crow[x] = a.map(|v| v * k);
            lrow[x] = (sum * k).max(1e-6);
            touched[x] = hit;
        }
        for (x, nr) in nrow.iter_mut().enumerate() {
            *nr = touched.get(x.saturating_sub(MARGIN)..=(x + MARGIN).min(cw - 1)).is_some_and(|t| t.iter().any(|&t| t));
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    cells.par_chunks_mut(cw).zip(lum.par_chunks_mut(cw)).zip(near.par_chunks_mut(cw)).enumerate().for_each(|(y, ((crow, lrow), nrow))| {
        process_row(y, crow, lrow, nrow);
    });
    #[cfg(target_arch = "wasm32")]
    for (y, ((crow, lrow), nrow)) in cells.chunks_mut(cw).zip(lum.chunks_mut(cw)).zip(near.chunks_mut(cw)).enumerate() {
        process_row(y, crow, lrow, nrow);
    }
    // leave out cells near clipped pixels and on strong edges (luminance ratios to the neighbours)
    let (lum, near_ref) = (&lum, &near);
    let (edge_lo, edge_hi) = (EDGE_LO.exp2(), EDGE_HI.exp2());
    par_rows(&mut cells, cw, |y, row| {
        let (ya, yb) = (y.saturating_sub(MARGIN), (y + MARGIN).min(ch - 1));
        let (ea, eb) = (y.saturating_sub(1), (y + 1).min(ch - 1));
        for (x, cell) in row.iter_mut().enumerate() {
            if cell[2] <= 0.0 {
                continue;
            }
            if (ya..=yb).any(|yy| near_ref.get(yy * cw + x).copied().unwrap_or(false)) {
                *cell = [0.0; 3];
                continue;
            }
            let l = lum.get(y * cw + x).copied().unwrap_or(1.0);
            let (xa, xb) = (x.saturating_sub(1), (x + 1).min(cw - 1));
            let (mut lo, mut hi) = (l, l);
            for yy in ea..=eb {
                for &v in lum.get(yy * cw + xa..=yy * cw + xb).unwrap_or(&[]) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            let keep = 1.0 - smoothstep(edge_lo, edge_hi, (hi / l).max(l / lo));
            *cell = cell.map(|v| v * keep);
        }
    });
    (Level { w: cw, h: ch, cells }, near)
}

/// The quarter-resolution rim level of `img`: every `4 × 4` block's white-balanced RGB of its unclipped pixels
/// above [`RIM_LO`] of the clip level, weighted (steeply) towards the clip, as densities. Pixels next to clipped
/// ones (`margin`, by half-resolution cell) are left out, like the colour sources: they were demosaiced partly from
/// clipped samples, and lens fringes along clipped content would pass for its colour.
fn rim(img: &Rgb32f, wb: [f32; 3], clip: f32, clipped: &impl Clipped, margin: &(dyn Fn(usize, usize) -> bool + Sync)) -> Level<4> {
    let (w, h) = (img.width, img.height);
    let (cw, ch) = (w.div_ceil(4), h.div_ceil(4));
    let mut cells = vec![[0f32; 4]; cw * ch];
    par_rows(&mut cells, cw, |y, row| {
        for (x, cell) in row.iter_mut().enumerate() {
            let (mut a, mut n) = ([0f32; 4], 0usize);
            for sy in 4 * y..(4 * y + 4).min(h) {
                for (i, p) in img.data.get(sy * w + 4 * x..sy * w + (4 * x + 4).min(w)).unwrap_or(&[]).iter().enumerate() {
                    n += 1;
                    let top = p[0].max(p[1]).max(p[2]) / clip;
                    if !(RIM_LO..1.0).contains(&top) || margin((4 * x + i) / 2, sy / 2) || clipped(sy * w + 4 * x + i, p).iter().any(|&c| c) {
                        continue;
                    }
                    let k = ((top - RIM_LO) / (1.0 - RIM_LO)).powi(4);
                    for c in 0..3 {
                        a[c] += k * p[c] * wb[c];
                    }
                    a[3] += k;
                }
            }
            *cell = a.map(|v| v / n.max(1) as f32);
        }
    });
    Level { w: cw, h: ch, cells }
}

/// The next coarser level: each `2 × 2` block's mean densities.
fn pull<const N: usize>(l: &Level<N>) -> Level<N> {
    let (w, h) = (l.w.div_ceil(2), l.h.div_ceil(2));
    let mut cells = vec![[0f32; N]; w * h];
    par_rows(&mut cells, w, |y, row| {
        for (x, cell) in row.iter_mut().enumerate() {
            let (mut s, mut n) = ([0f32; N], 0usize);
            for (sx, sy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                if let Some(c) = (sx < l.w && sy < l.h).then(|| l.cells.get(sy * l.w + sx)).flatten() {
                    for k in 0..N {
                        s[k] += c[k];
                    }
                    n += 1;
                }
            }
            *cell = s.map(|v| v / n.max(1) as f32);
        }
    });
    Level { w, h, cells }
}

/// The estimated values of the cells of `levels[0]` that `needed` marks (all of them when `None`), and of every
/// cell of the coarser levels: each level's own sources blended with the next coarser estimate. The top level is
/// its own sources' weighted mean (normalised: however sparse they are in a large empty picture, the empty part
/// never pulls the estimate anywhere), and `empty` (at zero weight) when it has none at all.
fn push<const N: usize>(levels: &mut [Level<N>], empty: [f32; N], needed: Option<&(dyn Fn(usize, usize) -> bool + Sync)>) {
    let estimate = |cell: [f32; N], prior: [f32; N]| -> [f32; N] {
        let weight = cell[N - 1];
        let d = weight + PRIOR;
        std::array::from_fn(|k| if k + 1 == N { weight } else { (cell[k] + PRIOR * prior[k]) / d })
    };
    let Some(last) = levels.last_mut() else { return };
    last.cells.iter_mut().for_each(|c| {
        let weight = c[N - 1];
        *c = if weight > 0.0 { std::array::from_fn(|k| if k + 1 == N { weight } else { c[k] / weight }) } else { empty };
    });
    for i in (0..levels.len().saturating_sub(1)).rev() {
        let (fine, coarse) = levels.split_at_mut(i + 1);
        let (Some(fine), Some(coarse)) = (fine.last_mut(), coarse.first()) else { continue };
        let w = fine.w;
        let needed = needed.filter(|_| i == 0);
        par_rows(&mut fine.cells, w, |y, row| {
            for (x, cell) in row.iter_mut().enumerate() {
                if needed.is_some_and(|f| !f(x, y)) {
                    continue;
                }
                *cell = estimate(*cell, sample(coarse, 2.0, x, y));
            }
        });
    }
}

/// How many stops two chromaticities differ by in any channel ratio.
fn unlike(a: [f32; 3], b: [f32; 3]) -> f32 {
    let ratio = |i: usize, j: usize| (a[i].max(1e-6) * b[j].max(1e-6)) / (a[j].max(1e-6) * b[i].max(1e-6));
    [(0, 1), (0, 2), (1, 2)].iter().map(|&(i, j)| ratio(i, j).log2().abs()).fold(0.0f32, f32::max)
}

/// How far channel `c` of a pixel (sensor values `p`, clipped channels `cl`) only bounds its true value from below:
/// fully once clipped, and increasingly from `FLOOR_LO` of the clip on, so that nothing compared with it jumps where
/// it clips.
fn bound(p: [f32; 3], cl: [bool; 3], clip: f32, c: usize) -> f32 {
    if cl[c] { 1.0 } else { smoothstep(FLOOR_LO, 1.0, p[c] / clip) }
}

/// How many stops a pixel's channels contradict chromaticity `r`. A saturated channel sets a lower bound.
/// Channels with unknown values do not constrain the estimate.
fn contradiction(p: [f32; 3], q: [f32; 3], clipping: Clipping, r: [f32; 3], clip: f32) -> f32 {
    let (mut lo, mut hi) = (f32::INFINITY, 0f32);
    for c in 0..3 {
        if clipping.unknown(c) {
            continue;
        }
        let level = q[c] / r[c].max(1e-3);
        hi = hi.max(level);
        let w = bound(p, clipping.clipped, clip, c);
        if r[c] > 1e-3 && w < 1.0 {
            lo = lo.min(level / (1.0 - w));
        }
    }
    if hi > 0.0 && lo > 0.0 && lo.is_finite() { (hi / lo).log2().max(0.0) } else { 0.0 }
}

/// Rebuild a pixel at chromaticity `r` using its known, unclipped channels to estimate the level.
/// Keep those channels and use saturated values as lower bounds.
fn rebuild(p: [f32; 3], q: [f32; 3], clipping: Clipping, r: [f32; 3], clip: f32, max_level: f32) -> Option<[f32; 3]> {
    // The white-balanced channel sum the unclipped channels imply: their mean, except that a channel about to clip
    // counts more and more as the floor it is about to become (continuous with the neighbours where it has).
    let (mut sum, mut near_clip, mut k) = (0f32, 0f32, 0usize);
    for c in 0..3 {
        if !clipping.clipped[c] && r[c] > 1e-3 {
            let s = q[c] / r[c];
            sum += s;
            near_clip = near_clip.max(s * smoothstep(FLOOR_LO, 1.0, p[c] / clip));
            k += 1;
        }
    }
    if k == 0 {
        return None;
    }
    // Two clipped channels must not be left at their clip levels: those differ by the white-balance gains (green
    // clips first), and the pair would show as magenta or red. They keep the estimate's ratio instead, at the level
    // that lifts each to its clip (no channel above the clipped neutral), and so does a channel about to clip (the
    // same floor, so nothing jumps where it clips).
    let floor = (0..3).filter(|&c| clipping.saturated[c]).map(|c| q[c] / r[c].max(1e-3)).fold(0.0f32, f32::max);
    let top = r[0].max(r[1]).max(r[2]).max(1e-3);
    let sum = (sum / k as f32).max(near_clip.max(floor).min(max_level / top));
    Some(std::array::from_fn(|c| match (clipping.clipped[c], clipping.saturated[c]) {
        (true, true) => q[c].max(sum * r[c]),
        (true, false) => sum * r[c],
        _ => q[c],
    }))
}

/// Reconstruct partially clipped channels, with a neutral that is neutral in camera RGB. See
/// [`reconstruct_with`].
pub fn reconstruct(img: &mut Rgb32f, wb: [f32; 3], clip: f32) -> usize {
    reconstruct_with(img, wb, clip, [1.0; 3])
}

/// Reconstruct partially clipped channels. `clip` is the sensor clip level in normalised units (use slightly
/// below 1.0, e.g. 0.99), `wb` the white-balance multipliers that will be applied afterwards, `white` the
/// white-balanced camera RGB that the colour model applied afterwards renders neutral (`[1, 1, 1]` for a model
/// that maps the white-balanced neutral to neutral), scaled to the brightness that `[1, 1, 1]` renders at.
/// Fully clipped pixels become `white` times the brightest plausible level; partly clipped ones take the
/// chromaticity of reliable unclipped content nearby, and that of `white` where there is none.
/// Returns the number of pixels that had at least one clipped channel; the others are left exactly as they were.
pub fn reconstruct_with(img: &mut Rgb32f, wb: [f32; 3], clip: f32, white: [f32; 3]) -> usize {
    rebuild_clipped(img, wb, clip, white, &|_, p| Clipping::from_values(p, clip))
}

/// Bits of a [`reconstruct_masked`] mask entry: which channels of the pixel had a clipped sample (saturated: their
/// value is a lower bound of the true one).
pub const CLIPPED_R: u8 = 1;
pub const CLIPPED_G: u8 = 2;
pub const CLIPPED_B: u8 = 4;
/// Bits of a [`reconstruct_masked`] mask entry for the colours of nearby clipped samples that can affect interpolation.
/// In a directional mask, a clipped sample of the measured colour can still affect the interpolated channels.
pub const FROM_CLIPPED_R: u8 = 8;
pub const FROM_CLIPPED_G: u8 = 16;
pub const FROM_CLIPPED_B: u8 = 32;
/// Bits 6 and 7 of a directional [`reconstruct_masked`] mask entry: the measured colour plus one.
/// Binned and bilinear masks leave this field at zero.
pub const MEASURED_SHIFT: u8 = 6;

pub(super) const CLIPPED_BITS: [u8; 3] = [CLIPPED_R, CLIPPED_G, CLIPPED_B];
pub(super) const FROM_CLIPPED_BITS: [u8; 3] = [FROM_CLIPPED_R, FROM_CLIPPED_G, FROM_CLIPPED_B];

/// [`reconstruct_with`] for a binned image ([`super::RawDevelopExt::develop_binned_masked`]): its values are block
/// means, and `mask` (one entry per pixel, bits [`CLIPPED_R`], [`CLIPPED_G`], [`CLIPPED_B`]) says which colours
/// had a clipped sample in the block. Those channels count as clipped with their mean as the lower bound of their
/// true value, so one clipped specular sample among many unclipped ones neither escapes reconstruction nor
/// (as the block's maximum would) lifts the channel to the clip level, which reads as a coloured speckle (#548).
///
/// For a full-size demosaic ([`super::RawDevelopExt::develop_balanced_masked`]), the mask records the colours
/// of clipped samples that can affect interpolation. In a directional mask, it also records the measured
/// colour so reconstruction can treat only the interpolated channels below clip as unknown. A dependency on the
/// measured colour alone triggers that treatment when every channel is below clip. This preserves partly clipped
/// detail when another channel is already saturated. A bilinear mask marks each interpolated channel separately.
/// Interpolated values at or above clip still count as lower bounds.
/// Reconstruction uses the known channels and limits brightness changes near strongly coloured highlights.
///
/// A mask of the wrong length is ignored (clipping is then told by the values alone).
pub fn reconstruct_masked(img: &mut Rgb32f, wb: [f32; 3], clip: f32, white: [f32; 3], mask: &[u8]) -> usize {
    if mask.len() != img.data.len() {
        return reconstruct_with(img, wb, clip, white);
    }
    rebuild_clipped(img, wb, clip, white, &|i, p| masked(mask[i], p, clip))
}

#[derive(Clone, Copy)]
struct Clipping {
    /// Channels to rebuild, including saturated channels and interpolated values of unknown accuracy.
    clipped: [bool; 3],
    saturated: [bool; 3],
}

impl Clipping {
    fn from_values(p: &[f32; 3], clip: f32) -> Self {
        let saturated = p.map(|v| v >= clip);
        Self { clipped: saturated, saturated }
    }

    fn unknown(self, c: usize) -> bool {
        self.clipped[c] && !self.saturated[c]
    }
}

#[inline]
fn masked(m: u8, p: &[f32; 3], clip: f32) -> Clipping {
    let bit = |b: u8| m & b != 0;
    let saturated = std::array::from_fn(|c| p[c] >= clip || bit(CLIPPED_BITS[c]));
    let mut unknown: [bool; 3] = std::array::from_fn(|c| bit(FROM_CLIPPED_BITS[c]) && !saturated[c]);
    if let Some(own) = ((m >> MEASURED_SHIFT) as usize).checked_sub(1).filter(|&c| !saturated[c]) {
        let from_measured_colour = std::mem::take(&mut unknown[own]);
        if unknown.iter().any(|&u| u) || (from_measured_colour && !saturated.iter().any(|&c| c)) {
            unknown = std::array::from_fn(|c| c != own && !saturated[c]);
        }
    }
    Clipping { clipped: std::array::from_fn(|c| saturated[c] || unknown[c]), saturated }
}

/// Which channels of pixel `i` (values `p`) need to be rebuilt.
trait Clipped: Fn(usize, &[f32; 3]) -> [bool; 3] + Sync {}
impl<F: Fn(usize, &[f32; 3]) -> [bool; 3] + Sync> Clipped for F {}

fn rebuild_clipped(img: &mut Rgb32f, wb: [f32; 3], clip: f32, white: [f32; 3], classify: &(impl Fn(usize, &[f32; 3]) -> Clipping + Sync)) -> usize {
    let (w, h) = (img.width, img.height);
    let clipped = |i, p: &[f32; 3]| classify(i, p).clipped;
    #[cfg(not(target_arch = "wasm32"))]
    let count = img.data.par_iter().enumerate().filter(|(i, p)| clipped(*i, p).iter().any(|&c| c)).count();
    #[cfg(target_arch = "wasm32")]
    let count = img.data.iter().enumerate().filter(|(i, p)| clipped(*i, p).iter().any(|&c| c)).count();
    if count == 0 || w == 0 || h == 0 {
        return 0;
    }
    // a white far from the camera's neutral (or not a number) is a broken colour model: keep the camera's
    let plausible = white.iter().all(|v| v.is_finite() && (1.0 / 16.0..=16.0).contains(v));
    let white = if plausible { white } else { [1.0; 3] };
    let total = white[0] + white[1] + white[2];
    let neutral = [white[0] / total, white[1] / total, white[2] / total];
    // A clipped pixel is never a source, so its chromaticity is always the bilinear sample of the
    // half-resolution estimate: the pyramid starts there, straight from the image.
    let (first, near) = sources(img, wb, clip, &clipped);
    let mut levels = vec![first];
    while let Some(l) = levels.last().filter(|l| l.w > 1 || l.h > 1) {
        let next = pull(l);
        levels.push(next);
    }
    // a clipped pixel samples the cells within one of its own, which are within one of a clipped cell
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let needed = |x: usize, y: usize| (y.saturating_sub(1)..=(y + 1).min(ch - 1)).any(|yy| near.get(yy * cw + x).copied().unwrap_or(false));
    push(&mut levels, [neutral[0], neutral[2], 0.0], Some(&needed));
    // How much reliable content lies around each needed cell: its density at any scale up to [`REACH`] (level
    // i's cells span 2^i cells of level 0). Near reliable content some fine level is dense with it; far from it
    // only the widest one may still hold a little, and the trust fades with that. Kept in the third entry of the
    // level 0 estimates, which clipped pixels sample with their colour.
    let reach = REACH * w.max(h) as f32 / 2.0;
    let widest = (0..levels.len()).find(|&i| (1u64 << i.min(62)) as f32 >= reach).unwrap_or(levels.len().saturating_sub(1));
    if let Some((first, coarser)) = levels.split_first_mut() {
        let support: Vec<(&Level<3>, f32)> = coarser.iter().take(widest).enumerate().map(|(i, l)| (l, (1u64 << (i + 1).min(62)) as f32)).collect();
        let fw = first.w;
        par_rows(&mut first.cells, fw, |y, row| {
            for (x, cell) in row.iter_mut().enumerate() {
                if !needed(x, y) {
                    continue;
                }
                let mut t = 0f32;
                for &(l, scale) in &support {
                    t = t.max(smoothstep(0.0, SUPPORT, sample(l, scale, x, y)[2]));
                    if t >= 1.0 {
                        break;
                    }
                }
                cell[2] = t;
            }
        });
    }
    let Some(field) = levels.first() else { return count };
    // the rim (level 0 of its pyramid holds the estimate), if there is one anywhere at all
    let rims = {
        let mut levels = vec![rim(img, wb, clip, &clipped, &needed)];
        while let Some(l) = levels.last().filter(|l| l.w > 1 || l.h > 1) {
            let next = pull(l);
            levels.push(next);
        }
        push(&mut levels, [0.0; 4], None);
        Some(levels).filter(|levels| levels.last().is_some_and(|top| top.cells.iter().any(|c| c[3] > 0.0)))
    };
    let rim_field = rims.as_ref().and_then(|l| l.first());
    let max_level = wb.iter().cloned().fold(0.0f32, f32::max) * clip;
    let solve = |x: usize, y: usize, p: [f32; 3], clipping: Clipping| -> [f32; 3] {
        let q = [p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]];
        let known_top = || (0..3).filter(|&c| !clipping.unknown(c)).map(|c| q[c]).fold(0.0f32, f32::max);
        // rebuilt with the colour of the reliable content around, and with neutral, blended by how much of it
        // there is and how well the pixel agrees with it (blending the results, not the colours, keeps the
        // estimate consistent with the pixel's own unclipped channels all the way)
        let e = sample(field, 2.0, x, y);
        let borrowed = [e[0], 1.0 - e[0] - e[1], e[1]];
        let edge = rim_field.map(|rim| sample(rim, 4.0, x, y));
        let mut trust = e[2].clamp(0.0, 1.0);
        if trust > 0.0 {
            let mut off = contradiction(p, q, clipping, borrowed, clip);
            if off > AGREE.0 {
                off = off.min(edge.map_or(0.0, |edge| {
                    let s = edge[0] + edge[1] + edge[2];
                    if s > 1e-6 { unlike([edge[0] / s, edge[1] / s, edge[2] / s], borrowed) } else { 0.0 }
                }));
            }
            trust *= 1.0 - smoothstep(AGREE.0, AGREE.1, off);
        }
        // either applies only as far as the pixel lies past the rim nearest its clip (in any channel); nearer, the
        // pixel is rebuilt with the rim's colour
        let at_rim = edge.and_then(|edge| {
            // stops between pixel and rim in each channel; a clipped channel's true value lies at least as far
            // above the rim's as its clip, and no further below (see [`bound`])
            let ratio = (0..3)
                .map(|c| {
                    if clipping.unknown(c) {
                        return 1.0;
                    }
                    let (a, b) = (q[c].max(1e-12), edge[c].max(1e-12));
                    if a >= b {
                        return a / b;
                    }
                    match bound(p, clipping.clipped, clip, c) {
                        w if w <= 0.0 => b / a,
                        w if w >= 1.0 => 1.0,
                        w => (b / a).powf(1.0 - w),
                    }
                })
                .fold(1.0f32, f32::max);
            let past = ratio.log2().min(64.0);
            let k = smoothstep(NEAR_RIM, NEAR_RIM + DEPTH, past);
            let s = edge[0] + edge[1] + edge[2];
            (k < 1.0 && s > 1e-6).then(|| rebuild(p, q, clipping, [edge[0] / s, edge[1] / s, edge[2] / s], clip, max_level)).flatten().map(|a| (a, k))
        });
        let past_rim = |b: [f32; 3]| -> [f32; 3] {
            let Some((a, k)) = at_rim else { return b };
            std::array::from_fn(|c| a[c] + (b[c] - a[c]) * k)
        };
        let local = (trust > 0.0).then(|| rebuild(p, q, clipping, borrowed, clip, max_level)).flatten().map(past_rim);
        let plain = (trust < 1.0).then(|| rebuild(p, q, clipping, neutral, clip, max_level)).flatten().map(past_rim);
        let mut out = match (local, plain) {
            (Some(a), Some(b)) => std::array::from_fn(|c| b[c] + (a[c] - b[c]) * trust),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => {
                let v = known_top().max(max_level);
                white.map(|c| c * v)
            }
        };
        // Fade into the fully clipped neutral on the way from the first channel clipping (no change: continuous
        // with the unclipped neighbours) to the last one clipping (all of it: continuous with the fully clipped
        // ones). The way is measured in stops: how far the rebuilt clipped channels lie above their clip,
        // against how far the lowest unclipped channel still lies below its own.
        let above = (0..3).filter(|&c| clipping.saturated[c]).map(|c| (out[c] / q[c].max(1e-9)).max(1.0).log2()).fold(0.0f32, f32::max);
        let lowest = (0..3).filter(|&c| !clipping.clipped[c]).map(|c| p[c]).fold(f32::MAX, f32::min);
        let below = (clip / lowest.max(1e-6)).log2().max(0.0);
        let way = if above > 0.0 && lowest < clip { above / (above + below) } else { 0.0 };
        // The brightness goes over in the second half of the way, and so does the colour of a strongly coloured
        // pixel; a pale one's goes over from the start ([`PALE`]).
        let u = [out[0] / white[0], out[1] / white[1], out[2] / white[2]];
        let (lo, hi) = (u[0].min(u[1]).min(u[2]), u[0].max(u[1]).max(u[2]));
        let saturation = if hi > 0.0 && hi.is_finite() { 1.0 - lo.max(0.0) / hi } else { 1.0 };
        let start = 0.5 * smoothstep(PALE.0, PALE.1, saturation);
        let (t_level, t_colour) = (smoothstep(0.5, 1.0, way), smoothstep(start, start + 0.5, way));
        if t_colour > 0.0 {
            let level = (out[0] + out[1] + out[2]) / total;
            let full = known_top().max(max_level);
            let v = level + (full - level) * t_level;
            out = std::array::from_fn(|c| out[c] + (white[c] * v - out[c]) * t_colour);
        }
        out
    };
    par_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let p = *px;
            let clipping = classify(y * w + x, &p);
            if !clipping.clipped.iter().any(|&b| b) {
                continue;
            }
            let mut out = solve(x, y, p, clipping);
            if (0..3).any(|c| clipping.unknown(c)) {
                // Rebuilding a highlight from its weakest channel can make its rim too dark.
                // Use the value-based estimate when the mask causes a large brightness drop.
                // Also limit brightening, with more room in pale areas where the mask removes colour fringes.
                let by_value = Clipping::from_values(&p, clip);
                let alt = if by_value.clipped.iter().any(|&b| b) { solve(x, y, p, by_value) } else { [p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]] };
                let luma = |c: [f32; 3]| LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2];
                let ratio = luma(out) / luma(alt).max(1e-9);
                let coloured = || {
                    // how strongly coloured the reliable content around is, and the rim nearest the clip
                    let e = sample(field, 2.0, x, y);
                    let around = [[e[0], 1.0 - e[0] - e[1], e[1]]].into_iter().chain(rim_field.map(|rim| {
                        let e = sample(rim, 4.0, x, y);
                        [e[0], e[1], e[2]]
                    }));
                    around
                        .map(|c| {
                            let u = [c[0] / white[0], c[1] / white[1], c[2] / white[2]];
                            let (lo, hi) = (u[0].min(u[1]).min(u[2]), u[0].max(u[1]).max(u[2]));
                            if hi > 1e-9 && hi.is_finite() { smoothstep(PALE.0, PALE.1, 1.0 - lo.max(0.0) / hi) } else { 0.0 }
                        })
                        .fold(0.0f32, f32::max)
                };
                if !ratio.is_finite() || ratio < CONTINUITY || (ratio > 1.0 / CONTINUITY && ratio.log2() * coloured() > -CONTINUITY.log2()) {
                    out = alt;
                }
            }
            *px = [out[0] / wb[0], out[1] / wb[1], out[2] / wb[2]];
        }
    });
    count
}

#[cfg(test)]
mod tests;
