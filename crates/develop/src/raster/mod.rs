//! Image buffers and basic raster operations for LightCraft.
//!
//! - [`Image<T>`]: interleaved, row-major pixels (`Rgb32f = Image<[f32; 3]>`, `Rgba8 = Image<[u8; 4]>`,
//!   `Plane = Image<f32>`).
//! - Resampling ([`resample`]): separable Lanczos-3 / Mitchell / box, rayon-parallel; point sampling for warps
//!   ([`sample`]: Lanczos-3 held to its neighbours' range).
//! - Blur ([`blur`]): separable Gaussian via repeated box filters (O(1) per pixel in the radius).
//! - Morphology ([`morph`]): square min / max filters (van Herk / Gil-Werman, O(1) per pixel in the radius).
//! - [`Histogram`] of display-encoded RGB + luminance.

pub mod blur;
pub mod histogram;
pub mod morph;
pub mod resample;
pub mod sample;

pub use histogram::{HdrBins, Histogram};

/// Run `f` over row chunks, in parallel on native targets.
pub fn par_rows<T: Send>(data: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    if row_len == 0 {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        data.par_chunks_mut(row_len).enumerate().for_each(|(y, row)| f(y, row));
    }
    #[cfg(target_arch = "wasm32")]
    {
        data.chunks_mut(row_len).enumerate().for_each(|(y, row)| f(y, row));
    }
}

/// Run `a` and `b` potentially in parallel (sequentially on wasm32). For
/// independent passes that each scale poorly on their own (small images, short rows).
pub fn par_join<A: Send, B: Send>(a: impl FnOnce() -> A + Send, b: impl FnOnce() -> B + Send) -> (A, B) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::join(a, b)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (a(), b())
    }
}

/// Interleaved row-major image.
#[derive(Clone, Debug, PartialEq)]
pub struct Image<T> {
    pub width: usize,
    pub height: usize,
    pub data: Vec<T>,
}

pub type Rgb32f = Image<[f32; 3]>;
pub type Rgba8 = Image<[u8; 4]>;
pub type Plane = Image<f32>;

