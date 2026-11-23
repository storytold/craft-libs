//! Crystal-layer film grain, the grain of process V7 and later ([`crate::finish::GrainModel`]).
//!
//! After the model of Aurélien Pierre, *Stochastic photographic grain synthesis from
//! crystallographic structure simulation* (2023), implemented from the article's description of
//! the method; no code was taken from it or from any implementation. `docs/grain.md` has the
//! method, the calibration and the measurements.
//!
//! - **Emulsion.** A stack of layers. Each layer has one crystal shape (a regular polygon of 3–8
//!   vertices, a random rotation, a log-normal width) and is a Boolean model: Poisson seeds grown
//!   into crystals, at the density that leaves half the layer covered ([`FILLING`]).
//! - **Capture.** A point's grain is the number of layers covering it, less its mean: many
//!   independent layers of different crystal sizes give dense, soft, cloudy grain.
//! - **Print.** The light a point captures is proportional to the light falling on it, and the
//!   print shows less grain towards paper white: the grain is `Y·(1 − Y)` in linear light, so
//!   it fades out at black and at white. It is added in the display-encoded domain (first order),
//!   where it is symmetric: it neither darkens shadows nor greys blown highlights.
//!
//! **Resolution independence.** The emulsion is drawn once per setting on two periodic tiles
//! (tori) in frame coordinates, at twice the resolution of a 6000 px export ([`PITCH`]). Each
//! output pixel takes the exact mean of the emulsion under its footprint from the tiles' summed
//! area tables (a rotated frame's pixel: of an axis-aligned square of its size), so a preview
//! shows exactly the export's grain box-downscaled to its size, and an
//! export larger than 6000 px still has crystals rather than enlarged pixels. The layers are
//! split between two tori of different sizes ([`TORI`]); their pattern only repeats as a whole
//! after tens of thousands of pixels.
//!
//! **Cost.** Drawing the tiles takes a few milliseconds and is cached per setting
//! ([`CrystalGrain::new`]); a pixel then costs two box sums (16 table reads), on the CPU and in the
//! GPU kernel (`finish.wgsl`, which reads the same tables).

use std::f64::consts::TAU;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};

use crate::raster::par_rows;
use craft_color::srgb_to_linear;
use craft_develop_model::Grain;

/// Long edge (output pixels) at which grain sizes and strengths are specified: a 24 MP export.
pub const REF_LONG: f64 = 6000.0;
/// Size of a tile pixel in reference pixels: tiles are drawn at twice the reference resolution.
pub const PITCH: f64 = 0.5;
/// Tile pixels per long edge of the frame.
pub const TILE_PER_LONG: f64 = REF_LONG / PITCH;
/// Sides of the two tori (tile pixels: 640 and 567 reference pixels). Layers alternate between
/// them; the sizes share no small factor, so the sum repeats only after 362,880 reference pixels.
pub const TORI: [usize; 2] = [1280, 1134];
/// Fraction of each layer covered by its crystals.
pub const FILLING: f64 = 0.5;
/// Rows per band when drawing a tile (each band is drawn in parallel).
const BAND: usize = 64;
/// Encoded luminance at which the strength is specified (mid grey, 128 of 255).
const MID_GREY: f32 = 128.0 / 255.0;
/// A footprint this large (tile pixels: a render 24 px long or less) gets no grain.
const MAX_FOOTPRINT: f32 = 500.0;
/// Settings whose tiles are kept (a preview and an export, a few thumbnails).
const CACHE: usize = 4;

/// The look of a grain setting: what the tiles are drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrystalLook {
    /// Median crystal width (circumscribed diameter, reference pixels).
    pub width: f64,
    /// Log standard deviation of the layers' crystal widths.
    pub spread: f64,
    /// Number of layers.
    pub layers: usize,
}

/// Piecewise-linear interpolation through `(x, y)` knots sorted by `x`, constant outside them.
fn interp(knots: &[(f64, f64)], x: f64) -> f64 {
    let (Some(first), Some(last)) = (knots.first(), knots.last()) else { return 0.0 };
    if x <= first.0 {
        return first.1;
    }
    for w in knots.windows(2) {
        if let [(x0, y0), (x1, y1)] = *w
            && x <= x1
        {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0).max(1e-9);
        }
    }
    last.1
}

/// Median crystal width at Roughness 50 by Size (reference pixels), fitted so the grain's width
/// matches Lightroom Classic's (`fit_widths`, `docs/grain.md`). Size 0 is as fine as the model
/// gets: crystals narrower than a pixel, uncorrelated pixels.
const WIDTH_BY_SIZE: [(f64, f64); 6] = [(0.0, 0.55), (10.0, 1.39), (25.0, 2.95), (50.0, 5.17), (75.0, 8.78), (100.0, 12.38)];
/// Width factor by Roughness.
const WIDTH_BY_ROUGHNESS: [(f64, f64); 3] = [(0.0, 0.734), (50.0, 1.0), (100.0, 1.422)];
/// Layers by Size: coarser grain averages more layers, so it stays a soft clump texture.
const LAYERS_BY_SIZE: [(f64, f64); 4] = [(0.0, 16.0), (25.0, 31.0), (50.0, 54.0), (100.0, 70.0)];

/// Log-width spread every look has on top of Roughness's. The look the tables were chosen from (a
/// prototype on whole-pixel crystals, `docs/grain.md`) rounded crystal widths to whole pixels,
/// which spreads their sizes by about this much; with it, the anti-aliased crystals here have the
/// prototype's texture at every scale, not just its width.
const SPREAD_FLOOR: f64 = 0.45;

/// A slider value as a number in `0..=100`; `fallback` for NaN.
fn slider(v: f64, fallback: f64) -> f64 {
    if v.is_nan() { fallback } else { v.clamp(0.0, 100.0) }
}

impl CrystalLook {
    pub fn new(size: f64, roughness: f64) -> CrystalLook {
        let (s, r) = (slider(size, 25.0), slider(roughness, 50.0));
        CrystalLook {
            width: interp(&WIDTH_BY_SIZE, s) * interp(&WIDTH_BY_ROUGHNESS, r),
            spread: (0.1 + 0.6 * r / 100.0).hypot(SPREAD_FLOOR),
            layers: interp(&LAYERS_BY_SIZE, s).round().clamp(2.0, 120.0) as usize,
        }
    }
}

