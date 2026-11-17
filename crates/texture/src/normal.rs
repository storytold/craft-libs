//! Tangent-space normal maps from a height plane.
//!
//! [`try_height_to_normal`] takes the 3×3 Sobel gradient of the height around each sample, scales
//! it by [`HeightNormalOptions::strength`] and normalizes `(-gx × s, gy × s, 1)`. Image `y` grows
//! downward, so height that rises toward the bottom of the image gives a positive OpenGL `y`.
//! Each unit normal is stored as `0.5 × n + 0.5` in red, green and blue, the encoding glTF 2.0
//! section 3.9.3 gives for normal textures
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>), with alpha 1. The output is
//! linear data. [`NormalConvention::DirectX`] negates `y` before the encoding.

use crate::source::{ScalarSource, TextureOpError, read_checked, try_buffer, try_color_image};
use crate::{Image, Limits};

/// Divisor of the slope scale in [`HeightNormalOptions::uv_tiles`]: the strength is
/// `height_strength × tile_res × block_scale / HEIGHT_SPAN`.
pub const HEIGHT_SPAN: f32 = 16.0;

/// The direction of the green channel of a normal map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NormalConvention {
    /// Green points up the image (X+, Y+, Z+), as glTF 2.0 normal textures do.
    #[default]
    OpenGl,
    /// Green points down the image (X+, Y-, Z+).
    DirectX,
}

/// Which samples a neighbor of a sample may be read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoundaryMode {
    /// A neighbor outside the image takes the nearest edge sample.
    #[default]
    ClampToImage,
    /// The plane is a grid of `block × block` material blocks, the top-left block at `(0, 0)`. A
    /// neighbor outside the block of the center sample, or outside the image, takes the nearest
    /// sample inside both. `block` is at least 1.
    ClampToBlock { block: u32 },
}

/// How [`try_height_to_normal`] turns heights into normals.
/// `#[non_exhaustive]`: start from [`HeightNormalOptions::default`] or
/// [`HeightNormalOptions::uv_tiles`] and set the fields you change.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct HeightNormalOptions {
    /// Multiplier of the Sobel gradient. 0 gives flat normals; a negative value inverts the
    /// relief. Default 1.
    pub strength: f32,
    /// Default [`NormalConvention::OpenGl`].
    pub convention: NormalConvention,
    /// Default [`BoundaryMode::ClampToImage`].
    pub boundary: BoundaryMode,
}

impl Default for HeightNormalOptions {
    fn default() -> Self {
        Self { strength: 1.0, convention: NormalConvention::OpenGl, boundary: BoundaryMode::ClampToImage }
    }
}

impl HeightNormalOptions {
    /// Options for a plane of `tile_res × tile_res` UV tiles: strength
    /// `height_strength × tile_res × block_scale / HEIGHT_SPAN`, neighbors clamped to the UV tile
    /// of the center sample, and OpenGL normals. `height_strength` must be finite, `tile_res` at
    /// least 1 and `block_scale` finite and above 0.
    pub fn uv_tiles(height_strength: f32, tile_res: u32, block_scale: f32) -> Result<Self, TextureOpError> {
        if !height_strength.is_finite() {
            return Err(TextureOpError::BadParameter { name: "height_strength", value: f64::from(height_strength) });
        }
        if tile_res == 0 {
            return Err(TextureOpError::BadParameter { name: "tile_res", value: 0.0 });
        }
        if !(block_scale.is_finite() && block_scale > 0.0) {
            return Err(TextureOpError::BadParameter { name: "block_scale", value: f64::from(block_scale) });
        }
        let strength = height_strength * tile_res as f32 * block_scale / HEIGHT_SPAN;
        if !strength.is_finite() {
            return Err(TextureOpError::BadParameter { name: "strength", value: f64::from(strength) });
        }
        Ok(Self { strength, convention: NormalConvention::OpenGl, boundary: BoundaryMode::ClampToBlock { block: tile_res } })
    }
}

