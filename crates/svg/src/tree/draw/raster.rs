//! Validate optional raster images with the same backends used by the SVG renderer.
//!
//! Header probing alone cannot detect damaged pixel data. Admit dimensions and
//! reserve work before decoding: even a failed decode keeps its charge.

use std::io::Cursor;
use std::num::NonZeroU64;

use crate::tree::{SvgRaster, draw::tiny_skia, usvg};

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
    let (format, data) = match kind {
        usvg::ImageKind::PNG(data) => (SvgRaster::Png, data),
        usvg::ImageKind::JPEG(data) => (SvgRaster::Jpeg, data),
        usvg::ImageKind::GIF(data) => (SvgRaster::Gif, data),
        usvg::ImageKind::WEBP(data) => (SvgRaster::WebP, data),
        usvg::ImageKind::SVG(_) => return Ok(()),
    };
    let pixels = probe_raster_pixels(data, max_pixels)?;
    *pixels_left = pixels_left.checked_sub(pixels).ok_or(RasterValidationFailure::Limit)?;
    decode_raster(format, data, pixels)
}

pub(in crate::tree) fn probe_raster_pixels(data: &[u8], max_pixels: u64) -> Result<u64, RasterValidationFailure> {
    // The allocation probe accounts for oversized GIF/WebP frames as well as the canvas.
    let (width, height) = craft_image_codecs::probe_allocation_size(data).map_err(|_| RasterValidationFailure::Invalid)?;
    let pixels = area(width, height)?;
    if pixels > max_pixels {
        return Err(RasterValidationFailure::Limit);
    }
    Ok(pixels)
}

pub(in crate::tree) fn decode_raster(format: SvgRaster, data: &[u8], pixels: u64) -> Result<(), RasterValidationFailure> {
    match format {
        SvgRaster::Png => {
            let decoded = tiny_skia::Pixmap::decode_png(data).map_err(|_| RasterValidationFailure::Invalid)?;
            rgba_bytes(decoded.width(), decoded.height(), pixels)?;
            Ok(())
        }
        SvgRaster::Jpeg => craft_image_codecs::validate_jpeg_rgba8(data, pixels).map_err(|error| match error {
            craft_image_codecs::CodecError::LimitExceeded(_) => RasterValidationFailure::Limit,
            _ => RasterValidationFailure::Invalid,
        }),
        SvgRaster::Gif => gif(data, pixels),
        SvgRaster::WebP => webp(data, pixels),
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
    let expected = rgba / 4 * channels;
    if decoder.output_buffer_size() != Some(expected) {
        return Err(RasterValidationFailure::Invalid);
    }
    // The decoder's metadata memory limit does not cap read_image scratch.
    // Admit the frame and exact output buffer before calling read_image.
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