/// Grain strength (8-bit levels at mid grey on a 6000 px export) for Amount, Size and Roughness,
/// measured on Lightroom Classic exports (the fit of open PR #1060's `docs/grain.md`).
pub fn strength_levels(amount: f64, size: f64, roughness: f64) -> f64 {
    let (s, d) = (slider(size, 25.0), slider(roughness, 50.0) / 100.0 - 0.5);
    slider(amount, 0.0) / 100.0 * (2.447 + 1.386 * (-s / 19.88).exp()).exp() * (1.0 - 0.084 * d + 1.6384 * d * d)
}

// ---- hashing

#[inline]
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[inline]
fn hash(a: u32, b: u32, c: u32, d: u32) -> u32 {
    mix(mix(mix(mix(a ^ 0x9e37_79b9) ^ b) ^ c.wrapping_mul(0x85eb_ca6b)) ^ d.wrapping_mul(0xc2b2_ae35))
}

/// A hash as a number in (0, 1].
#[inline]
fn unit(h: u32) -> f64 {
    (h as f64 + 0.5) / 4_294_967_296.0
}

/// The standard normal quantile of `p` in (0, 1) (Acklam's rational approximation, ~1e-9).
fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] =
        [-39.696_830_286_653_76, 220.946_098_424_520_5, -275.928_510_446_968_7, 138.357_751_867_269, -30.664_798_066_147_16, 2.506_628_277_459_239];
    const B: [f64; 5] = [-54.476_098_798_224_06, 161.585_836_858_040_9, -155.698_979_859_886_6, 66.801_311_887_719_72, -13.280_681_552_885_72];
    const C: [f64; 6] = [
        -0.007_784_894_002_430_293,
        -0.322_396_458_041_136_5,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [0.007_784_695_709_041_462, 0.322_467_129_070_039_8, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let tail =
        |q: f64| (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    if p < 0.024_25 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.024_25 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

/// The emulsion's layers. Their crystal widths are the `layers` quantiles of the log-normal
/// width distribution, dealt to the layers in an order the seed shuffles: every seed has the same
/// mix of sizes (so the same grain width and strength), in a different arrangement.
fn emulsion(seed: u32, look: &CrystalLook) -> Vec<Layer> {
    let n = look.layers.max(1);
    let mut order: Vec<(u32, usize)> = (0..n).map(|l| (hash(seed, l as u32, 0x5047, 0), l)).collect();
    order.sort_unstable();
    let mut rank = vec![0usize; n];
    for (r, &(_, l)) in order.iter().enumerate() {
        if let Some(slot) = rank.get_mut(l) {
            *slot = r;
        }
    }
    (0..n).map(|l| Layer::new(seed, l as u32, look, normal_quantile((rank.get(l).copied().unwrap_or(0) as f64 + 0.5) / n as f64))).collect()
}

fn normal(h1: u32, h2: u32) -> f64 {
    (-2.0 * unit(h1).ln()).sqrt() * (TAU * unit(h2)).cos()
}

/// The cumulative distribution of a Poisson count of mean `mu`, up to where it reaches 1 (at
/// most 512 entries; `mu` stays below ~150 here).
fn poisson_cdf(mu: f64) -> Vec<f64> {
    let mut cdf = Vec::new();
    let mut p = (-mu).exp();
    let mut cum = p;
    for k in 1..512u32 {
        cdf.push(cum);
        if cum >= 1.0 - 1e-12 {
            break;
        }
        p *= mu / k as f64;
        cum += p;
    }
    cdf
}

/// A Poisson count drawn from its `cdf` by hash `h`.
#[inline]
fn poisson(cdf: &[f64], h: u32) -> usize {
    let u = unit(h);
    cdf.partition_point(|&c| c < u)
}

// ---- one layer

/// A stream of hashes: a Weyl sequence through [`mix`].
struct Stream(u32);

impl Stream {
    #[inline]
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9);
        mix(self.0)
    }
}

/// Sub-pixel positions a crystal centre is rounded to, per axis, when drawn from its [`Stamp`].
const PHASES: usize = 16;
/// Crystals up to this circumradius (tile pixels) are drawn from a stamp.
const STAMP_R: f64 = 16.0;

/// A layer's crystal rasterised at every sub-pixel phase of its centre: for a centre at
/// `(X + qx/PHASES, Y + qy/PHASES)` (`X`, `Y` integers) it covers rows `Y + rows[qy].0 ..= Y +
/// rows[qy].1`, and in row `Y + rows[qy].0 + k` the pixels `X + o.0 ..= X + o.1`, with
/// `o = spans[(qy·PHASES + qx)·height + k]`.
///
/// The same rows as bit masks, for drawing a crystal without a branch per row: phase
/// `p = qy·PHASES + qx` spans pixels `X + extent[p].0 ..= X + extent[p].1`, and its row `k` is
/// `masks[p·height + k]` shifted to `X + extent[p].0` (rows past the phase's own are empty).
/// `masks` is empty when a phase is wider than 64 pixels.
#[derive(Clone, Debug)]
struct Stamp {
    rows: [(i32, i32); PHASES],
    height: usize,
    spans: Vec<(i32, i32)>,
    extent: Vec<(i32, i32)>,
    masks: Vec<u64>,
}

/// Where [`Layer::draw`] draws: rows `y0..y0 + rows` of a torus of side `t`, `w` words per row,
/// after `margin` scratch rows.
#[derive(Clone, Copy)]
struct Band {
    t: usize,
    y0: usize,
    rows: usize,
    w: usize,
    margin: usize,
}

/// One emulsion layer: its crystal (a regular polygon) and seed density, in tile pixels.
#[derive(Clone, Debug)]
struct Layer {
    /// The crystal as half-planes, relative to its centre: `x ≥ a + s·y` for each `(a, s)` of
    /// `left` (`nl` used), `x ≤ a + s·y` for `right` (`nr` used), `y0 ≤ y ≤ y1`.
    left: [(f64, f64); 8],
    right: [(f64, f64); 8],
    nl: usize,
    nr: usize,
    y0: f64,
    y1: f64,
    /// Circumradius.
    r: f64,
    /// Seeds per tile pixel.
    lambda: f64,
    seed: u32,
    index: u32,
    stamp: Option<Stamp>,
}

/// Crystals narrower than this (tile pixels) cover at most two neighbouring pixel centres: their
/// layer is drawn as independent pixels.
const FINE: f64 = 1.5;

impl Layer {
    /// Layer `index` of the emulsion of `seed`, its crystal width `z` log standard deviations
    /// from the median.
    fn new(seed: u32, index: u32, look: &CrystalLook, z: f64) -> Layer {
        let h = |k: u32| hash(seed, index, k, 0x51ed);
        let n = (4.5 + normal(h(1), h(2))).round().clamp(3.0, 8.0) as usize;
        let rot = TAU * unit(h(3));
        let width = look.width.max(0.05) * (look.spread * z.clamp(-2.5, 2.5)).exp();
        let r = width / 2.0 / PITCH;
        let vert = |k: usize| {
            let a = rot + TAU * k as f64 / n as f64;
            (r * a.cos(), r * a.sin())
        };
        let mut layer = Layer {
            left: [(0.0, 0.0); 8],
            right: [(0.0, 0.0); 8],
            nl: 0,
            nr: 0,
            y0: f64::INFINITY,
            y1: f64::NEG_INFINITY,
            r,
            lambda: -(1.0 - FILLING).ln() / (0.5 * n as f64 * r * r * (TAU / n as f64).sin()).max(1e-9),
            seed,
            index,
            stamp: None,
        };
        for k in 0..n {
            let ((x0, y0), (x1, y1)) = (vert(k), vert(k + 1));
            layer.y0 = layer.y0.min(y0);
            layer.y1 = layer.y1.max(y0);
            let dy = y1 - y0;
            if dy.abs() < 1e-12 {
                continue;
            }
            // counter-clockwise: the inside is left of each edge
            let sl = (x1 - x0) / dy;
            let line = (x0 - sl * y0, sl);
            if dy > 0.0 {
                if let Some(slot) = layer.right.get_mut(layer.nr) {
                    *slot = line;
                    layer.nr += 1;
                }
            } else if let Some(slot) = layer.left.get_mut(layer.nl) {
                *slot = line;
                layer.nl += 1;
            }
        }
        if r <= STAMP_R && 2.0 * r >= FINE {
            layer.stamp = Some(layer.stamp());
        }
        layer
    }

    fn stamp(&self) -> Stamp {
        let q = PHASES as f64;
        let mut rows = [(0, -1); PHASES];
        for (qy, r) in rows.iter_mut().enumerate() {
            let g = qy as f64 / q;
            *r = ((g + self.y0 - 0.5).ceil() as i32, (g + self.y1 - 0.5).floor() as i32);
        }
        let height = rows.iter().map(|(a, b)| (b - a + 1).max(0) as usize).max().unwrap_or(0);
        let mut spans = vec![(0, -1); PHASES * PHASES * height];
        for (qy, &(a, b)) in rows.iter().enumerate() {
            for k in 0..(b - a + 1).max(0) {
                let (xl, xr) = self.span((a + k) as f64 + 0.5 - qy as f64 / q);
                for qx in 0..PHASES {
                    let f = qx as f64 / q;
                    if let Some(o) = spans.get_mut((qy * PHASES + qx) * height + k as usize) {
                        *o = ((f + xl - 0.5).ceil() as i32, (f + xr - 0.5).floor() as i32);
                    }
                }
            }
        }
        let mut extent = vec![(0, -1); PHASES * PHASES];
        let mut masks = vec![0u64; PHASES * PHASES * height];
        let mut fits = true;
        for (p, e) in extent.iter_mut().enumerate() {
            let Some(rows) = spans.get(p * height..(p + 1) * height) else { continue };
            let used = || rows.iter().filter(|(o0, o1)| o0 <= o1);
            let (Some(lo), Some(hi)) = (used().map(|o| o.0).min(), used().map(|o| o.1).max()) else { continue };
            *e = (lo, hi);
            fits &= hi - lo < 64;
            for (m, &(o0, o1)) in masks.iter_mut().skip(p * height).zip(rows) {
                if o0 <= o1 && o1 - lo < 64 {
                    *m = (!0u64 >> (63 - (o1 - o0))) << (o0 - lo);
                }
            }
        }
        if !fits {
            masks.clear();
        }
        Stamp { rows, height, spans, extent, masks }
    }

    /// The crystal's horizontal extent at height `y` above its centre (convex: one interval).
    #[inline]
    fn span(&self, y: f64) -> (f64, f64) {
        let lo = self.left.iter().take(self.nl).fold(f64::NEG_INFINITY, |m, &(a, s)| m.max(a + s * y));
        let hi = self.right.iter().take(self.nr).fold(f64::INFINITY, |m, &(a, s)| m.min(a + s * y));
        (lo, hi)
    }

    /// Sets the bits of the pixels the layer covers in rows `y0..` of a torus of side `t`:
    /// `cover` holds whole rows of `w` words (bit `i % 64` of word `i / 64` is pixel `i`), the
    /// band's rows between `margin` rows above and below it (scratch: what lands there is not part
    /// of the band; `margin` is at least the stamp's height).
    fn draw(&self, t: usize, y0: usize, w: usize, margin: usize, cover: &mut [u64]) {
        let rows = (cover.len() / w.max(1)).saturating_sub(2 * margin);
        if 2.0 * self.r < FINE {
            // each pixel covered with probability FILLING (one half: one random bit)
            for (j, row) in cover.chunks_mut(w.max(1)).skip(margin).take(rows).enumerate() {
                let mut bits = Stream(hash(self.seed, self.index, (y0 + j) as u32, 0xb175));
                for v in row.iter_mut() {
                    *v = bits.next() as u64 | (bits.next() as u64) << 32;
                }
                if let (Some(last), r @ 1..=63) = (row.last_mut(), t % 64) {
                    *last &= (1u64 << r) - 1;
                }
            }
            return;
        }
        let tf = t as f64;
        let m = (tf / (4.0 * self.r).max(16.0)).floor().clamp(1.0, tf) as i64;
        let c = tf / m as f64;
        let cdf = poisson_cdf(self.lambda * c * c);
        let (lo, hi) = (y0 as f64 - self.r - 1.0, (y0 + rows) as f64 + self.r + 1.0);
        for cy in (lo / c).floor() as i64..=(hi / c).floor() as i64 {
            let wy = cy.rem_euclid(m) as u32;
            for cx in 0..m {
                let mut h = Stream(hash(self.seed ^ (self.index << 20), cx as u32, wy, 0xce11));
                for _ in 0..poisson(&cdf, h.next()) {
                    let (u, v) = (unit(h.next()), unit(h.next()));
                    self.splat((cx as f64 + u) * c, (cy as f64 + v) * c, Band { t, y0, rows, w, margin }, cover);
                }
            }
        }
    }

    /// Marks the pixels whose centres the crystal at (`px`, `py`) covers (see [`Self::draw`]).
    #[inline]
    fn splat(&self, px: f64, py: f64, band: Band, cover: &mut [u64]) {
        let Band { t, y0, rows, w, margin } = band;
        if let Some(st) = &self.stamp {
            let (bx, by) = (px.floor(), py.floor());
            let (qx, qy) = ((((px - bx) * PHASES as f64) as usize).min(PHASES - 1), (((py - by) * PHASES as f64) as usize).min(PHASES - 1));
            let (bx, by) = (bx as i64, by as i64);
            let Some(&(a, b)) = st.rows.get(qy) else { return };
            let (lo, hi) = ((by + a as i64).max(y0 as i64), (by + b as i64).min((y0 + rows) as i64 - 1));
            if hi < lo {
                return;
            }
            // within the torus across: the whole stamp, a fixed number of rows of masks (those
            // above or below the band land in the margins)
            let p = qy * PHASES + qx;
            if let (Some(&(e0, e1)), Some(masks)) = (st.extent.get(p), st.masks.get(p * st.height..(p + 1) * st.height)) {
                let (xs, top) = (bx + e0 as i64, by + a as i64 - y0 as i64 + margin as i64);
                if xs >= 0 && bx + (e1 as i64) < t as i64 && top >= 0 {
                    let (word, shift) = ((xs >> 6) as usize, (xs & 63) as u32);
                    // a bit past the row's last word would be a pixel ≥ t: the high word is then 0
                    let start = top as usize * w + word;
                    for (k, &m) in masks.iter().enumerate() {
                        let v = (m as u128) << shift;
                        let i = start + k * w;
                        if let Some(c) = cover.get_mut(i) {
                            *c |= v as u64;
                        }
                        if let Some(c) = cover.get_mut(i + 1) {
                            *c |= (v >> 64) as u64;
                        }
                    }
                    return;
                }
            }
            // rows `lo..=hi` of the band, spans `k0..` of the stamp's rows (`hi ≤ by + b`: within its height)
            let (k0, n) = ((lo - by - a as i64) as usize, (hi - lo + 1) as usize);
            let base = (qy * PHASES + qx) * st.height + k0;
            let (r0, r1) = (lo as usize - y0 + margin, hi as usize + 1 - y0 + margin);
            let (Some(spans), Some(band)) = (st.spans.get(base..base + n), cover.get_mut(r0 * w..r1 * w)) else { return };
            for (row, &(o0, o1)) in band.chunks_exact_mut(w.max(1)).zip(spans) {
                if o0 <= o1 {
                    set_wrapped(row, t, bx + o0 as i64, bx + o1 as i64);
                }
            }
            return;
        }
        let first = (py + self.y0 - 0.5).ceil().max(y0 as f64);
        let last = (py + self.y1 - 0.5).floor().min((y0 + rows) as f64 - 1.0);
        if last < first {
            return;
        }
        for j in first as usize..=last as usize {
            let (xl, xr) = self.span(j as f64 + 0.5 - py);
            let (i0, i1) = ((px + xl - 0.5).ceil() as i64, (px + xr - 0.5).floor() as i64);
            if i1 < i0 {
                continue;
            }
            let Some(row) = cover.get_mut((j - y0 + margin) * w..(j - y0 + margin + 1) * w) else { continue };
            set_wrapped(row, t, i0, i1);
        }
    }
}

/// Sets bits `a..=b` (`a ≤ b`) of a row of words.
#[inline]
fn set_bits(row: &mut [u64], a: usize, b: usize) {
    let (wa, wb) = (a >> 6, b >> 6);
    let (lo, hi) = (!0u64 << (a & 63), !0u64 >> (63 - (b & 63)));
    if wa == wb {
        if let Some(v) = row.get_mut(wa) {
            *v |= lo & hi;
        }
        return;
    }
    if let Some(v) = row.get_mut(wa) {
        *v |= lo;
    }
    for v in row.iter_mut().take(wb).skip(wa + 1) {
        *v = !0;
    }
    if let Some(v) = row.get_mut(wb) {
        *v |= hi;
    }
}

/// Sets pixels `i0..=i1` of a periodic row of `t` pixels (`i0 ≤ i1`, within one period of
/// `0..t`: the crystal centres are in `0..t`).
#[inline]
fn set_wrapped(row: &mut [u64], t: usize, i0: i64, i1: i64) {
    let ti = t as i64;
    if i0 >= 0 && i1 < ti {
        set_bits(row, i0 as usize, i1 as usize);
        return;
    }
    if t == 0 {
        return;
    }
    if i1 - i0 + 1 >= ti {
        set_bits(row, 0, t - 1);
        return;
    }
    let (a, b) = if i0 < 0 {
        (i0 + ti, i1 + ti)
    } else if i0 >= ti {
        (i0 - ti, i1 - ti)
    } else {
        (i0, i1)
    };
    let (a, b) = (a.clamp(0, ti - 1) as usize, b.max(0) as usize);
    if b < t {
        set_bits(row, a, b);
    } else {
        set_bits(row, a, t - 1);
        set_bits(row, 0, (b - t).min(t - 1));
    }
}

/// Bit planes of the layer counts (enough for 127 layers per torus).
const PLANES: usize = 7;

/// Adds one layer's `cover` bits to the bit-sliced counters `planes` (`PLANES × cover.len()`).
#[inline]
fn add_layer(planes: &mut [u64], cover: &[u64]) {
    let n = cover.len();
    for (i, &c) in cover.iter().enumerate() {
        let mut carry = c;
        for p in 0..PLANES {
            if carry == 0 {
                break;
            }
            let Some(slot) = planes.get_mut(p * n + i) else { break };
            let s = *slot;
            *slot = s ^ carry;
            carry &= s;
        }
    }
}

// ---- tiles

/// The emulsion drawn on the two tori: each as a summed-area table of its grain (layer count less
/// its mean), scaled so that the sum of both, averaged over a reference pixel, has unit variance.
pub struct CrystalTile {
    pub look: CrystalLook,
    /// `(side + 1)²` entries each: `sat[t][y·(side + 1) + x]` = sum over `[0, x) × [0, y)`.
    pub sats: [Vec<f32>; 2],
}

impl std::fmt::Debug for CrystalTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrystalTile").field("look", &self.look).finish_non_exhaustive()
    }
}

