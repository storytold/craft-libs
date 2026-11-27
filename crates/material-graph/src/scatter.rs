// SPDX-License-Identifier: MIT OR Apache-2.0
//! Scatter nodes: Tile Sampler, Tile Generator, Splatter, Splatter Circular.
//! Every new property defaults to the previous behaviour, so graphs saved before this module render unchanged.
use crate::math;
use crate::noise::rnd;
use crate::tex::{Tex, lum};
use std::f32::consts::TAU;

fn sm(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// Names of the built-in pattern shapes, index 0 = "use the pattern input (or the node's default shape)".
pub const PATTERN_NAMES: &[&str] = &[
    "Input / default",
    "Square",
    "Disc",
    "Paraboloid",
    "Bell",
    "Gaussian",
    "Thorn",
    "Pyramid",
    "Brick",
    "Gradation",
    "Waves",
    "Half Bell",
    "Ridged Bell",
    "Crescent",
    "Capsule",
    "Cone",
];

/// Built-in pattern value at local coordinates (lx, ly) in 0..1 (centre 0.5). `spec` is "Pattern Specific" (0..1).
pub fn builtin(kind: u32, lx: f32, ly: f32, spec: f32) -> f32 {
    let (x, y) = ((lx - 0.5) * 2.0, (ly - 0.5) * 2.0);
    let r = (x * x + y * y).sqrt();
    let v = match kind {
        1 => 1.0,
        2 => 1.0 - sm(0.85, 1.0, r),
        3 => 1.0 - r * r,
        4 => 0.5 + 0.5 * math::cos(r.min(1.0) * std::f32::consts::PI),
        5 => math::exp(-math::powi(r * 2.5, 2)),
        6 => math::powf((1.0 - r).max(0.0), 1.0 + spec * 6.0),
        7 => 1.0 - x.abs().max(y.abs()),
        8 => {
            let e = (1.0 - x.abs()).min(1.0 - y.abs());
            (e / (0.04 + spec * 0.5)).clamp(0.0, 1.0)
        }
        9 => lx,
        10 => 0.5 + 0.5 * math::cos(TAU * (1.0 + (spec * 8.0).round()) * lx),
        11 => 0.5 + 0.5 * math::cos(y.abs().min(1.0) * std::f32::consts::PI),
        12 => {
            let b = 0.5 + 0.5 * math::cos(r.min(1.0) * std::f32::consts::PI);
            1.0 - (1.0 - 2.0 * b).abs()
        }
        13 => {
            let o = 0.2 + spec * 0.6;
            let inner = (math::powi(x - o, 2) + y * y).sqrt();
            if r < 1.0 && inner > 0.8 { 1.0 - sm(0.85, 1.0, r) } else { 0.0 }
        }
        14 => {
            let half = 0.15 + spec * 0.7;
            let dx = (x.abs() - half).max(0.0);
            let d = (dx * dx + y * y).sqrt();
            1.0 - sm(0.85, 1.0, d)
        }
        15 => 1.0 - r,
        // the original Splatter disc (slightly wider soft edge)
        99 => 1.0 - sm(0.8, 1.0, r),
        _ => 1.0,
    };
    v.clamp(0.0, 1.0)
}

/// The patterns a scatter node can draw: up to six inputs and/or a built-in shape.
pub struct Patterns<'a> {
    pub inputs: Vec<&'a Tex>,
    /// 0 = use the inputs (fall back to `fallback`), 1.. = [`PATTERN_NAMES`] index
    pub builtin: u32,
    /// built-in used when no input is connected and `builtin == 0` (0 = solid)
    pub fallback: u32,
    /// 0 random per instance, 1 by instance number
    pub distribution: u32,
}

