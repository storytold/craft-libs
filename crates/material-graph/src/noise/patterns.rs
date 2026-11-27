//! Geometric pattern looks as point functions of (u, v), tileable with period 1: [`TriangleGrid`] (a simple
//! split-square grid and a jittered triangulated lattice), [`Weave`], [`WeaveGenerator`] and [`Starburst`].

use super::{rnd, smoothstep as smooth, wrem};
use crate::math;
use std::f32::consts::TAU;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A grid of triangles: each square split along its diagonal, with a random value per triangle and soft borders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleGrid {
    pub count: i32,
    pub border: f32,
    pub variation: f32,
    pub seed: u32,
}

impl TriangleGrid {
    // 0.7071 (not FRAC_1_SQRT_2) keeps the node's output bit-identical to before the extraction
    #[allow(clippy::approx_constant)]
    pub fn value(&self, u: f32, v: f32) -> f32 {
        let (c, bw, var, seed) = (self.count, self.border, self.variation, self.seed);
        let (x, y) = (u * c as f32, v * c as f32);
        let (ix, iy) = (x.floor(), y.floor());
        let (fx, fy) = (x - ix, y - iy);
        let tri = (fx + fy >= 1.0) as u32;
        let d = fx.min(fy).min(1.0 - fx).min(1.0 - fy).min((fx + fy - 1.0).abs() * 0.7071);
        let id = rnd(wrem(ix as i32, c), wrem(iy as i32, c), seed.wrapping_add(tri * 131));
        (1.0 - var + var * id) * smooth(0.0, bw.max(1e-4), d)
    }
}

/// A jittered lattice cut into triangles: heights interpolated over each triangle, a value per triangle or the
/// distance to the edges; optional thin lines along the edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleLattice {
    pub nx: i32,
    pub ny: i32,
    /// Jitter amplitude in grid units.
    pub amp: f32,
    pub quincunx: (f32, f32),
    pub height_random: f32,
    /// 0 all the same diagonal, 1 alternating, 2 random.
    pub triangulation: i32,
    /// 0 heights, 1 a value per triangle, 2 the distance to the edges.
    pub output: i32,
    pub line_thickness: f32,
    pub line_color: f32,
    pub line_background: f32,
    pub lines: bool,
    pub seed: u32,
}

impl TriangleLattice {
    /// Jittered lattice point (i, j) in grid units and its height; indices wrap so the pattern tiles.
    fn vert(&self, i: i32, j: i32) -> ([f32; 2], f32) {
        let (seed, amp) = (self.seed, self.amp);
        let (qx, qy) = self.quincunx;
        let (wi, wj) = (wrem(i, self.nx), wrem(j, self.ny));
        let jx = (rnd(wi, wj, seed) - 0.5) * 2.0 * amp;
        let jy = (rnd(wi, wj, seed.wrapping_add(7919)) - 0.5) * 2.0 * amp;
        let sh = if j.rem_euclid(2) == 1 { qx } else { 0.0 };
        let sv = if i.rem_euclid(2) == 1 { qy } else { 0.0 };
        let h = 0.5 + (rnd(wi, wj, seed.wrapping_add(104729)) - 0.5) * self.height_random;
        ([i as f32 + jx + sh * 0.5, j as f32 + jy + sv * 0.5], h)
    }

    pub fn value(&self, u: f32, v: f32) -> f32 {
        let (nx, ny, seed, hr) = (self.nx, self.ny, self.seed, self.height_random);
        let (x, y) = (u * nx as f32, v * ny as f32);
        let (ci, cj) = (x.floor() as i32, y.floor() as i32);
        let mut best = (f32::MAX, 0.0f32, 0.0f32);
        for dj in -1..=1 {
            for di in -1..=1 {
                let (i, j) = (ci.wrapping_add(di), cj.wrapping_add(dj));
                let (wi, wj) = (wrem(i, nx), wrem(j, ny));
                let alt = match self.triangulation {
                    1 => i.wrapping_add(j) & 1,
                    2 => (rnd(wi, wj, seed.wrapping_add(31)) > 0.5) as i32,
                    _ => 0,
                };
                let (a, ha) = self.vert(i, j);
                let (b, hb) = self.vert(i.wrapping_add(1), j);
                let (c, hc) = self.vert(i, j.wrapping_add(1));
                let (d, hd) = self.vert(i.wrapping_add(1), j.wrapping_add(1));
                let tris: [([[f32; 2]; 3], [f32; 3]); 2] = if alt == 0 {
                    [([a, b, d], [ha, hb, hd]), ([a, d, c], [ha, hd, hc])]
                } else {
                    [([a, b, c], [ha, hb, hc]), ([b, d, c], [hb, hd, hc])]
                };
                for (t, (p, hs)) in tris.iter().enumerate() {
                    let det = (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
                    if det.abs() < 1e-9 {
                        continue;
                    }
                    let l0 = ((p[1][1] - p[2][1]) * (x - p[2][0]) + (p[2][0] - p[1][0]) * (y - p[2][1])) / det;
                    let l1 = ((p[2][1] - p[0][1]) * (x - p[2][0]) + (p[0][0] - p[2][0]) * (y - p[2][1])) / det;
                    let l2 = 1.0 - l0 - l1;
                    let m = l0.min(l1).min(l2);
                    if m < -1e-5 {
                        continue;
                    }
                    // distance to the nearest triangle edge in grid units
                    let mut de = f32::MAX;
                    for (e0, e1) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                        let (ex, ey) = (e1[0] - e0[0], e1[1] - e0[1]);
                        let len = (ex * ex + ey * ey).sqrt().max(1e-9);
                        de = de.min(((x - e0[0]) * ey - (y - e0[1]) * ex).abs() / len);
                    }
                    let tid = rnd(wi, wj, seed.wrapping_add(555 + t as u32 * 13 + alt as u32 * 3));
                    let val = match self.output {
                        1 => 0.5 + (tid - 0.5) * hr,
                        2 => (de * 2.5).min(1.0),
                        _ => l0 * hs[0] + l1 * hs[1] + l2 * hs[2],
                    };
                    let cand = (-m, val, de);
                    if cand.0 < best.0 {
                        best = cand;
                    }
                }
            }
        }
        let (val, de) = (best.1, best.2);
        let th = self.line_thickness;
        if self.lines && th > 0.0 {
            let w = th * 6.0;
            let k = 1.0 - smooth(w * 0.5, w, de);
            lerp(lerp(val, self.line_background, 0.0), self.line_color, k)
        } else {
            val
        }
    }
}