/// Layer counts of torus `which` (side `t`): `t × t`, row-major.
fn draw_torus(which: usize, t: usize, seed: u32, look: &CrystalLook) -> Vec<u8> {
    let layers: Vec<Layer> = emulsion(seed, look).into_iter().filter(|l| l.index as usize % 2 == which).collect();
    let w = t.div_ceil(64);
    // scratch rows above and below a band, for the stamps' rows outside it (`Layer::splat`)
    let margin = layers.iter().filter_map(|l| l.stamp.as_ref().map(|s| s.height)).max().unwrap_or(0);
    let mut count = vec![0u8; t * t];
    par_rows(&mut count, t * BAND, |band, out| {
        let rows = out.len() / t.max(1);
        let n = rows * w;
        let (mut cover, mut planes) = (vec![0u64; n + 2 * margin * w], vec![0u64; PLANES * n]);
        for layer in &layers {
            cover.fill(0);
            layer.draw(t, band * BAND, w, margin, &mut cover);
            add_layer(&mut planes, cover.get(margin * w..margin * w + n).unwrap_or_default());
        }
        for (j, row) in out.chunks_mut(t.max(1)).enumerate() {
            for (i, o) in row.iter_mut().enumerate() {
                let (word, bit) = (j * w + i / 64, i % 64);
                *o = (0..PLANES).fold(0u8, |c, p| c | ((planes.get(p * n + word).map_or(0, |v| (v >> bit) & 1) as u8) << p));
            }
        }
    });
    count
}

