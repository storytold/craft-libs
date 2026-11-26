// SPDX-License-Identifier: MIT OR Apache-2.0
//! Seeded, tileable procedural noise, and the cell, spot and pattern looks built on it ([`cells`], [`patterns`]).
use crate::math;
use std::f32::consts::TAU;

pub mod cells;
pub mod patterns;

pub use cells::{BnwSpots, CellSource, Cells, Crystal, GaussianSpots, VoronoiFractal, VoronoiSettings};
pub use patterns::{Starburst, TriangleGrid, TriangleLattice, Weave, WeaveGenerator};

/// Smooth Hermite step from 0 at `a` to 1 at `b` (a hard step when `a == b`).
pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

pub fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// `x.rem_euclid(n)` that never panics: 0 for `n == 0`, and the wrapped result for `i32::MIN % -1`.
#[inline]
pub(crate) fn wrem(x: i32, n: i32) -> i32 {
    if n == 0 { 0 } else { x.wrapping_rem_euclid(n) }
}

pub fn rnd(x: i32, y: i32, seed: u32) -> f32 {
    (hash2(x, y, seed) >> 8) as f32 / 16_777_216.0
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Unit gradient vectors for 4096 evenly spaced angles (replaces a cos/sin per lattice corner).
fn grad_table() -> &'static [[f32; 2]; 4096] {
    static T: std::sync::OnceLock<Box<[[f32; 2]; 4096]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([[0.0f32; 2]; 4096]);
        for (i, e) in t.iter_mut().enumerate() {
            let a = i as f32 / 4096.0 * TAU;
            *e = [math::cos(a), math::sin(a)];
        }
        t
    })
}

/// Gradient at a lattice point: same pseudo-random angle as `rnd(x, y, seed) * TAU`, quantised to 4096 steps.
#[inline]
fn grad_at(x: i32, y: i32, seed: u32) -> [f32; 2] {
    grad_table().get((hash2(x, y, seed) >> 20) as usize).copied().unwrap_or([0.0; 2])
}

/// Tileable gradient (Perlin) noise in 0..1. `px`,`py` are integer lattice periods.
pub fn perlin(u: f32, v: f32, px: i32, py: i32, seed: u32) -> f32 {
    let px = px.max(1);
    let py = py.max(1);
    let x = u * px as f32;
    let y = v * py as f32;
    let fxl = x.floor();
    let fyl = y.floor();
    let ix = fxl as i32;
    let iy = fyl as i32;
    let fx = x - fxl;
    let fy = y - fyl;
    let tab = grad_table();
    let g = |cx: i32, cy: i32, dx: f32, dy: f32| {
        let c = tab.get((hash2(cx.rem_euclid(px), cy.rem_euclid(py), seed) >> 20) as usize).copied().unwrap_or([0.0; 2]);
        c[0] * dx + c[1] * dy
    };
    let n00 = g(ix, iy, fx, fy);
    let n10 = g(ix.wrapping_add(1), iy, fx - 1.0, fy);
    let n01 = g(ix, iy.wrapping_add(1), fx, fy - 1.0);
    let n11 = g(ix.wrapping_add(1), iy.wrapping_add(1), fx - 1.0, fy - 1.0);
    let sx = fade(fx);
    let sy = fade(fy);
    let n = lerp(lerp(n00, n10, sx), lerp(n01, n11, sx), sy);
    (n * std::f32::consts::FRAC_1_SQRT_2 + 0.5).clamp(0.0, 1.0)
}

