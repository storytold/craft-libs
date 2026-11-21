//! Mip chains and level choice from a sampling footprint.
//!
//! A [`MipChain`] holds an [`Image`] at level 0 and every smaller level down to 1 × 1. Each level
//! halves each side of the one before it, rounding down and keeping at least 1, so a chain over
//! `w × h` has `floor(log2(max(w, h))) + 1` levels, as the Vulkan specification (section 12.4.4,
//! <https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#resources-image-mip-level-sizing>)
//! and `ARB_texture_non_power_of_two`
//! (<https://registry.khronos.org/OpenGL/extensions/ARB/ARB_texture_non_power_of_two.txt>) size
//! the levels of an image of any size.
//!
//! [`MipChain::try_build`] makes each level from the one before it with a box filter. Each axis of
//! the larger level is split into runs of two texels, and when the axis has an odd number of
//! texels, the last run takes three; an axis of one texel is one run. A texel of the new level is
//! the mean of the texels in its rectangle of runs, each weighted the same, so no row or column is
//! dropped. Every channel, alpha included, is averaged on its own in `f64`, with no transfer
//! curve, clamp or premultiplication. The values are taken as linear: decode sRGB data, such as
//! glTF base color, before building the chain, and premultiply color by alpha first when
//! transparent texels must not bleed into their neighbors. A NaN or infinite texel makes the
//! texels of smaller levels that cover it non-finite.
//!
//! [`MipChain::lod`] computes the level of detail from the derivatives of the normalized texture
//! coordinate along the two screen axes, as section 20.3.7 of the Vulkan specification does:
//! `ρx = hypot(w × du/dx, h × dv/dx)`, `ρy = hypot(w × du/dy, h × dv/dy)` with the size of
//! level 0, `λ = log2(max(ρx, ρy)) + bias`, clamped to the sampler's `min_lod..=max_lod`. A
//! footprint of zero gives `min_lod`. [`MipChain::select`] picks the levels for that λ as the
//! same section does: the level `ceil(d + 0.5) − 1` for a nearest mip filter, so a tie takes the
//! lower-numbered, more detailed level (1.5 selects level 1, not level 2), and the levels `floor(d)` and the next one, weighted by the
//! fraction of `d`, for a linear mip filter, where `d` is λ clamped to the levels of the chain.
//! Within each level the texel filter is the sampler's magnification filter when λ is 0 or
//! less and the minification filter's texel filter otherwise (section 20.6.3,
//! <https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#textures-texel-filtering>).
//! The two level samples are summed in `f64`; a level with weight 0 is not read.

use crate::{Image, ImageError, Kind, Limits, MAX_ALLOC_BYTES, f32_of_sum};

/// Default [`MipLimits::max_chain_bytes`]: 1.5 GiB, which holds a chain over an 8192 × 8192
/// RGBA plane (about 1.33 GiB).
pub const DEFAULT_MAX_CHAIN_BYTES: u64 = 3 << 29;

/// Default [`MipSampler::max_lod`]: 1000, the value Vulkan names `VK_LOD_CLAMP_NONE`.
pub const LOD_CLAMP_NONE: f32 = 1000.0;

/// Caps on a mip chain, checked before any level is made.
/// `#[non_exhaustive]`: start from [`MipLimits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MipLimits {
    /// Caps on each level, level 0 included. Default [`Limits::default`].
    pub image: Limits,
    /// Most bytes of float data in all levels together, level 0 included. Default
    /// [`DEFAULT_MAX_CHAIN_BYTES`].
    pub max_chain_bytes: u64,
}

impl Default for MipLimits {
    fn default() -> Self {
        Self { image: Limits::default(), max_chain_bytes: DEFAULT_MAX_CHAIN_BYTES }
    }
}

/// Why a chain could not be made or sampled.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MipError {
    /// A level is outside [`MipLimits::image`], or its memory could not be reserved.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// All levels together need more bytes than [`MipLimits::max_chain_bytes`]. `required` is
    /// `u64::MAX` when the sum overflows.
    #[error("the mip chain needs {required} bytes, over the limit of {limit}")]
    ChainTooLarge { required: u64, limit: u64 },
    /// Memory for the list of levels could not be reserved.
    #[error("not enough memory for a list of {0} mip levels")]
    OutOfMemory(usize),
    /// [`MipChain::from_levels`] was given no level.
    #[error("a mip chain needs at least one level")]
    NoLevels,
    /// A level given to [`MipChain::from_levels`] has another size than the chain's level of that
    /// index, or another kind than level 0.
    #[error("mip level {index} is {found:?}, where {expected:?} is needed")]
    LevelMismatch { index: usize, expected: (u32, u32, Kind), found: (u32, u32, Kind) },
    /// A derivative, LOD, bias or LOD bound is NaN or infinite.
    #[error("parameter {name} is {value}, which is not finite")]
    BadParameter { name: &'static str, value: f64 },
    /// The sampler's `min_lod` is above its `max_lod`.
    #[error("min_lod {min_lod} is above max_lod {max_lod}")]
    BadLodBounds { min_lod: f32, max_lod: f32 },
}

/// How texels are read within one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexelFilter {
    /// [`Image::sample_nearest_axes`].
    Nearest,
    /// [`Image::sample_axes`] (bilinear).
    Linear,
}

impl TexelFilter {
    /// The filter of a glTF 2.0 `sampler.magFilter` code: 9728 `NEAREST` or 9729 `LINEAR`.
    /// `None` for any other code; a caller picks the filter when the field is absent.
    pub fn from_gltf(code: u32) -> Option<Self> {
        match code {
            9728 => Some(TexelFilter::Nearest),
            9729 => Some(TexelFilter::Linear),
            _ => None,
        }
    }
}