/// Integer summed-area table of `count` (`t × t`): `(t + 1)²` entries.
fn summed_area(count: &[u8], t: usize) -> Vec<u32> {
    let s = t + 1;
    let mut sat = vec![0u32; s * s];
    // row prefix sums in parallel, then each row adds the one above
    par_rows(&mut sat, s, |y, row| {
        let Some(src) = y.checked_sub(1).and_then(|y| count.get(y * t..(y + 1) * t)) else { return };
        let mut acc = 0u32;
        for (o, &c) in row.iter_mut().skip(1).zip(src) {
            acc += c as u32;
            *o = acc;
        }
    });
    for y in 1..s {
        let (above, below) = sat.split_at_mut(y * s);
        let (Some(prev), Some(row)) = (above.get((y - 1) * s..), below.get_mut(..s)) else { continue };
        for (o, &p) in row.iter_mut().zip(prev) {
            *o = o.wrapping_add(p);
        }
    }
    sat
}

/// Sum over the reference pixels (2 × 2 tile pixels) of torus `count` (`t × t`, `t` even) of
/// their squared deviation from `mean` (per tile pixel), and the number of reference pixels.
fn reference_square_sum(count: &[u8], t: usize, mean: f64) -> (f64, usize) {
    let h = t / 2;
    let mut rows = vec![0.0f64; h];
    par_rows(&mut rows, 1, |y, out| {
        let (Some(a), Some(b)) = (count.get(2 * y * t..(2 * y + 1) * t), count.get((2 * y + 1) * t..(2 * y + 2) * t)) else { return };
        let mut s = 0.0;
        for (&[a0, a1], &[b0, b1]) in a.as_chunks::<2>().0.iter().zip(b.as_chunks::<2>().0) {
            let q = (a0 as u32 + a1 as u32 + b0 as u32 + b1 as u32) as f64 / 4.0 - mean;
            s += q * q;
        }
        if let Some(o) = out.first_mut() {
            *o = s;
        }
    });
    (rows.iter().sum(), h * h)
}

