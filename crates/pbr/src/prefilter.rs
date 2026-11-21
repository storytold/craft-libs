//! The GGX-prefiltered specular chain.

use std::f32::consts::{PI, TAU};

use crate::environment::{EnvImage, half_size, pixel_dir};
use crate::vec3::V3;
use crate::{Limits, PIXEL_BYTES, PbrError, for_each_row, try_pixels};

/// How [`try_prefilter`] filters. `#[non_exhaustive]`: start from [`PrefilterOptions::default`]
/// and set the fields you change.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PrefilterOptions {
    /// Levels in the chain, 1 to [`Limits::max_levels`]. Default 6.
    pub levels: u32,
    /// Importance samples per output pixel, 1 to [`Limits::max_samples`]. Default 128.
    pub samples: u32,
    /// Added to each sample's source pyramid level, in levels. A larger value reads blurrier
    /// source levels and hides sampling noise. Finite. Default 1.
    pub lod_bias: f32,
    /// When set, source radiance is capped at this value before filtering, so a small bright
    /// light gives fewer isolated bright pixels in rough levels and less light overall. Finite
    /// and above 0. Level 0 is never capped. Default `None`.
    pub firefly_cap: Option<f32>,
}

impl Default for PrefilterOptions {
    fn default() -> Self {
        Self { levels: 6, samples: 128, lod_bias: 1.0, firefly_cap: None }
    }
}

impl PrefilterOptions {
    fn validate(&self, limits: &Limits) -> Result<(), PbrError> {
        if self.levels == 0 || self.levels > limits.max_levels {
            return Err(PbrError::BadOption("the level count is 0 or over the limit"));
        }
        if self.samples == 0 || self.samples > limits.max_samples {
            return Err(PbrError::BadOption("the sample count is 0 or over the limit"));
        }
        if !self.lod_bias.is_finite() {
            return Err(PbrError::BadOption("the level bias is not finite"));
        }
        if self.firefly_cap.is_some_and(|c| !c.is_finite() || c <= 0.0) {
            return Err(PbrError::BadOption("the firefly cap is not finite or not above 0"));
        }
        Ok(())
    }
}

/// Point `i` of `n` of the Hammersley set: `(i / n, radical inverse of i in base 2)`. Returns
/// `None` when `i` is not below `n`.
pub fn hammersley(i: u32, n: u32) -> Option<[f32; 2]> {
    (i < n).then(|| [i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10])
}

/// The GGX normal distribution `D` at `n_dot_h` for perceptual `roughness`, with
/// `α = max(roughness², 0.001)`: `α² / (π ((n·h)² (α² − 1) + 1)²)`, as glTF 2.0 Appendix B writes
/// it in section B.3.2. Over the hemisphere, `D(h) (n·h)` integrates to 1.
/// It is evaluated in `f64`, so it keeps its peak `1 / (π α²)` at `n·h = 1` for the smallest `α`.
pub fn ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let a = f64::from((roughness * roughness).max(1e-3));
    let c2 = f64::from(n_dot_h) * f64::from(n_dot_h);
    let a2 = a * a;
    (a2 / (std::f64::consts::PI * (c2 * (a2 - 1.0) + 1.0).powi(2)).max(f64::MIN_POSITIVE)) as f32
}

/// `D` of [`ggx_distribution`] in `f32` for the sample table, with `a = α`. The denominator is at
/// least `π α⁴`, which is above `f32::MIN_POSITIVE` for `α` of at least 0.001.
fn ggx_d(cos_t: f32, a: f32) -> f32 {
    a * a / (PI * ((cos_t * cos_t) * (a * a - 1.0) + 1.0).powi(2)).max(f32::MIN_POSITIVE)
}

/// Sizes of the source pyramid after level 0: halved while it is wider than 8, at most 9 more.
fn pyramid_sizes(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut sizes = Vec::new();
    let mut size = (w, h);
    while sizes.len() < 9 && size.0 > 8 {
        size = half_size(size.0, size.1);
        sizes.push(size);
    }
    sizes
}

/// The size of the output level after one of `size`: each side halved but kept at least 4 × 2,
/// and never larger than `size`.
fn next_level(size: (u32, u32)) -> (u32, u32) {
    ((size.0 / 2).max(size.0.min(4)), (size.1 / 2).max(size.1.min(2)))
}

