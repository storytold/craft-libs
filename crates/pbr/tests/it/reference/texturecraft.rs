//! TextureCraft's `crates/render/src/ibl.rs` at fe8ee1bdb0, without its tests, for the
//! differential tests. Only the imports changed: glam and the sRGB decoder come from this
//! crate's dependencies, and `ENVIRONMENTS` is defined here instead of imported from
//! `texturecraft_core::project` (the same six names).

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};
use rayon::prelude::*;

/// An equirect HDR image (linear RGB).
#[derive(Clone, Debug)]
pub struct EnvImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 3]>,
}

pub const ENVIRONMENTS: [&str; 6] = ["Studio Soft", "Studio Contrast", "Overcast", "Sunset", "Warehouse", "Night"];

pub fn dir_of(u: f32, v: f32) -> Vec3 {
    let (theta, phi) = (v * PI, u * TAU);
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

pub fn uv_of(d: Vec3) -> Vec2 {
    let phi = d.z.atan2(d.x);
    Vec2::new((phi / TAU).rem_euclid(1.0), d.y.clamp(-1.0, 1.0).acos() / PI)
}

/// A soft rectangular light: azimuth/elevation center (degrees), half sizes (degrees).
struct SoftBox {
    az: f32,
    el: f32,
    hw: f32,
    hh: f32,
    color: Vec3,
}

impl SoftBox {
    fn eval(&self, d: Vec3) -> Vec3 {
        let el = d.y.clamp(-1.0, 1.0).asin().to_degrees();
        let az = d.z.atan2(d.x).to_degrees();
        let daz = ((az - self.az + 540.0).rem_euclid(360.0)) - 180.0;
        let (x, y) = (daz.abs() / self.hw, (el - self.el).abs() / self.hh);
        let e = |t: f32| 1.0 - smooth((t - 0.8) / 0.4);
        self.color * e(x) * e(y)
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sky(d: Vec3, zenith: Vec3, horizon: Vec3, ground: Vec3) -> Vec3 {
    if d.y >= 0.0 { horizon.lerp(zenith, d.y.powf(0.6)) } else { horizon.lerp(ground, (-d.y).powf(0.25)) }
}

fn radiance(name: &str, d: Vec3) -> Vec3 {
    match name {
        "Studio Contrast" => {
            let base = sky(d, Vec3::splat(0.04), Vec3::splat(0.03), Vec3::splat(0.01));
            base + SoftBox { az: 35.0, el: 30.0, hw: 22.0, hh: 18.0, color: Vec3::new(14.0, 13.2, 12.0) }.eval(d)
                + SoftBox { az: -150.0, el: 40.0, hw: 6.0, hh: 30.0, color: Vec3::new(4.0, 4.4, 5.0) }.eval(d)
        }
        "Overcast" => {
            let base = sky(d, Vec3::new(1.2, 1.25, 1.35), Vec3::new(0.95, 0.95, 0.95), Vec3::new(0.18, 0.2, 0.15));
            base + SoftBox { az: 60.0, el: 55.0, hw: 40.0, hh: 25.0, color: Vec3::splat(0.6) }.eval(d)
        }
        "Sunset" => {
            let base = sky(d, Vec3::new(0.15, 0.25, 0.55), Vec3::new(1.4, 0.65, 0.3), Vec3::new(0.08, 0.06, 0.05));
            let sun_dir = dir_of(0.12, 0.47);
            let sun = smooth((d.dot(sun_dir) - 0.9985) / 0.0012) * 120.0;
            base + Vec3::new(1.0, 0.6, 0.3) * sun
        }
        "Warehouse" => {
            let mut c = sky(d, Vec3::splat(0.25), Vec3::splat(0.18), Vec3::splat(0.06));
            for k in 0..6 {
                c += SoftBox { az: -150.0 + k as f32 * 60.0, el: 50.0, hw: 6.0, hh: 3.0, color: Vec3::new(9.0, 8.5, 7.5) }.eval(d);
            }
            c + SoftBox { az: 90.0, el: 5.0, hw: 30.0, hh: 12.0, color: Vec3::new(1.6, 1.8, 2.2) }.eval(d)
        }
        "Night" => {
            let base = sky(d, Vec3::new(0.01, 0.015, 0.04), Vec3::new(0.03, 0.03, 0.06), Vec3::splat(0.005));
            base + SoftBox { az: 20.0, el: 15.0, hw: 10.0, hh: 10.0, color: Vec3::new(0.4, 1.6, 4.0) }.eval(d)
                + SoftBox { az: -110.0, el: 10.0, hw: 12.0, hh: 8.0, color: Vec3::new(4.0, 0.8, 1.6) }.eval(d)
        }
        // "Studio Soft" (default)
        _ => {
            let base = sky(d, Vec3::new(0.5, 0.52, 0.56), Vec3::new(0.32, 0.32, 0.33), Vec3::new(0.08, 0.075, 0.07));
            base + SoftBox { az: 40.0, el: 35.0, hw: 28.0, hh: 20.0, color: Vec3::new(5.5, 5.2, 4.8) }.eval(d)
                + SoftBox { az: -125.0, el: 20.0, hw: 35.0, hh: 22.0, color: Vec3::new(1.1, 1.25, 1.5) }.eval(d)
                + SoftBox { az: 175.0, el: 55.0, hw: 8.0, hh: 25.0, color: Vec3::new(3.0, 3.0, 3.0) }.eval(d)
        }
    }
}

/// Render a built-in environment (unknown names give the default studio).
pub fn environment(name: &str, width: u32) -> EnvImage {
    let w = width.clamp(16, 4096);
    let h = w / 2;
    let mut data = vec![[0.0f32; 3]; (w * h) as usize];
    data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let d = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            *px = radiance(name, d).to_array();
        }
    });
    EnvImage { width: w, height: h, data }
}