impl Patterns<'_> {
    fn count(&self) -> usize {
        self.inputs.len().max(1)
    }
    /// `pick` in 0..1 selects the input (random or by number, decided by the caller).
    fn value(&self, which: usize, lx: f32, ly: f32, spec: f32) -> f32 {
        if self.builtin > 0 {
            return builtin(self.builtin, lx, ly, spec);
        }
        match self.inputs.get(which % self.count()) {
            Some(t) => lum(t.sample(lx, ly)),
            None => builtin(self.fallback, lx, ly, spec),
        }
    }
    fn choose(&self, index: usize, r: f32) -> usize {
        let n = self.count();
        if self.distribution == 1 { index % n } else { ((r * n as f32) as usize).min(n - 1) }
    }
}

/// Blending of overlapping instances.
#[derive(Clone, Copy, PartialEq)]
pub enum Blend {
    Max,
    Add,
    Over,
    AddSub,
}
impl Blend {
    pub fn from_index(i: u32) -> Blend {
        match i {
            1 => Blend::Add,
            2 => Blend::Over,
            3 => Blend::AddSub,
            _ => Blend::Max,
        }
    }
    #[inline]
    fn apply(self, acc: f32, a: f32, col: f32) -> f32 {
        match self {
            Blend::Max => acc.max(a * col),
            Blend::Add => acc + a * col,
            Blend::Over => acc + (col - acc) * a,
            Blend::AddSub => acc + a * (col * 2.0 - 1.0),
        }
    }
}

pub struct TileMaps<'a> {
    pub mask: Option<&'a Tex>,
    pub scale: Option<&'a Tex>,
    pub rotation: Option<&'a Tex>,
    pub displacement: Option<&'a Tex>,
    pub color: Option<&'a Tex>,
    pub distribution: Option<&'a Tex>,
    pub background: Option<&'a Tex>,
}

pub struct TileP {
    pub nx: f32,
    pub ny: f32,
    pub size: f32,
    pub aspect: f32,
    pub size_mode: u32,
    pub scale: f32,
    pub scale_rand: f32,
    pub scale_seed: u32,
    pub size_rand: f32,
    pub rot: f32,
    pub rot_rand: f32,
    pub pattern_rot90: u32,
    pub symmetry_random: u32,
    pub pos_rand: f32,
    pub pos_rand_x: f32,
    pub pos_rand_y: f32,
    pub color: f32,
    pub color_rand: f32,
    pub lum_by_number: f32,
    pub lum_by_scale: f32,
    pub offset: f32,
    pub offset_mode: u32,
    pub offset_rand: f32,
    pub offset_seed: u32,
    pub global_offset: [f32; 2],
    pub seed: u32,
    pub thresh: f32,
    pub mask_invert: bool,
    pub mask_random: f32,
    pub mask_checker: u32,
    pub mask_h: bool,
    pub mask_v: bool,
    pub invert_mask: bool,
    pub scale_map_mult: f32,
    pub rotation_map_mult: f32,
    pub displacement_intensity: f32,
    pub displacement_angle: f32,
    pub color_map_mult: f32,
    pub pattern_specific: f32,
    pub pattern_specific_rand: f32,
    pub blend: Blend,
    pub opacity: f32,
    pub background: f32,
    pub reverse: bool,
}

impl TileP {
    /// Defaults that reproduce the original Tile Sampler (`gen == false`) or Tile Generator (`gen == true`).
    pub fn legacy(r#gen: bool) -> TileP {
        TileP {
            nx: if r#gen { 6.0 } else { 8.0 },
            ny: if r#gen { 6.0 } else { 8.0 },
            size: if r#gen { 0.92 } else { 0.8 },
            aspect: 1.0,
            size_mode: 0,
            scale: 1.0,
            scale_rand: 0.0,
            scale_seed: 0,
            size_rand: 0.0,
            rot: 0.0,
            rot_rand: 0.0,
            pattern_rot90: 0,
            symmetry_random: 0,
            pos_rand: 0.0,
            pos_rand_x: 1.0,
            pos_rand_y: 1.0,
            color: 1.0,
            color_rand: 0.0,
            lum_by_number: 0.0,
            lum_by_scale: 0.0,
            offset: 0.5,
            offset_mode: if r#gen { 1 } else { 0 },
            offset_rand: 0.0,
            offset_seed: 0,
            global_offset: [0.0, 0.0],
            seed: 0,
            thresh: 0.5,
            mask_invert: false,
            mask_random: 0.0,
            mask_checker: 0,
            mask_h: false,
            mask_v: false,
            invert_mask: false,
            scale_map_mult: 1.0,
            rotation_map_mult: 1.0,
            displacement_intensity: 0.5,
            displacement_angle: 0.0,
            color_map_mult: 1.0,
            pattern_specific: 0.5,
            pattern_specific_rand: 0.0,
            blend: Blend::Max,
            opacity: 1.0,
            background: 0.0,
            reverse: false,
        }
    }
}

