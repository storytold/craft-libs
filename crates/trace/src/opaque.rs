//! The outline of an image's opaque area: what Outline Object on an image's own fill or stroke
//! follows.

use craft_geom::{Affine, PathData};

use crate::raster::{MAX_PIXELS, fit_size, gif_first_frame_pixels, trace_header};
use crate::{Mode, Raster, TraceError, TraceParams, trace_within};

/// The most pixels [`opaque_outline`] traces: a larger image is reduced to this many first,
/// aspect kept ([`reduced`]), and its outline is scaled back up to the image's size.
pub const OUTLINE_MAX_PIXELS: u64 = 4 << 20;

/// The most anchor points an outline may have; an image whose outline would have more gives
/// [`TraceError::TooComplex`].
pub const OUTLINE_MAX_ANCHORS: usize = 100_000;

/// The most bytes [`decode_alpha`] decodes an image's samples into (256 MB: 64 megapixels at 8
/// bits a channel, 32 megapixels at 16).
pub const ALPHA_MAX_DECODE_BYTES: u64 = 256 << 20;

/// The alpha of the encoded image `bytes` (PNG, WebP, GIF, TIFF and the other formats the image
/// reader takes), one byte a pixel, row by row, with the image's width and height. `None` when the
/// image's format or color type has no alpha; the image is then not decoded. An image of more
/// than [`MAX_PIXELS`] (64 megapixels), or whose samples take more than [`ALPHA_MAX_DECODE_BYTES`],
/// gives [`TraceError::TooLarge`] before it is decoded.
pub fn decode_alpha(bytes: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, TraceError> {
    use craft_image_codecs::{ChannelLayout, DecodeOptions, Limits, RasterColorType, SampleType, decode_with};
    let decode_err = |e: craft_image_codecs::CodecError| TraceError::Decode(e.to_string());
    if let Some(info) = trace_header(bytes, MAX_PIXELS, ALPHA_MAX_DECODE_BYTES)? {
        if info.width == 0 || info.height == 0 {
            return Err(TraceError::Empty);
        }
        let might_have_alpha = match info.color_type {
            RasterColorType::GrayAlpha | RasterColorType::Rgba | RasterColorType::CmykAlpha => true,
            RasterColorType::Rgb => info.channels > 3,
            RasterColorType::Gray => info.channels > 1,
            // The shared TIFF inspector reports the base photometric type even
            // when an ExtraSamples channel carries alpha.
            RasterColorType::Cmyk => info.channels > 4,
            RasterColorType::Lab => info.channels > 3,
            RasterColorType::Indexed => true,
            _ => false,
        };
        if !might_have_alpha {
            return Ok(None);
        }
        let pixels = u64::from(info.width) * u64::from(info.height);
        let frame = gif_first_frame_pixels(bytes).unwrap_or(0);
        let total_bytes = pixels.saturating_mul(u64::from(info.channels)).saturating_mul(u64::from(info.bits_per_sample.max(8).div_ceil(8)));
        if pixels.max(frame) > MAX_PIXELS || total_bytes > ALPHA_MAX_DECODE_BYTES {
            return Err(TraceError::TooLarge { width: info.width, height: info.height, max: MAX_PIXELS >> 20 });
        }
    } else if gif_first_frame_pixels(bytes).is_some_and(|p| p > MAX_PIXELS) {
        return Err(TraceError::TooLarge { width: 0, height: 0, max: MAX_PIXELS >> 20 });
    }
    let opts =
        DecodeOptions { keep_orientation: true, limits: Limits { max_pixels: MAX_PIXELS, max_alloc: ALPHA_MAX_DECODE_BYTES, ..Limits::default() } };
    let img = decode_with(bytes, &opts).map_err(decode_err)?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(TraceError::Empty);
    }
    if !img.layout().has_alpha() {
        return Ok(None);
    }
    let alpha: Vec<u8> = match (img.layout(), img.sample_type()) {
        (ChannelLayout::Rgba, SampleType::U8) => img.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
        (ChannelLayout::GrayA, SampleType::U8) => img.data().as_chunks::<2>().0.iter().map(|p| p[1]).collect(),
        _ => img.to_rgba8().as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
    };
    Ok(Some((w, h, alpha)))
}