impl CrystalTile {
    /// Draws the emulsion of `look` with `seed`, or `None` if it has no grain (no variance).
    pub fn new(look: CrystalLook, seed: u32) -> Option<CrystalTile> {
        let [t0, t1] = TORI;
        let (c0, c1) = crate::raster::par_join(|| draw_torus(0, t0, seed, &look), || draw_torus(1, t1, seed, &look));
        let mut var = 0.0;
        let mut tori = Vec::with_capacity(2);
        for (count, t) in [(c0, t0), (c1, t1)] {
            let mean = count.iter().map(|&c| c as u64).sum::<u64>() as f64 / (t * t).max(1) as f64;
            let (sq, n) = reference_square_sum(&count, t, mean);
            var += sq / n.max(1) as f64;
            tori.push((summed_area(&count, t), t, mean));
        }
        if !(var > 1e-12 && var.is_finite()) {
            return None;
        }
        let k = 1.0 / var.sqrt();
        let mut sats = tori.into_iter().map(|(sat, t, mean)| {
            let s = t + 1;
            let mut out = vec![0.0f32; s * s];
            par_rows(&mut out, s, |y, row| {
                for (x, (o, &v)) in row.iter_mut().zip(sat.get(y * s..).unwrap_or_default()).enumerate() {
                    *o = ((v as f64 - mean * (x * y) as f64) * k) as f32;
                }
            });
            out
        });
        Some(CrystalTile { look, sats: [sats.next()?, sats.next()?] })
    }