struct Inst {
    cx: f32,
    cy: f32,
    sx: f32,
    sy: f32,
    sn: f32,
    cs: f32,
    flip_x: bool,
    flip_y: bool,
    col: f32,
    pat: usize,
    spec: f32,
    key: u32,
    on: bool,
}

fn tile_visible(i: i32, j: i32, mask: Option<f32>, p: &TileP) -> bool {
    let mut keep = true;
    if let Some(m) = mask {
        keep = if p.mask_invert { m < p.thresh } else { m >= p.thresh };
    }
    let mut proc_keep = true;
    if p.mask_random > 0.0 && rnd(i, j, p.seed.wrapping_add(31)) < p.mask_random {
        proc_keep = false;
    }
    match p.mask_checker {
        1 => proc_keep &= ((i + j) & 1) == 0,
        2 => proc_keep &= ((i + j) & 1) == 1,
        _ => {}
    }
    if p.mask_h {
        proc_keep &= (i & 1) == 0;
    }
    if p.mask_v {
        proc_keep &= (j & 1) == 0;
    }
    if p.invert_mask {
        proc_keep = !proc_keep;
    }
    keep && proc_keep
}

/// Grid of pattern instances (Tile Sampler / Tile Generator).
pub fn tile_grid(w: usize, h: usize, pats: &Patterns, maps: &TileMaps, p: &TileP) -> Tex {
    let nx = p.nx.round().clamp(1.0, 64.0) as i32;
    let ny = p.ny.round().clamp(1.0, 64.0) as i32;
    let (fnx, fny) = (nx as f32, ny as f32);
    let np = pats.count();
    let total = (nx * ny) as f32;
    // divisor that turns image distance into pattern-local distance
    let (dvx, dvy) = match p.size_mode {
        1 => (fnx.min(fny), fnx.min(fny)),
        2 => (1.0, 1.0),
        _ => (fnx, fny),
    };
    let samp = |t: Option<&Tex>, x: f32, y: f32| t.map(|t| lum(t.sample(x, y)));
    let mut max_half = 0.0f32;
    let mut inst: Vec<Inst> = Vec::with_capacity((nx * ny) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let sd = p.seed;
            let (r1, r2, r3, r4) = (rnd(i, j, sd), rnd(i, j, sd.wrapping_add(1)), rnd(i, j, sd.wrapping_add(2)), rnd(i, j, sd.wrapping_add(3)));
            let key = (j * nx + i) as u32;
            // row / column offset
            let (line, along_row) = match p.offset_mode {
                1 | 2 => (j, true),
                _ => (i, false),
            };
            let active = match p.offset_mode {
                1 | 3 => (line & 1) == 1,
                2 | 4 => (line & 1) == 0,
                _ => false,
            };
            let mut shift = if active { p.offset } else { 0.0 };
            if p.offset_mode != 0 && p.offset_rand > 0.0 {
                shift += (rnd(line, 77, p.offset_seed.wrapping_add(sd)) - 0.5) * 2.0 * p.offset_rand;
            }
            let (mut cx, mut cy) = ((i as f32 + 0.5) / fnx, (j as f32 + 0.5) / fny);
            if along_row || p.offset_mode == 0 {
                cx += shift / fnx;
            } else {
                cy += shift / fny;
            }
            cx += (r1 - 0.5) * p.pos_rand * p.pos_rand_x / fnx + p.global_offset[0];
            cy += (r2 - 0.5) * p.pos_rand * p.pos_rand_y / fny + p.global_offset[1];
            let (cx0, cy0) = (cx, cy);
            // map inputs sampled at the tile centre
            let on = tile_visible(i, j, samp(maps.mask, cx0, cy0), p);
            if let Some(d) = samp(maps.displacement, cx0, cy0) {
                let a = p.displacement_angle * TAU;
                let k = (d - 0.5) * 2.0 * p.displacement_intensity;
                cx += math::cos(a) * k / fnx;
                cy += math::sin(a) * k / fny;
            }
            let mut s = p.size * p.scale * (1.0 + (r3 - 0.5) * 2.0 * p.size_rand);
            if p.scale_rand > 0.0 {
                s *= 1.0 + (rnd(i, j, sd.wrapping_add(p.scale_seed).wrapping_add(41)) - 0.5) * 2.0 * p.scale_rand;
            }
            if let Some(m) = samp(maps.scale, cx0, cy0) {
                s *= 1.0 + p.scale_map_mult * (m - 1.0);
            }
            let s = s.max(0.01);
            let mut turns = p.rot + (r4 - 0.5) * 2.0 * p.rot_rand + 0.25 * p.pattern_rot90 as f32;
            if let Some(m) = samp(maps.rotation, cx0, cy0) {
                turns += m * p.rotation_map_mult;
            }
            let (sn, cs) = math::sin_cos(turns * TAU);
            let mut col = p.color * (1.0 - p.color_rand * rnd(i, j, sd.wrapping_add(9)));
            if p.lum_by_number > 0.0 {
                col *= 1.0 - p.lum_by_number * (key as f32 / (total - 1.0).max(1.0));
            }
            if p.lum_by_scale > 0.0 {
                let rel = (s / (p.size * p.scale * (1.0 + p.size_rand)).max(1e-3)).clamp(0.0, 1.0);
                col *= 1.0 - p.lum_by_scale * (1.0 - rel);
            }
            if let Some(m) = samp(maps.color, cx0, cy0) {
                col *= 1.0 + p.color_map_mult * (m - 1.0);
            }
            let pick = match samp(maps.distribution, cx0, cy0) {
                Some(d) => ((d * np as f32) as usize).min(np - 1),
                None => pats.choose(key as usize, rnd(i, j, sd.wrapping_add(21))),
            };
            let (fx, fy) = match p.symmetry_random {
                1 => (rnd(i, j, sd.wrapping_add(51)) < 0.5, false),
                2 => (false, rnd(i, j, sd.wrapping_add(52)) < 0.5),
                3 => (rnd(i, j, sd.wrapping_add(51)) < 0.5, rnd(i, j, sd.wrapping_add(52)) < 0.5),
                _ => (false, false),
            };
            let spec = (p.pattern_specific + (rnd(i, j, sd.wrapping_add(61)) - 0.5) * 2.0 * p.pattern_specific_rand).clamp(0.0, 1.0);
            let (sx, sy) = (s, s * p.aspect.max(0.01));
            max_half = max_half.max(sx.max(sy) * 0.5 / dvx.min(dvy) * fnx.min(fny));
            inst.push(Inst { cx, cy, sx, sy, sn, cs, flip_x: fx, flip_y: fy, col, pat: pick, spec, key, on });
        }
    }
    // neighbourhood radius in cells: 1 normally, more when tiles are larger than their cell
    let disp = if maps.displacement.is_some() { p.displacement_intensity } else { 0.0 };
    let reach = (max_half + p.pos_rand * p.pos_rand_x.max(p.pos_rand_y) * 0.5 + disp + p.offset_rand + 0.5).ceil();
    let rad = (reach as i32).clamp(1, 3);
    let bg_in = maps.background;
    Tex::from_fn(w, h, |u, v| {
        let (ci, cj) = ((u * fnx).floor() as i32, (v * fny).floor() as i32);
        let mut cand: [(u32, f32, f32); 49] = [(0, 0.0, 0.0); 49];
        let mut seen: [u32; 49] = [0; 49];
        let mut ns = 0;
        let mut nc = 0;
        let dedupe = nx < 2 * rad + 1 || ny < 2 * rad + 1;
        for dj in -rad..=rad {
            for di in -rad..=rad {
                let (wi, wj) = (ci.wrapping_add(di).rem_euclid(nx), cj.wrapping_add(dj).rem_euclid(ny));
                let Some(it) = inst.get((wj * nx + wi) as usize) else { continue };
                if !it.on {
                    continue;
                }
                if dedupe {
                    // a small grid wraps onto itself: count every instance only once
                    if seen.get(..ns).is_some_and(|s| s.contains(&it.key)) {
                        continue;
                    }
                    if let Some(e) = seen.get_mut(ns) {
                        *e = it.key;
                    }
                    ns += 1;
                }
                let dx = (u - it.cx + 0.5).rem_euclid(1.0) - 0.5;
                let dy = (v - it.cy + 0.5).rem_euclid(1.0) - 0.5;
                let lx = (it.cs * dx + it.sn * dy) * dvx / it.sx + 0.5;
                let ly = (-it.sn * dx + it.cs * dy) * dvy / it.sy + 0.5;
                if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                    continue;
                }
                let (lx, ly) = (if it.flip_x { 1.0 - lx } else { lx }, if it.flip_y { 1.0 - ly } else { ly });
                let a = pats.value(it.pat, lx, ly, it.spec);
                if let Some(e) = cand.get_mut(nc) {
                    *e = (it.key, a * p.opacity, it.col);
                    nc += 1;
                }
            }
        }
        let c = cand.get_mut(..nc).unwrap_or_default();
        if p.blend == Blend::Over || p.blend == Blend::AddSub {
            c.sort_by_key(|e| e.0);
        }
        let mut acc = match bg_in {
            Some(b) => lum(b.sample(u, v)),
            None => p.background,
        };
        if p.reverse {
            for e in c.iter().rev() {
                acc = p.blend.apply(acc, e.1, e.2);
            }
        } else {
            for e in c.iter() {
                acc = p.blend.apply(acc, e.1, e.2);
            }
        }
        let acc = if acc.is_finite() { acc } else { 0.0 };
        [acc, acc, acc, 1.0]
    })
}