/// The outline of the pixels of a `width × height` image whose `alpha` (one byte a pixel, row by
/// row) is at least half opaque (128 or more), in its pixel space (x right, y down; the image
/// spans `0..width × 0..height`). `None` when every pixel is: the outline is then the image's
/// frame. An image with no such pixel gives an empty outline.
///
/// The alpha is traced as Image Trace's Black and White mode traces a gray at threshold 128, with
/// its Paths and Corners at their defaults and no area left out as noise: the edge runs where the
/// alpha crosses one half, to a fraction of a pixel along anti-aliased edges, and the outline is
/// fitted with curves. Outer outlines and holes run in opposite directions, and an island in a
/// hole runs as outer outlines do, so the outline fills the same with either fill rule. An image
/// of more than [`OUTLINE_MAX_PIXELS`] is traced at that size ([`reduced`]).
pub fn opaque_outline(width: u32, height: u32, alpha: Vec<u8>) -> Result<Option<PathData>, TraceError> {
    let (w, h) = (width, height);
    if (w as usize).checked_mul(h as usize).is_none_or(|n| n == 0 || n != alpha.len()) {
        return Err(TraceError::Empty);
    }
    if alpha.iter().all(|&a| a >= 128) {
        return Ok(None);
    }
    if alpha.iter().all(|&a| a < 128) {
        return Ok(Some(PathData::default()));
    }
    let (nw, nh) = fit_size(w, h, OUTLINE_MAX_PIXELS);
    let alpha = if (nw, nh) == (w, h) { alpha } else { reduced(&alpha, w, h, nw, nh) };
    // Opaque pixels are black, transparent ones white.
    let mut rgba = Vec::with_capacity(alpha.len() * 4);
    for a in alpha {
        let g = 255 - a;
        rgba.extend_from_slice(&[g, g, g, 255]);
    }
    let gray = Raster::new(nw, nh, rgba)?;
    let params = TraceParams { mode: Mode::BlackAndWhite, threshold: 128, ignore_white: true, noise: 0, ..TraceParams::default() };
    let traced = trace_within(&gray, &params, OUTLINE_MAX_ANCHORS)?;
    let mut out = PathData::default();
    for p in traced.paths {
        out.subpaths.extend(p.path.subpaths);
    }
    if (nw, nh) != (w, h) {
        out = out.transformed(Affine::scale_non_uniform(f64::from(w) / f64::from(nw), f64::from(h) / f64::from(nh)));
    }
    Ok(Some(out))
}