impl EnvImage {
    fn get(&self, x: i64, y: i64) -> Vec3 {
        let xx = x.rem_euclid(self.width as i64) as u32;
        let yy = y.clamp(0, self.height as i64 - 1) as u32;
        self.data.get((yy * self.width + xx) as usize).map(|v| Vec3::from(*v)).unwrap_or(Vec3::ZERO)
    }

    /// Bilinear lookup by direction.
    pub fn sample(&self, d: Vec3) -> Vec3 {
        let uv = uv_of(d);
        let (fx, fy) = (uv.x * self.width as f32 - 0.5, uv.y * self.height as f32 - 0.5);
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = self.get(x0, y0).lerp(self.get(x0 + 1, y0), tx);
        let b = self.get(x0, y0 + 1).lerp(self.get(x0 + 1, y0 + 1), tx);
        a.lerp(b, ty)
    }

    /// Box-downsample by 2.
    pub fn half(&self) -> EnvImage {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut data = vec![[0.0f32; 3]; (w * h) as usize];
        data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let (sx, sy) = (x as i64 * 2, y as i64 * 2);
                let s = (self.get(sx, sy) + self.get(sx + 1, sy) + self.get(sx, sy + 1) + self.get(sx + 1, sy + 1)) * 0.25;
                *px = s.to_array();
            }
        });
        EnvImage { width: w, height: h, data }
    }

    /// An imported equirect image (`rgba` row-major, top row first, sRGB-encoded when `srgb`) as
    /// linear radiance: non-finite and negative values become 0, values are capped to the f16
    /// range, and images larger than [`MAX_IMPORT_WIDTH`] are box-downsampled. None when the
    /// pixel count does not match the size.
    pub fn from_rgba(width: u32, height: u32, rgba: &[[f32; 4]], srgb: bool) -> Option<EnvImage> {
        if width == 0 || height == 0 || rgba.len() as u64 != width as u64 * height as u64 {
            return None;
        }
        let lin = |v: f32| {
            let v = if v.is_finite() { v.clamp(0.0, 65_000.0) } else { 0.0 };
            if srgb { craft_texture::srgb_to_linear(v.min(1.0)) } else { v }
        };
        let data = rgba.par_iter().map(|p| [lin(p[0]), lin(p[1]), lin(p[2])]).collect();
        let mut img = EnvImage { width, height, data };
        while img.width > MAX_IMPORT_WIDTH || img.height > MAX_IMPORT_WIDTH {
            img = img.half();
        }
        // The mip chain needs at least 4×2 texels at its last level.
        if img.width < 128 || img.height < 64 {
            let (w, h) = (img.width.max(128), img.height.max(64));
            let data =
                (0..w * h).map(|i| img.sample(dir_of(((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32)).to_array()).collect();
            img = EnvImage { width: w, height: h, data };
        }
        Some(img)
    }
}

/// Widest imported environment kept (2K equirect: about 5.6 texels per degree).
pub const MAX_IMPORT_WIDTH: u32 = 2048;

fn hammersley(i: u32, n: u32) -> Vec2 {
    Vec2::new(i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10)
}

/// GGX-prefiltered mip chain: level `l` of `levels` is filtered for roughness `l / (levels-1)`.
pub fn prefilter(env: &EnvImage, levels: u32) -> Vec<EnvImage> {
    let levels = levels.clamp(1, 8);
    // Clamp fireflies, then build a box pyramid to sample from (filtered importance sampling).
    let clamped = EnvImage { data: env.data.iter().map(|c| c.map(|v| v.min(64.0))).collect(), ..env.clone() };
    let mut pyramid = vec![clamped];
    while pyramid.len() < 10 && pyramid.last().is_some_and(|p| p.width > 8) {
        if let Some(next) = pyramid.last().map(EnvImage::half) {
            pyramid.push(next);
        }
    }
    let mut out = vec![env.clone()];
    let mut size = (env.width, env.height);
    for l in 1..levels {
        size = ((size.0 / 2).max(4), (size.1 / 2).max(2));
        let rough = l as f32 / (levels - 1) as f32;
        let a = (rough * rough).max(1e-3);
        let samples = 128u32;
        let (w, h) = size;
        let sa_texel = 4.0 * PI / (env.width as f32 * env.height as f32);
        // With n = v, every sample's half vector, n·l and source level depend only on its index:
        // tabulate them once per level as (tangent-space half vector, n·l, pyramid level).
        let table: Vec<(Vec3, f32, &EnvImage)> = (0..samples)
            .filter_map(|i| {
                let xi = hammersley(i, samples);
                let phi = TAU * xi.x;
                let cos_t = ((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y)).max(0.0).sqrt();
                let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                let ndl = 2.0 * cos_t * cos_t - 1.0;
                if ndl <= 0.0 {
                    return None;
                }
                let dd = a * a / (PI * ((cos_t * cos_t) * (a * a - 1.0) + 1.0).powi(2)).max(1e-8);
                let sa_sample = 1.0 / (samples as f32 * dd / 4.0 + 1e-6);
                let lod = (0.5 * (sa_sample / sa_texel).log2() + 1.0).clamp(0.0, (pyramid.len() - 1) as f32);
                let src = pyramid.get(lod.round() as usize).unwrap_or(&pyramid[0]);
                Some((Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t), ndl, src))
            })
            .collect();
        let wsum: f32 = table.iter().map(|t| t.1).sum();
        let mut data = vec![[0.0f32; 3]; (w * h) as usize];
        data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let n = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
                let tx = up.cross(n).normalize();
                let ty = n.cross(tx);
                let mut acc = Vec3::ZERO;
                for (hv, ndl, src) in &table {
                    let hw = tx * hv.x + ty * hv.y + n * hv.z;
                    acc += src.sample(hw * (2.0 * hv.z) - n) * *ndl;
                }
                *px = (acc / wsum.max(1e-6)).to_array();
            }
        });
        out.push(EnvImage { width: w, height: h, data });
    }
    out
}

/// Diffuse irradiance as 9 SH coefficients (already convolved with the cosine lobe and divided
/// by π, so `Σ c·Y(n)` times albedo is the diffuse radiance).
pub fn sh9(env: &EnvImage) -> [[f32; 3]; 9] {
    let mut src = env.clone();
    while src.width > 128 {
        src = src.half();
    }
    let (w, h) = (src.width, src.height);
    let mut c = [Vec3::ZERO; 9];
    for y in 0..h {
        let theta = PI * (y as f32 + 0.5) / h as f32;
        let dw = (TAU / w as f32) * (PI / h as f32) * theta.sin();
        for x in 0..w {
            let d = dir_of((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            // No firefly clamp: the projection integrates, so a bright sun lights diffuse fully.
            let l = src.get(x as i64, y as i64) * dw;
            for (k, ylm) in sh_basis(d).iter().enumerate() {
                c[k] += l * *ylm;
            }
        }
    }
    let a = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];
    let mut out = [[0.0; 3]; 9];
    for k in 0..9 {
        out[k] = (c[k] * a[k] / PI).to_array();
    }
    out
}

pub fn sh_basis(d: Vec3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3.0 * z * z - 1.0),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}
