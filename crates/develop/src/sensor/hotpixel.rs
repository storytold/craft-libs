//! Hot pixels on sensors that don't list their defects.
//!
//! A hot pixel is a single photosite that reads far too bright: a leaky or stuck cell, most visible
//! at high ISO and in dark areas. Demosaicing turns it into a small saturated dot of its colour
//! (red, green or blue), and colour noise reduction can only make that dot fainter, since the photosite
//! is too bright, not just the wrong colour. DNGs list such pixels in their opcodes and Panasonic marks
//! them in the data (both repaired elsewhere); other raws, such as Sony ARWs, carry no list.
//!
//! [`RawImage::suppress_hot_pixels`] finds them in the mosaic, before demosaicing:
//!
//! 1. **Noise model.** On a sparse sample of the image, each photosite is compared with the median of
//!    its same-colour neighbours. The spread of that difference, per brightness, gives the noise
//!    σ(level) as `σ² = a · level + b` (shot noise plus read noise), fitted to the image itself.
//! 2. **Candidates.** A photosite more than [`K_HOT`] σ above *every* same-colour neighbour (the
//!    nearest eight, two photosites away on a Bayer grid, the nearest greens for green).
//! 3. **Real light is kept.** A star, a spark or a specular point lights more than one photosite: its
//!    direct neighbours are raised too. A candidate with at least [`MIN_LIT`] neighbours more than
//!    [`K_SPOT`] σ above their own surroundings is left alone. Hot photosites also come in adjacent
//!    pairs, so a single lit neighbour doesn't count.
//! 4. The rest are replaced by the median of their same-colour neighbours.
//!
//! If more than a share of the photosites would be replaced (by default [`HOT_PIXEL_MAX_SHARE`]; the caller may
//! choose another), the image doesn't fit the model (a synthetic pattern, a star field shot through a pinhole) and
//! nothing is changed. Real light kept in step 3 doesn't count: a dense star field still has its hot pixels removed.

use crate::sensor::{RawData, RawImage};
use rayon::prelude::*;

/// A candidate stands more than this many σ above the brightest of its same-colour neighbours (pure noise
/// hardly reaches 4.5 σ above the brightest of eight; measured on two ILCE-7M4 files at ISO 3200 and 25600).
const K_HOT: f32 = 5.0;
/// A direct neighbour this many σ above its own surroundings means the light is real.
const K_SPOT: f32 = 4.0;
/// ...and it takes this many lit neighbours (a hot pair has one).
const MIN_LIT: usize = 2;
/// Every this many rows and columns are sampled for the noise model.
const SAMPLE_STEP: usize = 5;
/// Brightness bins of the noise model (on a square-root scale), and the samples a bin needs.
const BINS: usize = 32;
const MIN_BIN_SAMPLES: usize = 64;
/// Photosites this close to the active area's edge are left alone (their neighbourhood is cut).
const MARGIN: usize = 4;
/// More replaced than this share of the photosites and the result is discarded (unless the caller chooses another).
pub const HOT_PIXEL_MAX_SHARE: f64 = 2e-3;

/// For one position in the CFA pattern: the offsets of the nearest same-colour photosites (at most 8,
/// within two photosites) and of the 8 direct neighbours.
#[derive(Clone)]
struct Phase {
    same: Vec<(isize, isize)>,
    around: [(isize, isize); 8],
}

fn phases(cfa: &crate::sensor::Cfa) -> Vec<Phase> {
    let around = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    let (pw, ph) = (cfa.width, cfa.height);
    (0..ph)
        .flat_map(|j| (0..pw).map(move |i| (i, j)))
        .map(|(i, j)| {
            // anchored far enough from 0 that i + dx stays positive
            let (x0, y0) = (i + 2 * pw, j + 2 * ph);
            let c = cfa.color_at(x0, y0);
            let mut same: Vec<(isize, isize)> = (-2isize..=2)
                .flat_map(|dy| (-2isize..=2).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| (dx, dy) != (0, 0) && cfa.color_at((x0 as isize + dx) as usize, (y0 as isize + dy) as usize) == c)
                .collect();
            same.sort_by_key(|&(dx, dy)| dx * dx + dy * dy);
            same.truncate(8);
            Phase { same, around }
        })
        .collect()
}

/// The noise σ at a brightness (in raw counts above black): `sqrt(a · level + b)`.
#[derive(Clone, Copy, Debug)]
struct Noise {
    a: f32,
    b: f32,
}