pub struct SplatterP {
    pub count: u32,
    pub grid: u32,
    pub scale: f32,
    pub scale_var: f32,
    pub aspect: f32,
    pub rot: f32,
    pub rot_var: f32,
    pub pattern_rot90: u32,
    pub lum_var: f32,
    pub disorder: f32,
    pub pan: [f32; 2],
    pub symmetry_random: u32,
    pub pattern_specific: f32,
    pub gain: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub background: f32,
    pub blend: Blend,
    pub opacity: f32,
    pub seed: u32,
}

impl SplatterP {
    pub fn legacy() -> SplatterP {
        SplatterP {
            count: 40,
            grid: 0,
            scale: 0.1,
            scale_var: 0.3,
            aspect: 1.0,
            rot: 0.0,
            rot_var: 1.0,
            pattern_rot90: 0,
            lum_var: 0.0,
            disorder: 1.0,
            pan: [0.0, 0.0],
            symmetry_random: 0,
            pattern_specific: 0.5,
            gain: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            background: 0.0,
            blend: Blend::Max,
            opacity: 1.0,
            seed: 0,
        }
    }
}

fn finish(acc: f32, gain: f32, lo: f32, hi: f32) -> [f32; 4] {
    let mut v = if acc.is_finite() { acc } else { 0.0 };
    if (gain - 1.0).abs() > 1e-6 {
        v = math::powf(v.clamp(0.0, 1.0), 1.0 / gain.max(1e-3));
    }
    if lo != 0.0 || hi != 1.0 {
        v = lo + (hi - lo) * v;
    }
    [v, v, v, 1.0]
}

