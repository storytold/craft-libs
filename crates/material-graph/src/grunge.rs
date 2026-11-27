// SPDX-License-Identifier: MIT OR Apache-2.0
//! Our own grunge map family (fifteen procedural looks: specks, drips, blotches, scratches, crazing, cracks).
//! Each recipe builds a tileable "dirt" field from our noise primitives; `GrungeP` then thresholds it with balance /
//! contrast like a grunge map. The recipes are our own procedural designs.
//! [`GrungeVariant`] holds the four looks of the `grunge` node (concrete, scratches, spots, smudges).
use crate::math;
use crate::noise;

mod variants;

pub use variants::{GrungeVariant, Variant};

#[derive(Clone, Copy)]
pub struct GrungeP {
    pub map: u32,
    pub seed: u32,
    pub balance: f32,
    pub contrast: f32,
    pub brush: f32,
    pub cracks: f32,
    pub disorder: f32,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Coverage threshold of each map (dirt where the field exceeds it); tuned so the default looks have a similar
/// amount of dirt to the originals.
pub const T0: [f32; 15] = [0.846, 0.733, 0.695, 0.214, 0.849, 0.775, 0.672, 0.504, 0.695, 0.405, 0.432, 0.797, 0.473, 0.701, 0.783];

fn blot(a: f32, b: f32, px: i32, seed: u32) -> f32 {
    noise::fbm(a, b, px, px, 5, 0.55, seed, 0)
}

fn streak(a: f32, b: f32, px: i32, py: i32, seed: u32) -> f32 {
    noise::fbm(a, b, px, py, 4, 0.55, seed, 0)
}

fn vband(b: f32, centre: f32) -> f32 {
    0.5 + 0.5 * math::cos(std::f32::consts::TAU * (b - centre))
}

fn crack(a: f32, b: f32, px: i32, seed: u32, width: f32) -> f32 {
    // jagged hairline cracks: a fine warp roughens the cell borders, the width wanders along each crack, short gaps
    // break them up and a second, smaller network adds branches
    let jx = noise::fbm(a, b, 48, 48, 3, 0.6, seed.wrapping_add(61), 0) - 0.5;
    let jy = noise::fbm(a, b, 48, 48, 3, 0.6, seed.wrapping_add(62), 0) - 0.5;
    let (ja, jb) = ((a + jx * 0.03).rem_euclid(1.0), (b + jy * 0.03).rem_euclid(1.0));
    let wv = 0.35 + 1.3 * noise::fbm(a, b, 36, 36, 2, 0.5, seed.wrapping_add(63), 0);
    let gap = smooth(0.28, 0.5, noise::fbm(a, b, 28, 28, 3, 0.55, seed.wrapping_add(64), 0));
    let (f1, f2, _) = noise::voronoi(ja, jb, px, px, seed, 1.0);
    let main = 1.0 - smooth(0.0, width * wv, f2 - f1);
    let (g1, g2, _) = noise::voronoi(ja, jb, px * 2 + 1, px * 2 + 1, seed.wrapping_add(65), 1.0);
    let branch = (1.0 - smooth(0.0, width * 0.5 * wv, g2 - g1)) * smooth(0.45, 0.65, noise::fbm(a, b, 14, 14, 3, 0.55, seed.wrapping_add(66), 0));
    (main.max(branch * 0.9)) * gap
}

/// Dirt field in 0..1 (higher = more dirt) for map 1..=15.
pub fn dirt(p: &GrungeP, u: f32, v: f32) -> f32 {
    let s = p.seed;
    let (mut a, mut b) = (u, v);
    if p.disorder > 0.0 {
        let w = noise::fbm(u, v, 4, 4, 3, 0.5, s.wrapping_add(900), 0) - 0.5;
        let w2 = noise::fbm(u, v, 4, 4, 3, 0.5, s.wrapping_add(901), 0) - 0.5;
        a = (u + w * p.disorder * 0.2).rem_euclid(1.0);
        b = (v + w2 * p.disorder * 0.2).rem_euclid(1.0);
    }
    // every map gets a ragged edge: a small turbulent warp plus fine grain, so nothing looks like a clean cell or disc
    let rag = if matches!(p.map, 9..=11 | 13) { 0.02 } else { 0.03 };
    let (wu, wv) = (noise::fbm(a, b, 20, 20, 4, 0.55, s.wrapping_add(940), 0) - 0.5, noise::fbm(a, b, 20, 20, 4, 0.55, s.wrapping_add(941), 0) - 0.5);
    let (a, b) = ((a + wu * rag * 2.0).rem_euclid(1.0), (b + wv * rag * 2.0).rem_euclid(1.0));
    let grain = (noise::fbm(a, b, 200, 200, 2, 0.6, s.wrapping_add(950), 0) - 0.5) * 0.25;
    let d = match p.map {
        1 => 0.55 * blot(a, b, 6, s) + 0.45 * noise::fbm(a, b, 96, 96, 2, 0.5, s.wrapping_add(1), 0),
        2 => 0.6 * streak(a, b, 48, 3, s) + 0.4 * blot(a, b, 4, s.wrapping_add(2)),
        3 => 0.65 * streak(a, b, 24, 2, s) + 0.35 * blot(a, b, 3, s.wrapping_add(3)),
        4 => 1.0 - noise::spots(a, b, 7, 7, s, |id| (0.2, 0.3 + id * 0.35), 0.0) * smooth(0.25, 0.5, blot(a, b, 3, s.wrapping_add(4))).max(0.35),
        5 => streak(a, b, 96, 2, s),
        6 => 0.5 * blot(a, b, 4, s) + 0.5 * math::powi(vband(b, 0.95), 2),
        7 => {
            let w = blot(a, b, 5, s.wrapping_add(6));
            0.7 * blot((a + w * 0.1).rem_euclid(1.0), (b + w * 0.1).rem_euclid(1.0), 7, s)
                + 0.3 * noise::fbm(a, b, 40, 40, 3, 0.5, s.wrapping_add(1), 0)
        }
        8 => noise::spots(a, b, 40, 40, s, |id| (0.04, 0.08 + id * 0.1), 0.0) * 0.6 + 0.6 * blot(a, b, 6, s.wrapping_add(7)) - 0.2,
        9 => 0.5 * blot(a, b, 8, s) + 0.5 * noise::fbm(a, b, 128, 128, 2, 0.6, s.wrapping_add(8), 0),
        10 => 0.7 * crack(a, b, 14, s, 0.05) * smooth(0.35, 0.6, blot(a, b, 5, s.wrapping_add(20))) + 0.65 * blot(a, b, 6, s.wrapping_add(9)) - 0.15,
        11 => {
            p.cracks * 1.0 * crack(a, b, 10, s, 0.06) * smooth(0.3, 0.55, blot(a, b, 4, s.wrapping_add(21))) + 0.5 * blot(a, b, 6, s.wrapping_add(10))
        }
        12 => noise::spots(a, b, 56, 56, s, |id| (0.03, 0.06 + id * 0.05), 0.0) * 0.7 + 0.45 * blot(a, b, 8, s.wrapping_add(12)),
        13 => {
            p.cracks * 0.9 * crack(a, b, 6, s, 0.04) * smooth(0.4, 0.6, blot(a, b, 4, s.wrapping_add(22))) + 0.45 * blot(a, b, 6, s.wrapping_add(11))
        }
        14 => 0.6 * streak(a, b, 30, 2, s) + 0.4 * blot(a, b, 5, s.wrapping_add(13)) * 1.2,
        _ => streak(a, b, 120, 6, s) * (0.55 + 0.6 * vband(b, 0.0)),
    };
    let d = if p.brush > 0.0 { d * (1.0 + p.brush * (streak(a, b, 60, 4, s.wrapping_add(31)) - 0.5) * 1.5) } else { d };
    (d + grain).clamp(0.0, 1.5)
}

/// Final grunge value: white where clean, black where dirty (before inversion).
pub fn grunge_map(p: &GrungeP, u: f32, v: f32, t0: f32) -> f32 {
    let d = dirt(p, u, v);
    let x = 0.5 + (t0 - d) * 1.5 + (p.balance - 0.5) * 0.5;
    let w = (1.0 - p.contrast.clamp(0.0, 1.0)) * 0.45 + 0.01;
    smooth(0.5 - w, 0.5 + w, x)
}
