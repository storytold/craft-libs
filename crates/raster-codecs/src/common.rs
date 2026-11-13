//! Bounded decoding and PNG encoding for common photographic and web image formats.
//!
//! This facade is for applications that need a single RGBA pixel buffer regardless of whether
//! the source was PNG, JPEG, BMP, GIF, TIFF, or WebP. Format sniffing and codec dispatch stay
//! inside this crate. Callers choose their own stricter file and image limits.

use std::io::Cursor;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageReader, Limits as ImageLimits, RgbaImage};

use crate::CodecError;

pub use image::{Rgba as Pixel, RgbaImage as Pixels};

/// Limits applied before allocating the decoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_file_bytes: usize,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_alloc_bytes: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self { max_file_bytes: 64 * 1024 * 1024, max_width: 10_000, max_height: 10_000, max_pixels: 16_000_000, max_alloc_bytes: 256 * 1024 * 1024 }
    }
}

/// Decode one frame into straight-alpha RGBA pixels. Animated images use the first frame.
pub fn decode_rgba(bytes: &[u8], limits: DecodeLimits) -> Result<RgbaImage, CodecError> {
    if bytes.len() > limits.max_file_bytes {
        return Err(CodecError::LimitExceeded(format!("{} encoded bytes exceed max {}", bytes.len(), limits.max_file_bytes)));
    }
    if is_heif(bytes) {
        return Err(CodecError::unsupported("image", "HEIC/HEIF is not supported by the enabled codecs"));
    }
    let reader = || -> Result<ImageReader<Cursor<&[u8]>>, CodecError> {
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| CodecError::malformed("image", e))?;
        if reader.format().is_none() {
            return Err(CodecError::unsupported("image", "unrecognised image format"));
        }
        let mut image_limits = ImageLimits::default();
        image_limits.max_image_width = Some(limits.max_width);
        image_limits.max_image_height = Some(limits.max_height);
        image_limits.max_alloc = Some(limits.max_alloc_bytes);
        reader.limits(image_limits);
        Ok(reader)
    };
    let (width, height) = reader()?.into_dimensions().map_err(|e| CodecError::malformed("image", e))?;
    check_dimensions(width, height, limits)?;
    let decoded = reader()?.decode().map_err(|e| CodecError::malformed("image", e))?;
    check_dimensions(decoded.width(), decoded.height(), limits)?;
    Ok(decoded.to_rgba8())
}

/// Encode an RGBA image as PNG.
pub fn encode_png(pixels: &RgbaImage) -> Result<Vec<u8>, CodecError> {
    let mut out = Cursor::new(Vec::new());
    pixels.write_to(&mut out, image::ImageFormat::Png).map_err(|e| CodecError::encode("PNG", e))?;
    Ok(out.into_inner())
}

/// Encode an RGBA image as standard Base64 containing PNG bytes, for a document field that stores
/// an inline PNG image.
pub fn encode_png_base64(pixels: &RgbaImage) -> Result<String, CodecError> {
    encode_png(pixels).map(|bytes| STANDARD.encode(bytes))
}

/// Decode standard Base64 containing one PNG into straight-alpha RGBA pixels.
///
/// The encoded length is checked before Base64 allocates the decoded buffer, and the decoded PNG
/// then passes through the same file, dimension, pixel-count, and allocation limits as any other
/// raster input.
pub fn decode_png_base64(text: &str, limits: DecodeLimits) -> Result<RgbaImage, CodecError> {
    let max_encoded_bytes = limits.max_file_bytes.div_ceil(3).saturating_mul(4);
    if text.len() > max_encoded_bytes {
        return Err(CodecError::LimitExceeded(format!("{} Base64 characters exceed max {max_encoded_bytes}", text.len())));
    }
    let bytes = STANDARD.decode(text).map_err(|e| CodecError::malformed("PNG Base64", e))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(CodecError::malformed("PNG Base64", "decoded data has no PNG signature"));
    }
    decode_rgba(&bytes, limits)
}

/// Encode RGBA pixels as JPEG at the requested quality (1–100).
pub fn encode_jpeg(pixels: &RgbaImage, quality: u8) -> Result<Vec<u8>, CodecError> {
    if !(1..=100).contains(&quality) {
        return Err(CodecError::InvalidImage(format!("JPEG quality {quality} is outside 1–100")));
    }
    let rgb = image::DynamicImage::ImageRgba8(pixels.clone()).to_rgb8();
    let mut out = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&rgb).map_err(|e| CodecError::encode("JPEG", e))?;
    Ok(out.into_inner())
}

/// Encode pixels according to a destination filename's extension.
pub fn encode_for_path(path: impl AsRef<std::path::Path>, pixels: &RgbaImage) -> Result<Vec<u8>, CodecError> {
    let extension = path.as_ref().extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "png" => encode_png(pixels),
        "jpg" | "jpeg" => encode_jpeg(pixels, 90),
        "bmp" => encode_dynamic(pixels, image::ImageFormat::Bmp),
        "gif" => encode_dynamic(pixels, image::ImageFormat::Gif),
        "tif" | "tiff" => encode_dynamic(pixels, image::ImageFormat::Tiff),
        "webp" => encode_dynamic(pixels, image::ImageFormat::WebP),
        other => Err(CodecError::unsupported("image", format!("cannot encode files with the .{other} extension"))),
    }
}

fn encode_dynamic(pixels: &RgbaImage, format: image::ImageFormat) -> Result<Vec<u8>, CodecError> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels.clone()).write_to(&mut out, format).map_err(|e| CodecError::encode("image", e))?;
    Ok(out.into_inner())
}

fn is_heif(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp") && matches!(bytes.get(8..12), Some(b"heic" | b"heix" | b"hevc" | b"mif1" | b"msf1" | b"avif"))
}

fn check_dimensions(width: u32, height: u32, limits: DecodeLimits) -> Result<(), CodecError> {
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| CodecError::LimitExceeded("image dimensions overflow".into()))?;
    if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height || pixels > limits.max_pixels {
        return Err(CodecError::LimitExceeded(format!("image dimensions {width}x{height} exceed configured limits")));
    }
    Ok(())
}