impl Noise {
    fn sigma(&self, level: f32) -> f32 {
        (self.a * level.max(0.0) + self.b).max(1.0).sqrt()
    }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

/// The sample view the passes work on: the u16 mosaic, its width, the active area, the CFA phases.
struct View<'a> {
    d: &'a [u16],
    w: usize,
    pw: usize,
    ph: usize,
    phases: &'a [Phase],
}

impl View<'_> {
    /// Value at full-image `(x, y)`; callers keep `(x, y)` inside the image.
    #[inline]
    fn at(&self, x: isize, y: isize) -> f32 {
        self.d[y as usize * self.w + x as usize] as f32
    }
    #[inline]
    fn phase(&self, x: usize, y: usize) -> &Phase {
        &self.phases[(y % self.ph) * self.pw + (x % self.pw)]
    }
    /// The same-colour neighbours' values around `(x, y)` into `buf`, and their maximum.
    #[inline]
    fn neighbours(&self, x: usize, y: usize, buf: &mut Vec<f32>) -> f32 {
        buf.clear();
        let mut max = f32::MIN;
        for &(dx, dy) in &self.phase(x, y).same {
            let v = self.at(x as isize + dx, y as isize + dy);
            max = max.max(v);
            buf.push(v);
        }
        max
    }
}

/// Remove Hot Pixels on the mosaic.
pub trait HotPixels {
    /// Replace isolated hot photosites by the median of their same-colour neighbours (see the module docs).
    /// Returns how many were replaced. Only single-plane integer CFA data is examined.
    fn suppress_hot_pixels(&mut self) -> usize;

    /// [`Self::suppress_hot_pixels`], giving up when more than `max_share` of the photosites (0..1) would be replaced.
    fn suppress_hot_pixels_with(&mut self, max_share: f64) -> usize;
}

impl HotPixels for RawImage {
    fn suppress_hot_pixels(&mut self) -> usize {
        self.suppress_hot_pixels_with(HOT_PIXEL_MAX_SHARE)
    }

