//! Equirect environment images, their construction and lookup, and the procedural environments.

use std::f32::consts::{PI, TAU};

use crate::vec3::{V3, unit};
use crate::{Limits, PbrError, for_each_row, try_pixels};

/// An equirect environment of linear RGB radiance, row-major with the top row first. Every value
/// is between 0 and [`MAX_RADIANCE`], and both sides are at least 1.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvImage {
    width: u32,
    height: u32,
    data: Vec<[f32; 3]>,
}

/// How [`EnvImage::try_from_rgba`] reads its color channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorEncoding {
    /// Linear radiance; values above 1 are kept.
    Linear,
    /// sRGB-encoded values in `0..=1`; larger values read as 1.
    Srgb,
}

/// Largest radiance an [`EnvImage`] holds. Sums of up to `u32::MAX` weighted samples of it stay
/// within `f32`, so averaging and filtering cannot overflow.
pub const MAX_RADIANCE: f32 = 1e28;

/// Names of the procedural environments, in [`Environment::ALL`] order.
pub const ENVIRONMENTS: [&str; 6] = ["Studio Soft", "Studio Contrast", "Overcast", "Sunset", "Warehouse", "Night"];

/// A procedural environment: a sky gradient with soft rectangular lights or a sun.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Environment {
    #[default]
    StudioSoft,
    StudioContrast,
    Overcast,
    Sunset,
    Warehouse,
    Night,
}

/// The direction of equirect coordinates `(u, v)`, each in `0..=1`.
pub fn dir_of(u: f32, v: f32) -> [f32; 3] {
    dir(u, v).to_array()
}

/// The equirect coordinates `(u, v)` of a unit direction, `u` in `0..1` and `v` in `0..=1`.
pub fn uv_of(d: [f32; 3]) -> [f32; 2] {
    uv(V3::from_array(d))
}