/// The minification filter: the texel filter within a level, and how levels are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MipMinFilter {
    /// Nearest texel of level 0; no other level is read.
    Nearest,
    /// Bilinear sample of level 0; no other level is read.
    Linear,
    /// Nearest texel of the nearest level.
    NearestMipmapNearest,
    /// Bilinear sample of the nearest level.
    LinearMipmapNearest,
    /// Nearest texels of the two nearest levels, blended.
    NearestMipmapLinear,
    /// Bilinear samples of the two nearest levels, blended (trilinear filtering).
    LinearMipmapLinear,
}

/// How a minification filter chooses levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MipMode {
    BaseOnly,
    Nearest,
    Linear,
}

impl MipMinFilter {
    /// The filter of a glTF 2.0 `sampler.minFilter` code: 9728 `NEAREST`, 9729 `LINEAR`, 9984
    /// `NEAREST_MIPMAP_NEAREST`, 9985 `LINEAR_MIPMAP_NEAREST`, 9986 `NEAREST_MIPMAP_LINEAR` or
    /// 9987 `LINEAR_MIPMAP_LINEAR`. `None` for any other code; a caller picks the filter when the
    /// field is absent.
    pub fn from_gltf(code: u32) -> Option<Self> {
        match code {
            9728 => Some(MipMinFilter::Nearest),
            9729 => Some(MipMinFilter::Linear),
            9984 => Some(MipMinFilter::NearestMipmapNearest),
            9985 => Some(MipMinFilter::LinearMipmapNearest),
            9986 => Some(MipMinFilter::NearestMipmapLinear),
            9987 => Some(MipMinFilter::LinearMipmapLinear),
            _ => None,
        }
    }

    /// The texel filter used within a level when the texture is minified.
    pub fn texel_filter(self) -> TexelFilter {
        match self {
            MipMinFilter::Nearest | MipMinFilter::NearestMipmapNearest | MipMinFilter::NearestMipmapLinear => TexelFilter::Nearest,
            MipMinFilter::Linear | MipMinFilter::LinearMipmapNearest | MipMinFilter::LinearMipmapLinear => TexelFilter::Linear,
        }
    }

    fn mode(self) -> MipMode {
        match self {
            MipMinFilter::Nearest | MipMinFilter::Linear => MipMode::BaseOnly,
            MipMinFilter::NearestMipmapNearest | MipMinFilter::LinearMipmapNearest => MipMode::Nearest,
            MipMinFilter::NearestMipmapLinear | MipMinFilter::LinearMipmapLinear => MipMode::Linear,
        }
    }
}

/// How [`MipChain`] samples are filtered and wrapped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MipSampler {
    pub min_filter: MipMinFilter,
    pub mag_filter: TexelFilter,
    /// The wrap mode of `u` and of `v` in every level: `true` repeats, `false` clamps to the edge
    /// texels, as for [`Image::sample_axes`].
    pub tiling: [bool; 2],
    /// Added to the LOD before the bounds. Must be finite.
    pub lod_bias: f32,
    /// Lowest LOD after the bias. Must be finite and at most `max_lod`.
    pub min_lod: f32,
    /// Highest LOD after the bias. Must be finite.
    pub max_lod: f32,
}

impl MipSampler {
    /// A sampler with these filters and wrap modes, a bias of 0 and LOD bounds `0..=LOD_CLAMP_NONE`.
    pub fn new(min_filter: MipMinFilter, mag_filter: TexelFilter, tiling: [bool; 2]) -> Self {
        Self { min_filter, mag_filter, tiling, lod_bias: 0.0, min_lod: 0.0, max_lod: LOD_CLAMP_NONE }
    }

    fn check(&self) -> Result<(), MipError> {
        finite("lod_bias", self.lod_bias)?;
        finite("min_lod", self.min_lod)?;
        finite("max_lod", self.max_lod)?;
        if self.min_lod > self.max_lod {
            return Err(MipError::BadLodBounds { min_lod: self.min_lod, max_lod: self.max_lod });
        }
        Ok(())
    }

    /// `base + lod_bias`, clamped to `min_lod..=max_lod`. The sampler has been checked, so the
    /// result is finite, also for a `base` of negative infinity.
    fn bound(&self, base: f64) -> f32 {
        (base + f64::from(self.lod_bias)).clamp(f64::from(self.min_lod), f64::from(self.max_lod)) as f32
    }
}

fn finite(name: &'static str, value: f32) -> Result<(), MipError> {
    if value.is_finite() { Ok(()) } else { Err(MipError::BadParameter { name, value: f64::from(value) }) }
}

/// The derivatives of a normalized texture coordinate `(u, v)` along the two screen axes:
/// `dx = (du/dx, dv/dx)` and `dy = (du/dy, dv/dy)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UvDerivatives {
    pub dx: [f32; 2],
    pub dy: [f32; 2],
}

/// The levels a sample reads: `(1 − blend) × level + blend × next`, each read with `filter`.
/// For a nearest mip filter, `next` equals `level` and `blend` is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelSelection {
    pub filter: TexelFilter,
    pub level: usize,
    pub next: usize,
    pub blend: f32,
}

/// An image and its smaller levels down to 1 × 1, each level of the same kind.
#[derive(Clone, Debug, PartialEq)]
pub struct MipChain {
    levels: Vec<Image>,
}

/// One side of the level after a level whose side is `n`.
fn half(n: u32) -> u32 {
    (n / 2).max(1)
}

