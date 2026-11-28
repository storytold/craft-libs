//! Hole filling for Spot Healing: patch-based completion, proximity match and texture synthesis.
//!
//! * [`complete`], content-aware fill: the multi-scale "space-time completion" EM scheme of
//!   Y. Wexler, E. Shechtman, M. Irani, *Space-Time Completion of Video*, IEEE TPAMI 2007, with the
//!   nearest-neighbour field computed by C. Barnes, E. Shechtman, A. Finkelstein, D. B. Goldman,
//!   *PatchMatch: A Randomized Correspondence Algorithm for Structural Image Editing*, SIGGRAPH 2009
//!   (random init, propagation, exponentially shrinking random search). Each EM iteration finds, for
//!   every patch overlapping the hole, its most similar fully-known patch, then re-estimates hole
//!   pixels by a similarity-weighted vote of the overlapping matches. Coarse-to-fine over an image
//!   pyramid (the coarsest level is initialised by membrane interpolation), so large holes stay cheap.
//! * `best_offset`, proximity match: the single displacement whose surrounding ring best matches the
//!   ring around the hole (sum of squared differences), i.e. an automatic Healing Brush source.
//! * `synthesize`, create texture: patch-based texture synthesis in the style of A. Efros,
//!   W. Freeman, *Image Quilting for Texture Synthesis and Transfer*, SIGGRAPH 2001 (blocks picked from
//!   random candidates by overlap error, feathered overlap instead of a min-cut seam).
//!
//! Ported from PhotoCraft's `algo::inpaint` (MIT OR Apache-2.0); its proximity match and texture
//! synthesis are left out.
//!
//! All functions take interleaved `w × h × ch` normalised floats (any colour model/depth) and a
//! `w × h` hole mask, and return a full buffer equal to the input outside the hole. They are
//! deterministic (fixed-seed hash RNG, results independent of thread count).

use crate::inpaint::Cancelled;
use crate::inpaint::membrane::membrane_fill;
use std::sync::atomic::{AtomicBool, Ordering};

/// Below this many grid cells a level runs single-threaded (thread hand-off costs more than it saves).
#[cfg(not(target_arch = "wasm32"))]
const PAR_MIN: usize = 128 * 128;

/// SplitMix64: tiny deterministic RNG.
#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform integer in `lo..=hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

/// Parameters for [`complete`].
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteParams {
    /// Patch radius (patches are `2r+1` square).
    pub patch_radius: usize,
    /// EM iterations at the coarsest level (finer levels use fewer, down to `min_em_iters`).
    pub em_iters: usize,
    pub min_em_iters: usize,
    /// PatchMatch passes per EM iteration.
    pub pm_iters: usize,
    /// Levels where the hole is wider than this (in that level's pixels) only *refine* the upsampled
    /// match field (one local PatchMatch pass and one vote), which keeps big holes tractable: the
    /// structure is decided at coarse scale, fine levels just add detail.
    pub refine_extent: usize,
    pub seed: u64,
}

impl Default for CompleteParams {
    fn default() -> Self {
        Self { patch_radius: 3, em_iters: 8, min_em_iters: 2, pm_iters: 2, refine_extent: 96, seed: 1 }
    }
}

/// One pyramid level.
#[derive(Clone)]
struct Level {
    w: usize,
    h: usize,
    img: Vec<f32>,
    hole: Vec<bool>,
}

fn downsample(l: &Level, ch: usize) -> Level {
    let (w, h) = (l.w.div_ceil(2), l.h.div_ceil(2));
    let mut img = vec![0.0f32; w * h * ch];
    let mut hole = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut n = 0.0f32;
            let o = (y * w + x) * ch;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                if sx >= l.w || sy >= l.h {
                    continue;
                }
                let si = sy * l.w + sx;
                if l.hole[si] {
                    hole[y * w + x] = true;
                } else {
                    for c in 0..ch {
                        img[o + c] += l.img[si * ch + c];
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                for c in 0..ch {
                    img[o + c] /= n;
                }
            }
        }
    }
    Level { w, h, img, hole }
}