    fn suppress_hot_pixels_with(&mut self, max_share: f64) -> usize {
        let Some(cfa) = self.cfa.clone() else { return 0 };
        let (w, h, a) = (self.width, self.height, self.active_area);
        let black = self.black.mean();
        let RawData::U16(d) = &mut self.data else { return 0 };
        if self.cpp != 1
            || !cfa.valid()
            || w.checked_mul(h) != Some(d.len())
            || a.width < 4 * MARGIN
            || a.height < 4 * MARGIN
            || a.x + a.width > w
            || a.y + a.height > h
            || !black.is_finite()
        {
            return 0;
        }
        let phases = phases(&cfa);
        if phases.iter().any(|p| p.same.len() < 4) {
            return 0;
        }
        let (x0, x1, y0, y1) = (a.x + MARGIN, a.x + a.width - MARGIN, a.y + MARGIN, a.y + a.height - MARGIN);
        let range = (self.white.iter().copied().fold(0.0f32, f32::max) - black).max(1.0);
        let fixes = {
            let v = View { d: d.as_slice(), w, pw: cfa.width, ph: cfa.height, phases: &phases };
            let Some(noise) = noise_model(&v, (x0, x1, y0, y1), black, range) else { return 0 };
            let candidates = candidates(&v, (x0, x1, y0, y1), black, noise);
            let mut buf = Vec::with_capacity(8);
            let n_cand = candidates.len();
            let kept: Vec<_> = candidates.into_iter().filter(|&(x, y, _)| !lit_around(&v, x, y, black, noise, &mut buf)).collect();
            // (real light kept above doesn't count against the limit)
            let sites = (x1 - x0) * (y1 - y0);
            let limit = max_share * sites as f64;
            if limit.is_nan() || kept.len() as f64 > limit {
                return 0;
            }
            if std::env::var_os("LIGHTCRAFT_PROFILE").is_some() {
                eprintln!(
                    "  hot pixels: noise σ² = {:.3}·L + {:.1} (σ at 0: {:.1}, at 1 %: {:.1}), {n_cand} candidates, {} real light",
                    noise.a,
                    noise.b,
                    noise.sigma(0.0),
                    noise.sigma(0.01 * range),
                    n_cand - kept.len()
                );
                // the strongest excesses below the threshold, to see how far the dots are
                let mut hist = [0usize; 12];
                let mut buf2 = Vec::with_capacity(8);
                for y in (y0..y1).step_by(1) {
                    for x in x0..x1 {
                        let s = v.at(x as isize, y as isize);
                        let max = v.neighbours(x, y, &mut buf2);
                        if s <= max {
                            continue;
                        }
                        let m = median(&mut buf2);
                        let k = ((s - max) / noise.sigma(m - black)) as usize;
                        hist[k.min(11)] += 1;
                    }
                }
                eprintln!("  hot pixels: photosites by (s - max neighbour) / σ, 0..11+: {hist:?}");
                // very bright photosites in dark surroundings, whatever the verdict
                let mut bright = 0;
                let mut buf4 = Vec::with_capacity(8);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let s = v.at(x as isize, y as isize) - black;
                        if s < 1500.0 {
                            continue;
                        }
                        v.neighbours(x, y, &mut buf4);
                        let m = median(&mut buf4) - black;
                        if m > 300.0 {
                            continue;
                        }
                        bright += 1;
                        if bright <= 4 {
                            eprintln!(
                                "  bright at ({x}, {y}), colour {}, s {s:.0}, median {m:.0}, lit {}:",
                                cfa.color_at(x, y),
                                lit_around(&v, x, y, black, noise, &mut buf4)
                            );
                            for dy in -3isize..=3 {
                                let row: Vec<String> =
                                    (-3isize..=3).map(|dx| format!("{:6.0}", v.at(x as isize + dx, y as isize + dy) - black)).collect();
                                eprintln!("    {}", row.join(" "));
                            }
                        }
                    }
                }
                eprintln!("  bright photosites in dark surroundings: {bright}");
                // a few candidates rejected as real light: their 5 × 5 neighbourhood above black
                let mut shown = 0;
                let mut buf3 = Vec::with_capacity(8);
                for y in y0..y1 {
                    for x in x0..x1 {
                        if shown >= 6 {
                            break;
                        }
                        let s = v.at(x as isize, y as isize);
                        let max = v.neighbours(x, y, &mut buf3);
                        if s <= max {
                            continue;
                        }
                        let m = median(&mut buf3);
                        if s - max > 8.0 * noise.sigma(m - black) && lit_around(&v, x, y, black, noise, &mut buf3) {
                            shown += 1;
                            eprintln!("  rejected at ({x}, {y}), colour {}:", cfa.color_at(x, y));
                            for dy in -2isize..=2 {
                                let row: Vec<String> =
                                    (-2isize..=2).map(|dx| format!("{:6.0}", v.at(x as isize + dx, y as isize + dy) - black)).collect();
                                eprintln!("    {}", row.join(" "));
                            }
                        }
                    }
                }
            }
            kept
        };
        for &(x, y, r) in &fixes {
            d[y * w + x] = r;
        }
        fixes.len()
    }
}

/// Fit `σ² = a · level + b` to the spread of (photosite − median of its same-colour neighbours) on a sample.
fn noise_model(v: &View<'_>, (x0, x1, y0, y1): (usize, usize, usize, usize), black: f32, range: f32) -> Option<Noise> {
    let bin_of = |level: f32| (((level.max(0.0) / range).sqrt() * BINS as f32) as usize).min(BINS - 1);
    let rows: Vec<usize> = (y0..y1).step_by(SAMPLE_STEP).collect();
    let per_row: Vec<Vec<(usize, f32, f32)>> = rows
        .par_iter()
        .map(|&y| {
            let mut buf = Vec::with_capacity(8);
            (x0..x1)
                .step_by(SAMPLE_STEP)
                .map(|x| {
                    v.neighbours(x, y, &mut buf);
                    let m = median(&mut buf);
                    let level = m - black;
                    (bin_of(level), level, (v.at(x as isize, y as isize) - m).abs())
                })
                .collect()
        })
        .collect();
    let mut bins: Vec<(Vec<f32>, f64, usize)> = vec![(Vec::new(), 0.0, 0); BINS];
    for (b, level, dev) in per_row.into_iter().flatten() {
        bins[b].0.push(dev);
        bins[b].1 += level as f64;
        bins[b].2 += 1;
    }
    // per bin: mean level and σ from the median absolute deviation (robust to the hot pixels themselves)
    let pts: Vec<(f64, f64, f64)> = bins
        .into_iter()
        .filter(|(dev, _, n)| *n >= MIN_BIN_SAMPLES && !dev.is_empty())
        .map(|(mut dev, sum, n)| {
            let s = 1.4826 * median(&mut dev) as f64;
            (sum / n as f64, s * s, n as f64)
        })
        .collect();
    if pts.is_empty() {
        return None;
    }
    // weighted least squares for σ² = a·L + b, both kept ≥ 0
    let (sw, sl, sv) = pts.iter().fold((0.0, 0.0, 0.0), |(w, l, s), &(lv, var, n)| (w + n, l + n * lv, s + n * var));
    let (ml, mv) = (sl / sw, sv / sw);
    let (cov, varl) = pts.iter().fold((0.0, 0.0), |(c, vl), &(lv, var, n)| (c + n * (lv - ml) * (var - mv), vl + n * (lv - ml) * (lv - ml)));
    let a = if varl > 0.0 { (cov / varl).max(0.0) } else { 0.0 };
    let b = (mv - a * ml).max(0.0);
    (a.is_finite() && b.is_finite()).then_some(Noise { a: a as f32, b: b as f32 })
}