impl<T: Copy + Default> Image<T> {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![T::default(); width * height] }
    }
    pub fn filled(width: usize, height: usize, v: T) -> Self {
        Self { width, height, data: vec![v; width * height] }
    }
    pub fn from_fn(width: usize, height: usize, f: impl Fn(usize, usize) -> T) -> Self {
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                data.push(f(x, y));
            }
        }
        Self { width, height, data }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> T {
        self.data[y * self.width + x]
    }
    /// Clamped access (edge extension).
    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> T {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.data[y * self.width + x]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: T) {
        self.data[y * self.width + x] = v;
    }
    pub fn row(&self, y: usize) -> &[T] {
        &self.data[y * self.width..(y + 1) * self.width]
    }
    pub fn row_mut(&mut self, y: usize) -> &mut [T] {
        &mut self.data[y * self.width..(y + 1) * self.width]
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    /// The image mirrored about its main diagonal (`width` and `height` swap; row-parallel). A
    /// buffer shorter than its size reads as `T::default()` where it runs out.
    pub fn transpose(&self) -> Self
    where
        T: Send + Sync,
    {
        let (w, h) = (self.width, self.height);
        let mut t = Image::<T>::new(h, w);
        par_rows(&mut t.data, h, |x, row| {
            for (y, v) in row.iter_mut().enumerate() {
                *v = self.data.get(y * w + x).copied().unwrap_or_default();
            }
        });
        t
    }
    /// [`Self::map`] into the same buffer (no second image in memory).
    pub fn map_in_place(&mut self, f: impl Fn(T) -> T + Sync + Send)
    where
        T: Send,
    {
        let w = self.width;
        par_rows(&mut self.data, w, |_, row| {
            for p in row.iter_mut() {
                *p = f(*p);
            }
        });
    }

    pub fn map<U: Copy + Default + Send>(&self, f: impl Fn(T) -> U + Sync + Send) -> Image<U>
    where
        T: Sync,
    {
        let mut out = Image::<U>::new(self.width, self.height);
        let w = self.width;
        par_rows(&mut out.data, w, |y, row| {
            let src = &self.data[y * w..(y + 1) * w];
            for (o, s) in row.iter_mut().zip(src) {
                *o = f(*s);
            }
        });
        out
    }
    /// Per-pixel combination of two images of the same size (row-parallel).
    pub fn zip_map<U: Copy + Default + Sync, V: Copy + Default + Send>(&self, other: &Image<U>, f: impl Fn(T, U) -> V + Sync + Send) -> Image<V>
    where
        T: Sync,
    {
        assert_eq!((self.width, self.height), (other.width, other.height), "zip_map: size mismatch");
        let mut out = Image::<V>::new(self.width, self.height);
        let w = self.width;
        par_rows(&mut out.data, w, |y, row| {
            let (a, b) = (&self.data[y * w..(y + 1) * w], &other.data[y * w..(y + 1) * w]);
            for ((o, s), t) in row.iter_mut().zip(a).zip(b) {
                *o = f(*s, *t);
            }
        });
        out
    }
    /// Crop to `[x0, x0+w) × [y0, y0+h)` (clamped to the image).
    pub fn crop(&self, x0: usize, y0: usize, w: usize, h: usize) -> Self {
        let x0 = x0.min(self.width);
        let y0 = y0.min(self.height);
        let w = w.min(self.width - x0);
        let h = h.min(self.height - y0);
        let mut data = Vec::with_capacity(w * h);
        for y in y0..y0 + h {
            data.extend_from_slice(&self.data[y * self.width + x0..y * self.width + x0 + w]);
        }
        Self { width: w, height: h, data }
    }
    /// [`Self::crop`] in place: rows move to the front of the same buffer (no second image).
    pub fn into_crop(mut self, x0: usize, y0: usize, w: usize, h: usize) -> Self {
        let x0 = x0.min(self.width);
        let y0 = y0.min(self.height);
        let w = w.min(self.width - x0);
        let h = h.min(self.height - y0);
        for (i, y) in (y0..y0 + h).enumerate() {
            let src = y * self.width + x0;
            self.data.copy_within(src..src + w, i * w);
        }
        self.data.truncate(w * h);
        Self { width: w, height: h, data: self.data }
    }
    /// Rotate 90° clockwise.
    pub fn rotate_cw(&self) -> Self {
        let (w, h) = (self.width, self.height);
        Image::from_fn(h, w, |x, y| self.get(y, h - 1 - x))
    }
    pub fn rotate_180(&self) -> Self {
        let mut d = self.data.clone();
        d.reverse();
        Self { width: self.width, height: self.height, data: d }
    }
    pub fn flip_h(&self) -> Self {
        let mut out = self.clone();
        for y in 0..self.height {
            out.row_mut(y).reverse();
        }
        out
    }
    /// Apply an EXIF orientation (stored → displayed).
    /// [`Self::oriented`] without a copy when `o` is the identity.
    pub fn into_oriented(self, o: craft_geom::Orientation) -> Self {
        if o == craft_geom::Orientation::Normal { self } else { self.oriented(o) }
    }

    pub fn oriented(&self, o: craft_geom::Orientation) -> Self {
        let (flip, turns) = o.to_parts();
        let mut img = if flip { self.flip_h() } else { self.clone() };
        match turns {
            1 => img = img.rotate_cw(),
            2 => img = img.rotate_180(),
            3 => img = img.rotate_180().rotate_cw(),
            _ => {}
        }
        img
    }
}

// Keep the sqrt-spaced 14-bit lookup from LightCraft's raster conversion. The uniform
// craft-color byte lookup makes different quantization choices for some near-black inputs.
#[inline]
fn encode_raster_srgb8(v: f32) -> u8 {
    const LAST: usize = (1 << 14) - 1;
    static LUT: std::sync::OnceLock<[u8; LAST + 1]> = std::sync::OnceLock::new();
    let lut = LUT.get_or_init(|| {
        std::array::from_fn(|i| {
            let t = i as f32 / LAST as f32;
            (craft_color::linear_to_srgb(t * t) * 255.0 + 0.5) as u8
        })
    });
    let i = (v.clamp(0.0, 1.0).sqrt() * LAST as f32 + 0.5) as usize;
    lut[i.min(LAST)]
}