/// Random scatter of a pattern (Splatter). `grid > 0` places `grid * grid` instances on a jittered grid instead
/// (`disorder` is the jitter).
struct SplatInst {
    cx: f32,
    cy: f32,
    sx: f32,
    sy: f32,
    sn: f32,
    cs: f32,
    l: f32,
    fx: bool,
    fy: bool,
    pat: usize,
}

/// Which per-instance value `splatter_data` writes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SplatData {
    /// a stable random gray level per instance (0 = background)
    Id,
    /// the instance centre in uv as red / green
    Center,
    /// the instance width / height in uv as red / green
    Size,
    /// the instance's brightness factor as gray
    Luminance,
}

fn splat_instances(pats: &Patterns, p: &SplatterP) -> Vec<SplatInst> {
    type S = SplatInst;
    let seed = p.seed;
    let n = if p.grid > 0 { p.grid.wrapping_mul(p.grid) } else { p.count.min(2000) };
    let inst: Vec<S> = (0..n)
        .map(|k| {
            let i = k as i32;
            let (mut cx, mut cy) = if p.grid > 0 {
                let (gx, gy) = ((k % p.grid) as f32, (k / p.grid) as f32);
                let g = p.grid as f32;
                ((gx + 0.5 + (rnd(i, 1, seed) - 0.5) * p.disorder) / g, (gy + 0.5 + (rnd(i, 2, seed) - 0.5) * p.disorder) / g)
            } else {
                (rnd(i, 1, seed), rnd(i, 2, seed))
            };
            cx += p.pan[0];
            cy += p.pan[1];
            let s = (p.scale * (1.0 + (rnd(i, 3, seed) - 0.5) * 2.0 * p.scale_var)).max(1e-3);
            let r = p.rot * TAU + (rnd(i, 4, seed) - 0.5) * p.rot_var * TAU + 0.25 * TAU * p.pattern_rot90 as f32;
            let (sn, cs) = math::sin_cos(r);
            let (fx, fy) = match p.symmetry_random {
                1 => (rnd(i, 6, seed) < 0.5, false),
                2 => (false, rnd(i, 7, seed) < 0.5),
                3 => (rnd(i, 6, seed) < 0.5, rnd(i, 7, seed) < 0.5),
                _ => (false, false),
            };
            S {
                cx,
                cy,
                sx: s,
                sy: (s * p.aspect.max(0.01)).max(1e-3),
                sn,
                cs,
                l: 1.0 - p.lum_var * rnd(i, 5, seed),
                fx,
                fy,
                pat: pats.choose(k as usize, rnd(i, 8, seed)),
            }
        })
        .collect();
    inst
}

