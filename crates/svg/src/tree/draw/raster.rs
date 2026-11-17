//! Validate optional raster images with the same backends used by the SVG renderer.
//!
//! Header probing alone cannot detect damaged pixel data. Admit dimensions and
//! reserve work before decoding: even a failed decode keeps its charge.

use std::io::Cursor;
use std::num::NonZeroU64;

use crate::tree::{draw::tiny_skia, usvg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterValidationFailure {
    /// The renderer cannot decode the encoded image.
    Invalid,
    /// Header dimensions, cumulative work or decoder allocation exceed admission.
    Limit,
}

/// Validate the renderer's first frame within per-image and remaining work limits.
/// Failed attempted decodes keep their pixel charge. No decoded pixels are retained. SVG
/// children use the same wrapped resolver and need no second recursive walk.
pub fn validate_raster(kind: &usvg::ImageKind, max_pixels: u64, pixels_left: &mut u64) -> Result<(), RasterValidationFailure> {
    let data = match kind {
        usvg::ImageKind::PNG(data) | usvg::ImageKind::JPEG(data) | usvg::ImageKind::GIF(data) | usvg::ImageKind::WEBP(data) => data,
        usvg::ImageKind::SVG(_) => return Ok(()),
    };
    // The allocation probe includes oversized GIF/WebP frames, not merely their
    // canvas. Reserve before asking any rendering decoder to allocate pixels.
    let (width, height) = craft_image_codecs::probe_allocation_size(data).map_err(|_| RasterValidationFailure::Invalid)?;
    let pixels = area(width, height)?;
    if pixels > max_pixels {
        return Err(RasterValidationFailure::Limit);
    }
    *pixels_left = pixels_left.checked_sub(pixels).ok_or(RasterValidationFailure::Limit)?;
    match kind {
        usvg::ImageKind::PNG(data) => {
            let decoded = tiny_skia::Pixmap::decode_png(data).map_err(|_| RasterValidationFailure::Invalid)?;
            rgba_bytes(decoded.width(), decoded.height(), pixels)?;
            Ok(())
        }
        usvg::ImageKind::JPEG(data) => jpeg(data, pixels),
        usvg::ImageKind::GIF(data) => gif(data, pixels),
        usvg::ImageKind::WEBP(data) => webp(data, pixels),
        usvg::ImageKind::SVG(_) => Ok(()),
    }
}

fn area(width: u32, height: u32) -> Result<u64, RasterValidationFailure> {
    if width == 0 || height == 0 {
        return Err(RasterValidationFailure::Invalid);
    }
    u64::from(width).checked_mul(u64::from(height)).ok_or(RasterValidationFailure::Limit)
}

/// Decoder headers must not request more output than the allocation probe paid for.
fn rgba_bytes(width: u32, height: u32, admitted: u64) -> Result<usize, RasterValidationFailure> {
    let pixels = area(width, height)?;
    if pixels > admitted {
        return Err(RasterValidationFailure::Limit);
    }
    usize::try_from(pixels.checked_mul(4).ok_or(RasterValidationFailure::Limit)?).map_err(|_| RasterValidationFailure::Limit)
}

fn buffer(length: usize) -> Result<Vec<u8>, RasterValidationFailure> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| RasterValidationFailure::Limit)?;
    output.resize(length, 0);
    Ok(output)
}

fn jpeg(data: &[u8], admitted: u64) -> Result<(), RasterValidationFailure> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    // Preserve the rendering decoder's default limits and RGBA conversion; a
    // general photo decoder may accept formats the SVG renderer cannot draw.
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(data), options);
    decoder.decode_headers().map_err(|_| RasterValidationFailure::Invalid)?;
    let info = decoder.info().ok_or(RasterValidationFailure::Invalid)?;
    let length = rgba_bytes(u32::from(info.width), u32::from(info.height), admitted)?;
    if decoder.output_colorspace() != Some(ColorSpace::RGBA) || decoder.output_buffer_size() != Some(length) {
        return Err(RasterValidationFailure::Invalid);
    }
    let mut output = buffer(length)?;
    decoder.decode_into(&mut output).map_err(|_| RasterValidationFailure::Invalid)
}

fn gif(data: &[u8], admitted: u64) -> Result<(), RasterValidationFailure> {
    let bytes = admitted.checked_mul(4).ok_or(RasterValidationFailure::Limit)?;
    // Do not loosen the renderer's default 50 MB GIF frame limit. Smaller
    // imports also keep their own admitted allocation ceiling during decode.
    let limit = NonZeroU64::new(bytes.min(50_000_000)).ok_or(RasterValidationFailure::Invalid)?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    let mut decoder = options.read_info(data).map_err(gif_error)?;
    let frame = decoder.read_next_frame().map_err(gif_error)?.ok_or(RasterValidationFailure::Invalid)?;
    let expected = rgba_bytes(u32::from(frame.width), u32::from(frame.height), admitted)?;
    if frame.buffer.len() != expected {
        return Err(RasterValidationFailure::Invalid);
    }
    Ok(())
}

fn gif_error(error: gif::DecodingError) -> RasterValidationFailure {
    match error {
        gif::DecodingError::MemoryLimit | gif::DecodingError::OutOfMemory => RasterValidationFailure::Limit,
        _ => RasterValidationFailure::Invalid,
    }
}

fn webp(data: &[u8], admitted: u64) -> Result<(), RasterValidationFailure> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(data)).map_err(webp_error)?;
    let (width, height) = decoder.dimensions();
    let rgba = rgba_bytes(width, height, admitted)?;
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let expected = usize::try_from(area(width, height)?.checked_mul(channels).ok_or(RasterValidationFailure::Limit)?)
        .map_err(|_| RasterValidationFailure::Limit)?;
    if expected > rgba {
        return Err(RasterValidationFailure::Limit);
    }
    if decoder.output_buffer_size() != Some(expected) {
        return Err(RasterValidationFailure::Invalid);
    }
    // A small valid image can have a compressed chunk larger than its pixels.
    // Bound decoder scratch without rejecting that ordinary encoding overhead.
    let admitted_bytes =
        usize::try_from(admitted.checked_mul(4).ok_or(RasterValidationFailure::Limit)?).map_err(|_| RasterValidationFailure::Limit)?;
    decoder.set_memory_limit(admitted_bytes.max(data.len()));
    let mut output = buffer(expected)?;
    decoder.read_image(&mut output).map_err(webp_error)
}

fn webp_error(error: image_webp::DecodingError) -> RasterValidationFailure {
    match error {
        image_webp::DecodingError::MemoryLimitExceeded | image_webp::DecodingError::ImageTooLarge => RasterValidationFailure::Limit,
        _ => RasterValidationFailure::Invalid,
    }
}

#[cfg(test)]
#[path = "raster_tests.rs"]
mod raster_tests;