    /// Sum of torus `which`'s grain over the box `[x0, x0 + w] × [y0, y0 + w]` (tile pixels,
    /// `0 ≤ w < side`), the table bilinearly interpolated: the exact integral of the pixels.
    #[inline]
    fn box_sum(&self, which: usize, x0: f32, y0: f32, w: f32) -> f32 {
        let (Some(sat), Some(&t)) = (self.sats.get(which), TORI.get(which)) else { return 0.0 };
        let tf = t as f32;
        let (x0, y0) = (x0 - (x0 / tf).floor() * tf, y0 - (y0 / tf).floor() * tf);
        let at = |x: f32, y: f32| integral(sat, t, x, y);
        at(x0 + w, y0 + w) - at(x0, y0 + w) - at(x0 + w, y0) + at(x0, y0)
    }

    /// Mean grain over the square of side `w` centred on (`x`, `y`) (tile pixels): unit variance
    /// at `w` = 2 (a 6000 px render).
    #[inline]
    pub fn field(&self, x: f32, y: f32, w: f32) -> f32 {
        let (x0, y0) = (x - 0.5 * w, y - 0.5 * w);
        (self.box_sum(0, x0, y0, w) + self.box_sum(1, x0, y0, w)) / (w * w)
    }
}

/// The table `sat` (side `t + 1`) bilinearly interpolated at (`x`, `y`) in `0..=t`.
#[inline]
fn bilinear(sat: &[f32], t: usize, x: f32, y: f32) -> f32 {
    let s = t + 1;
    let (i, j) = ((x.max(0.0) as usize).min(t - 1), (y.max(0.0) as usize).min(t - 1));
    let (fx, fy) = (x - i as f32, y - j as f32);
    let g = |i: usize, j: usize| sat.get(j * s + i).copied().unwrap_or(0.0);
    let a = g(i, j) + (g(i + 1, j) - g(i, j)) * fx;
    let b = g(i, j + 1) + (g(i + 1, j + 1) - g(i, j + 1)) * fx;
    a + (b - a) * fy
}

/// The periodic grain's integral over `[0, x) × [0, y)` for `x`, `y` in `0..2t`.
#[inline]
fn integral(sat: &[f32], t: usize, x: f32, y: f32) -> f32 {
    let tf = t as f32;
    let (wx, wy) = (x >= tf, y >= tf);
    let (rx, ry) = (if wx { x - tf } else { x }, if wy { y - tf } else { y });
    let mut v = bilinear(sat, t, rx, ry);
    if wx {
        v += bilinear(sat, t, tf, ry);
    }
    if wy {
        v += bilinear(sat, t, rx, tf);
    }
    if wx && wy {
        v += bilinear(sat, t, tf, tf);
    }
    v
}

/// The integral of torus `sat` (side `t`) over `[0, i) × [0, across)` (`along_x`) or
/// `[0, across) × [0, i)`, for `i` in `0..=t`: the table cut along a line, with the periodic part
/// of `across` folded in (`across` in `0..2t`).
fn line(sat: &[f32], t: usize, along_x: bool, across: f32, out: &mut Vec<f64>) {
    let (tf, n) = (t as f32, t + 1);
    let wrap = across >= tf;
    let a = (if wrap { across - tf } else { across }).clamp(0.0, tf);
    let j = (a as usize).min(t.saturating_sub(1));
    let f = (a - j as f32) as f64;
    out.clear();
    out.resize(n, 0.0);
    // entry `i` of table line `k` (a row when cutting along x, a column when along y)
    let at = |k: usize, i: usize| if along_x { sat.get(k * n + i) } else { sat.get(i * n + k) }.copied().unwrap_or(0.0) as f64;
    for (i, o) in out.iter_mut().enumerate() {
        let (p, q) = (at(j, i), at(j + 1, i));
        *o = p + (q - p) * f + if wrap { at(t, i) } else { 0.0 };
    }
}

/// [`line`] interpolated at `x ≥ 0` (any number of periods along).
#[inline]
fn along(line: &[f64], t: usize, inv_t: f64, x: f64) -> f64 {
    let tf = t as f64;
    let q = (x * inv_t).floor().max(0.0);
    let r = (x - q * tf).clamp(0.0, tf);
    let i = (r as usize).min(t.saturating_sub(1));
    let f = r - i as f64;
    let (Some(&a), Some(&b), Some(&end)) = (line.get(i), line.get(i + 1), line.get(t)) else { return 0.0 };
    a + (b - a) * f + q * end
}

impl CrystalTile {
    /// The grain of a line of `out.len()` output pixels side by side, each a square of side `w`
    /// (tile pixels), the first centred on (`x`, `y`), the next `step` (`±w`) along x (`along_x`)
    /// or y. The table is cut once along the line's two box edges; each pixel then costs one
    /// interpolation per torus instead of [`Self::field`]'s eight table lookups.
    pub fn row_field(&self, x: f32, y: f32, w: f32, step: f32, along_x: bool, out: &mut [f32]) {
        out.fill(0.0);
        let n = out.len();
        let (pos, across) = if along_x { (x, y) } else { (y, x) };
        let sign = if step < 0.0 { -1.0 } else { 1.0 };
        let first = pos as f64 - 0.5 * step as f64;
        let lowest = first.min(first + n as f64 * step as f64);
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for (which, &t) in TORI.iter().enumerate() {
            let Some(sat) = self.sats.get(which) else { continue };
            let tf = t as f32;
            let lo = across - 0.5 * w;
            let lo = lo - (lo / tf).floor() * tf;
            line(sat, t, along_x, lo, &mut near);
            line(sat, t, along_x, lo + w, &mut far);
            let shift = (lowest / t as f64).floor() * t as f64;
            let inv_t = 1.0 / t as f64;
            let edge = |k: usize| {
                let e = first + k as f64 * step as f64 - shift;
                along(&far, t, inv_t, e) - along(&near, t, inv_t, e)
            };
            let mut prev = edge(0);
            for (k, o) in out.iter_mut().enumerate() {
                let next = edge(k + 1);
                *o += (sign * (next - prev)) as f32;
                prev = next;
            }
        }
        let area = w * w;
        out.iter_mut().for_each(|v| *v /= area);
    }
}