/// The sizes of every level of a chain over a `w × h` level 0, from level 0 down to 1 × 1.
/// Empty when a side is 0.
pub fn level_sizes(w: u32, h: u32) -> impl Iterator<Item = (u32, u32)> {
    let first = (w > 0 && h > 0).then_some((w, h));
    std::iter::successors(first, |&(w, h)| (w > 1 || h > 1).then(|| (half(w), half(h))))
}

/// The level count and the bytes of all levels of a chain over `w × h` of `kind`, or the error
/// `limits` give, with `max_alloc_bytes` as the largest allocation the target allows.
fn plan_within(w: u32, h: u32, kind: Kind, limits: &MipLimits, max_alloc_bytes: u64) -> Result<(usize, u64), MipError> {
    let limit = limits.max_chain_bytes;
    let mut count = 0usize;
    let mut total = 0u64;
    for (lw, lh) in level_sizes(w, h) {
        let floats = limits.image.check_within(lw, lh, kind, max_alloc_bytes)?;
        let bytes = (floats as u64).checked_mul(size_of::<f32>() as u64);
        total = bytes.and_then(|b| total.checked_add(b)).ok_or(MipError::ChainTooLarge { required: u64::MAX, limit })?;
        count += 1;
    }
    if count == 0 {
        return Err(ImageError::BadSize(w, h).into());
    }
    if total > limit {
        return Err(MipError::ChainTooLarge { required: total, limit });
    }
    Ok((count, total))
}

/// The run of source texels `start..end` that texel `i` of a side of `dst` texels covers on a
/// source side of `src` texels: two texels, and every remaining texel for the last one.
fn span(i: u32, dst: u32, src: u32) -> (usize, usize) {
    let start = 2 * i as usize;
    if i + 1 == dst { (start, src as usize) } else { (start, start + 2) }
}

/// The box-filtered level after `src`, checked against `limits` and reserved first.
fn downsample(src: &Image, limits: &Limits) -> Result<Image, ImageError> {
    let (sw, sh) = (src.width(), src.height());
    let (w, h) = (half(sw), half(sh));
    let c = src.channels();
    let data = src.data();
    let next = Image::try_generate(w, h, src.kind(), limits, |x, y| {
        let ((x0, x1), (y0, y1)) = (span(x, w, sw), span(y, h, sh));
        let mut sum = [0.0f64; 4];
        for row in y0..y1 {
            let base = (row * sw as usize + x0) * c;
            for texel in data.get(base..base + (x1 - x0) * c).unwrap_or(&[]).chunks_exact(c) {
                for (s, v) in sum.iter_mut().zip(texel) {
                    *s += f64::from(*v);
                }
            }
        }
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        sum.map(|s| f32_of_sum(s / n))
    })?;
    Ok(next.with_integer_sampling(src.integer_sampling()))
}

impl MipChain {
    /// The bytes of float data in every level of a chain over a `w × h` level 0 of `kind`, or the
    /// error [`MipChain::try_build`] would return for that size. Nothing is allocated, so a caller
    /// can check a size read from a file header before decoding it.
    pub fn required_bytes(w: u32, h: u32, kind: Kind, limits: &MipLimits) -> Result<u64, MipError> {
        plan_within(w, h, kind, limits, MAX_ALLOC_BYTES).map(|(_, bytes)| bytes)
    }

    /// The chain with `base` as level 0 and every smaller level made by the box filter of the
    /// module documentation. Every level, `base` included, is checked against `limits.image`, and
    /// the bytes of all levels against `limits.max_chain_bytes`, before any level is made. Each
    /// level's memory is reserved before it is filled. Levels keep the integer-sampling flag of
    /// `base`. No partial chain is returned on an error.
    pub fn try_build(base: Image, limits: &MipLimits) -> Result<MipChain, MipError> {
        let (count, _) = plan_within(base.width(), base.height(), base.kind(), limits, MAX_ALLOC_BYTES)?;
        let mut levels = Vec::new();
        levels.try_reserve_exact(count).map_err(|_| MipError::OutOfMemory(count))?;
        let mut current = base;
        while current.width() > 1 || current.height() > 1 {
            let next = downsample(&current, &limits.image)?;
            levels.push(std::mem::replace(&mut current, next));
        }
        levels.push(current);
        Ok(MipChain { levels })
    }

    /// A chain over levels made elsewhere, such as the levels a file stores. Level `i` must have
    /// the size [`level_sizes`] gives for level 0's size and level 0's kind, and every level must
    /// be present down to 1 × 1. The levels are checked against `limits` as in
    /// [`MipChain::try_build`]; their pixels are not read or copied.
    pub fn from_levels(levels: Vec<Image>, limits: &MipLimits) -> Result<MipChain, MipError> {
        let Some(first) = levels.first() else {
            return Err(MipError::NoLevels);
        };
        let kind = first.kind();
        let (count, _) = plan_within(first.width(), first.height(), kind, limits, MAX_ALLOC_BYTES)?;
        let mut expected = level_sizes(first.width(), first.height());
        for index in 0..count.max(levels.len()) {
            let want = expected.next().map(|(w, h)| (w, h, kind));
            let found = levels.get(index).map(|l| (l.width(), l.height(), l.kind()));
            match (want, found) {
                (Some(want), Some(found)) if want == found => {}
                (Some(want), found) => {
                    return Err(MipError::LevelMismatch { index, expected: want, found: found.unwrap_or((0, 0, kind)) });
                }
                (None, Some(found)) => return Err(MipError::LevelMismatch { index, expected: (0, 0, kind), found }),
                (None, None) => break,
            }
        }
        Ok(MipChain { levels })
    }