/// The unit OpenGL normal of the height neighborhood `h`, rows top to bottom and columns left to
/// right with the center at `h[1][1]`. The Sobel gradients are
/// `gx = (h[0][2] + 2 h[1][2] + h[2][2] - h[0][0] - 2 h[1][0] - h[2][0]) / 8` and
/// `gy = (h[2][0] + 2 h[2][1] + h[2][2] - h[0][0] - 2 h[0][1] - h[0][2]) / 8`, and the result is
/// `(-gx × strength, gy × strength, 1)` normalized, computed in `f64`. NaN or infinite input
/// gives `[0, 0, 1]`.
pub fn normal_from_neighborhood(h: [[f32; 3]; 3], strength: f32) -> [f32; 3] {
    let h = h.map(|row| row.map(f64::from));
    let gx = (h[0][2] + 2.0 * h[1][2] + h[2][2] - h[0][0] - 2.0 * h[1][0] - h[2][0]) / 8.0;
    let gy = (h[2][0] + 2.0 * h[2][1] + h[2][2] - h[0][0] - 2.0 * h[0][1] - h[0][2]) / 8.0;
    let s = f64::from(strength);
    let (x, y) = (-gx * s, gy * s);
    let len = (x * x + y * y + 1.0).sqrt();
    if !len.is_finite() {
        return [0.0, 0.0, 1.0];
    }
    [(x / len) as f32, (y / len) as f32, (1.0 / len) as f32]
}

/// A unit OpenGL normal stored as `0.5 × n + 0.5` per component, with `y` negated first for
/// [`NormalConvention::DirectX`].
pub fn encode_normal(n: [f32; 3], convention: NormalConvention) -> [f32; 3] {
    let y = match convention {
        NormalConvention::OpenGl => n[1],
        NormalConvention::DirectX => -n[1],
    };
    [n[0] * 0.5 + 0.5, y * 0.5 + 0.5, n[2] * 0.5 + 0.5]
}

/// The RGBA normal map of the height plane `source`, the same size.
///
/// Each sample's neighbors are clamped as [`HeightNormalOptions::boundary`] says, and a neighbor
/// that [`ScalarSource::covered`] reports as not covered takes the center height. Rows are read
/// from `source` one at a time, and at most three rows are kept per worker besides the output.
///
/// Returns [`TextureOpError::BadParameter`] for a non-finite strength or a block of 0, an
/// [`crate::ImageError`] when the size is outside `limits` or memory runs out, both before any
/// row is read, [`TextureOpError::NonFiniteSample`] for a NaN or infinite height, and
/// [`TextureOpError::Source`] when a row cannot be read.
pub fn try_height_to_normal<S: ScalarSource + ?Sized>(source: &S, options: &HeightNormalOptions, limits: &Limits) -> Result<Image, TextureOpError> {
    if !options.strength.is_finite() {
        return Err(TextureOpError::BadParameter { name: "strength", value: f64::from(options.strength) });
    }
    let block = match options.boundary {
        BoundaryMode::ClampToImage => None,
        BoundaryMode::ClampToBlock { block: 0 } => return Err(TextureOpError::BadParameter { name: "block", value: 0.0 }),
        BoundaryMode::ClampToBlock { block } => Some(block),
    };
    let (w, h) = source.dimensions();
    let pass = Pass { source, strength: options.strength, convention: options.convention, block, w, h };
    try_color_image(w, h, limits, |y0, rows| pass.fill(y0, rows))
}

/// The first and last index on an axis of `n` samples that a neighbor of index `i` may take.
fn span(i: u32, n: u32, block: Option<u32>) -> (u32, u32) {
    let last = n.saturating_sub(1);
    match block {
        None => (0, last),
        Some(b) => {
            let first = i / b * b;
            (first, first.saturating_add(b - 1).min(last))
        }
    }
}

/// The indices of the previous sample, `i` and the next sample, clamped to `span`.
fn neighbors(i: u32, (first, last): (u32, u32)) -> [u32; 3] {
    [i.saturating_sub(1).max(first), i, i.saturating_add(1).min(last)]
}

struct Pass<'a, S: ?Sized> {
    source: &'a S,
    strength: f32,
    convention: NormalConvention,
    block: Option<u32>,
    w: u32,
    h: u32,
}

impl<S: ScalarSource + ?Sized> Pass<'_, S> {
    /// Fill the RGBA rows starting at row `y0`.
    fn fill(&self, y0: u32, rows: &mut [f32]) -> Result<(), TextureOpError> {
        let mut cache = RowCache::new(self.w, self.h)?;
        for (y, out) in (y0..).zip(rows.chunks_exact_mut(self.w as usize * 4)) {
            let ys = neighbors(y, span(y, self.h, self.block));
            cache.load(self.source, ys)?;
            let input = ys.map(|y| cache.row(y));
            for (x, px) in (0u32..).zip(out.as_chunks_mut::<4>().0) {
                let n = normal_from_neighborhood(self.neighborhood(x, y, ys, input), self.strength);
                let [r, g, b] = encode_normal(n, self.convention);
                *px = [r, g, b, 1.0];
            }
        }
        Ok(())
    }

    /// The 3×3 heights around `(x, y)`, read from the rows `ys`, whose samples are `rows`.
    fn neighborhood(&self, x: u32, y: u32, ys: [u32; 3], rows: [&[f32]; 3]) -> [[f32; 3]; 3] {
        let xs = neighbors(x, span(x, self.w, self.block));
        let center = rows[1].get(x as usize).copied().unwrap_or(0.0);
        let mut h = [[center; 3]; 3];
        for ((hr, row), sy) in h.iter_mut().zip(rows).zip(ys) {
            for (hv, sx) in hr.iter_mut().zip(xs) {
                if (sx, sy) != (x, y) && self.source.covered(sx, sy) {
                    *hv = row.get(sx as usize).copied().unwrap_or(center);
                }
            }
        }
        h
    }
}