impl Rgb32f {
    /// Bilinear sample at continuous pixel coordinates (pixel centres at +0.5).
    #[inline]
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 3] {
        // a pixel past the edge reads the edge pixel, so a coordinate held a pixel outside gives
        // the same sample; an infinite one (a crop of no size's inverse mapping) then reads the
        // edge too, instead of a NaN weight and a neighbour index past isize::MAX
        let fx = x.clamp(-1.0, self.width as f32 + 1.0) - 0.5;
        let fy = y.clamp(-1.0, self.height as f32 + 1.0) - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.get_clamped(x0, y0);
        let b = self.get_clamped(x0 + 1, y0);
        let c = self.get_clamped(x0, y0 + 1);
        let d = self.get_clamped(x0 + 1, y0 + 1);
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bot = c[i] + (d[i] - c[i]) * tx;
            top + (bot - top) * ty
        })
    }

    pub fn luminance(&self) -> Plane {
        self.map(crate::colorops::luminance_2020)
    }

    /// Encode to 8-bit sRGB from linear sRGB values (no gamut conversion).
    pub fn to_srgb8(&self) -> Rgba8 {
        self.map(|p| [encode_raster_srgb8(p[0]), encode_raster_srgb8(p[1]), encode_raster_srgb8(p[2]), 255])
    }
}

impl Rgba8 {
    /// Raw bytes (RGBA, row-major).
    pub fn as_bytes(&self) -> Vec<u8> {
        self.data.iter().flat_map(|p| *p).collect()
    }
    pub fn from_bytes(width: usize, height: usize, bytes: &[u8]) -> Option<Self> {
        if bytes.len() != width * height * 4 {
            return None;
        }
        Some(Self { width, height, data: bytes.as_chunks::<4>().0.iter().map(|c| [c[0], c[1], c[2], c[3]]).collect() })
    }