    /// Every level, level 0 first. Never empty.
    pub fn levels(&self) -> &[Image] {
        &self.levels
    }

    /// Level `index`, or `None` past the last level.
    pub fn level(&self, index: usize) -> Option<&Image> {
        self.levels.get(index)
    }

    /// The index of the last level, the 1 × 1 one.
    fn last(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }

    /// The LOD of a footprint: `log2` of the larger of the two scaled derivative lengths, plus the
    /// sampler's bias, clamped to its LOD bounds. Errors for a NaN or infinite derivative and for
    /// an invalid sampler. Finite derivatives of any size are scaled in `f64`, so they do not
    /// overflow; a footprint of zero gives `min_lod`.
    pub fn lod(&self, derivatives: UvDerivatives, sampler: &MipSampler) -> Result<f32, MipError> {
        sampler.check()?;
        let UvDerivatives { dx, dy } = derivatives;
        for (name, value) in [("du/dx", dx[0]), ("dv/dx", dx[1]), ("du/dy", dy[0]), ("dv/dy", dy[1])] {
            finite(name, value)?;
        }
        let (w, h) = self.levels.first().map_or((1.0, 1.0), |l| (f64::from(l.width()), f64::from(l.height())));
        let rho_x = (w * f64::from(dx[0])).hypot(h * f64::from(dx[1]));
        let rho_y = (w * f64::from(dy[0])).hypot(h * f64::from(dy[1]));
        Ok(sampler.bound(rho_x.max(rho_y).log2()))
    }

    /// The levels and texel filter for `lod`, a value [`MipChain::lod`] returns or any finite LOD
    /// after bias and bounds. `lod` is clamped to the chain's levels.
    pub fn select(&self, lod: f32, sampler: &MipSampler) -> Result<LevelSelection, MipError> {
        finite("lod", lod)?;
        let filter = if lod <= 0.0 { sampler.mag_filter } else { sampler.min_filter.texel_filter() };
        let last = self.last();
        let d = lod.clamp(0.0, last as f32);
        let (level, next, blend) = match sampler.min_filter.mode() {
            MipMode::BaseOnly => (0, 0, 0.0),
            MipMode::Nearest => {
                // In `f64` the half-level offset is exact for every `f32` LOD.
                let level = ((f64::from(d) + 0.5).ceil() - 1.0).max(0.0) as usize;
                (level.min(last), level.min(last), 0.0)
            }
            MipMode::Linear => {
                let floor = d.floor();
                let level = (floor as usize).min(last);
                (level, (level + 1).min(last), d - floor)
            }
        };
        Ok(LevelSelection { filter, level, next, blend })
    }

    /// A sample at normalized `(u, v)` with the LOD of `derivatives`: [`MipChain::lod`], then
    /// [`MipChain::sample_selected`].
    pub fn sample_grad(&self, u: f32, v: f32, derivatives: UvDerivatives, sampler: &MipSampler) -> Result<[f32; 4], MipError> {
        let lod = self.lod(derivatives, sampler)?;
        Ok(self.sample_selected(u, v, self.select(lod, sampler)?, sampler.tiling))
    }

    /// A sample at normalized `(u, v)` with an explicit LOD in place of derivatives. The sampler's
    /// bias and bounds apply to `base_lod` as they do to a computed one.
    pub fn sample_lod(&self, u: f32, v: f32, base_lod: f32, sampler: &MipSampler) -> Result<[f32; 4], MipError> {
        sampler.check()?;
        finite("lod", base_lod)?;
        let lod = sampler.bound(f64::from(base_lod));
        Ok(self.sample_selected(u, v, self.select(lod, sampler)?, sampler.tiling))
    }

