//! # craft-texture
//!
//! Linear float image planes and texture sampling. An [`Image`] is a grayscale (1 channel) or
//! color (RGBA) plane of `f32`, row-major with the top row first, shared copy-on-write through an
//! `Arc` so a copy costs a reference count. Values are stored as given: HDR values above 1,
//! negative values and non-finite values are kept.
//!
//! Sampling takes normalized `(u, v)` with `(0, 0)` at the upper-left corner and `v` increasing
//! downward, as glTF 2.0 section 3.8.3 defines texture coordinates
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>). The `tile` argument picks the
//! wrap mode of both axes: repeat (glTF `REPEAT`, 10497) or clamp to edge (`CLAMP_TO_EDGE`, 33071).
//! [`Image::sample_nearest`] and [`Image::sample`] (bilinear) use the texel coordinates, wrapping
//! and weights of the Vulkan specification, sections 20.4.1, 20.6.1 and 20.6.3
//! (<https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#textures-texel-filtering>).
//!
//! Sizes are bounded. [`Limits`] caps the side, the pixel count and the bytes, and the `try_`
//! constructors return an [`ImageError`] before allocating anything over them. A plane is also
//! never larger than `isize::MAX` bytes, the most a `Vec` can hold on the target: `isize::MAX` is
//! 2 GiB minus one byte on 32-bit targets such as `wasm32`. The `try_` forms return
//! [`ImageError::TooLarge`] for a plane over that bound, also when the caller raised the limits
//! past it. The constructors without `try_` clamp each side to `1..=MAX_SIDE`, and [`Image::map`]
//! clamps the same way when its result would be over the bound. Every operation that allocates a
//! pixel buffer has a `try_` form that reserves that buffer first and returns
//! [`ImageError::OutOfMemory`] when the reservation fails. The `try_` forms still make small
//! allocations that are not fallible: the `Arc` that holds a new image's buffer, and the boxed
//! resampler of [`Image::try_resize`]. The forms without `try_` allocate like `Vec`. A failed
//! infallible allocation ends the process. Rows are generated on several threads only with the
//! `parallel` feature.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::Arc;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Largest side the clamping constructors make, and the default [`Limits::max_side`].
pub const MAX_SIDE: u32 = 8192;

/// Whether an image holds one channel or four.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Gray,
    Color,
}

impl Kind {
    pub fn channels(self) -> usize {
        match self {
            Kind::Gray => 1,
            Kind::Color => 4,
        }
    }
}

/// Why an image could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ImageError {
    /// A side is zero or longer than [`Limits::max_side`].
    #[error("image size {0}x{1} has a zero side or a side over the limit")]
    BadSize(u32, u32),
    /// The pixel count or the byte count is over [`Limits`], or the byte count is over
    /// `isize::MAX`, the largest allocation the target allows.
    #[error("image size {0}x{1} needs more pixels or bytes than the limits or the target allow")]
    TooLarge(u32, u32),
    /// The float buffer does not hold width × height × channels values.
    #[error("image data holds {found} floats where {expected} are needed")]
    BadDataLength { expected: usize, found: usize },
    /// Memory for the pixels could not be reserved.
    #[error("not enough memory for a {0}x{1} image")]
    OutOfMemory(u32, u32),
}

/// Caps on the size of an image, checked before anything is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Longest side in pixels. Default [`MAX_SIDE`].
    pub max_side: u32,
    /// Most pixels (width × height). Default 2^26, which is 8192 × 8192.
    pub max_pixels: u64,
    /// Most bytes of float data (pixels × channels × 4). Default 1 GiB, an 8192 × 8192 color plane.
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_side: MAX_SIDE, max_pixels: 1 << 26, max_bytes: 1 << 30 }
    }
}

/// The largest allocation in bytes that a `Vec` can make on the target.
const MAX_ALLOC_BYTES: u64 = isize::MAX as u64;

/// Bytes of float data in a `w × h` image of `kind`, or `None` when the count overflows `u64`.
fn checked_byte_len(w: u32, h: u32, kind: Kind) -> Option<u64> {
    u64::from(w).checked_mul(u64::from(h))?.checked_mul(kind.channels() as u64)?.checked_mul(size_of::<f32>() as u64)
}

impl Limits {
    /// The number of floats a `w × h` image of `kind` holds, or the error the limits give for that
    /// size. A size whose bytes are over `isize::MAX` is [`ImageError::TooLarge`] whatever the
    /// limits are. Nothing is allocated.
    pub fn check(&self, w: u32, h: u32, kind: Kind) -> Result<usize, ImageError> {
        self.check_within(w, h, kind, MAX_ALLOC_BYTES)
    }

    /// [`Limits::check`] with `max_alloc_bytes` as the largest allocation the target allows.
    fn check_within(&self, w: u32, h: u32, kind: Kind, max_alloc_bytes: u64) -> Result<usize, ImageError> {
        if w == 0 || h == 0 || w > self.max_side || h > self.max_side {
            return Err(ImageError::BadSize(w, h));
        }
        let too_large = ImageError::TooLarge(w, h);
        let pixels = u64::from(w).checked_mul(u64::from(h)).ok_or_else(|| too_large.clone())?;
        let floats = pixels.checked_mul(kind.channels() as u64).ok_or_else(|| too_large.clone())?;
        let bytes = floats.checked_mul(size_of::<f32>() as u64).ok_or_else(|| too_large.clone())?;
        if pixels > self.max_pixels || bytes > self.max_bytes || bytes > max_alloc_bytes {
            return Err(too_large);
        }
        usize::try_from(floats).map_err(|_| too_large)
    }
}