// ---- the render's grain

/// The crystal grain of one render.
#[derive(Clone, Debug)]
pub struct CrystalGrain {
    pub tile: Arc<CrystalTile>,
    /// Encoded-luminance amplitude at unit field and unit tone weight.
    pub k: f32,
    /// An output pixel's side in tile pixels.
    pub foot: f32,
}

type Key = (u64, u64, u32);

/// The cache key of the tile of `look` and `seed`.
fn key_of(look: CrystalLook, seed: u32) -> Key {
    (look.width.to_bits(), look.spread.to_bits() ^ (look.layers as u64).rotate_left(40), seed)
}
/// Recently drawn tiles (the most recent last).
type Cache = Mutex<Vec<(Key, Arc<CrystalTile>)>>;
static TILES: Cache = Mutex::new(Vec::new());

/// The tile being drawn in the background for a drag ([`tile`]): one at a time.
static DRAWING: Mutex<Option<Key>> = Mutex::new(None);

/// The tile a drawing hands to the renders waiting for it, set when it is drawn.
type Slot = Arc<OnceLock<Arc<CrystalTile>>>;

/// The tiles being drawn, by cache and key, and the condition their drawers signal when done: a
/// render that misses a tile another is drawing waits for it rather than drawing it again. The
/// drawer hands the tile over in its slot, so the waiters get it even if the cache has evicted it
/// by the time they wake.
static IN_FLIGHT: Mutex<Vec<((usize, Key), Slot)>> = Mutex::new(Vec::new());
static DRAWN: Condvar = Condvar::new();

/// Marks a tile drawn (or failed) when dropped and wakes the renders waiting for it.
struct Drawing {
    id: (usize, Key),
    done: Slot,
}

impl Drawing {
    /// Registers the draw of `id` in `busy` (the locked [`IN_FLIGHT`]).
    fn register(busy: &mut Vec<((usize, Key), Slot)>, id: (usize, Key)) -> Drawing {
        let done = Slot::default();
        busy.push((id, done.clone()));
        Drawing { id, done }
    }

    /// Hands `t` to the renders waiting for it (they wake when this is dropped).
    fn finish(&self, t: &Arc<CrystalTile>) {
        // (already set: only the first tile counts, and every drawing draws the same one)
        let _ = self.done.set(t.clone());
    }
}

impl Drop for Drawing {
    fn drop(&mut self) {
        IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner).retain(|(d, _)| *d != self.id);
        DRAWN.notify_all();
    }
}

/// Registers the tile of `id` as being drawn, or `None` if another thread is drawing it.
fn start_drawing(id: (usize, Key)) -> Option<Drawing> {
    let mut busy = IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner);
    if busy.iter().any(|(d, _)| *d == id) {
        return None;
    }
    Some(Drawing::register(&mut busy, id))
}

/// The tile of `key` in `cache`.
fn cached(cache: &Cache, key: Key) -> Option<Arc<CrystalTile>> {
    let tiles = cache.lock().unwrap_or_else(PoisonError::into_inner);
    tiles.iter().find(|(k, _)| *k == key).map(|(_, t)| t.clone())
}

/// The tile of `look` and `seed`, drawn or from the cache. Unless `wait`, a tile not drawn yet
/// is drawn in the background and the last one drawn for `seed` stands in for it meanwhile: a
/// Size or Roughness drag doesn't wait for every step's tile (tens of milliseconds on a slow
/// CPU), and the full-quality render when it ends waits for the exact one. With no tile cached
/// for `seed` nothing can stand in, so even without `wait` this waits for a draw already running
/// or draws the tile.
fn tile(look: CrystalLook, seed: u32, wait: bool) -> Option<Arc<CrystalTile>> {
    tile_in(&TILES, look, seed, wait)
}

/// [`tile`] with the tiles cached in `cache` (a test's own, which other tests can't evict from).
fn tile_in(cache: &'static Cache, look: CrystalLook, seed: u32, wait: bool) -> Option<Arc<CrystalTile>> {
    let key = key_of(look, seed);
    {
        let tiles = cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, t)) = tiles.iter().find(|(k, _)| *k == key) {
            return Some(t.clone());
        }
        // (wasm has no threads to draw on)
        if !wait
            && !cfg!(target_arch = "wasm32")
            && let Some((_, stand_in)) = tiles.iter().rev().find(|(k, _)| k.2 == seed)
        {
            let stand_in = stand_in.clone();
            drop(tiles);
            draw_in_background(cache, key, look, seed);
            return Some(stand_in);
        }
    }
    let id = (std::ptr::from_ref(cache) as usize, key);
    // a pool worker draws the tile itself: blocked, it could hold up the drawing it waits for
    // (its stack may carry a part of that drawing's parallel work); wasm has no other threads
    #[cfg(not(target_arch = "wasm32"))]
    let on_worker = rayon::current_thread_index().is_some();
    #[cfg(target_arch = "wasm32")]
    let on_worker = true;
    let drawing = if !on_worker {
        // checked, waited for and registered under one lock: two renders that miss the tile
        // together can't both find it neither cached nor being drawn (`cached` takes the cache's
        // lock under this one; nothing takes them the other way round)
        let mut busy = IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner);
        let mut waited_for: Option<Slot> = None;
        loop {
            if let Some(t) = waited_for.as_ref().and_then(|done| done.get().cloned()).or_else(|| cached(cache, key)) {
                return Some(t);
            }
            let Some((_, done)) = busy.iter().find(|(d, _)| *d == id) else { break };
            waited_for = Some(done.clone());
            busy = DRAWN.wait(busy).unwrap_or_else(PoisonError::into_inner);
        }
        Some(Drawing::register(&mut busy, id))
    } else {
        start_drawing(id)
    };
    #[cfg(test)]
    if let Some(d) = &drawing {
        tests::hold_for_waiters(d);
    }
    let t = Arc::new(CrystalTile::new(look, seed)?);
    if let Some(d) = &drawing {
        d.finish(&t);
    }
    keep(cache, key, t.clone());
    #[cfg(test)]
    if let Some(d) = &drawing {
        tests::evict_before_waking(cache, d);
    }
    Some(t)
}

