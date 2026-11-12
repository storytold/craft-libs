//! A raw file's embedded camera JPEG, decoded: what the as-shot white balance estimate ([`crate::sensor::wb_estimate`])
//! and the camera look fit ([`crate::look`]) compare the sensor with. Pixels only: the JPEG is never shown or
//! rendered from here.
//!
//! The JPEG is read in the colour space the raw states for it ([`craft_raw::embedded_preview_color_space`]: sRGB or
//! Adobe RGB), sRGB when it states none. An ICC profile embedded in the JPEG itself is not read (LightCraft's
//! codec layer honours one; camera JPEGs rarely carry one).

use crate::color::transfer::{gamma_to_linear, srgb_to_linear};
use crate::color::{ADOBE_RGB, REC2020, SRGB};
use crate::raster::Rgb32f;
use crate::raster::resample::{Filter, fit};
use craft_raw::PreviewColorSpace;

/// Most pixels a camera JPEG may decode to here.
pub const MAX_PREVIEW_PIXELS: usize = 1 << 26;

/// An 8-bit RGB JPEG decode: `(rows of RGB triplets, width, height)`. `None` when it isn't a JPEG this decoder
/// reads, or it holds more than `max_pixels`.
pub fn decode_rgb8(jpeg: &[u8], max_pixels: usize) -> Option<(Vec<u8>, usize, usize)> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let opts =
        DecoderOptions::default().set_max_width(1 << 16).set_max_height(1 << 16).set_strict_mode(false).jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(jpeg), opts);
    // the header first: its dimensions size every allocation the decode makes
    d.decode_headers().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > max_pixels) {
        return None;
    }
    let px = d.decode().ok()?;
    (px.len() >= w * h * 3).then_some((px, w, h))
}

/// The raw's embedded camera JPEG (the largest) as linear Rec.2020, fitted to `max_edge` (in linear light; Lanczos-3
/// from a factor of 2 down, Mitchell below), not oriented, with the long edge of the JPEG as stored. `None` without
/// a JPEG preview this decoder reads.
pub fn embedded_working(bytes: &[u8], max_edge: usize, max_pixels: usize) -> Option<(Rgb32f, usize)> {
    let jpeg = craft_raw::embedded_preview(bytes).filter(|j| j.starts_with(&[0xff, 0xd8]))?;
    let (px, w, h) = decode_rgb8(&jpeg, max_pixels)?;
    let adobe = craft_raw::embedded_preview_color_space(bytes) == Some(PreviewColorSpace::AdobeRgb);
    // Adobe RGB (1998): gamma 563/256
    let lin: Box<dyn Fn(u8) -> f32> =
        if adobe { Box::new(|v| gamma_to_linear(f32::from(v) / 255.0, 563.0 / 256.0)) } else { Box::new(|v| srgb_to_linear(f32::from(v) / 255.0)) };
    let lut: Vec<f32> = (0..=255u8).map(&lin).collect();
    let at = |i: usize| px.get(i).map_or(0.0, |&v| lut[usize::from(v)]);
    let img = Rgb32f::from_fn(w, h, |x, y| {
        let i = (y * w + x) * 3;
        [at(i), at(i + 1), at(i + 2)]
    });
    let img = if w.max(h) > max_edge && max_edge > 0 {
        let factor = w.max(h) as f64 / max_edge as f64;
        fit(&img, max_edge, max_edge, if factor >= 2.0 { Filter::Lanczos3 } else { Filter::Mitchell })
    } else {
        img
    };
    let m = (if adobe { ADOBE_RGB } else { SRGB }).to_space(&REC2020).to_f32();
    Some((img.map(|p| std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])), w.max(h)))
}