/// A gray or RGBA plane of linear `f32`. Width and height are at least 1, and the data holds
/// exactly width × height × channels floats.
#[derive(Clone, Debug)]
pub struct Image {
    w: u32,
    h: u32,
    kind: Kind,
    /// Row-major, `kind.channels()` floats per pixel.
    data: Arc<Vec<f32>>,
}

/// Images are equal when their size, kind and floats are equal. Floats compare as `f32` does, so
/// an image holding NaN is unequal to every image, its clones included.
impl PartialEq for Image {
    fn eq(&self, o: &Image) -> bool {
        self.w == o.w && self.h == o.h && self.kind == o.kind && self.data == o.data
    }
}

/// Clamp a requested size into `1..=MAX_SIDE` on each side.
pub fn clamp_size(w: u32, h: u32) -> (u32, u32) {
    (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE))
}

/// The size and float count of an image of `kind` requested at `w × h` without a limit check:
/// `w × h` when its bytes fit in `max_alloc_bytes`, otherwise each side clamped to
/// `1..=MAX_SIDE`, and 1 × 1 when even the clamped size does not fit. Nothing is allocated.
fn bounded_size(w: u32, h: u32, kind: Kind, max_alloc_bytes: u64) -> (u32, u32, usize) {
    let floats = |w: u32, h: u32| {
        let bytes = checked_byte_len(w, h, kind).filter(|&bytes| bytes <= max_alloc_bytes)?;
        usize::try_from(bytes / size_of::<f32>() as u64).ok()
    };
    if let Some(len) = floats(w, h) {
        return (w, h, len);
    }
    let (w, h) = clamp_size(w, h);
    match floats(w, h) {
        Some(len) => (w, h, len),
        None => (1, 1, kind.channels()),
    }
}

/// Integer texel index `i` on an axis of `n >= 1` texels: `i mod n` when repeating, otherwise
/// clamped to `0..=n - 1`.
#[inline]
fn wrap(i: i64, n: u32, tile: bool) -> usize {
    let n = i64::from(n.max(1));
    if tile { i.rem_euclid(n) as usize } else { i.clamp(0, n - 1) as usize }
}

/// The largest `f32` below 1.
const BELOW_ONE: f32 = 1.0 - f32::EPSILON / 2.0;

/// A normalized coordinate brought into a range whose texel indices fit in `i64`: its fractional
/// part, below 1, when repeating, since the repeat period is 1, and `-1..=2` when clamping, where
/// every value outside `0..=1` already selects an edge texel. `None` for NaN and infinity.
#[inline]
fn reduce(t: f32, tile: bool) -> Option<f32> {
    if !t.is_finite() {
        return None;
    }
    Some(if tile { t.rem_euclid(1.0).min(BELOW_ONE) } else { t.clamp(-1.0, 2.0) })
}

/// Fill `data` row by row with `f(x, y)`, keeping the first `c` channels of each result.
fn fill<F: Fn(u32, u32) -> [f32; 4] + Sync>(data: &mut [f32], w: u32, c: usize, f: &F) {
    let row_len = (w as usize).max(1) * c;
    let fill_row = |(y, row): (usize, &mut [f32])| {
        for (x, px) in row.chunks_exact_mut(c).enumerate() {
            let p = f(x as u32, y as u32);
            px.copy_from_slice(&p[..c]);
        }
    };
    #[cfg(feature = "parallel")]
    data.par_chunks_mut(row_len).enumerate().for_each(fill_row);
    #[cfg(not(feature = "parallel"))]
    data.chunks_mut(row_len).enumerate().for_each(fill_row);
}

impl Image {
    /// An image of `w × h`, or of the clamped size [`bounded_size`] gives when its bytes would be
    /// over `isize::MAX`. `f` is called only for pixels inside the size that was made.
    fn build<F: Fn(u32, u32) -> [f32; 4] + Sync>(w: u32, h: u32, kind: Kind, f: F) -> Image {
        let (w, h, len) = bounded_size(w, h, kind, MAX_ALLOC_BYTES);
        let mut data = vec![0.0f32; len];
        fill(&mut data, w, kind.channels(), &f);
        Image { w, h, kind, data: Arc::new(data) }
    }

    /// An image checked against `limits`, with its memory reserved before any pixel is generated.
    fn try_build<F: Fn(u32, u32) -> [f32; 4] + Sync>(w: u32, h: u32, kind: Kind, limits: &Limits, f: F) -> Result<Image, ImageError> {
        let len = limits.check(w, h, kind)?;
        let mut data = Vec::new();
        data.try_reserve_exact(len).map_err(|_| ImageError::OutOfMemory(w, h))?;
        data.resize(len, 0.0);
        fill(&mut data, w, kind.channels(), &f);
        Ok(Image { w, h, kind, data: Arc::new(data) })
    }

    /// A constant image, each side clamped to `1..=MAX_SIDE`.
    pub fn filled(w: u32, h: u32, kind: Kind, v: [f32; 4]) -> Image {
        let (w, h) = clamp_size(w, h);
        Image::build(w, h, kind, |_, _| v)
    }

    /// A constant image, or an error when the size is outside `limits` or memory runs out.
    pub fn try_filled(w: u32, h: u32, kind: Kind, v: [f32; 4], limits: &Limits) -> Result<Image, ImageError> {
        Image::try_build(w, h, kind, limits, |_, _| v)
    }