/// Sizes of output levels 1 and up, reserved fallibly. Called after [`check_budget`].
fn level_sizes(w: u32, h: u32, levels: u32) -> Result<Vec<(u32, u32)>, PbrError> {
    let mut sizes = Vec::new();
    sizes.try_reserve_exact(levels.saturating_sub(1) as usize).map_err(|_| PbrError::OutOfMemory)?;
    let mut size = (w, h);
    for _ in 1..levels {
        size = next_level(size);
        sizes.push(size);
    }
    Ok(sizes)
}

/// The pixels of output levels 1 and up, without listing them: once a level keeps its size,
/// every later level has that size too.
fn level_pixels(w: u32, h: u32, levels: u32) -> Option<u64> {
    let mut size = (w, h);
    let mut total = 0u64;
    let mut left = u64::from(levels.saturating_sub(1));
    while left > 0 {
        let next = next_level(size);
        let pixels = u64::from(next.0) * u64::from(next.1);
        if next == size {
            return total.checked_add(pixels.checked_mul(left)?);
        }
        total = total.checked_add(pixels)?;
        left -= 1;
        size = next;
    }
    Some(total)
}

fn pixels_of(sizes: &[(u32, u32)]) -> Option<u64> {
    sizes.iter().try_fold(0u64, |acc, &(w, h)| acc.checked_add(u64::from(w) * u64::from(h)))
}

/// Checks the bytes held at once (the source copies, the pyramid, the levels with their list
/// entries, and one sample table) and the sample work of a call before anything is allocated.
fn check_budget(env: &EnvImage, options: &PrefilterOptions, pyramid: &[(u32, u32)], limits: &Limits) -> Result<(), PbrError> {
    let (w, h) = (env.width(), env.height());
    let src = u64::from(w) * u64::from(h);
    // One level is a copy of the source; no pyramid, capped copy or sample table is made.
    let filtering = options.levels > 1;
    let copies = if filtering && options.firefly_cap.is_some() { 2 } else { 1 };
    let pyramid = if filtering { pixels_of(pyramid) } else { Some(0) };
    let levels = level_pixels(w, h, options.levels);
    let pixels = pyramid.zip(levels).and_then(|(p, l)| src.checked_mul(copies)?.checked_add(p)?.checked_add(l));
    let table = if filtering { u64::from(options.samples) * size_of::<Sample<'_>>() as u64 } else { 0 };
    let lists = u64::from(options.levels) * (size_of::<(u32, u32)>() + size_of::<EnvImage>()) as u64;
    let bytes = pixels.and_then(|p| p.checked_mul(PIXEL_BYTES)?.checked_add(table)?.checked_add(lists));
    limits.check_total_bytes(bytes, w, h)?;
    let needed = levels.and_then(|l| l.checked_mul(u64::from(options.samples))).unwrap_or(u64::MAX);
    if needed > limits.max_work {
        return Err(PbrError::TooMuchWork { needed, limit: limits.max_work });
    }
    Ok(())
}

/// A copy of `env` with every value capped at `cap`.
fn capped(env: &EnvImage, cap: f32) -> Result<EnvImage, PbrError> {
    let mut data = try_pixels(env.pixels().len())?;
    for (out, p) in data.iter_mut().zip(env.pixels()) {
        *out = p.map(|v| v.min(cap));
    }
    rebuild(env.width(), env.height(), data)
}

fn rebuild(w: u32, h: u32, data: Vec<[f32; 3]>) -> Result<EnvImage, PbrError> {
    let limits = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX, ..Limits::default() };
    EnvImage::try_from_linear(w, h, data, &limits)
}

/// One importance sample: its tangent-space half vector, `n·l` and the pyramid level it reads.
type Sample<'a> = (V3, f32, &'a EnvImage);