/// Adds `t` to `cache` (the most recent last).
fn keep(cache: &Cache, key: Key, t: Arc<CrystalTile>) {
    let mut tiles = cache.lock().unwrap_or_else(PoisonError::into_inner);
    tiles.retain(|(k, _)| *k != key);
    if tiles.len() >= CACHE {
        tiles.remove(0);
    }
    tiles.push((key, t));
}

/// Draws the tile of `key` on the thread pool unless another is being drawn (the next render
/// asks again for what it needs then).
fn draw_in_background(cache: &'static Cache, key: Key, look: CrystalLook, seed: u32) {
    let marker = {
        let mut drawing = DRAWING.lock().unwrap_or_else(PoisonError::into_inner);
        if drawing.is_some() {
            return;
        }
        // (`None`: a render is drawing it already)
        let Some(marker) = start_drawing((std::ptr::from_ref(cache) as usize, key)) else { return };
        *drawing = Some(key);
        marker
    };
    spawn_in_background(move || {
        if let Some(t) = CrystalTile::new(look, seed) {
            let t = Arc::new(t);
            marker.finish(&t);
            keep(cache, key, t);
        }
        drop(marker);
        *DRAWING.lock().unwrap_or_else(PoisonError::into_inner) = None;
    });
}

/// Runs `draw` on the background pool.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_in_background(draw: impl FnOnce() + Send + 'static) {
    // on a pool of its own: the renders of the drag don't queue behind its parallel bands
    match background_pool() {
        Some(pool) => pool.spawn(draw),
        None => rayon::spawn(draw),
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_in_background(draw: impl FnOnce() + Send + 'static) {
    draw();
}

/// Half the cores, for tiles drawn in the background (`None`: the pool couldn't be made).
#[cfg(not(target_arch = "wasm32"))]
fn background_pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, |n| n.get()).div_ceil(2);
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("grain-tile-{i}")).build().ok()
    })
    .as_ref()
}

/// The tone weight of an encoded channel value `v` (0..1): `Y·dV/dY` of the sRGB curve, i.e. how
/// much `v` moves when the channel's linear light is scaled.
#[inline]
pub fn lift(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 { v } else { (v + 0.055) / 2.4 }
}

/// The tone weight at mid grey (what [`CrystalGrain::k`] is calibrated at).
fn mid_weight() -> f32 {
    lift(MID_GREY) * (1.0 - srgb_to_linear(MID_GREY))
}

impl CrystalGrain {
    /// The grain of `g` at `px_per_long` output pixels per long edge (`None`: no grain).
    pub fn new(g: &Grain, px_per_long: f64) -> Option<CrystalGrain> {
        Self::with(g, px_per_long, true)
    }

    /// [`Self::new`] for a draft render (a drag): while its tile is drawn, the last tile drawn for
    /// the same seed stands in for it (see [`tile`]).
    pub fn draft(g: &Grain, px_per_long: f64) -> Option<CrystalGrain> {
        Self::with(g, px_per_long, false)
    }

    fn with(g: &Grain, px_per_long: f64, wait: bool) -> Option<CrystalGrain> {
        let levels = strength_levels(g.amount, g.size, g.roughness);
        if !(levels > 0.0 && px_per_long.is_finite() && px_per_long > 0.0) {
            return None;
        }
        let foot = (TILE_PER_LONG / px_per_long) as f32;
        if !(foot.is_finite() && foot > 0.0 && foot < MAX_FOOTPRINT) {
            return None;
        }
        let tile = tile(CrystalLook::new(g.size, g.roughness), g.seed, wait)?;
        Some(CrystalGrain { tile, k: (levels / 255.0) as f32 / mid_weight(), foot })
    }

    /// Display-encoded colour `e` with grain, at frame position (`gx`, `gy`) (fractions of the
    /// frame's long edge): the mean over an axis-aligned square footprint centred there, which
    /// for a rotated frame's pixel stands in for its rotated footprint (`docs/grain.md`).
    #[inline]
    pub fn apply(&self, e: [f32; 3], gx: f32, gy: f32) -> [f32; 3] {
        if !(gx.is_finite() && gy.is_finite()) {
            return e;
        }
        let tpl = TILE_PER_LONG as f32;
        self.apply_field(e, self.tile.field(gx * tpl, gy * tpl, self.foot))
    }

    /// The grain field of a row of `out.len()` pixels whose first centre is at (`gx`, `gy`) and
    /// which step (`dx`, `dy`) (fractions of the long edge), when the step is along x or y (an
    /// unrotated frame in any orientation) and the pixels are squares of the render's size
    /// (`false` otherwise: use [`Self::apply`] per pixel).
    pub fn row(&self, gx: f64, gy: f64, dx: f64, dy: f64, out: &mut [f32]) -> bool {
        let tpl = TILE_PER_LONG;
        let ((sx, sy), w) = ((dx * tpl, dy * tpl), self.foot as f64);
        let (step, along_x) = if sy.abs() <= 1e-6 {
            (sx, true)
        } else if sx.abs() <= 1e-6 {
            (sy, false)
        } else {
            return false;
        };
        if !(gx.is_finite() && gy.is_finite() && step.is_finite()) || (step.abs() - w).abs() > 1e-4 * w {
            return false;
        }
        self.tile.row_field((gx * tpl) as f32, (gy * tpl) as f32, self.foot, step as f32, along_x, out);
        true
    }

    /// Display-encoded colour `e` with grain field value `g` (see [`CrystalTile::field`]).
    #[inline]
    pub fn apply_field(&self, e: [f32; 3], g: f32) -> [f32; 3] {
        let lum = (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]).clamp(0.0, 1.0);
        let k = self.k * g * (1.0 - srgb_to_linear(lum));
        if !k.is_finite() {
            return e;
        }
        e.map(|v| v + k * lift(v))
    }
}

#[cfg(test)]
#[path = "tests_crystal.rs"]
mod tests;