/// Up to three decoded rows, each tagged with its row index.
struct RowCache {
    slots: [(Option<u32>, Vec<f32>); 3],
}

impl RowCache {
    fn new(w: u32, h: u32) -> Result<RowCache, TextureOpError> {
        let row = || try_buffer(w as usize, w, h);
        Ok(RowCache { slots: [(None, row()?), (None, row()?), (None, row()?)] })
    }

    /// Read each row of `ys` that is not held yet into a slot that holds none of `ys`.
    fn load<S: ScalarSource + ?Sized>(&mut self, source: &S, ys: [u32; 3]) -> Result<(), TextureOpError> {
        for y in ys {
            if self.slots.iter().any(|(k, _)| *k == Some(y)) {
                continue;
            }
            if let Some((k, row)) = self.slots.iter_mut().find(|(k, _)| !k.is_some_and(|k| ys.contains(&k))) {
                *k = None;
                read_checked(source, y, row)?;
                *k = Some(y);
            }
        }
        Ok(())
    }

    /// The samples of row `y`, empty when it is not held.
    fn row(&self, y: u32) -> &[f32] {
        self.slots.iter().find(|(k, _)| *k == Some(y)).map_or(&[], |(_, row)| row.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImageError, Kind, SourceError};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An 8-bit channel value, as an 8-bit PNG stores it.
    fn byte(v: f32) -> i32 {
        (v.clamp(0.0, 1.0) * 255.0).round() as i32
    }

    fn normals(src: &Image, options: &HeightNormalOptions) -> Image {
        match try_height_to_normal(src, options, &Limits::default()) {
            Ok(img) => img,
            Err(e) => panic!("normal map failed: {e}"),
        }
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tol)
    }

    fn rgb(img: &Image, x: i64, y: i64) -> [f32; 3] {
        let p = img.get(x, y, false);
        [p[0], p[1], p[2]]
    }

    #[test]
    fn height_becomes_normal_detail() {
        // Height rises to the right and down the image.
        let height = Image::gray(64, 64, |x, y| (x as f32 + y as f32) / 160.0);
        let Ok(options) = HeightNormalOptions::uv_tiles(8.0, 64, 1.0) else { panic!("valid options") };
        let n = normals(&height, &options);
        let p = n.get(30, 30, false).map(byte);
        assert!(p[0] < 120, "slope up to the right tilts the normal left: {p:?}");
        assert!(p[1] > 136, "slope up toward the image bottom tilts an OpenGL normal up: {p:?}");
        assert_eq!(n.get(30, 30, false)[3], 1.0);
        let Ok(flat) = HeightNormalOptions::uv_tiles(0.0, 64, 1.0) else { panic!("valid options") };
        assert_eq!(normals(&height, &flat).get(30, 30, false).map(byte), [128, 128, 255, 255]);
    }

    #[test]
    fn directx_flips_green() {
        let (x, y) = (0.3f32, 0.4f32);
        let n = [x, y, (1.0 - x * x - y * y).sqrt()];
        let (a, b) = (encode_normal(n, NormalConvention::OpenGl).map(byte), encode_normal(n, NormalConvention::DirectX).map(byte));
        assert_eq!(a[0], byte(x * 0.5 + 0.5));
        assert_eq!(a[1], byte(y * 0.5 + 0.5), "{a:?}");
        assert_eq!(a[0], b[0]);
        assert!((a[1] + b[1] - 255).abs() <= 1, "{a:?} vs {b:?}");
        assert_eq!(a[2], b[2]);
    }