/// The `w × h` `alpha` reduced to `nw × nh` (at most `w × h`): each pixel holds the mean of the
/// block of pixels it covers, and at least 128 when that block holds a pixel of 128 or more, so an
/// opaque line thinner than a block stays inside the outline. Along an edge the outline can then
/// run up to half a block further out than the full-size image's would.
fn reduced(alpha: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let span = |i: u32, n: u32, of: u32| {
        let at = |i: u32| usize::try_from(u64::from(i) * u64::from(of) / u64::from(n.max(1))).unwrap_or(usize::MAX);
        let start = at(i);
        start..at(i + 1).max(start + 1)
    };
    let cols: Vec<_> = (0..nw).map(|x| span(x, nw, w)).collect();
    let mut out = Vec::with_capacity(nw as usize * nh as usize);
    let (mut sums, mut maxes) = (vec![0u64; cols.len()], vec![0u8; cols.len()]);
    for ty in 0..nh {
        let rows = span(ty, nh, h);
        sums.fill(0);
        maxes.fill(0);
        for y in rows.clone() {
            let Some(row) = alpha.get(y * w as usize..(y + 1) * w as usize) else { break };
            for (x, c) in cols.iter().enumerate() {
                for &a in row.get(c.clone()).unwrap_or_default() {
                    sums[x] += u64::from(a);
                    maxes[x] = maxes[x].max(a);
                }
            }
        }
        for (x, c) in cols.iter().enumerate() {
            let count = (rows.len() * c.len()).max(1) as u64;
            let mean = u8::try_from((sums[x] + count / 2) / count).unwrap_or(u8::MAX);
            out.push(if maxes[x] >= 128 { mean.max(128) } else { mean });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use kurbo::Shape;

    use super::*;

    /// The width, height and alpha of `img`, as [`opaque_outline`] takes them.
    fn alpha(img: &Raster) -> (u32, u32, Vec<u8>) {
        (img.width, img.height, img.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect())
    }

    fn outline(img: &Raster) -> Result<Option<PathData>, TraceError> {
        let (w, h, a) = alpha(img);
        opaque_outline(w, h, a)
    }

    /// A `w × h` raster, opaque red inside the disc of radius `r` around the middle and
    /// transparent outside it.
    fn disc(w: u32, h: u32, r: f64) -> Raster {
        let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
        Raster::from_fn(w, h, |x, y| {
            let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
            if dx.hypot(dy) <= r { [255, 0, 0, 255] } else { [0, 0, 0, 0] }
        })
    }

    fn area(p: &PathData) -> f64 {
        p.to_bezpath().area().abs()
    }

    #[test]
    fn the_outline_follows_the_opaque_disc_not_the_frame() {
        let outline = outline(&disc(100, 100, 30.0)).unwrap().unwrap();
        let b = outline.bounds().unwrap();
        for (got, want) in [(b.x0, 20.0), (b.y0, 20.0), (b.x1, 80.0), (b.y1, 80.0)] {
            assert!((got - want).abs() < 1.0, "{b:?}");
        }
        let disc_area = std::f64::consts::PI * 30.0 * 30.0;
        assert!((area(&outline) - disc_area).abs() < disc_area * 0.02, "{}", area(&outline));
        // Fitted with curves, not one line per pixel step.
        assert!(outline.anchor_count() < 40, "{} anchors", outline.anchor_count());
    }

    #[test]
    fn an_opaque_image_keeps_its_frame_and_a_transparent_one_has_no_outline() {
        let opaque = Raster::from_fn(10, 10, |_, _| [0, 0, 255, 128]);
        assert_eq!(outline(&opaque).unwrap(), None);
        let clear = Raster::from_fn(10, 10, |_, _| [0, 0, 255, 127]);
        assert!(outline(&clear).unwrap().unwrap().is_empty());
        // One pixel either way.
        assert_eq!(outline(&Raster::from_fn(1, 1, |_, _| [9, 9, 9, 255])).unwrap(), None);
        assert!(outline(&Raster::from_fn(1, 1, |_, _| [9, 9, 9, 0])).unwrap().unwrap().is_empty());
        // No pixels at all, or an alpha that doesn't match the size.
        assert!(opaque_outline(0, 0, vec![]).is_err());
        assert!(opaque_outline(2, 2, vec![0]).is_err());
    }

    #[test]
    fn pixels_less_than_half_opaque_are_outside() {
        // A 20 px square, opaque, inside a 6 px band of alpha `band`, on a transparent 40 px image.
        let framed = |band: u8| {
            Raster::from_fn(40, 40, |x, y| {
                let d = x.min(y).min(39 - x).min(39 - y);
                let alpha = match d {
                    10.. => 255,
                    4.. => band,
                    _ => 0,
                };
                [0, 0, 0, alpha]
            })
        };
        let inner = outline(&framed(127)).unwrap().unwrap().bounds().unwrap();
        assert!((inner.x0 - 10.0).abs() < 0.5 && (inner.x1 - 30.0).abs() < 0.5, "{inner:?}");
        // A half-opaque band is inside; the edge runs through the middle of its outer pixels,
        // which are half covered.
        let outer = outline(&framed(128)).unwrap().unwrap().bounds().unwrap();
        assert!((outer.x0 - 4.5).abs() < 0.25 && (outer.x1 - 35.5).abs() < 0.25, "{outer:?}");
        let outer = outline(&framed(255)).unwrap().unwrap().bounds().unwrap();
        assert!((outer.x0 - 4.0).abs() < 0.25 && (outer.x1 - 36.0).abs() < 0.25, "{outer:?}");
    }

    #[test]
    fn a_hole_in_the_opaque_area_is_a_hole_in_the_outline() {
        let ring = Raster::from_fn(60, 60, |x, y| {
            let d = (f64::from(x) + 0.5 - 30.0).hypot(f64::from(y) + 0.5 - 30.0);
            [0, 0, 0, if (10.0..=25.0).contains(&d) { 255 } else { 0 }]
        });
        let outline = outline(&ring).unwrap().unwrap();
        let want = std::f64::consts::PI * (25.0 * 25.0 - 10.0 * 10.0);
        assert!((outline.to_bezpath().area().abs() - want).abs() < want * 0.03, "{}", outline.to_bezpath().area());
        assert!(!outline.to_bezpath().contains(kurbo::Point::new(30.0, 30.0)));
        assert!(outline.to_bezpath().contains(kurbo::Point::new(30.0, 12.0)));
    }

    #[test]
    fn the_outline_fills_the_same_with_either_fill_rule() {
        // A ring with an opaque island in its hole.
        let target = Raster::from_fn(80, 80, |x, y| {
            let d = (f64::from(x) + 0.5 - 40.0).hypot(f64::from(y) + 0.5 - 40.0);
            [0, 0, 0, if (25.0..=35.0).contains(&d) || d <= 10.0 { 255 } else { 0 }]
        });
        let path = outline(&target).unwrap().unwrap().to_bezpath();
        let winding = |x: f64, y: f64| path.winding(kurbo::Point::new(x, y));
        let (ring, hole, island, outside) = (winding(40.0, 10.0), winding(40.0, 20.0), winding(40.0, 40.0), winding(2.0, 2.0));
        // Non-zero and even-odd agree when every winding number is -1, 0 or 1.
        assert!(ring.abs() == 1 && island == ring && hole == 0 && outside == 0, "{ring} {hole} {island} {outside}");
    }

    #[test]
    fn a_large_image_is_traced_smaller_and_scaled_back() {
        let big = disc(2400, 2000, 800.0);
        assert!(u64::from(big.width) * u64::from(big.height) > OUTLINE_MAX_PIXELS);
        let b = outline(&big).unwrap().unwrap().bounds().unwrap();
        for (got, want) in [(b.x0, 400.0), (b.y0, 200.0), (b.x1, 2000.0), (b.y1, 1800.0)] {
            assert!((got - want).abs() < 3.0, "{b:?}");
        }
    }

    #[test]
    fn a_line_thinner_than_the_reduction_stays_in_a_large_images_outline() {
        // 39 megapixels, reduced to 4: each traced pixel covers about 3 × 3. The only opaque
        // pixels are a line 1 pixel wide, which averages to about a third opaque.
        let (w, h) = (8192u32, 4800u32);
        let mut a = vec![0u8; (w * h) as usize];
        for y in 1000..3000 {
            a[(y * w + 2001) as usize] = 255;
        }
        let b = opaque_outline(w, h, a).unwrap().unwrap().bounds().unwrap();
        assert!(b.x0 > 1990.0 && b.x1 < 2012.0 && (b.y0 - 1000.0).abs() < 6.0 && (b.y1 - 3000.0).abs() < 6.0, "{b:?}");
    }

    #[test]
    fn an_outline_of_too_many_anchors_is_refused() {
        // A checkerboard of 2 px squares: more than 50,000 opaque squares.
        let checker = Raster::from_fn(640, 640, |x, y| [0, 0, 0, if (x / 2 + y / 2) % 2 == 0 { 255 } else { 0 }]);
        assert!(matches!(outline(&checker), Err(TraceError::TooComplex { max: OUTLINE_MAX_ANCHORS })));
    }

    /// `img` encoded as `format`.
    fn encoded(img: &craft_image_codecs::Image, format: craft_image_codecs::Format) -> Vec<u8> {
        craft_image_codecs::encode(img, format, &Default::default()).unwrap()
    }

    #[test]
    fn decode_alpha_reads_the_alpha_of_formats_that_have_one() {
        use craft_image_codecs::{ChannelLayout, Format, Image, SampleType};
        let mut rgba_bytes = Vec::with_capacity(4 * 4 * 4);
        let mut luma_a_bytes = Vec::with_capacity(4 * 4 * 2);
        let mut rgba16_bytes = Vec::with_capacity(4 * 4 * 8);
        let mut rgb_bytes = Vec::with_capacity(4 * 4 * 3);
        let mut want = Vec::with_capacity(16);
        for y in 0..4u32 {
            for x in 0..4u32 {
                let a = u8::try_from((x + 4 * y) * 16).unwrap();
                want.push(a);
                rgba_bytes.extend_from_slice(&[10, 20, 30, a]);
                luma_a_bytes.extend_from_slice(&[18, a]);
                let a16 = u16::from(a) * 257;
                for v in [10u16 * 257, 20 * 257, 30 * 257, a16] {
                    rgba16_bytes.extend_from_slice(&v.to_ne_bytes());
                }
                rgb_bytes.extend_from_slice(&[10, 20, 30]);
            }
        }
        let rgba = Image::from_u8(4, 4, ChannelLayout::Rgba, rgba_bytes).unwrap();
        let luma_a = Image::from_u8(4, 4, ChannelLayout::GrayA, luma_a_bytes).unwrap();
        let rgba16 = Image::from_raw(4, 4, ChannelLayout::Rgba, SampleType::U16, rgba16_bytes).unwrap();
        for (name, img) in [("rgba", rgba), ("gray and alpha", luma_a), ("16 bits", rgba16)] {
            let got = decode_alpha(&encoded(&img, Format::Png)).unwrap();
            assert_eq!(got, Some((4, 4, want.clone())), "{name}");
        }
        let rgb = Image::from_u8(4, 4, ChannelLayout::Rgb, rgb_bytes).unwrap();
        assert_eq!(decode_alpha(&encoded(&rgb, Format::Png)).unwrap(), None);
        assert_eq!(decode_alpha(&encoded(&rgb, Format::Jpeg)).unwrap(), None);
        assert!(decode_alpha(b"not an image").is_err());
    }

    #[test]
    fn decode_alpha_expands_gray_and_rgb_png_transparency_keys() {
        // Original synthetic 2x1 PNGs: first pixel matches tRNS; second is opaque.
        // Generated from PNG chunks and zlib scanlines, without an external image asset.
        let gray: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 0, 0, 0, 0, 209, 73, 32, 86, 0, 0, 0, 2, 116,
            82, 78, 83, 0, 0, 118, 147, 205, 56, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 168, 7, 0, 0, 130, 0, 128, 32, 18, 25, 54, 0, 0, 0,
            0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        let rgb: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 123, 64, 232, 221, 0, 0, 0, 6, 116,
            82, 78, 83, 0, 255, 0, 0, 0, 0, 164, 194, 192, 29, 0, 0, 0, 15, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 192, 240, 159, 1, 0, 7, 255,
            1, 255, 1, 127, 137, 167, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        for bytes in [gray, rgb] {
            assert_eq!(decode_alpha(bytes).unwrap(), Some((2, 1, vec![0, 255])));
        }
    }

    #[test]
    fn decode_alpha_preserves_cmyk_and_lab_tiff_extra_samples() {
        use craft_image_codecs::{ChannelLayout, Format, Image};
        for (layout, samples) in
            [(ChannelLayout::CmykA, vec![10, 20, 30, 40, 0, 50, 60, 70, 80, 255]), (ChannelLayout::LabA, vec![128, 128, 128, 0, 192, 128, 128, 255])]
        {
            let img = Image::from_u8(2, 1, layout, samples).unwrap();
            assert_eq!(decode_alpha(&encoded(&img, Format::Tiff)).unwrap(), Some((2, 1, vec![0, 255])), "{layout:?}");
        }
        for (layout, samples) in
            [(ChannelLayout::Cmyk, vec![10, 20, 30, 40, 50, 60, 70, 80]), (ChannelLayout::Lab, vec![128, 128, 128, 192, 128, 128])]
        {
            let img = Image::from_u8(2, 1, layout, samples).unwrap();
            assert_eq!(decode_alpha(&encoded(&img, Format::Tiff)).unwrap(), None, "{layout:?}");
        }
    }
}