    /// Generate every pixel with `f(x, y)` (pixel indices), each side clamped to `1..=MAX_SIDE`.
    pub fn generate(w: u32, h: u32, kind: Kind, f: impl Fn(u32, u32) -> [f32; 4] + Sync) -> Image {
        let (w, h) = clamp_size(w, h);
        Image::build(w, h, kind, f)
    }

    /// Generate every pixel with `f(x, y)`, or return an error when the size is outside `limits`
    /// or memory runs out.
    pub fn try_generate(w: u32, h: u32, kind: Kind, limits: &Limits, f: impl Fn(u32, u32) -> [f32; 4] + Sync) -> Result<Image, ImageError> {
        Image::try_build(w, h, kind, limits, f)
    }

    /// Generate from normalized coordinates at the pixel centers, `(x + 0.5) / w` and
    /// `(y + 0.5) / h`, each side clamped to `1..=MAX_SIDE` first.
    pub fn generate_uv(w: u32, h: u32, kind: Kind, f: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Image {
        let (w, h) = clamp_size(w, h);
        let (iw, ih) = (1.0 / w as f32, 1.0 / h as f32);
        Image::build(w, h, kind, |x, y| f((x as f32 + 0.5) * iw, (y as f32 + 0.5) * ih))
    }

    /// [`Image::generate_uv`] for a size checked against `limits`.
    pub fn try_generate_uv(w: u32, h: u32, kind: Kind, limits: &Limits, f: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Result<Image, ImageError> {
        let (iw, ih) = (1.0 / w.max(1) as f32, 1.0 / h.max(1) as f32);
        Image::try_build(w, h, kind, limits, |x, y| f((x as f32 + 0.5) * iw, (y as f32 + 0.5) * ih))
    }

    /// A gray image generated with `f(x, y)`, each side clamped to `1..=MAX_SIDE`.
    pub fn gray(w: u32, h: u32, f: impl Fn(u32, u32) -> f32 + Sync) -> Image {
        Image::generate(w, h, Kind::Gray, |x, y| [f(x, y), 0.0, 0.0, 0.0])
    }

    /// An image over existing row-major data, checked against `limits` and against the length
    /// width × height × channels. The data is not copied.
    pub fn from_data(w: u32, h: u32, kind: Kind, data: impl Into<Arc<Vec<f32>>>, limits: &Limits) -> Result<Image, ImageError> {
        let expected = limits.check(w, h, kind)?;
        let data = data.into();
        if data.len() != expected {
            return Err(ImageError::BadDataLength { expected, found: data.len() });
        }
        Ok(Image { w, h, kind, data })
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The row-major floats, `channels()` per pixel.
    pub fn data(&self) -> &[f32] {
        self.data.as_slice()
    }

    /// The shared buffer behind [`Image::data`].
    pub fn shared_data(&self) -> &Arc<Vec<f32>> {
        &self.data
    }

    pub fn channels(&self) -> usize {
        self.kind.channels()
    }

    pub fn is_gray(&self) -> bool {
        self.kind == Kind::Gray
    }

    /// The pixel at integer coordinates (wrapped or clamped), as RGBA (gray → `[v, v, v, 1]`).
    #[inline]
    pub fn get(&self, x: i64, y: i64, tile: bool) -> [f32; 4] {
        let (x, y) = (wrap(x, self.w, tile), wrap(y, self.h, tile));
        let i = y * self.w as usize + x;
        match self.kind {
            Kind::Gray => {
                let v = self.data.get(i).copied().unwrap_or(0.0);
                [v, v, v, 1.0]
            }
            Kind::Color => {
                let s = self.data.get(i * 4..i * 4 + 4).unwrap_or(&[0.0, 0.0, 0.0, 1.0]);
                [s[0], s[1], s[2], s[3]]
            }
        }
    }

    /// The gray value, or the Rec. 709 luminance of a color image.
    #[inline]
    pub fn get_gray(&self, x: i64, y: i64, tile: bool) -> f32 {
        let p = self.get(x, y, tile);
        match self.kind {
            Kind::Gray => p[0],
            Kind::Color => luminance(p),
        }
    }

    /// Bilinear sample at normalized `(u, v)`; pixel centers sit at `(i + 0.5) / w`. The texel
    /// coordinate is `u × w − 0.5`; its floor and the next index are wrapped or clamped, and the
    /// four texels are weighted `(1 − α)(1 − β)`, `α(1 − β)`, `(1 − α)β` and `αβ` by the fractional
    /// parts `α` and `β`. A texel with weight 0 does not enter the sum, so an infinite neighbor
    /// leaves a sample at a texel center unchanged. NaN or infinite coordinates give `[0, 0, 0, 1]`.
    #[inline]
    pub fn sample(&self, u: f32, v: f32, tile: bool) -> [f32; 4] {
        let (Some(u), Some(v)) = (reduce(u, tile), reduce(v, tile)) else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let taps = [((1.0 - tx) * (1.0 - ty), x0, y0), (tx * (1.0 - ty), x0 + 1, y0), ((1.0 - tx) * ty, x0, y0 + 1), (tx * ty, x0 + 1, y0 + 1)];
        // Summed in f64: the rounded weights can add up to slightly more than 1, which would
        // overflow a finite HDR value near `f32::MAX` in f32. A finite sum is clamped to the f32
        // range; an infinite texel still gives an infinite or NaN sample.
        let mut o = [0.0f64; 4];
        for (weight, x, y) in taps {
            if weight == 0.0 {
                continue;
            }
            let p = self.get(x, y, tile);
            for (o, p) in o.iter_mut().zip(p) {
                *o += f64::from(weight) * f64::from(p);
            }
        }
        o.map(|v| if v.is_finite() { v.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32 } else { v as f32 })
    }

    #[inline]
    pub fn sample_gray(&self, u: f32, v: f32, tile: bool) -> f32 {
        let p = self.sample(u, v, tile);
        match self.kind {
            Kind::Gray => p[0],
            Kind::Color => luminance(p),
        }
    }

    /// Nearest sample at normalized `(u, v)`: the texel `floor(u × w)`, wrapped or clamped. NaN or
    /// infinite coordinates give `[0, 0, 0, 1]`.
    #[inline]
    pub fn sample_nearest(&self, u: f32, v: f32, tile: bool) -> [f32; 4] {
        let (Some(u), Some(v)) = (reduce(u, tile), reduce(v, tile)) else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let (x, y) = ((u * self.w as f32).floor() as i64, (v * self.h as f32).floor() as i64);
        // A repeated coordinate is below 1, so its texel is below the size even where `u × w`
        // rounds up to it.
        let (x, y) = if tile { (x.min(i64::from(self.w) - 1), y.min(i64::from(self.h) - 1)) } else { (x, y) };
        self.get(x, y, tile)
    }

    /// Map every pixel (as RGBA) to a new image of `kind`, same size. When a result of that size
    /// would be over `isize::MAX` bytes, which only an image allowed by raised [`Limits`] can
    /// reach, each side is clamped to `1..=MAX_SIDE` and the result maps the top-left part of this
    /// image. [`Image::try_map`] returns [`ImageError::TooLarge`] instead.
    pub fn map(&self, kind: Kind, f: impl Fn([f32; 4]) -> [f32; 4] + Sync) -> Image {
        Image::build(self.w, self.h, kind, |x, y| f(self.get(i64::from(x), i64::from(y), false)))
    }

    /// [`Image::map`] with the new image checked against `limits` and its memory reserved first.
    pub fn try_map(&self, kind: Kind, limits: &Limits, f: impl Fn([f32; 4]) -> [f32; 4] + Sync) -> Result<Image, ImageError> {
        Image::try_build(self.w, self.h, kind, limits, |x, y| f(self.get(i64::from(x), i64::from(y), false)))
    }

    /// [`Image::to_kind`] with the result checked against `limits`, also when the kind is
    /// unchanged, and the memory of a new image reserved first.
    pub fn try_to_kind(&self, kind: Kind, limits: &Limits) -> Result<Image, ImageError> {
        limits.check(self.w, self.h, kind)?;
        match (self.kind, kind) {
            (Kind::Gray, Kind::Gray) | (Kind::Color, Kind::Color) => Ok(self.clone()),
            (Kind::Color, Kind::Gray) => self.try_map(Kind::Gray, limits, |p| [luminance(p), 0.0, 0.0, 0.0]),
            (Kind::Gray, Kind::Color) => self.try_map(Kind::Color, limits, |p| p),
        }
    }

    /// This image as grayscale (luminance of a color image), same size.
    pub fn to_gray(&self) -> Image {
        match self.kind {
            Kind::Gray => self.clone(),
            Kind::Color => self.map(Kind::Gray, |p| [luminance(p), 0.0, 0.0, 0.0]),
        }
    }

    /// This image as RGBA (gray copied to RGB, alpha 1). A gray image goes through
    /// [`Image::map`], so a result over `isize::MAX` bytes is clamped as `map` describes.
    pub fn to_color(&self) -> Image {
        match self.kind {
            Kind::Color => self.clone(),
            Kind::Gray => self.map(Kind::Color, |p| p),
        }
    }

    /// [`Image::to_gray`] or [`Image::to_color`].
    pub fn to_kind(&self, kind: Kind) -> Image {
        match kind {
            Kind::Gray => self.to_gray(),
            Kind::Color => self.to_color(),
        }
    }

    /// Resample to `w × h`, each side clamped to `1..=MAX_SIDE`: a box average when both sides
    /// shrink by whole factors, otherwise bilinear samples at the new pixel centers.
    pub fn resize(&self, w: u32, h: u32, tile: bool) -> Image {
        let (w, h) = clamp_size(w, h);
        if w == self.w && h == self.h {
            return self.clone();
        }
        Image::build(w, h, self.kind, self.resampler(w, h, tile))
    }

    /// [`Image::resize`] without the clamp: the new size is checked against `limits`, also when it
    /// equals the current size, and its memory reserved first.
    pub fn try_resize(&self, w: u32, h: u32, tile: bool, limits: &Limits) -> Result<Image, ImageError> {
        limits.check(w, h, self.kind)?;
        if w == self.w && h == self.h {
            return Ok(self.clone());
        }
        Image::try_build(w, h, self.kind, limits, self.resampler(w, h, tile))
    }

    /// The pixel function of a resample to `w × h`, both at least 1.
    fn resampler(&self, w: u32, h: u32, tile: bool) -> Box<dyn Fn(u32, u32) -> [f32; 4] + Sync + '_> {
        let (w, h) = (w.max(1), h.max(1));
        if w <= self.w && h <= self.h && self.w.is_multiple_of(w) && self.h.is_multiple_of(h) {
            let (fx, fy) = (i64::from(self.w / w), i64::from(self.h / h));
            let inv = 1.0 / (fx * fy) as f64;
            return Box::new(move |x, y| {
                // Summed in f64, so finite HDR values near `f32::MAX` average without overflow.
                let mut s = [0.0f64; 4];
                for j in 0..fy {
                    for i in 0..fx {
                        let p = self.get(i64::from(x) * fx + i, i64::from(y) * fy + j, tile);
                        for (s, p) in s.iter_mut().zip(p) {
                            *s += f64::from(p);
                        }
                    }
                }
                s.map(|v| (v * inv) as f32)
            });
        }
        let (iw, ih) = (1.0 / w as f32, 1.0 / h as f32);
        Box::new(move |x, y| self.sample((x as f32 + 0.5) * iw, (y as f32 + 0.5) * ih, tile))
    }

    /// Raw float access for gray images (`None` for color).
    pub fn gray_data(&self) -> Option<&[f32]> {
        (self.kind == Kind::Gray).then_some(self.data.as_slice())
    }

    /// 8-bit sRGB-encoded RGBA for display (gray shown as is; color through the sRGB curve).
    /// Values are clamped to `0..=1` first; NaN becomes 0. The result has 4 bytes per pixel, never
    /// more than the float data's own bytes.
    pub fn to_rgba8_display(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.display_len()];
        self.fill_display(&mut out);
        out
    }

    /// [`Image::to_rgba8_display`] with its memory reserved first.
    pub fn try_to_rgba8_display(&self) -> Result<Vec<u8>, ImageError> {
        let len = self.display_len();
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|_| ImageError::OutOfMemory(self.w, self.h))?;
        out.resize(len, 0);
        self.fill_display(&mut out);
        Ok(out)
    }