/// The samples of one level. With `n = v`, every sample's half vector, `n·l` and source level
/// depend only on its index, so they are computed once per level.
fn sample_table<'a>(rough: f32, options: &PrefilterOptions, src_pixels: f32, pyramid: &[&'a EnvImage]) -> Result<Vec<Sample<'a>>, PbrError> {
    let a = (rough * rough).max(1e-3);
    let samples = options.samples;
    let sa_texel = 4.0 * PI / src_pixels;
    let top = pyramid.len().saturating_sub(1);
    let mut table = Vec::new();
    table.try_reserve_exact(samples as usize).map_err(|_| PbrError::OutOfMemory)?;
    table.extend((0..samples).filter_map(|i| {
        let [xu, xv] = hammersley(i, samples)?;
        let phi = TAU * xu;
        let cos_t = ((1.0 - xv) / (1.0 + (a * a - 1.0) * xv)).max(0.0).sqrt();
        let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
        let ndl = 2.0 * cos_t * cos_t - 1.0;
        if ndl <= 0.0 {
            return None;
        }
        let dd = ggx_d(cos_t, a);
        let sa_sample = 1.0 / (samples as f32 * dd / 4.0 + 1e-6);
        let lod = (0.5 * (sa_sample / sa_texel).log2() + options.lod_bias).clamp(0.0, top as f32);
        let src = pyramid.get(lod.round() as usize).or(pyramid.first()).copied()?;
        Some((V3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t), ndl, src))
    }));
    Ok(table)
}

/// One filtered level of `w × h` pixels.
fn filter_level(w: u32, h: u32, table: &[Sample<'_>]) -> Result<EnvImage, PbrError> {
    let wsum: f32 = table.iter().map(|t| t.1).sum();
    let mut data = try_pixels(w as usize * h as usize)?;
    let (wu, hu) = (w as usize, h as usize);
    for_each_row(&mut data, wu, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let n = pixel_dir(x, y, wu, hu);
            let up = if n.y.abs() < 0.999 { V3::Y } else { V3::X };
            let tx = up.cross(n).normalize();
            let ty = n.cross(tx);
            let mut acc = V3::ZERO;
            for (hv, ndl, src) in table {
                let hw = tx * hv.x + ty * hv.y + n * hv.z;
                acc += src.sample_v(hw * (2.0 * hv.z) - n) * *ndl;
            }
            *px = (acc / wsum.max(1e-6)).to_radiance_array();
        }
    });
    rebuild(w, h, data)
}

/// The GGX-prefiltered chain of `env`. Level `l` of `options.levels` is filtered for roughness
/// `l / (levels − 1)`; level 0 is a copy of `env`, and each later level halves each side but
/// keeps at least 4 × 2 pixels and is never larger than the level before. Returns an error, before
/// allocating, for an option out of range, for a source over the size limits, or when the source
/// copies, the pyramid, the levels and the sample table together are over [`Limits::max_bytes`]
/// or the sample work is over [`Limits::max_work`].
pub fn try_prefilter(env: &EnvImage, options: &PrefilterOptions, limits: &Limits) -> Result<Vec<EnvImage>, PbrError> {
    options.validate(limits)?;
    let (w, h) = (env.width(), env.height());
    limits.check(w, h)?;
    let pyramid_size = pyramid_sizes(w, h);
    check_budget(env, options, &pyramid_size, limits)?;
    let sizes = level_sizes(w, h, options.levels)?;

    let mut out = Vec::new();
    out.try_reserve_exact(sizes.len() + 1).map_err(|_| PbrError::OutOfMemory)?;
    let mut first = try_pixels(env.pixels().len())?;
    first.copy_from_slice(env.pixels());
    out.push(rebuild(w, h, first)?);
    if sizes.is_empty() {
        return Ok(out);
    }
    let base = match options.firefly_cap {
        Some(cap) => Some(capped(env, cap)?),
        None => None,
    };
    let base = base.as_ref().unwrap_or(env);
    let unbounded = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX, ..*limits };
    let mut rest: Vec<EnvImage> = Vec::new();
    for _ in &pyramid_size {
        let next = rest.last().unwrap_or(base).try_half(&unbounded)?;
        rest.push(next);
    }
    let pyramid: Vec<&EnvImage> = std::iter::once(base).chain(rest.iter()).collect();
    let src_pixels = w as f32 * h as f32;
    let steps = (options.levels - 1) as f32;
    for (l, &(lw, lh)) in sizes.iter().enumerate() {
        let rough = (l + 1) as f32 / steps;
        let table = sample_table(rough, options, src_pixels, &pyramid)?;
        out.push(filter_level(lw, lh, &table)?);
    }
    Ok(out)
}