/// Photosites far above every same-colour neighbour, with the value to replace them by.
fn candidates(v: &View<'_>, (x0, x1, y0, y1): (usize, usize, usize, usize), black: f32, noise: Noise) -> Vec<(usize, usize, u16)> {
    (y0..y1)
        .into_par_iter()
        .flat_map_iter(|y| {
            let mut buf = Vec::with_capacity(8);
            let mut out = Vec::new();
            for x in x0..x1 {
                let s = v.at(x as isize, y as isize);
                let max = v.neighbours(x, y, &mut buf);
                if s <= max {
                    continue;
                }
                let m = median(&mut buf);
                if s - max > K_HOT * noise.sigma(m - black) {
                    out.push((x, y, m.round().clamp(0.0, u16::MAX as f32) as u16));
                }
            }
            out
        })
        .collect()
}

/// Are at least [`MIN_LIT`] direct neighbours of `(x, y)` clearly above their own same-colour surroundings
/// (real light)? A single one is not enough: hot photosites also come in adjacent pairs.
fn lit_around(v: &View<'_>, x: usize, y: usize, black: f32, noise: Noise, buf: &mut Vec<f32>) -> bool {
    v.phase(x, y)
        .around
        .iter()
        .filter(|&&(dx, dy)| {
            let (nx, ny) = ((x as isize + dx) as usize, (y as isize + dy) as usize);
            let s = v.at(nx as isize, ny as isize);
            v.neighbours(nx, ny, buf);
            let m = median(buf);
            s - m > K_SPOT * noise.sigma(m - black)
        })
        .count()
        >= MIN_LIT
}

#[cfg(test)]
mod tests {
    use crate::sensor::*;