    /// The blend of the selected levels' samples at `(u, v)`, each level wrapped by `tiling` on its
    /// own. `blend` is clamped to `0..=1`, and a NaN `blend` is taken as 0. A level with weight 0 is
    /// not read, so a NaN texel there does not enter the sample. NaN or infinite coordinates give
    /// `[0, 0, 0, 1]`.
    pub fn sample_selected(&self, u: f32, v: f32, selection: LevelSelection, tiling: [bool; 2]) -> [f32; 4] {
        let read = |index: usize| {
            self.levels.get(index).map_or([0.0, 0.0, 0.0, 1.0], |level| match selection.filter {
                TexelFilter::Nearest => level.sample_nearest_axes(u, v, tiling),
                TexelFilter::Linear => level.sample_axes(u, v, tiling),
            })
        };
        let blend = if selection.blend.is_nan() { 0.0 } else { selection.blend.clamp(0.0, 1.0) };
        let mut sum = [0.0f64; 4];
        for (weight, index) in [(1.0 - blend, selection.level), (blend, selection.next)] {
            if weight == 0.0 {
                continue;
            }
            for (s, p) in sum.iter_mut().zip(read(index)) {
                *s += f64::from(weight) * f64::from(p);
            }
        }
        sum.map(f32_of_sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: u32, h: u32, values: &[f32]) -> Image {
        match Image::from_data(w, h, Kind::Gray, values.to_vec(), &Limits::default()) {
            Ok(image) => image,
            Err(e) => panic!("valid {w} x {h} image: {e}"),
        }
    }

    fn chain(base: Image) -> MipChain {
        match MipChain::try_build(base, &MipLimits::default()) {
            Ok(chain) => chain,
            Err(e) => panic!("chain builds: {e}"),
        }
    }

    fn sizes(chain: &MipChain) -> Vec<(u32, u32)> {
        chain.levels().iter().map(|l| (l.width(), l.height())).collect()
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn levels_halve_each_side_down_to_one() {
        assert_eq!(sizes(&chain(Image::filled(1, 1, Kind::Gray, [1.0; 4]))), [(1, 1)]);
        assert_eq!(sizes(&chain(Image::filled(5, 3, Kind::Gray, [1.0; 4]))), [(5, 3), (2, 1), (1, 1)]);
        assert_eq!(sizes(&chain(Image::filled(6, 10, Kind::Color, [1.0; 4]))), [(6, 10), (3, 5), (1, 2), (1, 1)]);
        assert_eq!(sizes(&chain(Image::filled(7, 1, Kind::Gray, [1.0; 4]))), [(7, 1), (3, 1), (1, 1)]);
        let wide = chain(Image::filled(8192, 1, Kind::Gray, [1.0; 4]));
        assert_eq!(wide.levels().len(), 14);
        assert_eq!(sizes(&wide).last(), Some(&(1, 1)));
        // The level count of ARB_texture_non_power_of_two: 1 + floor(log2(max(w, h))).
        for w in 1..=40u32 {
            for h in 1..=40u32 {
                assert_eq!(level_sizes(w, h).count() as u32, 1 + w.max(h).ilog2(), "{w} x {h}");
            }
        }
        assert_eq!(level_sizes(0, 4).count(), 0);
    }

    #[test]
    fn a_two_by_two_level_averages_to_its_mean() {
        let c = chain(gray(2, 2, &[1.0, 3.0, 5.0, 7.0]));
        assert_eq!(c.level(1).map(|l| l.data().to_vec()), Some(vec![4.0]));
        assert!(c.level(2).is_none());
    }

    #[test]
    fn odd_sides_put_the_last_texels_in_the_last_run() {
        // v(x, y) = x + 10y on 5 x 3: column 0 averages x 0..2, column 1 averages x 2..5, and the
        // one row averages y 0..3.
        let values: Vec<f32> = (0..15).map(|i| (i % 5) as f32 + 10.0 * (i / 5) as f32).collect();
        let c = chain(gray(5, 3, &values));
        assert_eq!(c.level(1).map(|l| l.data().to_vec()), Some(vec![10.5, 13.0]));
        assert_eq!(c.level(2).map(|l| l.data().to_vec()), Some(vec![11.75]));
        // 3 x 1: one run of three texels.
        assert_eq!(chain(gray(3, 1, &[1.0, 2.0, 6.0])).level(1).map(|l| l.data().to_vec()), Some(vec![3.0]));
    }

    #[test]
    fn a_constant_image_keeps_its_value_in_every_level() {
        let v = [0.25, 2.0, -1.0, 0.5];
        let c = chain(Image::filled(7, 5, Kind::Color, v));
        assert_eq!(c.levels().len(), 3);
        for level in c.levels() {
            for px in level.data().as_chunks::<4>().0 {
                assert_eq!(*px, v);
            }
        }
    }

    #[test]
    fn a_checkerboard_converges_to_its_mean() {
        for (w, h) in [(8, 8), (16, 4)] {
            let board = Image::gray(w, h, |x, y| ((x + y) % 2) as f32);
            let c = chain(board);
            for level in &c.levels()[1..] {
                assert!(level.data().iter().all(|&v| v == 0.5), "{w} x {h}");
            }
        }
        // Alpha is averaged on its own: a checkerboard alpha under a constant color.
        let rgba = Image::generate(4, 4, Kind::Color, |x, y| [0.2, 0.4, 0.6, ((x + y) % 2) as f32]);
        let c = chain(rgba);
        assert_eq!(c.level(2).map(|l| l.data().to_vec()), Some(vec![0.2, 0.4, 0.6, 0.5]));
    }

    #[test]
    fn non_finite_texels_reach_only_the_levels_over_them() {
        let c = chain(gray(4, 2, &[f32::NAN, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0]));
        let l1 = c.level(1).map(|l| l.data().to_vec()).unwrap_or_default();
        assert!(l1[0].is_nan());
        assert_eq!(l1[1], 2.0);
        assert!(c.level(2).is_some_and(|l| l.data()[0].is_nan()));
    }

    #[test]
    fn levels_keep_the_integer_sampling_flag() {
        let c = chain(Image::filled(4, 4, Kind::Gray, [0.5; 4]).with_integer_sampling(true));
        assert!(c.levels().iter().all(Image::integer_sampling));
    }

    fn sampler(min_filter: MipMinFilter) -> MipSampler {
        MipSampler::new(min_filter, TexelFilter::Linear, [true; 2])
    }

    fn footprint(texels: f32, side: f32) -> UvDerivatives {
        UvDerivatives { dx: [texels / side, 0.0], dy: [0.0, texels / side] }
    }

    #[test]
    fn the_lod_follows_the_footprint() {
        let c = chain(Image::filled(8, 8, Kind::Gray, [1.0; 4]));
        let s = sampler(MipMinFilter::LinearMipmapLinear);
        for (texels, lod) in [(1.0, 0.0), (2.0, 1.0), (4.0, 2.0)] {
            assert_eq!(c.lod(footprint(texels, 8.0), &s), Ok(lod), "footprint {texels}");
        }
        assert!(c.lod(footprint(3.0, 8.0), &s).is_ok_and(|l| near(l, 3.0f32.log2())));
        // 8 x 4: rho_x = 8 x 0.25 = 2 and rho_y = 4 x 0.25 = 1, so the LOD is 1.
        let rect = chain(Image::filled(8, 4, Kind::Gray, [1.0; 4]));
        let d = UvDerivatives { dx: [0.25, 0.0], dy: [0.0, 0.25] };
        assert_eq!(rect.lod(d, &s), Ok(1.0));
        assert_eq!(rect.lod(d, &MipSampler { lod_bias: 0.25, ..s }), Ok(1.25));
        // A diagonal step: hypot(2, 2) gives log2(2 sqrt 2) = 1.5.
        let diag = UvDerivatives { dx: [0.25, 0.25], dy: [0.0, 0.0] };
        assert!(c.lod(diag, &s).is_ok_and(|l| near(l, 1.5)));
        // A footprint of zero takes min_lod.
        assert_eq!(c.lod(UvDerivatives::default(), &s), Ok(0.0));
        assert_eq!(c.lod(UvDerivatives::default(), &MipSampler { min_lod: -2.0, ..s }), Ok(-2.0));
    }

    #[test]
    fn levels_are_selected_by_the_lod() {
        let c = chain(Image::filled(8, 8, Kind::Gray, [1.0; 4]));
        let nearest = sampler(MipMinFilter::NearestMipmapNearest);
        let linear = sampler(MipMinFilter::LinearMipmapLinear);
        let pick = |lod: f32, s: &MipSampler| c.select(lod, s).map(|l| (l.level, l.next));
        assert_eq!(pick(0.0, &nearest), Ok((0, 0)));
        assert_eq!(pick(1.0, &nearest), Ok((1, 1)));
        assert_eq!(pick(2.0, &nearest), Ok((2, 2)));
        assert_eq!(pick(3.0f32.log2(), &nearest), Ok((2, 2)));
        // A tie takes the lower-numbered level: 0.5 selects level 0 and 1.5 selects level 1.
        assert_eq!(pick(0.5, &nearest), Ok((0, 0)));
        assert_eq!(pick(1.5, &nearest), Ok((1, 1)));
        // The value just above a tie takes the higher level, although `d + 0.5` rounds back to the
        // tie in `f32`.
        assert_eq!(pick(f32::from_bits(0x3f00_0001), &nearest), Ok((1, 1)));
        assert_eq!(pick(f32::from_bits(0x3fc0_0001), &nearest), Ok((2, 2)));
        assert_eq!(pick(1.25, &nearest), Ok((1, 1)));
        assert_eq!(c.select(1.25, &linear), Ok(LevelSelection { filter: TexelFilter::Linear, level: 1, next: 2, blend: 0.25 }));
        assert!(c.select(3.0f32.log2(), &linear).is_ok_and(|l| l.level == 1 && l.next == 2 && near(l.blend, 3.0f32.log2() - 1.0)));
        // Without mip levels only level 0 is read.
        assert_eq!(pick(2.5, &sampler(MipMinFilter::Linear)), Ok((0, 0)));
    }

    #[test]
    fn a_footprint_past_the_last_level_clamps_to_it() {
        let c = chain(Image::filled(8, 8, Kind::Gray, [1.0; 4]));
        let s = sampler(MipMinFilter::LinearMipmapLinear);
        let last = LevelSelection { filter: TexelFilter::Linear, level: 3, next: 3, blend: 0.0 };
        assert_eq!(c.select(3.0, &s), Ok(last));
        assert_eq!(c.select(7.5, &s), Ok(last));
        for texels in [1.0e6, 1.0e30, f32::MAX] {
            let lod = c.lod(footprint(texels, 8.0), &s);
            assert!(lod.as_ref().is_ok_and(|l| l.is_finite() && *l > 3.0), "footprint {texels}");
            assert_eq!(lod.and_then(|l| c.select(l, &s)), Ok(last));
        }
        let huge = UvDerivatives { dx: [f32::MAX, f32::MAX], dy: [f32::MIN, f32::MIN] };
        assert!(c.lod(huge, &s).is_ok_and(f32::is_finite));
        assert_eq!(c.lod(footprint(1.0e6, 8.0), &MipSampler { max_lod: 1.0, ..s }), Ok(1.0));
    }

    #[test]
    fn non_finite_footprints_and_lods_are_errors() {
        let c = chain(Image::filled(8, 8, Kind::Gray, [1.0; 4]));
        let s = sampler(MipMinFilter::LinearMipmapLinear);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let d = UvDerivatives { dx: [0.1, bad], dy: [0.0, 0.0] };
            assert!(matches!(c.lod(d, &s), Err(MipError::BadParameter { name: "dv/dx", .. })));
            assert!(matches!(c.sample_grad(0.5, 0.5, d, &s), Err(MipError::BadParameter { .. })));
            assert!(matches!(c.select(bad, &s), Err(MipError::BadParameter { name: "lod", .. })));
            assert!(matches!(c.sample_lod(0.5, 0.5, bad, &s), Err(MipError::BadParameter { name: "lod", .. })));
            for (name, broken) in [
                ("lod_bias", MipSampler { lod_bias: bad, ..s }),
                ("min_lod", MipSampler { min_lod: bad, ..s }),
                ("max_lod", MipSampler { max_lod: bad, ..s }),
            ] {
                assert!(matches!(c.lod(footprint(1.0, 8.0), &broken), Err(MipError::BadParameter { name: n, .. }) if n == name));
                assert!(matches!(c.sample_lod(0.5, 0.5, 0.0, &broken), Err(MipError::BadParameter { name: n, .. }) if n == name));
            }
        }
        let reversed = MipSampler { min_lod: 2.0, max_lod: 1.0, ..s };
        assert_eq!(c.lod(footprint(1.0, 8.0), &reversed), Err(MipError::BadLodBounds { min_lod: 2.0, max_lod: 1.0 }));
    }

    /// An 8 x 8 chain whose levels hold 0, 4, 12 and 100.
    fn stepped() -> MipChain {
        let levels = [(8, 0.0), (4, 4.0), (2, 12.0), (1, 100.0)].map(|(side, v)| Image::filled(side, side, Kind::Gray, [v; 4]));
        match MipChain::from_levels(levels.to_vec(), &MipLimits::default()) {
            Ok(chain) => chain,
            Err(e) => panic!("valid levels: {e}"),
        }
    }

    #[test]
    fn linear_mip_filtering_blends_two_levels() {
        let c = stepped();
        for texel in [TexelFilter::Nearest, TexelFilter::Linear] {
            let min = if texel == TexelFilter::Linear { MipMinFilter::LinearMipmapLinear } else { MipMinFilter::NearestMipmapLinear };
            let s = MipSampler::new(min, texel, [false; 2]);
            // 0.75 x 4 + 0.25 x 12.
            assert_eq!(c.sample_lod(0.3, 0.6, 1.25, &s).map(|p| p[0]), Ok(6.0));
            assert_eq!(c.sample_lod(0.3, 0.6, 1.0, &MipSampler { lod_bias: 0.25, ..s }).map(|p| p[0]), Ok(6.0));
            let nearest = MipSampler { min_filter: MipMinFilter::NearestMipmapNearest, ..s };
            assert_eq!(c.sample_lod(0.3, 0.6, 1.25, &nearest).map(|p| p[0]), Ok(4.0));
        }
        let s = sampler(MipMinFilter::LinearMipmapLinear);
        assert_eq!(c.sample_grad(0.5, 0.5, footprint(2.0, 8.0), &s).map(|p| p[0]), Ok(4.0));
        assert_eq!(c.sample_grad(0.5, 0.5, footprint(1.0e9, 8.0), &s).map(|p| p[0]), Ok(100.0));
        assert_eq!(c.sample_lod(f32::NAN, 0.5, 1.25, &s), Ok([0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_level_with_weight_zero_is_not_read() {
        let levels = vec![Image::filled(2, 2, Kind::Gray, [3.0; 4]), Image::filled(1, 1, Kind::Gray, [f32::NAN; 4])];
        let c = MipChain::from_levels(levels, &MipLimits::default());
        let s = sampler(MipMinFilter::LinearMipmapLinear);
        assert_eq!(c.as_ref().map(|c| c.sample_lod(0.5, 0.5, 0.0, &s)), Ok(Ok([3.0, 3.0, 3.0, 1.0])));
        assert!(c.is_ok_and(|c| c.sample_lod(0.5, 0.5, 0.5, &s).is_ok_and(|p| p[0].is_nan())));
    }

    #[test]
    fn each_level_wraps_each_axis_on_its_own() {
        // Level 1 holds x + 10y on 4 x 4; level 0 is a constant 8 x 8.
        let levels = vec![
            Image::filled(8, 8, Kind::Gray, [0.0; 4]),
            Image::gray(4, 4, |x, y| (x + 10 * y) as f32),
            Image::filled(2, 2, Kind::Gray, [0.0; 4]),
            Image::filled(1, 1, Kind::Gray, [0.0; 4]),
        ];
        let Ok(c) = MipChain::from_levels(levels, &MipLimits::default()) else { panic!("valid levels") };
        let s = |tiling| MipSampler::new(MipMinFilter::NearestMipmapNearest, TexelFilter::Nearest, tiling);
        assert_eq!(c.sample_lod(-0.1, 0.1, 1.0, &s([true, false])).map(|p| p[0]), Ok(3.0));
        assert_eq!(c.sample_lod(-0.1, 0.1, 1.0, &s([false, true])).map(|p| p[0]), Ok(0.0));
        assert_eq!(c.sample_lod(0.1, -0.1, 1.0, &s([false, true])).map(|p| p[0]), Ok(30.0));
        assert_eq!(c.sample_lod(0.1, -0.1, 1.0, &s([true, false])).map(|p| p[0]), Ok(0.0));
    }

    #[test]
    fn magnification_uses_the_mag_filter_on_level_zero() {
        let c = chain(gray(2, 1, &[0.0, 10.0]));
        let s = MipSampler::new(MipMinFilter::LinearMipmapNearest, TexelFilter::Nearest, [false; 2]);
        assert_eq!(c.sample_lod(0.5, 0.5, -1.0, &s).map(|p| p[0]), Ok(10.0));
        assert_eq!(c.sample_lod(0.5, 0.5, 0.0, &s).map(|p| p[0]), Ok(10.0));
        // Above 0 the minification filter's bilinear read applies, still on level 0.
        assert_eq!(c.sample_lod(0.5, 0.5, 0.5, &s).map(|p| p[0]), Ok(5.0));
        assert_eq!(c.select(0.5, &s).map(|l| l.filter), Ok(TexelFilter::Linear));
    }

    #[test]
    fn gltf_codes_map_to_filters() {
        let min = [
            (9728, MipMinFilter::Nearest),
            (9729, MipMinFilter::Linear),
            (9984, MipMinFilter::NearestMipmapNearest),
            (9985, MipMinFilter::LinearMipmapNearest),
            (9986, MipMinFilter::NearestMipmapLinear),
            (9987, MipMinFilter::LinearMipmapLinear),
        ];
        for (code, filter) in min {
            assert_eq!(MipMinFilter::from_gltf(code), Some(filter));
        }
        let texel = [MipMinFilter::Nearest, MipMinFilter::Linear, MipMinFilter::NearestMipmapNearest, MipMinFilter::LinearMipmapNearest]
            .map(MipMinFilter::texel_filter);
        assert_eq!(texel, [TexelFilter::Nearest, TexelFilter::Linear, TexelFilter::Nearest, TexelFilter::Linear]);
        assert_eq!(MipMinFilter::NearestMipmapLinear.texel_filter(), TexelFilter::Nearest);
        assert_eq!(MipMinFilter::LinearMipmapLinear.texel_filter(), TexelFilter::Linear);
        assert_eq!(TexelFilter::from_gltf(9728), Some(TexelFilter::Nearest));
        assert_eq!(TexelFilter::from_gltf(9729), Some(TexelFilter::Linear));
        for code in [0, 9730, 9983, 9988, 10497, u32::MAX] {
            assert_eq!(MipMinFilter::from_gltf(code), None);
            assert_eq!(TexelFilter::from_gltf(code), None);
        }
        assert_eq!(TexelFilter::from_gltf(9984), None);
    }

    #[test]
    fn the_chain_bytes_are_checked_before_building() {
        // 64 x 64 gray: 4 x (4096 + 1024 + 256 + 64 + 16 + 4 + 1) bytes.
        let base = Image::filled(64, 64, Kind::Gray, [1.0; 4]);
        let tight = MipLimits { max_chain_bytes: 21_843, ..MipLimits::default() };
        assert_eq!(MipChain::try_build(base.clone(), &tight), Err(MipError::ChainTooLarge { required: 21_844, limit: 21_843 }));
        let enough = MipLimits { max_chain_bytes: 21_844, ..MipLimits::default() };
        assert!(MipChain::try_build(base.clone(), &enough).is_ok_and(|c| c.levels().len() == 7));
        let narrow = MipLimits { image: Limits { max_side: 32, ..Limits::default() }, ..MipLimits::default() };
        assert_eq!(MipChain::try_build(base, &narrow), Err(MipError::Image(ImageError::BadSize(64, 64))));
        // The default ceiling holds a chain over a full 8192 x 8192 RGBA plane: 16 bytes a pixel
        // over (4^14 - 1) / 3 pixels.
        assert_eq!(MipChain::required_bytes(8192, 8192, Kind::Color, &MipLimits::default()), Ok(1_431_655_760));
        assert_eq!(MipChain::required_bytes(0, 8, Kind::Gray, &MipLimits::default()), Err(MipError::Image(ImageError::BadSize(0, 8))));
        assert_eq!(MipChain::required_bytes(8193, 1, Kind::Gray, &MipLimits::default()), Err(MipError::Image(ImageError::BadSize(8193, 1))));
    }

    #[test]
    fn chain_sizes_do_not_overflow_or_pass_the_target_bound() {
        let open = MipLimits { image: Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX }, max_chain_bytes: u64::MAX };
        assert_eq!(MipChain::required_bytes(u32::MAX, u32::MAX, Kind::Color, &open), Err(MipError::Image(ImageError::TooLarge(u32::MAX, u32::MAX))));
        // isize::MAX on a 32-bit target such as wasm32: the gray chain over 16,385 x 8,192 fits,
        // its color level 0 is over 2 GiB.
        let bound_32 = i32::MAX as u64;
        assert!(plan_within(16_385, 8_192, Kind::Gray, &open, bound_32).is_ok_and(|(count, _)| count == 15));
        assert_eq!(plan_within(16_385, 8_192, Kind::Color, &open, bound_32), Err(MipError::Image(ImageError::TooLarge(16_385, 8_192))));
    }

    #[test]
    fn given_levels_must_match_the_chain() {
        let l = MipLimits::default();
        let level = |w, h, kind| Image::filled(w, h, kind, [0.0; 4]);
        assert_eq!(MipChain::from_levels(Vec::new(), &l), Err(MipError::NoLevels));
        assert!(MipChain::from_levels(vec![level(1, 1, Kind::Gray)], &l).is_ok());
        let wrong_size = vec![level(4, 2, Kind::Gray), level(2, 2, Kind::Gray), level(1, 1, Kind::Gray)];
        assert_eq!(
            MipChain::from_levels(wrong_size, &l),
            Err(MipError::LevelMismatch { index: 1, expected: (2, 1, Kind::Gray), found: (2, 2, Kind::Gray) })
        );
        let wrong_kind = vec![level(2, 2, Kind::Gray), level(1, 1, Kind::Color)];
        assert_eq!(
            MipChain::from_levels(wrong_kind, &l),
            Err(MipError::LevelMismatch { index: 1, expected: (1, 1, Kind::Gray), found: (1, 1, Kind::Color) })
        );
        let missing = vec![level(4, 4, Kind::Gray), level(2, 2, Kind::Gray)];
        assert_eq!(
            MipChain::from_levels(missing, &l),
            Err(MipError::LevelMismatch { index: 2, expected: (1, 1, Kind::Gray), found: (0, 0, Kind::Gray) })
        );
        let extra = vec![level(2, 2, Kind::Gray), level(1, 1, Kind::Gray), level(1, 1, Kind::Gray)];
        assert_eq!(
            MipChain::from_levels(extra, &l),
            Err(MipError::LevelMismatch { index: 2, expected: (0, 0, Kind::Gray), found: (1, 1, Kind::Gray) })
        );
        let tight = MipLimits { max_chain_bytes: 4, ..MipLimits::default() };
        assert_eq!(
            MipChain::from_levels(vec![level(2, 1, Kind::Gray), level(1, 1, Kind::Gray)], &tight),
            Err(MipError::ChainTooLarge { required: 12, limit: 4 })
        );
    }
}