    /// Build an image from exactly `width * height` RGBA pixels.
    ///
    /// This copies each pixel once, straight into the image's own buffer, so a caller that already
    /// holds a pixel array (an egui `ColorImage`, say) does not need a second full byte vector.
    /// `None` for a zero or overflowing size, or a pixel count that is not exactly `width * height`.
    pub fn from_pixels(width: usize, height: usize, pixels: impl IntoIterator<Item = [u8; 4]>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = width.checked_mul(height)?;
        let mut data = Vec::new();
        data.try_reserve_exact(expected).ok()?;
        for (i, pixel) in pixels.into_iter().enumerate() {
            if i == expected {
                return None;
            }
            data.push(pixel);
        }
        (data.len() == expected).then_some(Self { width, height, data })
    }
    /// Linear Rec.709/sRGB primaries float image from 8-bit sRGB.
    pub fn to_linear(&self) -> Rgb32f {
        let lut = craft_color::srgb_u8_to_linear_table();
        self.map(|p| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_geom::Orientation;

    /// A sample at an infinite coordinate (the inverse mapping of a crop of no size) cast to
    /// isize::MAX, and its right / lower neighbour overflowed (a panic where overflow is checked):
    /// the neighbours saturate and the edge pixel comes back.
    #[test]
    fn bilinear_samples_at_infinite_coordinates_read_the_edge() {
        let img = Rgb32f::from_fn(3, 2, |x, y| [x as f32, y as f32, 1.0]);
        assert_eq!(img.sample_bilinear(f32::INFINITY, f32::INFINITY), [2.0, 1.0, 1.0]);
        assert_eq!(img.sample_bilinear(f32::MAX, 0.0), [2.0, 0.0, 1.0]);
        assert_eq!(img.sample_bilinear(f32::NEG_INFINITY, f32::NEG_INFINITY), [0.0, 0.0, 1.0]);
        assert_eq!(img.sample_bilinear(1.5, 0.5), [1.0, 0.0, 1.0], "an ordinary sample is as before");
    }

    #[test]
    fn transpose_swaps_the_axes() {
        let p = Plane::from_fn(5, 3, |x, y| (y * 10 + x) as f32);
        let t = p.transpose();
        assert_eq!((t.width, t.height), (3, 5));
        assert!((0..3).all(|y| (0..5).all(|x| t.get(y, x) == p.get(x, y))));
        assert_eq!(t.transpose(), p);
        // a buffer shorter than its size reads as zero where it runs out, never a panic
        let short = Plane { width: 3, height: 2, data: vec![1.0; 4] };
        assert_eq!(short.transpose().data, [1.0, 1.0, 1.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn crop_and_rotate() {
        let img = Image::<u32>::from_fn(4, 3, |x, y| (y * 10 + x) as u32);
        let c = img.crop(1, 1, 2, 2);
        assert_eq!(c.data, vec![11, 12, 21, 22]);
        // the in-place variants equal the copying ones
        for (x, y, w, h) in [(1, 1, 2, 2), (0, 0, 4, 3), (2, 0, 9, 9), (0, 2, 4, 1), (3, 2, 1, 1)] {
            assert_eq!(img.clone().into_crop(x, y, w, h), img.crop(x, y, w, h));
        }
        for v in 1..=8 {
            let o = Orientation::from_exif(v);
            assert_eq!(img.clone().into_oriented(o), img.oriented(o));
        }
        let mut m = img.clone();
        m.map_in_place(|p| p * 2 + 1);
        assert_eq!(m, img.map(|p| p * 2 + 1));
        let r = img.rotate_cw();
        assert_eq!((r.width, r.height), (3, 4));
        // top-left of rotated = bottom-left of original
        assert_eq!(r.get(0, 0), 20);
        assert_eq!(r.get(2, 0), 0);
        assert_eq!(img.rotate_cw().rotate_cw().rotate_cw().rotate_cw(), img);
        assert_eq!(img.rotate_cw().rotate_cw(), img.rotate_180());
    }

    #[test]
    fn orientation_matches_geom_mapping() {
        let img = Image::<u32>::from_fn(5, 3, |x, y| (y * 10 + x) as u32);
        for v in 1..=8 {
            let o = Orientation::from_exif(v);
            let out = img.oriented(o);
            for (sx, sy) in [(0usize, 0usize), (4, 0), (2, 1), (0, 2)] {
                let (dx, dy) = o.map(sx as f64 + 0.5, sy as f64 + 0.5, 5.0, 3.0);
                assert_eq!(out.get(dx as usize, dy as usize), img.get(sx, sy), "orientation {v}");
            }
        }
    }

    #[test]
    fn bilinear_at_centres_is_exact() {
        let img = Rgb32f::from_fn(3, 3, |x, y| [x as f32, y as f32, 1.0]);
        assert_eq!(img.sample_bilinear(1.5, 2.5), [1.0, 2.0, 1.0]);
        let m = img.sample_bilinear(1.0, 1.5);
        assert!((m[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn raster_srgb8_keeps_lightcraft_float_quantization() {
        // Referenced to LightCraft's sqrt-domain conversion, not the 8-bit decode grid.
        let samples = [[0.00013, 0.00016, 0.0005], [0.003_130_8, 0.18, 0.5], [-0.1, 1.0, 1.1]];
        let img = Rgb32f::from_fn(samples.len(), 1, |x, _| samples[x]);
        assert_eq!(img.to_srgb8().data, [[0, 1, 2, 255], [10, 118, 188, 255], [0, 255, 255, 255]]);
    }

    #[test]
    fn srgb8_roundtrip() {
        let img = Rgba8::from_fn(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 7, 255]);
        assert_eq!(img.to_linear().to_srgb8(), img);
        assert_eq!(Rgba8::from_bytes(16, 16, &img.as_bytes()).unwrap(), img);
    }

    #[test]
    fn from_pixels_requires_exactly_the_stated_pixels() {
        let img = Rgba8::from_fn(4, 3, |x, y| [x as u8, y as u8, 9, 255]);
        let rebuilt = Rgba8::from_pixels(4, 3, img.as_bytes().as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2], p[3]])).unwrap();
        assert_eq!(rebuilt, img);
        assert!(Rgba8::from_pixels(4, 3, vec![[0, 0, 0, 0]; 11]).is_none(), "too few pixels");
        assert!(Rgba8::from_pixels(4, 3, vec![[0, 0, 0, 0]; 13]).is_none(), "too many pixels");
        assert!(Rgba8::from_pixels(0, 3, std::iter::empty()).is_none());
        assert!(Rgba8::from_pixels(usize::MAX, 2, std::iter::empty()).is_none());
    }
}