/// Data of the top-most instance under every pixel. An instance covers a pixel in its box where its value times the opacity
/// is above 0.001, so the faintest edge of a soft pattern has no data.
pub fn splatter_data(w: usize, h: usize, pats: &Patterns, p: &SplatterP, what: SplatData) -> Tex {
    let inst = splat_instances(pats, p);
    Tex::from_fn(w, h, |u, v| {
        let mut hit: Option<(usize, &SplatInst)> = None;
        for (k, it) in inst.iter().enumerate() {
            let dx = (u - it.cx + 0.5).rem_euclid(1.0) - 0.5;
            let dy = (v - it.cy + 0.5).rem_euclid(1.0) - 0.5;
            let lx = (it.cs * dx + it.sn * dy) / it.sx + 0.5;
            let ly = (-it.sn * dx + it.cs * dy) / it.sy + 0.5;
            if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                continue;
            }
            let (lx, ly) = (if it.fx { 1.0 - lx } else { lx }, if it.fy { 1.0 - ly } else { ly });
            if pats.value(it.pat, lx, ly, p.pattern_specific) * p.opacity > 1e-3 {
                hit = Some((k, it));
            }
        }
        match hit {
            None => [0.0, 0.0, 0.0, 1.0],
            Some((k, it)) => match what {
                SplatData::Id => {
                    let g = 0.1 + 0.9 * rnd(k as i32, 31, p.seed);
                    [g, g, g, 1.0]
                }
                SplatData::Center => [it.cx.rem_euclid(1.0), it.cy.rem_euclid(1.0), 0.0, 1.0],
                SplatData::Size => [it.sx.min(1.0), it.sy.min(1.0), 0.0, 1.0],
                SplatData::Luminance => [it.l, it.l, it.l, 1.0],
            },
        }
    })
}