// Search offsets add/subtract a full image dimension in i32. Summed-area queries
// add two u32 entries before subtracting. Both the padded table and the larger
// per-pixel nearest-neighbour/valid-coordinate arrays must fit a Vec.
fn supported_grid(w: usize, h: usize) -> bool {
    let max_dimension = (i32::MAX / 2) as usize;
    w <= max_dimension
        && h <= max_dimension
        && w.checked_mul(h).is_some_and(|n| n <= (u32::MAX / 2) as usize)
        && w.checked_mul(h).and_then(|n| n.checked_mul(std::mem::size_of::<(i32, i32)>())).is_some_and(|bytes| bytes <= isize::MAX as usize)
        && w.checked_add(1)
            .and_then(|pw| h.checked_add(1).and_then(|ph| pw.checked_mul(ph)))
            .and_then(|n| n.checked_mul(std::mem::size_of::<u32>()))
            .is_some_and(|bytes| bytes <= isize::MAX as usize)
}

/// Reserve the already counted source centres exactly, then retain row-major order.
fn source_centres(w: usize, valid: &[bool], count: usize, check: &impl Fn() -> Result<(), Cancelled>) -> Result<Vec<(i32, i32)>, Cancelled> {
    let mut centres = Vec::with_capacity(count);
    for (i, &is_valid) in valid.iter().enumerate() {
        if i % (w * 32) == 0 {
            check()?;
        }
        if is_valid {
            centres.push(((i % w) as i32, (i / w) as i32));
        }
    }
    Ok(centres)
}

/// Summed-area table of a boolean grid (`(w+1) × (h+1)`).
fn integral(w: usize, h: usize, m: &[bool]) -> Vec<u32> {
    let mut s = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(m[y * w + x]);
            s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + row;
        }
    }
    s
}

/// Count of set cells in the clipped window `[x0, x1) × [y0, y1)`.
fn window_count(s: &[u32], gw: usize, gh: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> u32 {
    let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
    let (x1, y1) = ((x1.max(0) as usize).min(gw), (y1.max(0) as usize).min(gh));
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let w1 = gw + 1;
    s[y1 * w1 + x1] + s[y0 * w1 + x0] - s[y0 * w1 + x1] - s[y1 * w1 + x0]
}

fn hole_bbox(w: usize, h: usize, hole: &[bool]) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
                });
            }
        }
    }
    b
}

/// Per-level PatchMatch/EM state.
struct Solver<'a> {
    w: usize,
    h: usize,
    ch: usize,
    r: i32,
    img: &'a mut Vec<f32>,
    hole: &'a [bool],
    /// Valid source centres: the whole patch is inside the grid and contains no hole pixel.
    valid: Vec<bool>,
    valid_list: Vec<(i32, i32)>,
    /// Target centres (patch overlaps the hole), row-major.
    targets: Vec<bool>,
    /// Cancellation: checked per row band; a cancelled solve's output is discarded.
    ctl: &'a (dyn Fn() -> bool + Sync),
}