    /// Bytes of the display copy: 4 per pixel, never more than the float data's own bytes.
    fn display_len(&self) -> usize {
        self.data.len() / self.channels() * 4
    }

    fn fill_display(&self, out: &mut [u8]) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let pixel = |(i, px): (usize, &mut [u8])| match self.kind {
            Kind::Gray => {
                let v = q(self.data.get(i).copied().unwrap_or(0.0));
                px.copy_from_slice(&[v, v, v, 255]);
            }
            Kind::Color => {
                let s = self.data.get(i * 4..i * 4 + 4).unwrap_or(&[0.0, 0.0, 0.0, 1.0]);
                px.copy_from_slice(&[q(linear_to_srgb(s[0])), q(linear_to_srgb(s[1])), q(linear_to_srgb(s[2])), q(s[3])]);
            }
        };
        #[cfg(feature = "parallel")]
        out.par_chunks_mut(4).enumerate().for_each(pixel);
        #[cfg(not(feature = "parallel"))]
        out.chunks_mut(4).enumerate().for_each(pixel);
    }

    /// Mean of each channel (as RGBA), e.g. for tests and auto levels. A NaN or infinite pixel
    /// makes its channel's mean non-finite.
    pub fn mean(&self) -> [f32; 4] {
        let c = self.channels();
        let n = f64::from(self.w) * f64::from(self.h);
        let mut s = [0f64; 4];
        for px in self.data.chunks(c) {
            for (k, v) in px.iter().enumerate() {
                s[k] += f64::from(*v);
            }
        }
        if c == 1 {
            let m = (s[0] / n) as f32;
            return [m, m, m, 1.0];
        }
        s.map(|v| (v / n) as f32)
    }

    /// Minimum and maximum over all channels (alpha excluded). NaN values are skipped; `(0, 0)`
    /// when every value is NaN.
    pub fn range(&self) -> (f32, f32) {
        let c = self.channels();
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for px in self.data.chunks(c) {
            for v in px.iter().take(c.min(3)) {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
        if lo > hi { (0.0, 0.0) } else { (lo, hi) }
    }
}

/// Rec. 709 luminance of a linear RGB(A) pixel.
#[inline]
pub fn luminance(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// The sRGB encoding of a linear value. NaN stays NaN.
#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if v.is_nan() || v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The linear value of an sRGB-encoded value. NaN stays NaN.
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Encode the bitmap name and its requested color space without ambiguity.
/// Plain sRGB names remain readable by simple resource callbacks.
pub fn resource_key(name: &str, linear: bool) -> String {
    if linear {
        format!("\u{1}L{name}")
    } else if name.starts_with('\u{1}') {
        format!("\u{1}S{name}")
    } else {
        name.to_string()
    }
}

/// Split a key made by [`resource_key`] into its name and data-map flag.
pub fn split_resource_key(key: &str) -> (&str, bool) {
    if let Some(name) = key.strip_prefix("\u{1}L") {
        (name, true)
    } else if let Some(name) = key.strip_prefix("\u{1}S") {
        (name, false)
    } else {
        (key, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_survives_huge_coordinates() {
        let img = Image::gray(8, 8, |x, _| x as f32);
        for u in [1.0e30_f32, -1.0e30, f32::MAX, f32::MIN] {
            for tile in [true, false] {
                let p = img.sample(u, u, tile);
                assert!(p.iter().all(|c| c.is_finite()));
            }
        }
    }

    #[test]
    fn resource_keys_round_trip() {
        for name in ["n", "", "\u{1}Lnormal", "\u{1}Sn", "n\u{1}linear"] {
            for linear in [false, true] {
                assert_eq!(split_resource_key(&resource_key(name, linear)), (name, linear));
            }
        }
    }

    #[test]
    fn sample_wraps_and_clamps() {
        let img = Image::gray(4, 1, |x, _| x as f32);
        assert_eq!(img.get(-1, 0, true)[0], 3.0);
        assert_eq!(img.get(-1, 0, false)[0], 0.0);
        // Halfway between the last and the (wrapped) first pixel.
        assert!((img.sample(1.0, 0.5, true)[0] - 1.5).abs() < 1e-5);
    }

    #[test]
    fn resize_box_and_bilinear() {
        let img = Image::gray(4, 4, |x, _| if x < 2 { 0.0 } else { 1.0 });
        let small = img.resize(2, 2, true);
        assert_eq!(small.data.as_slice(), &[0.0, 1.0, 0.0, 1.0]);
        let big = small.resize(8, 8, false);
        assert_eq!(big.w, 8);
        assert!(big.data[0] < big.data[7]);
    }

    #[test]
    fn hostile_sizes_are_clamped() {
        let img = Image::filled(0, u32::MAX, Kind::Gray, [0.5; 4]);
        assert_eq!((img.w, img.h), (1, MAX_SIDE));
        assert_eq!(img.sample(f32::NAN, 0.0, true), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn conversions() {
        let c = Image::filled(2, 2, Kind::Color, [1.0, 1.0, 1.0, 1.0]);
        assert!((c.to_gray().data[0] - 1.0).abs() < 1e-5);
        assert_eq!(c.to_rgba8_display()[0], 255);
    }

    /// Texel values 0, 5 and 1: the blend of the last and first texels (0.5) differs from the
    /// middle texel (5), so a wrong repeat period shows.
    fn stripe() -> Image {
        Image::gray(3, 1, |x, _| [0.0, 5.0, 1.0][x as usize])
    }

    #[test]
    fn repeat_keeps_its_period_at_large_coordinates() {
        let img = stripe();
        // Every whole number repeats u = 0, which falls halfway between the last and first texels.
        let at_zero = img.sample(0.0, 0.5, true)[0];
        assert!((at_zero - 0.5).abs() < 1e-6);
        for u in [1.0e9_f32, -1.0e9, 4.0e9, 3.0e12] {
            assert_eq!(img.sample(u, 0.5, true)[0], at_zero, "bilinear u = {u}");
            assert_eq!(img.sample_nearest(u, 0.5, true)[0], 0.0, "nearest u = {u}");
        }
        assert_eq!(img.sample_nearest(1.0e19, 0.5, true)[0], 0.0);
        assert_eq!(img.sample_nearest(-1.0e19, 0.5, true)[0], 0.0);
    }

    #[test]
    fn clamp_selects_the_edges_at_large_coordinates() {
        let img = stripe();
        for u in [2.0_f32, 1.0e9, 1.0e19, f32::MAX] {
            assert_eq!(img.sample(u, 0.5, false)[0], 1.0);
            assert_eq!(img.sample_nearest(u, 0.5, false)[0], 1.0);
        }
        for u in [-1.0_f32, -1.0e9, -1.0e19, f32::MIN] {
            assert_eq!(img.sample(u, 0.5, false)[0], 0.0);
            assert_eq!(img.sample_nearest(u, 0.5, false)[0], 0.0);
        }
    }

    #[test]
    fn nearest_follows_the_texel_floor() {
        let img = Image::gray(4, 1, |x, _| x as f32);
        // floor(u × 4): u = 1.0 repeats to texel 0 and clamps to texel 3.
        assert_eq!(img.sample_nearest(0.24, 0.5, true)[0], 0.0);
        assert_eq!(img.sample_nearest(0.25, 0.5, true)[0], 1.0);
        assert_eq!(img.sample_nearest(1.0, 0.5, true)[0], 0.0);
        assert_eq!(img.sample_nearest(1.0, 0.5, false)[0], 3.0);
        assert_eq!(img.sample_nearest(-0.01, 0.5, true)[0], 3.0);
    }

    #[test]
    fn bilinear_weights_follow_the_fractional_texel_coordinate() {
        // u × 4 − 0.5 = 1.25: texel 1 weighted 0.75 and texel 2 weighted 0.25.
        let img = Image::gray(4, 1, |x, _| [0.0, 2.0, 6.0, 0.0][x as usize]);
        assert!((img.sample(1.75 / 4.0, 0.5, false)[0] - 3.0).abs() < 1e-6);
        // Two axes: the four texels of a 2 × 2 image at the image center weigh a quarter each.
        let sq = Image::gray(2, 2, |x, y| [1.0, 3.0, 5.0, 11.0][(y * 2 + x) as usize]);
        assert!((sq.sample(0.5, 0.5, false)[0] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn small_negative_coordinates_repeat_to_the_last_texel() {
        let img = Image::gray(4, 1, |x, _| x as f32);
        for u in [-1.0e-8_f32, -1.0e-30, -f32::MIN_POSITIVE] {
            assert_eq!(img.sample_nearest(u, 0.5, true)[0], 3.0, "u = {u}");
            assert!((img.sample(u, 0.5, true)[0] - 1.5).abs() < 1e-5, "u = {u}");
        }
        let wide = Image::gray(MAX_SIDE, 1, |x, _| x as f32);
        assert_eq!(wide.sample_nearest(-1.0e-8, 0.5, true)[0], (MAX_SIDE - 1) as f32);
    }

    #[test]
    fn bilinear_ignores_zero_weight_texels_and_does_not_overflow() {
        let l = Limits::default();
        let Ok(img) = Image::from_data(2, 1, Kind::Gray, vec![1.0, f32::INFINITY], &l) else { panic!("valid 2 x 1 image") };
        // The center of texel 0 gives texel 0, though its neighbor is infinite.
        assert_eq!(img.sample(0.25, 0.5, false)[0], 1.0);
        let Ok(hdr) = Image::from_data(2, 1, Kind::Gray, vec![f32::MAX, -f32::MAX], &l) else { panic!("valid 2 x 1 image") };
        assert_eq!(hdr.sample(0.5, 0.5, false)[0], 0.0);
        // Rounded weights can sum past 1; a constant image of the largest float stays finite.
        let max = Image::filled(2, 2, Kind::Gray, [f32::MAX; 4]);
        let t = 0.5 + 2.0_f32.powi(-24);
        for tile in [false, true] {
            assert!(max.sample(0.250_012_5, t, tile)[0].is_finite(), "tile = {tile}");
            assert_eq!(max.sample(t, t, tile)[0], f32::MAX, "tile = {tile}");
        }
    }

    #[test]
    fn equality_does_not_depend_on_buffer_sharing() {
        let img = Image::filled(2, 1, Kind::Gray, [f32::NAN; 4]);
        let rebuilt = Image::filled(2, 1, Kind::Gray, [f32::NAN; 4]);
        // NaN pixels compare unequal as floats do, whether or not the buffer is shared.
        assert_ne!(img, rebuilt);
        assert_ne!(img, img.clone());
        let finite = Image::filled(2, 1, Kind::Gray, [0.5; 4]);
        assert_eq!(finite, finite.clone());
        assert_eq!(finite, Image::filled(2, 1, Kind::Gray, [0.5; 4]));
    }

    #[test]
    fn box_shrink_averages_large_hdr_values() {
        let img = Image::filled(2, 2, Kind::Color, [f32::MAX, -f32::MAX, 1.0, 1.0]);
        assert_eq!(img.resize(1, 1, false).data(), &[f32::MAX, -f32::MAX, 1.0, 1.0]);
    }

    #[test]
    fn fallible_operations_check_the_limits() {
        let img = Image::gray(4, 4, |x, y| (x + 4 * y) as f32);
        let l = Limits::default();
        assert_eq!(img.try_resize(2, 2, true, &l), Ok(img.resize(2, 2, true)));
        assert_eq!(img.try_resize(7, 3, false, &l), Ok(img.resize(7, 3, false)));
        assert_eq!(img.try_resize(0, 2, false, &l), Err(ImageError::BadSize(0, 2)));
        let small = Limits { max_bytes: 100, ..Limits::default() };
        assert_eq!(img.try_resize(8, 8, false, &small), Err(ImageError::TooLarge(8, 8)));
        assert_eq!(img.try_to_kind(Kind::Color, &small), Err(ImageError::TooLarge(4, 4)));
        assert_eq!(img.try_to_kind(Kind::Color, &l), Ok(img.to_color()));
        assert_eq!(img.try_to_kind(Kind::Gray, &small), Ok(img.clone()));
        assert_eq!(img.try_map(Kind::Gray, &l, |p| [p[0] * 2.0; 4]), Ok(img.map(Kind::Gray, |p| [p[0] * 2.0; 4])));
        assert_eq!(img.try_to_rgba8_display(), Ok(img.to_rgba8_display()));
    }

    #[test]
    fn unchanged_results_are_checked_against_the_limits() {
        let img = Image::gray(4, 4, |x, y| (x + 4 * y) as f32);
        let tiny = Limits { max_pixels: 8, ..Limits::default() };
        assert_eq!(img.try_resize(4, 4, false, &tiny), Err(ImageError::TooLarge(4, 4)));
        assert_eq!(img.try_to_kind(Kind::Gray, &tiny), Err(ImageError::TooLarge(4, 4)));
        let narrow = Limits { max_side: 3, ..Limits::default() };
        assert_eq!(img.try_resize(4, 4, false, &narrow), Err(ImageError::BadSize(4, 4)));
    }

    #[test]
    fn generate_uv_spans_the_clamped_size() {
        let img = Image::generate_uv(MAX_SIDE + 100, 1, Kind::Gray, |u, _| [u, 0.0, 0.0, 0.0]);
        assert_eq!(img.w, MAX_SIDE);
        let last = img.data[MAX_SIDE as usize - 1];
        assert!((last - (MAX_SIDE as f32 - 0.5) / MAX_SIDE as f32).abs() < 1e-6, "{last}");
    }

    #[test]
    fn checked_sizes_reject_zero_and_long_sides() {
        let l = Limits::default();
        assert_eq!(Image::try_filled(0, 4, Kind::Gray, [0.0; 4], &l), Err(ImageError::BadSize(0, 4)));
        assert_eq!(Image::try_filled(4, 0, Kind::Color, [0.0; 4], &l), Err(ImageError::BadSize(4, 0)));
        assert_eq!(Image::from_data(0, 1, Kind::Gray, Vec::new(), &l), Err(ImageError::BadSize(0, 1)));
        assert_eq!(Image::try_generate(MAX_SIDE + 1, 1, Kind::Gray, &l, |_, _| [0.0; 4]), Err(ImageError::BadSize(MAX_SIDE + 1, 1)));
        assert!(Image::try_filled(3, 2, Kind::Color, [0.25; 4], &l).is_ok_and(|i| i.data().len() == 24 && i.width() == 3));
    }

    #[test]
    fn checked_sizes_respect_the_pixel_and_byte_limits() {
        let l = Limits { max_bytes: 1000, ..Limits::default() };
        assert_eq!(l.check(15, 16, Kind::Gray), Ok(240));
        assert_eq!(l.check(16, 16, Kind::Gray), Err(ImageError::TooLarge(16, 16)));
        assert_eq!(Image::try_filled(8, 8, Kind::Color, [0.0; 4], &l), Err(ImageError::TooLarge(8, 8)));
        let l = Limits { max_pixels: 100, ..Limits::default() };
        assert_eq!(l.check(10, 10, Kind::Color), Ok(400));
        assert_eq!(l.check(10, 11, Kind::Gray), Err(ImageError::TooLarge(10, 11)));
        // The default limits accept a full 8192 × 8192 color plane and nothing longer.
        assert_eq!(Limits::default().check(MAX_SIDE, MAX_SIDE, Kind::Color), Ok(1 << 28));
    }

    #[test]
    fn checked_sizes_do_not_overflow() {
        let open = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX };
        assert_eq!(open.check(u32::MAX, u32::MAX, Kind::Color), Err(ImageError::TooLarge(u32::MAX, u32::MAX)));
        assert_eq!(open.check(u32::MAX, u32::MAX, Kind::Gray), Err(ImageError::TooLarge(u32::MAX, u32::MAX)));
    }

    /// `isize::MAX` on a 32-bit target such as `wasm32`.
    const BOUND_32: u64 = i32::MAX as u64;

    #[test]
    fn checked_sizes_respect_target_isize_bound() {
        let open = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX };
        // 16,385 × 8,192 gray is 512 MiB and 16 KiB; its color form is over 2 GiB.
        assert_eq!(open.check_within(16_385, 8_192, Kind::Gray, BOUND_32), Ok(16_385 * 8_192));
        assert_eq!(open.check_within(16_385, 8_192, Kind::Color, BOUND_32), Err(ImageError::TooLarge(16_385, 8_192)));
        assert_eq!(open.check_within(MAX_SIDE, MAX_SIDE, Kind::Color, BOUND_32), Ok(1 << 28));
    }

    #[test]
    fn infallible_expansion_clamps_at_target_bound() {
        assert_eq!(bounded_size(16_385, 8_192, Kind::Gray, BOUND_32), (16_385, 8_192, 16_385 * 8_192));
        assert_eq!(bounded_size(16_385, 8_192, Kind::Color, BOUND_32), (MAX_SIDE, MAX_SIDE, 1 << 28));
        assert_eq!(bounded_size(MAX_SIDE, MAX_SIDE, Kind::Color, BOUND_32), (MAX_SIDE, MAX_SIDE, 1 << 28));
        assert_eq!(bounded_size(3, 2, Kind::Color, BOUND_32), (3, 2, 24));
        // A bound below even the clamped size gives one pixel.
        assert_eq!(bounded_size(16_385, 8_192, Kind::Color, 64), (1, 1, 4));
        assert_eq!(bounded_size(u32::MAX, u32::MAX, Kind::Gray, MAX_ALLOC_BYTES), (MAX_SIDE, MAX_SIDE, 1 << 26));
    }

    #[test]
    fn checked_byte_len_rejects_arithmetic_overflow() {
        assert_eq!(checked_byte_len(u32::MAX, u32::MAX, Kind::Color), None);
        assert_eq!(checked_byte_len(u32::MAX, u32::MAX, Kind::Gray), None);
        assert_eq!(checked_byte_len(MAX_SIDE, MAX_SIDE, Kind::Color), Some(1 << 30));
    }

    #[test]
    fn from_data_checks_the_length() {
        let l = Limits::default();
        assert_eq!(Image::from_data(2, 2, Kind::Color, vec![0.0], &l), Err(ImageError::BadDataLength { expected: 16, found: 1 }));
        assert_eq!(Image::from_data(2, 2, Kind::Color, vec![0.0; 17], &l), Err(ImageError::BadDataLength { expected: 16, found: 17 }));
        let img = Image::from_data(2, 1, Kind::Gray, vec![-2.0, 3.5], &l);
        assert!(img.is_ok_and(|i| i.get(0, 0, false)[0] == -2.0 && i.get(1, 0, false)[0] == 3.5));
    }

    #[test]
    fn non_finite_pixels_have_defined_results() {
        let l = Limits::default();
        let img = Image::from_data(1, 1, Kind::Color, vec![f32::NAN, f32::INFINITY, -1.0, 2.0], &l);
        let Ok(img) = img else { panic!("valid 1 x 1 image") };
        assert_eq!(img.to_rgba8_display(), vec![0, 255, 0, 255]);
        let g = Image::from_data(3, 1, Kind::Gray, vec![f32::NAN, -4.0, 8.0], &l);
        let Ok(g) = g else { panic!("valid 3 x 1 image") };
        assert_eq!(g.range(), (-4.0, 8.0));
        assert!(g.mean()[0].is_nan());
    }

    #[test]
    fn srgb_encoding_keeps_nan() {
        assert!(linear_to_srgb(f32::NAN).is_nan());
        assert!(srgb_to_linear(f32::NAN).is_nan());
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(linear_to_srgb(0.5)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn whole_factor_shrink_stays_exact_with_custom_limits() {
        let l = Limits { max_side: 16, ..Limits::default() };
        let img = Image::try_generate(4, 2, Kind::Color, &l, |x, y| [x as f32, y as f32, 0.0, 1.0]);
        let Ok(img) = img else { panic!("valid 4 x 2 image") };
        let half = img.resize(2, 1, false);
        assert_eq!(half.data(), &[0.5, 0.5, 0.0, 1.0, 2.5, 0.5, 0.0, 1.0]);
        assert_eq!(img.to_gray().width(), 4);
    }
}