    #[test]
    fn converted_maps_invert_and_derive_normals() {
        let height = Image::gray(64, 64, |x, y| (x as f32 + y as f32) / 160.0);
        let Ok(gl) = HeightNormalOptions::uv_tiles(8.0, 64, 1.0) else { panic!("valid options") };
        let dx = HeightNormalOptions { convention: NormalConvention::DirectX, ..gl };
        let (a, b) = (normals(&height, &gl).get(30, 30, false), normals(&height, &dx).get(30, 30, false));
        assert_eq!(a[0], b[0]);
        assert_eq!(a[2], b[2]);
        assert!(byte(a[1]) > 136 && (byte(a[1]) + byte(b[1]) - 255).abs() <= 1, "DirectX flips green: {a:?} vs {b:?}");
        assert!((a[1] + b[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn uv_tiles_export_one_file_per_tile_without_padding_across() {
        // Two 16-sample UV tiles side by side: a slope in the left one, a cliff of 100 in the right.
        let atlas = Image::gray(32, 16, |x, y| if x < 16 { 0.05 * x as f32 + 0.02 * y as f32 } else { 100.0 });
        let left = Image::gray(16, 16, |x, y| 0.05 * x as f32 + 0.02 * y as f32);
        let Ok(options) = HeightNormalOptions::uv_tiles(1.0, 16, 1.0) else { panic!("valid options") };
        let (a, l) = (normals(&atlas, &options), normals(&left, &HeightNormalOptions { boundary: BoundaryMode::ClampToImage, ..options }));
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(rgb(&a, x, y), rgb(&l, x, y), "({x}, {y}) reads only its own tile");
            }
            // The right tile is flat at its own left edge although the left tile is lower.
            assert_eq!(rgb(&a, 16, y), [0.5, 0.5, 1.0]);
        }
        // Without the tile clamp the cliff tilts the last column of the left tile.
        let open = normals(&atlas, &HeightNormalOptions { boundary: BoundaryMode::ClampToImage, ..options });
        assert!(open.get(15, 5, false)[0] < 0.1);
    }

    #[test]
    fn a_plane_gives_the_sobel_normal() {
        let (a, b, s) = (0.03f32, -0.07f32, 5.0f32);
        let plane = Image::gray(9, 7, |x, y| a * x as f32 + b * y as f32);
        let len = ((a * s).powi(2) + (b * s).powi(2) + 1.0).sqrt();
        let want = [-a * s / len, b * s / len, 1.0 / len];
        let h = |x: i64, y: i64| plane.get(x, y, false)[0];
        let hood = [[h(3, 2), h(4, 2), h(5, 2)], [h(3, 3), h(4, 3), h(5, 3)], [h(3, 4), h(4, 4), h(5, 4)]];
        assert!(close(normal_from_neighborhood(hood, s), want, 1e-6));
        let n = normals(&plane, &HeightNormalOptions { strength: s, ..HeightNormalOptions::default() });
        let enc = want.map(|c| 0.5 * c + 0.5);
        for (x, y) in [(1, 1), (4, 3), (7, 5)] {
            assert!(close(rgb(&n, x, y), enc, 1e-6), "({x}, {y}): {:?} vs {enc:?}", rgb(&n, x, y));
        }
        // At the image edge the outside neighbors repeat the edge, halving that axis's slope.
        let edge = normal_from_neighborhood([[h(0, 2), h(0, 2), h(1, 2)], [h(0, 3), h(0, 3), h(1, 3)], [h(0, 4), h(0, 4), h(1, 4)]], s);
        assert!(close(rgb(&n, 0, 3), encode_normal(edge, NormalConvention::OpenGl), 1e-6));
    }

    /// A height plane whose samples in columns `x >= 2` are not covered.
    struct Island(Image);

    impl ScalarSource for Island {
        fn dimensions(&self) -> (u32, u32) {
            self.0.dimensions()
        }
        fn read_row(&self, y: u32, x0: u32, out: &mut [f32]) -> Result<(), SourceError> {
            self.0.read_row(y, x0, out)
        }
        fn covered(&self, x: u32, _y: u32) -> bool {
            x < 2
        }
    }

    #[test]
    fn uncovered_neighbors_take_the_center_height() {
        let island = Island(Image::gray(4, 3, |x, _| [0.0, 1.0, 50.0, 50.0][x as usize]));
        let Ok(n) = try_height_to_normal(&island, &HeightNormalOptions::default(), &Limits::default()) else { panic!("normal map") };
        // At x = 1 the right neighbors are not covered and read as 1, so gx = (1 - 0) / 2.
        let want = normal_from_neighborhood([[0.0, 1.0, 1.0]; 3], 1.0);
        assert!(close(rgb(&n, 1, 1), encode_normal(want, NormalConvention::OpenGl), 1e-6));
    }

    #[test]
    fn zero_and_one_sample_planes() {
        let one = Image::gray(1, 1, |_, _| 7.0);
        assert_eq!(normals(&one, &HeightNormalOptions { strength: 100.0, ..HeightNormalOptions::default() }).data(), &[0.5, 0.5, 1.0, 1.0]);
        let empty = Empty { reads: AtomicUsize::new(0) };
        assert_eq!(try_height_to_normal(&empty, &HeightNormalOptions::default(), &Limits::default()), Err(ImageError::BadSize(0, 0).into()));
        assert_eq!(empty.reads.load(Ordering::Relaxed), 0);
    }

    /// A source with no samples that counts its reads.
    struct Empty {
        reads: AtomicUsize,
    }

    impl ScalarSource for Empty {
        fn dimensions(&self) -> (u32, u32) {
            (0, 0)
        }
        fn read_row(&self, _y: u32, _x0: u32, _out: &mut [f32]) -> Result<(), SourceError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    /// A source of any claimed size that fails every read.
    struct Failing(u32, u32);

    impl ScalarSource for Failing {
        fn dimensions(&self) -> (u32, u32) {
            (self.0, self.1)
        }
        fn read_row(&self, _y: u32, _x0: u32, _out: &mut [f32]) -> Result<(), SourceError> {
            Err(SourceError("tile unavailable".into()))
        }
    }

    #[test]
    fn sizes_outside_the_limits_are_rejected_before_reading() {
        let open = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX };
        let huge = Failing(u32::MAX, u32::MAX);
        let o = HeightNormalOptions::default();
        assert_eq!(try_height_to_normal(&huge, &o, &open), Err(ImageError::TooLarge(u32::MAX, u32::MAX).into()));
        let small = Limits { max_pixels: 15, ..Limits::default() };
        assert_eq!(try_height_to_normal(&Failing(4, 4), &o, &small), Err(ImageError::TooLarge(4, 4).into()));
        assert_eq!(try_height_to_normal(&Failing(9000, 1), &o, &Limits::default()), Err(ImageError::BadSize(9000, 1).into()));
    }

    #[test]
    fn read_failures_and_non_finite_input_are_errors() {
        let o = HeightNormalOptions::default();
        let l = Limits::default();
        assert!(matches!(try_height_to_normal(&Failing(3, 3), &o, &l), Err(TextureOpError::Source { .. })));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let Ok(img) = Image::from_data(3, 2, Kind::Gray, vec![0.0, 0.0, 0.0, 0.0, bad, 0.0], &l) else { panic!("valid image") };
            assert_eq!(try_height_to_normal(&img, &o, &l), Err(TextureOpError::NonFiniteSample { x: 1, y: 1 }));
            let strength = HeightNormalOptions { strength: bad, ..o };
            assert!(matches!(try_height_to_normal(&Failing(3, 3), &strength, &l), Err(TextureOpError::BadParameter { name: "strength", .. })));
            assert!(matches!(HeightNormalOptions::uv_tiles(bad, 16, 1.0), Err(TextureOpError::BadParameter { name: "height_strength", .. })));
            assert!(matches!(HeightNormalOptions::uv_tiles(1.0, 16, bad), Err(TextureOpError::BadParameter { name: "block_scale", .. })));
            assert_eq!(normal_from_neighborhood([[bad; 3]; 3], 1.0), [0.0, 0.0, 1.0]);
        }
        assert!(HeightNormalOptions::uv_tiles(1.0, 16, 0.0).is_err());
        assert!(HeightNormalOptions::uv_tiles(1.0, 0, 1.0).is_err());
        assert!(matches!(HeightNormalOptions::uv_tiles(f32::MAX, 16, 1.0), Err(TextureOpError::BadParameter { name: "strength", .. })));
        let block0 = HeightNormalOptions { boundary: BoundaryMode::ClampToBlock { block: 0 }, ..o };
        assert!(matches!(try_height_to_normal(&Failing(3, 3), &block0, &l), Err(TextureOpError::BadParameter { name: "block", .. })));
    }

    #[test]
    fn huge_finite_heights_give_unit_normals() {
        let l = Limits::default();
        let Ok(img) = Image::from_data(3, 1, Kind::Gray, vec![-f32::MAX, 0.0, f32::MAX], &l) else { panic!("valid image") };
        let n = normals(&img, &HeightNormalOptions { strength: f32::MAX, ..HeightNormalOptions::default() });
        let p = rgb(&n, 1, 0);
        assert!(p.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)), "{p:?}");
        assert!(p[0] < 1e-6, "the slope points the normal along -x: {p:?}");
    }
}