impl Solver<'_> {
    #[inline]
    fn is_valid(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.valid[y as usize * self.w + x as usize]
    }

    /// SSD between the (clipped) target patch at `t` and the source patch at `s`, per sample.
    fn dist(&self, t: (i32, i32), s: (i32, i32), cutoff: f32) -> f32 {
        let (w, h, ch, r) = (self.w as i32, self.h as i32, self.ch, self.r);
        let diameter = f64::from(2 * r + 1);
        let full = (diameter * diameter) as f32 * ch as f32;
        let mut sum = 0.0f32;
        let mut n = 0usize;
        for dy in -r..=r {
            let ty = t.1 + dy;
            if ty < 0 || ty >= h {
                continue;
            }
            let sy = s.1 + dy;
            for dx in -r..=r {
                let tx = t.0 + dx;
                if tx < 0 || tx >= w {
                    continue;
                }
                let ti = (ty as usize * self.w + tx as usize) * ch;
                let si = (sy as usize * self.w + (s.0 + dx) as usize) * ch;
                for c in 0..ch {
                    let d = self.img[ti + c] - self.img[si + c];
                    sum += d * d;
                }
                n += ch;
            }
            // Early out: the mean can only be ≥ sum / (full patch size).
            if sum > cutoff * full {
                return f32::INFINITY;
            }
        }
        if n == 0 { f32::INFINITY } else { sum / n as f32 }
    }

    /// One PatchMatch pass over all target rows, in parallel bands (each band propagates within itself).
    fn patchmatch(&self, nnf: &mut [(i32, i32)], cost: &mut [f32], pass: usize, seed: u64, search_radius: i32) {
        const BAND: usize = 16;
        let w = self.w;
        let reverse = pass % 2 == 1;
        let band_fn = |bi: usize, nn: &mut [(i32, i32)], co: &mut [f32]| {
            if (self.ctl)() {
                return;
            }
            let mut rng = Rng::new(seed ^ (bi as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ ((pass as u64) << 40));
            let y0 = bi * BAND;
            let rows = nn.len() / w;
            let step: i32 = if reverse { -1 } else { 1 };
            for ry in 0..rows {
                let ly = if reverse { rows - 1 - ry } else { ry };
                let y = y0 + ly;
                for rx in 0..w {
                    let x = if reverse { w - 1 - rx } else { rx };
                    let li = ly * w + x;
                    if !self.targets[y * w + x] {
                        continue;
                    }
                    let t = (x as i32, y as i32);
                    let (mut best, mut bc) = (nn[li], co[li]);
                    // Propagation from the already-visited neighbours (within this band).
                    let nx = x as i32 - step;
                    if nx >= 0 && (nx as usize) < w {
                        let c = nn[ly * w + nx as usize];
                        let cand = (c.0 + step, c.1);
                        if self.is_valid(cand.0, cand.1) && cand != best {
                            let d = self.dist(t, cand, bc);
                            if d < bc {
                                (best, bc) = (cand, d);
                            }
                        }
                    }
                    let nly = ly as i32 - step;
                    if nly >= 0 && (nly as usize) < rows {
                        let c = nn[nly as usize * w + x];
                        let cand = (c.0, c.1 + step);
                        if self.is_valid(cand.0, cand.1) && cand != best {
                            let d = self.dist(t, cand, bc);
                            if d < bc {
                                (best, bc) = (cand, d);
                            }
                        }
                    }
                    // Random search in exponentially shrinking windows.
                    let mut rad = search_radius;
                    while rad >= 1 {
                        let cand = (best.0 + rng.range(-rad, rad), best.1 + rng.range(-rad, rad));
                        if self.is_valid(cand.0, cand.1) && cand != best {
                            let d = self.dist(t, cand, bc);
                            if d < bc {
                                (best, bc) = (cand, d);
                            }
                        }
                        rad /= 2;
                    }
                    nn[li] = best;
                    co[li] = bc;
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if w * self.h >= PAR_MIN {
            use rayon::prelude::*;
            nnf.par_chunks_mut(BAND * w).zip(cost.par_chunks_mut(BAND * w)).enumerate().for_each(|(bi, (nn, co))| band_fn(bi, nn, co));
            return;
        }
        for (bi, (nn, co)) in nnf.chunks_mut(BAND * w).zip(cost.chunks_mut(BAND * w)).enumerate() {
            band_fn(bi, nn, co);
        }
    }

    /// Re-estimate hole pixels by a similarity-weighted vote of all overlapping patch matches.
    fn vote(&mut self, nnf: &[(i32, i32)], cost: &[f32]) {
        let (w, h, ch, r) = (self.w, self.h, self.ch, self.r);
        // σ² from the 75th percentile of match costs (Wexler et al. use a robust scale like this).
        let mut cs: Vec<f32> = cost.iter().zip(&self.targets).filter(|(c, t)| **t && c.is_finite()).map(|(c, _)| *c).collect();
        let sigma2 = if cs.is_empty() {
            1.0
        } else {
            let k = (cs.len() * 3 / 4).min(cs.len() - 1);
            let (_, v, _) = cs.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
            v.max(1e-6)
        };
        let img = &*self.img;
        let row = |y: usize| -> Vec<(usize, Vec<f32>)> {
            let mut out = Vec::new();
            if (self.ctl)() {
                return out;
            }
            for x in 0..w {
                if !self.hole[y * w + x] {
                    continue;
                }
                let mut acc = vec![0.0f32; ch];
                let mut wsum = 0.0f32;
                for dy in -r..=r {
                    let ty = y as i32 + dy;
                    if ty < 0 || ty as usize >= h {
                        continue;
                    }
                    for dx in -r..=r {
                        let tx = x as i32 + dx;
                        if tx < 0 || tx as usize >= w {
                            continue;
                        }
                        let ti = ty as usize * w + tx as usize;
                        if !self.targets[ti] || !cost[ti].is_finite() {
                            continue;
                        }
                        let s = nnf[ti];
                        let (sx, sy) = (s.0 - dx, s.1 - dy);
                        let wt = (-cost[ti] / (2.0 * sigma2)).exp().max(1e-8);
                        let si = (sy as usize * w + sx as usize) * ch;
                        for c in 0..ch {
                            acc[c] += img[si + c] * wt;
                        }
                        wsum += wt;
                    }
                }
                if wsum > 0.0 {
                    acc.iter_mut().for_each(|v| *v /= wsum);
                    out.push((x, acc));
                }
            }
            out
        };
        #[cfg(not(target_arch = "wasm32"))]
        let rows: Vec<Vec<(usize, Vec<f32>)>> = if w * h >= PAR_MIN {
            use rayon::prelude::*;
            (0..h).into_par_iter().map(row).collect()
        } else {
            (0..h).map(row).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let rows: Vec<Vec<(usize, Vec<f32>)>> = (0..h).map(row).collect();
        for (y, r) in rows.into_iter().enumerate() {
            for (x, v) in r {
                self.img[(y * w + x) * ch..(y * w + x + 1) * ch].copy_from_slice(&v);
            }
        }
    }
}

/// Content-aware completion of `hole` (see module docs). Returns `None` when there is no fully-known
/// patch to copy from (the caller can fall back to [`membrane_fill`]).
pub fn complete(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], p: &CompleteParams) -> Option<Vec<f32>> {
    // Never cancelled, so never `Err`.
    complete_with(w, h, ch, img, hole, p, &|| false).unwrap_or(None)
}

/// [`complete`] that can be cancelled (checked per EM iteration, PatchMatch pass and row band)
/// (`cancelled` is polled per EM iteration, PatchMatch pass and row band): `Err(Cancelled)` once it
/// returned `true`. `Ok(None)` also for mismatched buffer sizes or `ch == 0`.
pub fn complete_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    p: &CompleteParams,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Vec<f32>>, Cancelled> {
    // The workers poll `cancelled` too and can only stop early, not return an error. One `true` from it,
    // wherever it was seen, is kept, so the next check out here ends the call and a half-finished pass
    // is never returned as a result.
    let seen = AtomicBool::new(false);
    let cancelled: &(dyn Fn() -> bool + Sync) = &|| {
        if !seen.load(Ordering::Relaxed) && cancelled() {
            seen.store(true, Ordering::Relaxed);
        }
        seen.load(Ordering::Relaxed)
    };
    let check = || if cancelled() { Err(Cancelled) } else { Ok(()) };
    if ch == 0 || Some(hole.len()) != w.checked_mul(h) || Some(img.len()) != hole.len().checked_mul(ch) {
        return Ok(None);
    }
    // No pixels (a zero width or height, whatever the other is): nothing to scan and nothing to fill.
    if hole.is_empty() {
        return Ok(Some(img.to_vec()));
    }
    if !supported_grid(w, h) {
        check()?;
        return Ok(None);
    }
    let Some((bx0, by0, bx1, by1)) = hole_bbox(w, h, hole) else { return Ok(Some(img.to_vec())) };
    let r = p.patch_radius.max(1);
    // A patch wider than the image has no place to be copied from, which is the `None` the search below
    // would reach anyway. Said here, before `2 * r + 1` and `r as i32` can overflow on an absurd radius.
    if r > (w.min(h) - 1) / 2 {
        check()?;
        return Ok(None);
    }
    let psz = 2 * r + 1;
    // Pyramid: shrink until the hole is a few patches across.
    let mut levels = vec![Level { w, h, img: img.to_vec(), hole: hole.to_vec() }];
    let mut ext = (bx1 - bx0).max(by1 - by0);
    let mut exts = vec![ext];
    while let Some(l) = levels.last() {
        check()?;
        if ext <= 2 * psz || l.w / 2 < 3 * psz || l.h / 2 < 3 * psz {
            break;
        }
        let d = downsample(l, ch);
        levels.push(d);
        ext = ext.div_ceil(2);
        exts.push(ext);
    }
    let nlev = levels.len();
    let mut nnf: Vec<(i32, i32)> = Vec::new();
    let mut prev: Option<(usize, usize, Vec<f32>)> = None;
    for li in (0..nlev).rev() {
        check()?;
        let Level { w: lw, h: lh, img: mut limg, hole: lhole } = levels[li].clone();
        // Initialise the hole: membrane at the coarsest level, upsampled result elsewhere.
        match &prev {
            None => limg = membrane_fill(lw, lh, ch, &limg, &lhole),
            Some((pw, ph, pimg)) => {
                for y in 0..lh {
                    for x in 0..lw {
                        if lhole[y * lw + x] {
                            let (sx, sy) = ((x / 2).min(pw - 1), (y / 2).min(ph - 1));
                            let (d, s) = ((y * lw + x) * ch, (sy * pw + sx) * ch);
                            limg[d..d + ch].copy_from_slice(&pimg[s..s + ch]);
                        }
                    }
                }
            }
        }
        let ri = r as i32;
        let hs = integral(lw, lh, &lhole);
        let mut valid = vec![false; lw * lh];
        let mut valid_count = 0;
        let mut targets = vec![false; lw * lh];
        for y in 0..lh as i32 {
            if y % 32 == 0 {
                check()?;
            }
            for x in 0..lw as i32 {
                let cnt = window_count(&hs, lw, lh, x - ri, y - ri, x + ri + 1, y + ri + 1);
                let inside = x >= ri && y >= ri && x + ri < lw as i32 && y + ri < lh as i32;
                let i = y as usize * lw + x as usize;
                if inside && cnt == 0 {
                    valid[i] = true;
                    valid_count += 1;
                }
                targets[i] = cnt > 0;
            }
        }
        if valid_count == 0 {
            if li == 0 {
                return Ok(None);
            }
            prev = Some((lw, lh, limg));
            nnf.clear();
            continue;
        }
        let valid_list = source_centres(lw, &valid, valid_count, &check)?;
        let mut solver = Solver { w: lw, h: lh, ch, r: ri, img: &mut limg, hole: &lhole, valid, valid_list, targets, ctl: cancelled };
        // NNF init: upsampled from the coarser level where possible, random otherwise.
        let mut rng = Rng::new(p.seed ^ (li as u64) << 20);
        let pw_prev = prev.as_ref().map_or(0, |p| p.0);
        let old = std::mem::take(&mut nnf);
        let mut new_nnf = vec![(0i32, 0i32); lw * lh];
        for y in 0..lh {
            if y % 32 == 0 {
                check()?;
            }
            for x in 0..lw {
                if !solver.targets[y * lw + x] {
                    continue;
                }
                let mut cand = None;
                if !old.is_empty() && pw_prev > 0 {
                    let pi = (y / 2) * pw_prev + (x / 2);
                    if let Some(&(sx, sy)) = old.get(pi) {
                        let c = (sx * 2 + (x % 2) as i32, sy * 2 + (y % 2) as i32);
                        if solver.is_valid(c.0, c.1) {
                            cand = Some(c);
                        }
                    }
                }
                new_nnf[y * lw + x] = cand.unwrap_or_else(|| solver.valid_list[(rng.next() % solver.valid_list.len() as u64) as usize]);
            }
        }
        nnf = new_nnf;
        let refine_only = li + 1 < nlev && exts[li] > p.refine_extent;
        let em = if li + 1 == nlev {
            p.em_iters
        } else if refine_only {
            1
        } else {
            (p.em_iters >> (nlev - 1 - li)).max(p.min_em_iters)
        };
        let pm_iters = if refine_only { 1 } else { p.pm_iters };
        let full_search = (lw.max(lh)) as i32;
        let mut cost = vec![f32::INFINITY; lw * lh];
        for it in 0..em {
            // Costs change as hole pixels are re-estimated: refresh before each PatchMatch.
            for (i, c) in cost.iter_mut().enumerate() {
                if i % (lw * 32).max(1) == 0 {
                    check()?;
                }
                if solver.targets[i] {
                    *c = solver.dist(((i % lw) as i32, (i / lw) as i32), nnf[i], f32::INFINITY);
                }
            }
            // Wide random search on coarse levels / first iteration, local refinement afterwards.
            let radius = if refine_only {
                2
            } else if li + 1 == nlev || it == 0 {
                full_search
            } else {
                (4 * psz).min(full_search as usize) as i32
            };
            for pass in 0..pm_iters {
                check()?;
                solver.patchmatch(&mut nnf, &mut cost, pass + it * pm_iters, p.seed.wrapping_add((li * 1000 + it) as u64), radius);
            }
            check()?;
            solver.vote(&nnf, &cost);
            check()?;
        }
        prev = Some((lw, lh, limg));
    }
    Ok(prev.map(|(_, _, img)| img))
}

/// Proximity match: the displacement `(dx, dy)` (within `max_radius`) whose surroundings best match the
/// ring of `ring` pixels around the hole, with the displaced hole fully inside the grid and outside the
#[cfg(test)]
mod tests {
    use super::*;

    /// Smooth gradient plus deterministic fine noise.
    fn texture(w: usize, h: usize) -> Vec<f32> {
        let mut rng = Rng::new(7);
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let n = (rng.next() % 1000) as f32 / 1000.0 * 0.04 - 0.02;
                [0.3 + 0.004 * x + n, 0.5 + 0.002 * y + n, 0.4 + n]
            })
            .collect()
    }

    fn disc(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Vec<bool> {
        (0..w * h).map(|i| ((i % w) as f32 - cx).hypot((i / w) as f32 - cy) < r).collect()
    }

    fn hole_rmse(a: &[f32], b: &[f32], hole: &[bool], ch: usize) -> f32 {
        let (mut e, mut n) = (0.0f32, 0);
        for (i, hh) in hole.iter().enumerate() {
            if *hh {
                for c in 0..ch {
                    e += (a[i * ch + c] - b[i * ch + c]).powi(2);
                    n += 1;
                }
            }
        }
        (e / n as f32).sqrt()
    }

    #[test]
    fn grid_limits_reject_narrowing_and_integral_overflow_without_allocating() {
        assert!(supported_grid(32, 32));
        assert!(supported_grid(16_384, 8_192));
        assert_eq!(supported_grid(20_000, 20_000), usize::BITS > 32);
        assert!(!supported_grid(i32::MAX as usize, 1));
        assert!(!supported_grid(65_536, 65_536));
        assert!(!supported_grid(usize::MAX, usize::MAX));
    }

    #[test]
    fn completion_fills_blemish_from_texture() {
        let (w, h, ch) = (96, 96, 3);
        let clean = texture(w, h);
        let hole = disc(w, h, 48.0, 48.0, 9.0);
        let mut dirty = clean.clone();
        for (i, hh) in hole.iter().enumerate() {
            if *hh {
                dirty[i * ch..i * ch + ch].copy_from_slice(&[0.02, 0.02, 0.02]);
            }
        }
        let out = complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap();
        let before = hole_rmse(&dirty, &clean, &hole, ch);
        let after = hole_rmse(&out, &clean, &hole, ch);
        assert!(after < 0.06 && after < before * 0.15, "rmse before {before} after {after}");
        // Outside the hole untouched.
        assert!(out.iter().zip(&dirty).zip(hole.iter().flat_map(|h| [*h; 3])).all(|((a, b), h)| h || a == b));
        // Deterministic.
        assert_eq!(out, complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap());
    }

    #[test]
    fn completion_continues_structure() {
        // Vertical stripes: the filled hole should keep the stripes (not a blur).
        let (w, h, ch) = (80, 80, 1);
        let clean: Vec<f32> = (0..w * h).map(|i| if (i % w / 4) % 2 == 0 { 0.2 } else { 0.8 }).collect();
        let hole = disc(w, h, 40.0, 40.0, 7.0);
        let dirty: Vec<f32> = clean.iter().zip(&hole).map(|(c, h)| if *h { 0.5 } else { *c }).collect();
        let out = complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap();
        let err = hole_rmse(&out, &clean, &hole, ch);
        assert!(err < 0.15, "{err}");
    }
}
