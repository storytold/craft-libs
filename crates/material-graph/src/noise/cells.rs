//! Cell and spot looks as point functions of (u, v), tileable with period 1: [`Cells`] (four styles),
//! [`Crystal`], [`VoronoiFractal`] (one Voronoi level or a fractal sum of several), [`BnwSpots`] and
//! [`GaussianSpots`]. Callers that warp the plane pass warped coordinates.

use super::{Metric, Vor, fbm, perlin, rnd, smoothstep as smooth, spots, voronoi_ext, voronoi_gen, voronoi_style};
use crate::math;

/// Where a [`Cells`] style-1 cell takes its value from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSource {
    /// The cell's own random id.
    Random,
    /// A value re-rolled per cell with its own seed.
    Pseudorandom(u32),
}

/// Cellular noise: 0 soft cells, 1 flat cells, 2 cell borders, 3 cells with a hardness falloff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cells {
    pub style: u32,
    /// Cells per side.
    pub scale: i32,
    /// Cells down the texture; `scale` unless the cells are kept square on a non-square texture.
    pub rows: i32,
    pub seed: u32,
    pub jitter: f32,
    pub luminance_random: f32,
    pub edge_width: f32,
    pub hardness: f32,
    pub source: CellSource,
    /// The node's extra outputs: 0 the style's look, 1 the cell id map, 2 the distance to the centre, 3 the borders.
    pub output: u32,
}

impl Cells {
    pub fn new(style: u32, scale: i32, seed: u32) -> Cells {
        Cells {
            style,
            scale,
            rows: scale,
            seed,
            jitter: 1.0,
            luminance_random: 0.0,
            edge_width: 0.25,
            hardness: 2.0,
            source: CellSource::Random,
            output: 0,
        }
    }

    pub fn value(&self, u: f32, v: f32) -> f32 {
        self.value_with(u, v, None)
    }

    /// The value, with `image(u, v)` (a grey value at the cell centre) as the flat cells' value where given.
    pub fn value_with(&self, u: f32, v: f32, image: Option<&dyn Fn(f32, f32) -> f32>) -> f32 {
        let p = self.scale;
        let vr = voronoi_ext(u, v, p, self.rows, self.seed, self.jitter);
        let cell_val = || match (image, self.source) {
            (Some(img), _) => img(u + vr.dx / p as f32, v + vr.dy / p as f32),
            (None, CellSource::Pseudorandom(pseed)) => rnd(
                (((u + vr.dx / p as f32) * p as f32).floor()) as i32,
                (((v + vr.dy / p as f32) * p as f32).floor()) as i32,
                pseed.wrapping_add(4242),
            ),
            (None, CellSource::Random) => vr.id1,
        };
        match self.output {
            1 => return vr.id1,
            2 => return vr.f1,
            3 => return smooth(0.0, self.edge_width, vr.f2 - vr.f1),
            _ => {}
        }
        match self.style {
            0 => (1.0 - vr.f1) * (1.0 - self.luminance_random * vr.id1),
            1 => cell_val(),
            2 => smooth(0.0, self.edge_width, vr.f2 - vr.f1),
            _ => vr.id1 * (1.0 - math::powf(vr.f1, self.hardness)),
        }
    }
}

/// Faceted crystal cells: 0 cone facets, 1 lit planar facets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crystal {
    pub style: u32,
    pub scale: i32,
    pub seed: u32,
}

impl Crystal {
    pub fn value(&self, u: f32, v: f32) -> f32 {
        let p = self.scale;
        let vr = voronoi_ext(u, v, p, p, self.seed, 1.0);
        let l = (vr.dx * vr.dx + vr.dy * vr.dy).sqrt().max(1e-4);
        let facet = if self.style == 0 { 1.0 - vr.f1 } else { 0.5 + 0.5 * ((vr.dx / l) * 0.6 + (vr.dy / l) * 0.8) };
        let edge = smooth(0.0, 0.08, vr.f2 - vr.f1).max(0.2);
        (vr.id1 * 0.55 + facet * 0.45) * edge
    }
}

/// Voronoi with a style, metric, distortion and (as a fractal) several levels summed, maxed or min-ed.
#[derive(Debug, Clone, PartialEq)]
pub struct VoronoiFractal {
    scale: i32,
    seed: u32,
    jitter: f32,
    style: u32,
    aspect: f32,
    offset: (f32, f32),
    distortion: f32,
    distortion_scale: i32,
    metric: Metric,
    thickness: f32,
    rounded: f32,
    distance_scale: f32,
    id_seed: u32,
    opacity: f32,
    blend: u32,
    fractal: bool,
    /// (cells, amplitude) per level.
    levels: Vec<(i32, f32)>,
    norm: f32,
}

/// The settings of a [`VoronoiFractal`], as the node clamps them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoronoiSettings {
    pub scale: i32,
    pub seed: u32,
    pub jitter: f32,
    /// [`voronoi_style`] style.
    pub style: u32,
    pub aspect: f32,
    pub offset: (f32, f32),
    pub distortion: f32,
    pub distortion_scale: i32,
    pub metric: Metric,
    pub thickness: f32,
    pub rounded: f32,
    pub distance_scale: f32,
    /// Seed of the cell ids (usually `seed`).
    pub id_seed: u32,
    /// Fractal only: levels, roughness, lacunarity, opacity of the upper levels, blend (0 sum, 1 max, 2 min).
    pub min_level: u32,
    pub max_level: u32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub opacity: f32,
    pub blend: u32,
}