pub fn splatter(w: usize, h: usize, pats: &Patterns, p: &SplatterP) -> Tex {
    let inst = splat_instances(pats, p);
    Tex::from_fn(w, h, |u, v| {
        let mut acc = p.background;
        for it in &inst {
            let dx = (u - it.cx + 0.5).rem_euclid(1.0) - 0.5;
            let dy = (v - it.cy + 0.5).rem_euclid(1.0) - 0.5;
            let lx = (it.cs * dx + it.sn * dy) / it.sx + 0.5;
            let ly = (-it.sn * dx + it.cs * dy) / it.sy + 0.5;
            if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                continue;
            }
            let (lx, ly) = (if it.fx { 1.0 - lx } else { lx }, if it.fy { 1.0 - ly } else { ly });
            let a = pats.value(it.pat, lx, ly, p.pattern_specific) * p.opacity;
            acc = p.blend.apply(acc, a, it.l);
        }
        finish(acc, p.gain, p.out_min, p.out_max)
    })
}

pub struct CircularP {
    pub amount: u32,
    pub amount_rand: f32,
    pub amount_rand_min: u32,
    pub rings: u32,
    pub radius: f32,
    pub radius_rand: f32,
    pub ring_mult: f32,
    pub angle_rand: f32,
    pub spiral: f32,
    pub spread: f32,
    pub size: f32,
    pub size_rand: f32,
    pub scale_by_number: f32,
    pub invert_number: bool,
    pub scale_by_ring: f32,
    pub invert_ring: bool,
    pub rot: f32,
    pub rot_rand: f32,
    pub center_orientation: f32,
    pub ring_rot: f32,
    pub ring_rot_rand: f32,
    pub ring_rot_offset: f32,
    pub pos_rand: f32,
    pub symmetry_random: u32,
    pub pattern_specific: f32,
    pub color: f32,
    pub lum_rand: f32,
    pub lum_by_number: f32,
    pub lum_by_ring: f32,
    pub mask_random: f32,
    pub background: f32,
    pub blend: Blend,
    pub opacity: f32,
    pub seed: u32,
}

impl CircularP {
    pub fn legacy() -> CircularP {
        CircularP {
            amount: 12,
            amount_rand: 0.0,
            amount_rand_min: 1,
            rings: 1,
            radius: 0.3,
            radius_rand: 0.0,
            ring_mult: 0.5,
            angle_rand: 0.0,
            spiral: 0.0,
            spread: 1.0,
            size: 0.1,
            size_rand: 0.0,
            scale_by_number: 0.0,
            invert_number: false,
            scale_by_ring: 0.0,
            invert_ring: false,
            rot: 0.0,
            rot_rand: 0.0,
            center_orientation: 0.0,
            ring_rot: 0.0,
            ring_rot_rand: 0.0,
            ring_rot_offset: 0.0,
            pos_rand: 0.0,
            symmetry_random: 0,
            pattern_specific: 0.5,
            color: 1.0,
            lum_rand: 0.0,
            lum_by_number: 0.0,
            lum_by_ring: 0.0,
            mask_random: 0.0,
            background: 0.0,
            blend: Blend::Max,
            opacity: 1.0,
            seed: 0,
        }
    }
}