    /// A `w × h` RGGB mosaic, black 512, white 16383, flat grey 2000 counts above black with
    /// deterministic noise of about σ ≈ 30 counts.
    fn mosaic(w: usize, h: usize) -> RawImage {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut noise = || {
            // sum of 4 uniforms, about N(0, 1)
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
        let data = (0..w * h).map(|_| (512.0 + 2000.0 + 30.0 * noise()).round() as u16).collect();
        RawImage {
            format: RawFormat::Arw,
            width: w,
            height: h,
            cpp: 1,
            data: RawData::U16(data),
            cfa: Cfa::bayer("RGGB"),
            bits: 14,
            black: BlackLevel::uniform(512.0),
            white: vec![16383.0],
            active_area: Rect::new(0, 0, w, h),
            crop: Rect::new(0, 0, w, h),
            orientation: Orientation::Normal,
            color: ColorData::default(),
            wb_multipliers: None,
            linearized: false,
            opcodes: OpcodeLists::default(),
            lens_metadata: Default::default(),
            pixel_aspect: 1.0,
            camera: Default::default(),
            cinema: Default::default(),
            level_sources: Default::default(),
        }
    }

    fn get(r: &RawImage, x: usize, y: usize) -> u16 {
        match &r.data {
            RawData::U16(d) => d[y * r.width + x],
            RawData::F32(_) => 0,
        }
    }

    fn set(r: &mut RawImage, x: usize, y: usize, v: u16) {
        let w = r.width;
        if let RawData::U16(d) = &mut r.data {
            d[y * w + x] = v;
        }
    }

    #[test]
    fn flat_noise_is_left_alone() {
        let mut r = mosaic(256, 256);
        let before = r.data.clone();
        assert_eq!(r.suppress_hot_pixels(), 0);
        assert_eq!(r.data, before);
    }

    #[test]
    fn single_hot_photosites_of_each_colour_are_replaced() {
        let mut r = mosaic(256, 256);
        // red (even, even), green (odd, even), blue (odd, odd)
        for &(x, y) in &[(100, 100), (101, 60), (151, 151)] {
            set(&mut r, x, y, 9000);
        }
        assert_eq!(r.suppress_hot_pixels(), 3);
        for &(x, y) in &[(100, 100), (101, 60), (151, 151)] {
            let v = get(&r, x, y) as f32;
            assert!((v - 2512.0).abs() < 120.0, "({x}, {y}) = {v}");
        }
    }

    #[test]
    fn an_adjacent_hot_pair_is_replaced() {
        let mut r = mosaic(256, 256);
        set(&mut r, 80, 80, 9000);
        set(&mut r, 80, 81, 8000);
        assert_eq!(r.suppress_hot_pixels(), 2);
        assert!(get(&r, 80, 80) < 2700 && get(&r, 80, 81) < 2700);
    }

    #[test]
    fn a_small_star_is_kept() {
        let mut r = mosaic(256, 256);
        // a point of light over a 3 × 3 block: brightest in the middle, its neighbours lit too
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let v = if dx == 0 && dy == 0 { 12000 } else { 5000 };
                set(&mut r, (120 + dx) as usize, (90 + dy) as usize, v);
            }
        }
        let before = r.data.clone();
        assert_eq!(r.suppress_hot_pixels(), 0);
        assert_eq!(r.data, before);
    }

    #[test]
    fn a_hot_photosite_in_a_dark_area_is_replaced() {
        let mut r = mosaic(256, 256);
        // a dark patch near black (noise is then read noise, smaller)
        for y in 32..96 {
            for x in 32..96 {
                set(&mut r, x, y, 512 + 20 + ((x * 7 + y * 13) % 11) as u16);
            }
        }
        set(&mut r, 64, 64, 1500);
        assert_eq!(r.suppress_hot_pixels(), 1);
        assert!(get(&r, 64, 64) < 600);
    }

    #[test]
    fn a_dense_star_field_keeps_its_stars_and_loses_its_hot_pixels() {
        let mut r = mosaic(256, 256);
        // 225 small stars: more candidates than 0.2 % of the photosites, every one of them real light
        for sy in 0..15 {
            for sx in 0..15 {
                let (cx, cy) = (16 + 16 * sx, 16 + 16 * sy);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let v = if dx == 0 && dy == 0 { 12000 } else { 5000 };
                        set(&mut r, (cx + dx) as usize, (cy + dy) as usize, v);
                    }
                }
            }
        }
        let hot = [(24, 24), (105, 72), (201, 153)];
        for &(x, y) in &hot {
            set(&mut r, x, y, 9000);
        }
        assert_eq!(r.suppress_hot_pixels(), 3);
        for &(x, y) in &hot {
            assert!(get(&r, x, y) < 2700, "({x}, {y}) = {}", get(&r, x, y));
        }
        assert_eq!(get(&r, 16, 16), 12000, "a star is left alone");
    }

    #[test]
    fn the_limit_is_the_callers() {
        let mut r = mosaic(256, 256);
        let hot = [(40, 40), (101, 60), (151, 151), (200, 30)];
        for &(x, y) in &hot {
            set(&mut r, x, y, 9000);
        }
        let before = r.data.clone();
        // 4 of 248 × 248 photosites: above a limit of 0.005 %, within 0.01 %
        assert_eq!(r.suppress_hot_pixels_with(5e-5), 0);
        assert_eq!(r.data, before);
        assert_eq!(r.suppress_hot_pixels_with(1e-4), 4);
        assert_eq!(r.clone().suppress_hot_pixels_with(f64::NAN), 0, "a limit that is not a number replaces nothing");
    }

    #[test]
    fn too_many_candidates_change_nothing() {
        let mut r = mosaic(128, 128);
        for y in (8..120).step_by(4) {
            for x in (8..120).step_by(4) {
                set(&mut r, x, y, 12000);
            }
        }
        let before = r.data.clone();
        assert_eq!(r.suppress_hot_pixels(), 0);
        assert_eq!(r.data, before);
    }
}