/// Woven threads over and under each other; `count` is even so the pattern wraps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weave {
    pub count: f32,
    /// Thread width 0.1..1.
    pub width: f32,
}

impl Weave {
    pub fn value(&self, u: f32, v: f32) -> f32 {
        let (cnt, wd) = (self.count, self.width);
        let (x, y) = (u * cnt, v * cnt);
        let (i, j) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - i as f32, y - j as f32);
        let hv = (1.0 - ((2.0 * fx - 1.0).abs() / wd).clamp(0.0, 1.0)).sqrt();
        let hh = (1.0 - ((2.0 * fy - 1.0).abs() / wd).clamp(0.0, 1.0)).sqrt();
        if i.wrapping_add(j) & 1 == 0 { hv.max(0.5 * hh) } else { hh.max(0.5 * hv) }
    }
}

/// Woven threads in slots with gaps between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaveGenerator {
    /// Slots per side.
    pub nx: f32,
    pub ny: f32,
    /// How far a thread dips where it passes under its neighbour, 0..1.
    pub shape: f32,
    pub gap: (f32, f32),
    /// The node's extra outputs: 0 the weave, 1 the horizontal threads, 2 the vertical threads, 3 all threads (masks).
    pub output: u32,
}

impl WeaveGenerator {
    /// `tile` repeats of a weave of `weave` threads.
    pub fn new(tile: (i32, i32), weave: i32, shape: f32, gap: (f32, f32)) -> WeaveGenerator {
        WeaveGenerator {
            nx: tile.0.wrapping_mul(weave).wrapping_mul(2) as f32,
            ny: tile.1.wrapping_mul(weave).wrapping_mul(2) as f32,
            shape,
            gap,
            output: 0,
        }
    }

    pub fn value(&self, u: f32, v: f32) -> f32 {
        let (nx, ny, shape) = (self.nx, self.ny, self.shape);
        let (gx, gy) = self.gap;
        let (nxi, nyi) = (nx as i32, ny as i32);
        let (x, y) = (u * nx, v * ny);
        let (i, j) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - i as f32, y - j as f32);
        let (i, j) = (wrem(i, nxi), wrem(j, nyi));
        let in_gx = fx < gx * 0.5 || fx > 1.0 - gx * 0.5;
        let in_gy = fy < gy * 0.5 || fy > 1.0 - gy * 0.5;
        let thread = !in_gx && !in_gy;
        let horizontal = i.wrapping_add(j) & 1 == 0;
        match self.output {
            1 => return (thread && horizontal) as u32 as f32,
            2 => return (thread && !horizontal) as u32 as f32,
            3 => return thread as u32 as f32,
            _ => {}
        }
        if in_gx && in_gy {
            return 0.0;
        }
        if in_gx || in_gy {
            return 0.5;
        }
        // a thread runs along the slot; it dips towards both ends where it passes under its neighbour
        let along = if horizontal { fx } else { fy };
        let e = (along - 0.5).abs() * 2.0;
        if shape <= 0.0 { 1.0 } else { 1.0 - 0.5 * smooth(1.0 - shape, 1.0, e) }
    }
}

/// Star-like rays, one star per cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Starburst {
    pub count: i32,
    pub rays: f32,
    pub sharpness: f32,
    pub size: f32,
    pub rotation: f32,
    pub rotation_random: f32,
    pub seed: u32,
}

impl Starburst {
    pub fn value(&self, u: f32, v: f32) -> f32 {
        let c = self.count;
        let (x, y) = (u * c as f32, v * c as f32);
        let (ix, iy) = (x.floor(), y.floor());
        let (dx, dy) = (x - ix - 0.5, y - iy - 0.5);
        let r = (dx * dx + dy * dy).sqrt();
        let off = self.rotation + self.rotation_random * rnd(wrem(ix as i32, c), wrem(iy as i32, c), self.seed);
        let a = (math::atan2(dy, dx) / TAU + off) * self.rays;
        let ray = 1.0 - (2.0 * (a - a.floor()) - 1.0).abs();
        math::powf(ray, self.sharpness) * (1.0 - r * 2.0 / self.size).clamp(0.0, 1.0)
    }
}