pub(crate) fn dir(u: f32, v: f32) -> V3 {
    let (theta, phi) = (v * PI, u * TAU);
    V3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

fn uv(d: V3) -> [f32; 2] {
    let phi = d.z.atan2(d.x);
    [(phi / TAU).rem_euclid(1.0), d.y.clamp(-1.0, 1.0).acos() / PI]
}

/// The direction at the center of pixel `(x, y)` of a `w × h` image.
pub(crate) fn pixel_dir(x: usize, y: usize, w: usize, h: usize) -> V3 {
    dir((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32)
}

impl EnvImage {
    /// An environment of linear radiance, `width × height` pixels. Returns an error when the size
    /// is over `limits`, the buffer does not hold `width × height` pixels, or a value is NaN,
    /// negative or above [`MAX_RADIANCE`].
    pub fn try_from_linear(width: u32, height: u32, data: Vec<[f32; 3]>, limits: &Limits) -> Result<EnvImage, PbrError> {
        let expected = limits.check(width, height)?;
        if data.len() != expected {
            return Err(PbrError::BadDataLength { expected, found: data.len() });
        }
        if let Some(index) = data.iter().position(|p| p.iter().any(|v| !(0.0..=MAX_RADIANCE).contains(v))) {
            return Err(PbrError::BadRadiance { index });
        }
        Ok(EnvImage { width, height, data })
    }

    /// An environment from RGBA pixels (alpha is ignored), row-major with the top row first.
    /// NaN, infinite and negative values become 0 and values are capped at 65,000, the largest
    /// a half float holds near its top. [`ColorEncoding::Srgb`] values are decoded to linear after
    /// capping at 1. Returns an error when the size is over `limits` or the slice does not hold
    /// `width × height` pixels. The size is kept as given.
    pub fn try_from_rgba(width: u32, height: u32, rgba: &[[f32; 4]], encoding: ColorEncoding, limits: &Limits) -> Result<EnvImage, PbrError> {
        let expected = limits.check(width, height)?;
        if rgba.len() != expected {
            return Err(PbrError::BadDataLength { expected, found: rgba.len() });
        }
        let lin = |v: f32| {
            let v = if v.is_finite() { v.clamp(0.0, 65_000.0) } else { 0.0 };
            match encoding {
                ColorEncoding::Linear => v,
                ColorEncoding::Srgb => craft_texture::srgb_to_linear(v.min(1.0)),
            }
        };
        let mut data = try_pixels(expected)?;
        for (out, p) in data.iter_mut().zip(rgba) {
            *out = [lin(p[0]), lin(p[1]), lin(p[2])];
        }
        Ok(EnvImage { width, height, data })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, row-major with the top row first.
    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.data
    }

    pub fn into_pixels(self) -> Vec<[f32; 3]> {
        self.data
    }

    /// The pixel at column `x` (wrapped) and row `y` (clamped).
    pub(crate) fn get(&self, x: i64, y: i64) -> V3 {
        let w = i64::from(self.width.max(1));
        let xx = x.rem_euclid(w) as usize;
        let yy = y.clamp(0, i64::from(self.height.max(1)) - 1) as usize;
        self.data.get(yy * self.width as usize + xx).map_or(V3::ZERO, |v| V3::from_array(*v))
    }

    /// Bilinear lookup by direction. `d` need not be unit length. Returns
    /// [`PbrError::BadDirection`] when it is not finite or has zero length.
    pub fn sample(&self, d: [f32; 3]) -> Result<[f32; 3], PbrError> {
        let d = unit(d).ok_or(PbrError::BadDirection)?;
        Ok(self.sample_v(d).to_radiance_array())
    }

    /// Bilinear lookup by a finite direction.
    pub(crate) fn sample_v(&self, d: V3) -> V3 {
        let [u, v] = uv(d);
        let (fx, fy) = (u * self.width as f32 - 0.5, v * self.height as f32 - 0.5);
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = self.get(x0, y0).lerp(self.get(x0 + 1, y0), tx);
        let b = self.get(x0, y0 + 1).lerp(self.get(x0 + 1, y0 + 1), tx);
        a.lerp(b, ty)
    }

    /// Box-downsamples by 2 to `max(width / 2, 1) × max(height / 2, 1)`. Each output pixel
    /// averages the source area it covers, so the last column or row of an odd side is kept in
    /// the average. Returns an error when the source or the result is over `limits`.
    pub fn try_half(&self, limits: &Limits) -> Result<EnvImage, PbrError> {
        limits.check(self.width, self.height)?;
        let (w, h) = half_size(self.width, self.height);
        let len = limits.check(w, h)?;
        let mut data = try_pixels(len)?;
        if self.width.is_multiple_of(2) && self.height.is_multiple_of(2) {
            for_each_row(&mut data, w as usize, |y, row| {
                for (x, px) in row.iter_mut().enumerate() {
                    let (sx, sy) = (x as i64 * 2, y as i64 * 2);
                    let s = (self.get(sx, sy) + self.get(sx + 1, sy) + self.get(sx, sy + 1) + self.get(sx + 1, sy + 1)) * 0.25;
                    *px = s.to_radiance_array();
                }
            });
        } else {
            for_each_row(&mut data, w as usize, |y, row| {
                let ys = box_span(self.height, h, y);
                for (x, px) in row.iter_mut().enumerate() {
                    let mut acc = V3::ZERO;
                    for (sy, fy) in ys.clone() {
                        for (sx, fx) in box_span(self.width, w, x) {
                            acc += self.get(sx as i64, sy as i64) * (fx * fy);
                        }
                    }
                    *px = acc.to_radiance_array();
                }
            });
        }
        Ok(EnvImage { width: w, height: h, data })
    }

    /// Resamples to `width × height` by bilinear lookup at each output pixel's direction. Returns
    /// an error when the source or the result is over `limits`.
    pub fn try_resampled(&self, width: u32, height: u32, limits: &Limits) -> Result<EnvImage, PbrError> {
        limits.check(self.width, self.height)?;
        let len = limits.check(width, height)?;
        let mut data = try_pixels(len)?;
        let (w, h) = (width as usize, height as usize);
        for_each_row(&mut data, w, |y, row| {
            for (x, px) in row.iter_mut().enumerate() {
                *px = self.sample_v(pixel_dir(x, y, w, h)).to_radiance_array();
            }
        });
        Ok(EnvImage { width, height, data })
    }
}

/// The size [`EnvImage::try_half`] gives.
pub(crate) fn half_size(w: u32, h: u32) -> (u32, u32) {
    ((w / 2).max(1), (h / 2).max(1))
}

/// The source pixels that output pixel `i` covers along one axis when `src` pixels become `dst`,
/// each with the share of the output pixel's width it covers. The shares add up to 1. Nothing is
/// allocated.
fn box_span(src: u32, dst: u32, i: usize) -> impl Iterator<Item = (usize, f32)> + Clone {
    let scale = f64::from(src) / f64::from(dst);
    let (lo, hi) = (i as f64 * scale, (i as f64 + 1.0) * scale);
    let first = lo.floor() as usize;
    let last = (hi.ceil() as usize).min(src as usize);
    (first..last).filter_map(move |s| {
        let cover = (hi.min(s as f64 + 1.0) - lo.max(s as f64)) / scale;
        (cover > 0.0).then_some((s, cover as f32))
    })
}

/// A soft rectangular light: azimuth and elevation of its center and its half sizes, in degrees.
struct SoftBox {
    az: f32,
    el: f32,
    hw: f32,
    hh: f32,
    color: V3,
}

impl SoftBox {
    fn eval(&self, d: V3) -> V3 {
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

fn sky(d: V3, zenith: V3, horizon: V3, ground: V3) -> V3 {
    if d.y >= 0.0 { horizon.lerp(zenith, d.y.powf(0.6)) } else { horizon.lerp(ground, (-d.y).powf(0.25)) }
}

const fn soft(az: f32, el: f32, hw: f32, hh: f32, color: V3) -> SoftBox {
    SoftBox { az, el, hw, hh, color }
}

impl Environment {
    /// Every environment, in menu order.
    pub const ALL: [Environment; 6] = [
        Environment::StudioSoft,
        Environment::StudioContrast,
        Environment::Overcast,
        Environment::Sunset,
        Environment::Warehouse,
        Environment::Night,
    ];

    /// The display name, one of [`ENVIRONMENTS`].
    pub fn name(self) -> &'static str {
        match self {
            Environment::StudioSoft => "Studio Soft",
            Environment::StudioContrast => "Studio Contrast",
            Environment::Overcast => "Overcast",
            Environment::Sunset => "Sunset",
            Environment::Warehouse => "Warehouse",
            Environment::Night => "Night",
        }
    }

    /// The environment with display name `name`, or `None` for another name.
    pub fn from_name(name: &str) -> Option<Environment> {
        Environment::ALL.into_iter().find(|e| e.name() == name)
    }

    fn radiance(self, d: V3) -> V3 {
        match self {
            Environment::StudioContrast => {
                let base = sky(d, V3::splat(0.04), V3::splat(0.03), V3::splat(0.01));
                base + soft(35.0, 30.0, 22.0, 18.0, V3::new(14.0, 13.2, 12.0)).eval(d) + soft(-150.0, 40.0, 6.0, 30.0, V3::new(4.0, 4.4, 5.0)).eval(d)
            }
            Environment::Overcast => {
                let base = sky(d, V3::new(1.2, 1.25, 1.35), V3::new(0.95, 0.95, 0.95), V3::new(0.18, 0.2, 0.15));
                base + soft(60.0, 55.0, 40.0, 25.0, V3::splat(0.6)).eval(d)
            }
            Environment::Sunset => {
                let base = sky(d, V3::new(0.15, 0.25, 0.55), V3::new(1.4, 0.65, 0.3), V3::new(0.08, 0.06, 0.05));
                let sun = smooth((d.dot(dir(0.12, 0.47)) - 0.9985) / 0.0012) * 120.0;
                base + V3::new(1.0, 0.6, 0.3) * sun
            }
            Environment::Warehouse => {
                let mut c = sky(d, V3::splat(0.25), V3::splat(0.18), V3::splat(0.06));
                for k in 0..6 {
                    c += soft(-150.0 + k as f32 * 60.0, 50.0, 6.0, 3.0, V3::new(9.0, 8.5, 7.5)).eval(d);
                }
                c + soft(90.0, 5.0, 30.0, 12.0, V3::new(1.6, 1.8, 2.2)).eval(d)
            }
            Environment::Night => {
                let base = sky(d, V3::new(0.01, 0.015, 0.04), V3::new(0.03, 0.03, 0.06), V3::splat(0.005));
                base + soft(20.0, 15.0, 10.0, 10.0, V3::new(0.4, 1.6, 4.0)).eval(d) + soft(-110.0, 10.0, 12.0, 8.0, V3::new(4.0, 0.8, 1.6)).eval(d)
            }
            Environment::StudioSoft => {
                let base = sky(d, V3::new(0.5, 0.52, 0.56), V3::new(0.32, 0.32, 0.33), V3::new(0.08, 0.075, 0.07));
                base + soft(40.0, 35.0, 28.0, 20.0, V3::new(5.5, 5.2, 4.8)).eval(d)
                    + soft(-125.0, 20.0, 35.0, 22.0, V3::new(1.1, 1.25, 1.5)).eval(d)
                    + soft(175.0, 55.0, 8.0, 25.0, V3::new(3.0, 3.0, 3.0)).eval(d)
            }
        }
    }

    /// Renders the environment at `width × width / 2` pixels. Returns [`PbrError::BadSize`] for a
    /// width under 2 and an error when the size is over `limits`.
    pub fn try_render(self, width: u32, limits: &Limits) -> Result<EnvImage, PbrError> {
        let height = width / 2;
        let len = limits.check(width, height)?;
        let mut data = try_pixels(len)?;
        let (w, h) = (width as usize, height as usize);
        for_each_row(&mut data, w, |y, row| {
            for (x, px) in row.iter_mut().enumerate() {
                *px = self.radiance(pixel_dir(x, y, w, h)).to_array();
            }
        });
        Ok(EnvImage { width, height, data })
    }
}

/// A dim studio environment of 8 × 4 pixels for an environment light that has no image: a gray
/// gradient whose radiance in direction `d` is proportional to `0.15 + 1.2 max(0, d·key)`, with
/// the key direction `(0.66, 0.75, 0)` up and toward +X. It is scaled so that the mean of its
/// pixels, each weighted by `sin θ` at its center, is 0.3.
pub fn default_environment() -> Result<EnvImage, PbrError> {
    let key = V3::new(0.66, 0.75, 0.0);
    let (w, h) = (8usize, 4usize);
    let mut data = try_pixels(w * h)?;
    let (mut mean, mut weight) = (0.0f32, 0.0f32);
    for (i, px) in data.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let sa = ((y as f32 + 0.5) / h as f32 * PI).sin();
        let l = 0.15 + 1.2 * pixel_dir(x, y, w, h).dot(key).max(0.0);
        *px = [l; 3];
        mean += l * sa;
        weight += sa;
    }
    let k = if mean.is_finite() && mean > 1e-6 { 0.3 * weight / mean } else { 1.0 };
    for px in &mut data {
        *px = px.map(|c| c * k);
    }
    Ok(EnvImage { width: w as u32, height: h as u32, data })
}