/// Fractal noise. mode: 0 = fBm, 1 = ridged, 2 = turbulence.
#[allow(clippy::too_many_arguments)]
pub fn fbm(u: f32, v: f32, px: i32, py: i32, octaves: u32, gain: f32, seed: u32, mode: u32) -> f32 {
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;
    let cx = px.max(1);
    let cy = py.max(1);
    // Every octave is rotated by about 26.6 degrees and scaled by sqrt(5) with the INTEGER matrix [[2, -1], [1, 2]], so
    // the lattice directions of different octaves never line up (no visible grid) and the noise still tiles exactly.
    let (mut a, mut b) = (u, v);
    for o in 0..octaves.clamp(1, 12) {
        let n = perlin(a, b, cx, cy, seed.wrapping_add(o.wrapping_mul(131)));
        let n = match mode {
            1 => 1.0 - (2.0 * n - 1.0).abs(),
            2 => (2.0 * n - 1.0).abs(),
            _ => n,
        };
        sum += n * amp;
        norm += amp;
        amp *= gain;
        let (na, nb) = (2.0 * a - b, a + 2.0 * b);
        // keep the coordinates small: whole periods do not change a periodic function
        a = na.rem_euclid(1.0);
        b = nb.rem_euclid(1.0);
    }
    if norm > 0.0 {
        // averaged octaves cluster around mid grey; stretch them so the result uses most of the 0..1 range
        let m = sum / norm;
        let k = match mode {
            0 => 1.7,
            _ => 1.35,
        };
        (0.5 + (m - 0.5) * k).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Fractal noise with the gradient lattices of all octaves built once (no hashing per pixel). Gives exactly the same
/// values as `fbm`; use it when many pixels share the same parameters.
pub struct Fbm {
    grids: Vec<GradGrid>,
    px: i32,
    py: i32,
    octaves: u32,
    gain: f32,
    seed: u32,
    mode: u32,
}

impl Fbm {
    pub fn new(px: i32, py: i32, octaves: u32, gain: f32, seed: u32, mode: u32) -> Fbm {
        let (cx, cy) = (px.max(1), py.max(1));
        let octaves = octaves.clamp(1, 12);
        // very coarse lattices would need huge tables: those fall back to hashing in `sample`
        let grids = if (cx as i64) * (cy as i64) <= (1 << 20) {
            (0..octaves).map(|o| GradGrid::new(cx, cy, seed.wrapping_add(o.wrapping_mul(131)))).collect()
        } else {
            Vec::new()
        };
        Fbm { grids, px: cx, py: cy, octaves, gain, seed, mode }
    }

    pub fn sample(&self, u: f32, v: f32) -> f32 {
        if self.grids.is_empty() {
            return fbm(u, v, self.px, self.py, self.octaves, self.gain, self.seed, self.mode);
        }
        let (mut amp, mut sum, mut norm) = (1.0f32, 0.0f32, 0.0f32);
        let (mut a, mut b) = (u, v);
        for g in &self.grids {
            let n = g.sample(a, b);
            let n = match self.mode {
                1 => 1.0 - (2.0 * n - 1.0).abs(),
                2 => (2.0 * n - 1.0).abs(),
                _ => n,
            };
            sum += n * amp;
            norm += amp;
            amp *= self.gain;
            let (na, nb) = (2.0 * a - b, a + 2.0 * b);
            a = na - crate::tex::ffloor(na);
            b = nb - crate::tex::ffloor(nb);
        }
        if norm > 0.0 {
            let m = sum / norm;
            let k = if self.mode == 0 { 1.7 } else { 1.35 };
            (0.5 + (m - 0.5) * k).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Tileable Voronoi. Returns (f1, f2, cell_id) with distances normalised to ~0..1.
pub fn voronoi(u: f32, v: f32, px: i32, py: i32, seed: u32, jitter: f32) -> (f32, f32, f32) {
    let px = px.max(1);
    let py = py.max(1);
    let x = u * px as f32;
    let y = v * py as f32;
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let mut f1 = f32::MAX;
    let mut f2 = f32::MAX;
    let mut id = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cx = ix.wrapping_add(dx);
            let cy = iy.wrapping_add(dy);
            let wx = cx.rem_euclid(px);
            let wy = cy.rem_euclid(py);
            let fx = cx as f32 + 0.5 + (rnd(wx, wy, seed) - 0.5) * jitter;
            let fy = cy as f32 + 0.5 + (rnd(wx, wy, seed.wrapping_add(77)) - 0.5) * jitter;
            let d = (math::powi(fx - x, 2) + math::powi(fy - y, 2)).sqrt();
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = rnd(wx, wy, seed.wrapping_add(999));
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    let k = std::f32::consts::FRAC_1_SQRT_2;
    ((f1 * k).clamp(0.0, 1.0), (f2 * k).clamp(0.0, 1.0), id)
}

/// Soft round spots, one disc per cell, tileable. `shape(id)` gives the (inner, outer) radius of a cell's disc in the
/// same normalised distance units as `voronoi`; discs may overlap the neighbouring cells, so the union of the 3x3
/// surrounding discs is taken (no straight cuts where cells meet). `grain` is added to the distance.
pub fn spots<F: Fn(f32) -> (f32, f32)>(u: f32, v: f32, px: i32, py: i32, seed: u32, shape: F, grain: f32) -> f32 {
    let (px, py) = (px.max(1), py.max(1));
    let (x, y) = (u * px as f32, v * py as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let mut best = 0.0f32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix.wrapping_add(dx), iy.wrapping_add(dy));
            let (wx, wy) = (cx.rem_euclid(px), cy.rem_euclid(py));
            let fx = cx as f32 + 0.5 + (rnd(wx, wy, seed) - 0.5);
            let fy = cy as f32 + 0.5 + (rnd(wx, wy, seed.wrapping_add(77)) - 0.5);
            let d = ((math::powi(fx - x, 2) + math::powi(fy - y, 2)).sqrt() * k + grain).max(0.0);
            let (lo, hi) = shape(rnd(wx, wy, seed.wrapping_add(999)));
            let t = ((d - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
            best = best.max(1.0 - t * t * (3.0 - 2.0 * t));
        }
    }
    best
}

pub struct Vor {
    pub f1: f32,
    pub f2: f32,
    pub id1: f32,
    pub id2: f32,
    pub dx: f32,
    pub dy: f32,
}

/// Tileable Voronoi with the two nearest cells and the vector to the nearest site.
pub fn voronoi_ext(u: f32, v: f32, px: i32, py: i32, seed: u32, jitter: f32) -> Vor {
    let px = px.max(1);
    let py = py.max(1);
    let x = u * px as f32;
    let y = v * py as f32;
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut r = Vor { f1: f32::MAX, f2: f32::MAX, id1: 0.0, id2: 0.0, dx: 0.0, dy: 0.0 };
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix.wrapping_add(dx), iy.wrapping_add(dy));
            let (wx, wy) = (cx.rem_euclid(px), cy.rem_euclid(py));
            let fx = cx as f32 + 0.5 + (rnd(wx, wy, seed) - 0.5) * jitter;
            let fy = cy as f32 + 0.5 + (rnd(wx, wy, seed.wrapping_add(77)) - 0.5) * jitter;
            let d = (math::powi(fx - x, 2) + math::powi(fy - y, 2)).sqrt();
            let id = rnd(wx, wy, seed.wrapping_add(999));
            if d < r.f1 {
                r.f2 = r.f1;
                r.id2 = r.id1;
                r.f1 = d;
                r.id1 = id;
                r.dx = fx - x;
                r.dy = fy - y;
            } else if d < r.f2 {
                r.f2 = d;
                r.id2 = id;
            }
        }
    }
    let k = std::f32::consts::FRAC_1_SQRT_2;
    r.f1 = (r.f1 * k).clamp(0.0, 1.0);
    r.f2 = (r.f2 * k).clamp(0.0, 1.0);
    r
}

/// Precomputed gradient lattice for fast, repeated tileable Perlin lookups (no hashing or trig per sample).
/// `sample` returns exactly the same values as `perlin(u, v, px, py, seed)`.
pub struct GradGrid {
    px: i32,
    py: i32,
    g: Vec<[f32; 2]>,
}

impl GradGrid {
    pub fn new(px: i32, py: i32, seed: u32) -> GradGrid {
        let px = px.clamp(1, 4096);
        let py = py.clamp(1, 4096);
        let mut g = Vec::with_capacity((px as usize) * (py as usize));
        for y in 0..py {
            for x in 0..px {
                g.push(grad_at(x, y, seed));
            }
        }
        GradGrid { px, py, g }
    }

    /// Periodic in u and v with period 1, for any (also negative or > 1) coordinates.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = u * self.px as f32;
        let y = v * self.py as f32;
        let (fxl, fyl) = (crate::tex::ffloor(x), crate::tex::ffloor(y));
        let (fx, fy) = (x - fxl, y - fyl);
        // wrap the two lattice columns / rows once instead of once per corner
        let x0 = (fxl as i64).rem_euclid(self.px as i64) as usize;
        let y0 = (fyl as i64).rem_euclid(self.py as i64) as usize;
        let x1 = if x0 + 1 == self.px as usize { 0 } else { x0 + 1 };
        let y1 = if y0 + 1 == self.py as usize { 0 } else { y0 + 1 };
        let row0 = y0 * self.px as usize;
        let row1 = y1 * self.px as usize;
        let at = |i: usize| self.g.get(i).copied().unwrap_or([0.0; 2]);
        let (c00, c10, c01, c11) = (at(row0 + x0), at(row0 + x1), at(row1 + x0), at(row1 + x1));
        let n00 = c00[0] * fx + c00[1] * fy;
        let n10 = c10[0] * (fx - 1.0) + c10[1] * fy;
        let n01 = c01[0] * fx + c01[1] * (fy - 1.0);
        let n11 = c11[0] * (fx - 1.0) + c11[1] * (fy - 1.0);
        let (sx, sy) = (fade(fx), fade(fy));
        let n = lerp(lerp(n00, n10, sx), lerp(n01, n11, sx), sy);
        (n * std::f32::consts::FRAC_1_SQRT_2 + 0.5).clamp(0.0, 1.0)
    }
}

/// The shared "Disorder" block of the noise nodes: the sampling position is displaced by a smooth, tileable random
/// field. `disorder` is the amount, `speed` the distance it moves per unit of disorder, `aniso` (0..1) narrows the displacement
/// to one direction and `angle_turns` sets that direction.
#[derive(Clone, Copy)]
pub struct Disorder {
    amp: f32,
    seed: u32,
    aniso: f32,
    cs: f32,
    sn: f32,
}

impl Disorder {
    pub fn new(seed: u32, disorder: f32, speed: f32, aniso: f32, angle_turns: f32) -> Disorder {
        let (sn, cs) = math::sin_cos(angle_turns * TAU);
        Disorder { amp: disorder.max(0.0) * speed.max(0.0) * 0.3, seed, aniso: aniso.clamp(0.0, 1.0), cs, sn }
    }
    pub fn warp(&self, u: f32, v: f32) -> (f32, f32) {
        if self.amp <= 0.0 {
            return (u, v);
        }
        let mut dx = (perlin(u, v, 3, 3, self.seed.wrapping_add(101)) - 0.5) * self.amp;
        let mut dy = (perlin(u, v, 3, 3, self.seed.wrapping_add(202)) - 0.5) * self.amp;
        if self.aniso > 0.0 {
            // project onto the chosen direction (the factor restores the length lost by the projection)
            let along = (dx * self.cs + dy * self.sn) * std::f32::consts::SQRT_2;
            dx += (along * self.cs - dx) * self.aniso;
            dy += (along * self.sn - dy) * self.aniso;
        }
        (u + dx, v + dy)
    }
}

/// Seed of another tile of the infinite noise plane (Tile offset, integer part).
pub fn tile_seed(seed: u32, tx: i32, ty: i32) -> u32 {
    hash2(tx, ty, seed.wrapping_add(0x7157))
}

/// Distance metrics of the Voronoi node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Metric {
    Euclidean,
    Manhattan,
    Chebyshev,
    Minkowski(f32),
}