/// Instances placed on one or more circles (Splatter Circular).
pub fn splatter_circular(w: usize, h: usize, pats: &Patterns, p: &CircularP) -> Tex {
    struct S {
        cx: f32,
        cy: f32,
        sx: f32,
        sn: f32,
        cs: f32,
        l: f32,
        fx: bool,
        fy: bool,
        pat: usize,
    }
    let seed = p.seed;
    let rings = p.rings.clamp(1, 10);
    let mut inst: Vec<S> = Vec::new();
    let mut order = 0u32;
    for ring in 0..rings {
        let rk = ring as i32 * 977;
        let mut n = p.amount.clamp(1, 64 * 4);
        if p.amount_rand > 0.0 {
            let lo = p.amount_rand_min.clamp(1, n) as f32;
            n = (n as f32 + (lo - n as f32) * p.amount_rand * rnd(rk, 11, seed)).round().max(1.0) as u32;
        }
        let ring_frac = if rings > 1 { ring as f32 / (rings - 1) as f32 } else { 0.0 };
        let ring_r = p.radius * math::powi(p.ring_mult, ring as i32);
        let ring_spin = (p.ring_rot * ring as f32 + p.ring_rot_offset + (rnd(rk, 12, seed) - 0.5) * 2.0 * p.ring_rot_rand) * TAU;
        for k in 0..n {
            let i = (k as i32).wrapping_add(rk);
            let frac = k as f32 / n as f32;
            let a = frac * p.spread.clamp(0.0, 1.0) * TAU + ring_spin + (rnd(i, 13, seed) - 0.5) * 2.0 * p.angle_rand * TAU / n as f32;
            let rr = (ring_r * (1.0 + (rnd(i, 14, seed) - 0.5) * 2.0 * p.radius_rand) + p.spiral * 0.2 * frac).max(0.0);
            let cx = 0.5 + rr * math::cos(a) + (rnd(i, 1, seed) - 0.5) * p.pos_rand * 0.2;
            let cy = 0.5 + rr * math::sin(a) + (rnd(i, 2, seed) - 0.5) * p.pos_rand * 0.2;
            let mut s = p.size * (1.0 + (rnd(i, 3, seed) - 0.5) * 2.0 * p.size_rand);
            let num = if p.invert_number { 1.0 - frac } else { frac };
            s *= 1.0 - p.scale_by_number * (1.0 - num);
            let rng = if p.invert_ring { 1.0 - ring_frac } else { ring_frac };
            s *= 1.0 - p.scale_by_ring * (1.0 - rng);
            let s = s.max(0.005);
            let face = (a + std::f32::consts::PI) * p.center_orientation;
            let r = (p.rot + (rnd(i, 4, seed) - 0.5) * 2.0 * p.rot_rand) * TAU + face;
            let (sn, cs) = math::sin_cos(r);
            let (fx, fy) = match p.symmetry_random {
                1 => (rnd(i, 6, seed) < 0.5, false),
                2 => (false, rnd(i, 7, seed) < 0.5),
                3 => (rnd(i, 6, seed) < 0.5, rnd(i, 7, seed) < 0.5),
                _ => (false, false),
            };
            let mut l = p.color * (1.0 - p.lum_rand * rnd(i, 5, seed));
            l *= 1.0 - p.lum_by_number * (1.0 - num);
            l *= 1.0 - p.lum_by_ring * (1.0 - rng);
            if p.mask_random > 0.0 && rnd(i, 15, seed) < p.mask_random {
                continue;
            }
            let pat = if pats.distribution == 2 { ring as usize % pats.count() } else { pats.choose(order as usize, rnd(i, 8, seed)) };
            inst.push(S { cx, cy, sx: s, sn, cs, l, fx, fy, pat });
            order += 1;
        }
    }
    Tex::from_fn(w, h, |u, v| {
        let mut acc = p.background;
        for it in &inst {
            let (dx, dy) = (u - it.cx, v - it.cy);
            let (lx, ly) = ((it.cs * dx + it.sn * dy) / it.sx + 0.5, (-it.sn * dx + it.cs * dy) / it.sx + 0.5);
            if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
                continue;
            }
            let (lx, ly) = (if it.fx { 1.0 - lx } else { lx }, if it.fy { 1.0 - ly } else { ly });
            let a = pats.value(it.pat, lx, ly, p.pattern_specific) * p.opacity;
            acc = p.blend.apply(acc, a, it.l);
        }
        finish(acc, 1.0, 0.0, 1.0)
    })
}