impl VoronoiSettings {
    /// The node's defaults.
    pub fn new(scale: i32, seed: u32) -> VoronoiSettings {
        VoronoiSettings {
            scale,
            seed,
            jitter: 1.0,
            style: 0,
            aspect: 1.0,
            offset: (0.0, 0.0),
            distortion: 0.0,
            distortion_scale: 3,
            metric: Metric::from_index(0, 2.0),
            thickness: 0.25,
            rounded: 0.0,
            distance_scale: 1.0,
            id_seed: seed,
            min_level: 0,
            max_level: 3,
            roughness: 0.5,
            lacunarity: 2.0,
            opacity: 1.0,
            blend: 0,
        }
    }
}

impl VoronoiFractal {
    /// One level (`fractal` false) or the levels `min_level..=max_level`.
    pub fn new(s: &VoronoiSettings, fractal: bool) -> VoronoiFractal {
        let (lo, hi) = (s.min_level, s.max_level.max(s.min_level));
        let p = s.scale;
        let levels: Vec<(i32, f32)> = if fractal {
            (lo..=hi)
                .map(|l| {
                    (((p as f32) * math::powi(s.lacunarity, l as i32)).round().clamp(1.0, 512.0) as i32, math::powi(s.roughness, (l - lo) as i32))
                })
                .collect()
        } else {
            vec![(p, 1.0)]
        };
        let norm: f32 = levels.iter().map(|l| l.1).sum::<f32>().max(1e-6);
        VoronoiFractal {
            scale: p,
            seed: s.seed,
            jitter: s.jitter,
            style: s.style,
            aspect: s.aspect,
            offset: s.offset,
            distortion: s.distortion,
            distortion_scale: s.distortion_scale,
            metric: s.metric,
            thickness: s.thickness,
            rounded: s.rounded,
            distance_scale: s.distance_scale,
            id_seed: s.id_seed,
            opacity: s.opacity,
            blend: s.blend,
            fractal,
            levels,
            norm,
        }
    }

    fn one(&self, u: f32, v: f32, px: i32, sd: u32) -> f32 {
        let (seed, dist, dscale) = (self.seed, self.distortion, self.distortion_scale);
        let py = ((px as f32 * self.aspect).round().max(1.0)) as i32;
        let (u, v) = (u + self.offset.0, v + self.offset.1);
        let (u, v) = if dist > 0.0 {
            (
                u + (perlin(u, v, dscale, dscale, seed.wrapping_add(5)) - 0.5) * dist * 0.3,
                v + (perlin(u, v, dscale, dscale, seed.wrapping_add(6)) - 0.5) * dist * 0.3,
            )
        } else {
            (u, v)
        };
        let r: Vor = voronoi_gen(u, v, px, py, sd, self.id_seed.wrapping_add(sd.wrapping_sub(seed)), self.jitter, self.metric);
        (voronoi_style(&r, self.style, self.thickness, self.rounded) * self.distance_scale).clamp(0.0, 1.0)
    }

    pub fn value(&self, u: f32, v: f32) -> f32 {
        if !self.fractal {
            return self.one(u, v, self.scale, self.seed);
        }
        let (blend, opac) = (self.blend, self.opacity);
        let mut acc: f32 = if blend == 2 { 1.0 } else { 0.0 };
        for (i, (px, amp)) in self.levels.iter().enumerate() {
            let val = self.one(u, v, *px, self.seed.wrapping_add((i as u32).wrapping_mul(131)));
            match blend {
                1 => acc = acc.max(val * if i == 0 { 1.0 } else { opac }),
                2 => acc = acc.min(1.0 - (1.0 - val) * if i == 0 { 1.0 } else { opac }),
                _ => acc += amp * val * if i == 0 { 1.0 } else { opac },
            }
        }
        if blend == 0 { acc / self.norm } else { acc }.clamp(0.0, 1.0)
    }
}

/// Black-and-white spots: 0 large round, 1 bigger, 2 small grainy spots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BnwSpots {
    pub style: u32,
    pub scale: i32,
    pub seed: u32,
}

impl BnwSpots {
    pub fn value(&self, u: f32, v: f32) -> f32 {
        self.value_warped(u, v, (u, v))
    }

    /// The spots at the warped point `(a, b)`; the grain of style 2 stays at the unwarped `(u, v)`.
    pub fn value_warped(&self, u: f32, v: f32, (a, b): (f32, f32)) -> f32 {
        let (var, p, seed) = (self.style, self.scale, self.seed);
        let grain = if var >= 2 { (fbm(u, v, p.wrapping_mul(4), p.wrapping_mul(4), 3, 0.5, seed.wrapping_add(5), 0) - 0.5) * 0.25 } else { 0.0 };
        spots(
            a,
            b,
            p,
            p,
            seed,
            |id| {
                let rad = match var {
                    0 => 0.18 + 0.5 * id * id,
                    1 => 0.3 + 0.55 * id,
                    _ => 0.08 + 0.3 * id,
                };
                (rad * 0.8, rad)
            },
            grain,
        )
    }
}

/// Soft gaussian-like bumps of varying size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaussianSpots {
    pub scale: i32,
    pub seed: u32,
    /// 0.05..1.
    pub size: f32,
    /// 0..1.
    pub size_variation: f32,
}

impl GaussianSpots {
    pub fn value(&self, u: f32, v: f32) -> f32 {
        let (size, var) = (self.size, self.size_variation);
        // spots() blends smoothly from 1 to 0 between the inner and outer radius, i.e. a gaussian-like bump
        spots(
            u,
            v,
            self.scale,
            self.scale,
            self.seed,
            |id| {
                let r = size * 0.55 * (1.0 - var + var * id);
                (0.0, r.max(0.02))
            },
            0.0,
        )
    }
}