impl Metric {
    pub fn from_index(i: u32, p: f32) -> Metric {
        match i {
            1 => Metric::Manhattan,
            2 => Metric::Chebyshev,
            3 => Metric::Minkowski(p.clamp(0.25, 16.0)),
            _ => Metric::Euclidean,
        }
    }
    #[inline]
    fn dist(self, dx: f32, dy: f32) -> f32 {
        let (ax, ay) = (dx.abs(), dy.abs());
        match self {
            Metric::Euclidean => (dx * dx + dy * dy).sqrt(),
            Metric::Manhattan => ax + ay,
            Metric::Chebyshev => ax.max(ay),
            Metric::Minkowski(p) => math::powf(math::powf(ax, p) + math::powf(ay, p), 1.0 / p),
        }
    }
}

/// Tileable Voronoi with a selectable distance metric. `id_seed` seeds the random cell values (so the colours can
/// be re-rolled without moving the cells). Distances are normalised like [`voronoi_ext`].
#[allow(clippy::too_many_arguments)]
pub fn voronoi_gen(u: f32, v: f32, px: i32, py: i32, seed: u32, id_seed: u32, jitter: f32, metric: Metric) -> Vor {
    let (px, py) = (px.max(1), py.max(1));
    let (x, y) = (u * px as f32, v * py as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut r = Vor { f1: f32::MAX, f2: f32::MAX, id1: 0.0, id2: 0.0, dx: 0.0, dy: 0.0 };
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (ix.wrapping_add(dx), iy.wrapping_add(dy));
            let (wx, wy) = (cx.rem_euclid(px), cy.rem_euclid(py));
            let fx = cx as f32 + 0.5 + (rnd(wx, wy, seed) - 0.5) * jitter;
            let fy = cy as f32 + 0.5 + (rnd(wx, wy, seed.wrapping_add(77)) - 0.5) * jitter;
            let d = metric.dist(fx - x, fy - y);
            let id = rnd(wx, wy, id_seed.wrapping_add(999));
            if d < r.f1 {
                r.f2 = r.f1;
                r.id2 = r.id1;
                r.f1 = d;
                r.id1 = id;
                r.dx = fx - x;
                r.dy = fy - y;
            } else if d < r.f2 {
                r.f2 = d;
                r.id2 = id;
            }
        }
    }
    let k = std::f32::consts::FRAC_1_SQRT_2;
    r.f1 = (r.f1 * k).clamp(0.0, 1.0);
    r.f2 = (r.f2 * k).clamp(0.0, 1.0);
    r
}

/// Voronoi "Style": 0 F1, 1 random colour, 2 edge, 3 F2, 4 F2-F1, 5 F1*F2, 6 F1/F2.
/// `thick` is the edge thickness, `round` rounds the edge profile (0 = linear).
pub fn voronoi_style(r: &Vor, style: u32, thick: f32, round: f32) -> f32 {
    match style {
        1 => r.id1,
        2 => {
            let e = ((r.f2 - r.f1) / thick.max(1e-3)).clamp(0.0, 1.0);
            e + (e * e * (3.0 - 2.0 * e) - e) * round.clamp(0.0, 1.0)
        }
        3 => r.f2,
        4 => ((r.f2 - r.f1) * 2.0).clamp(0.0, 1.0),
        5 => (r.f1 * r.f2 * 3.0).clamp(0.0, 1.0),
        6 => (r.f1 / r.f2.max(1e-4)).clamp(0.0, 1.0),
        _ => r.f1,
    }
}
