//! Bounded decoding and PNG encoding for common photographic and web image formats.
//!
//! This facade decodes supported images into a bounded straight-alpha RGBA buffer. Format
//! detection and codec dispatch stay inside `craft-image-codecs`; callers choose stricter limits.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::RgbaImage;

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
    let decode_limits = craft_image_codecs::Limits {
        max_width: limits.max_width,
        max_height: limits.max_height,
        max_pixels: limits.max_pixels,
        max_alloc: limits.max_alloc_bytes,
    };
    let decoded = craft_image_codecs::decode_with(bytes, &craft_image_codecs::DecodeOptions { limits: decode_limits, ..Default::default() })
        .map_err(map_codec_error)?;
    let output_bytes = u64::from(decoded.width()).saturating_mul(u64::from(decoded.height())).saturating_mul(4);
    if output_bytes > limits.max_alloc_bytes {
        return Err(CodecError::LimitExceeded(format!("{output_bytes} RGBA bytes exceed max {}", limits.max_alloc_bytes)));
    }
    RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8())
        .ok_or_else(|| CodecError::malformed("image", "decoded RGBA dimensions do not match its pixel buffer"))
}

/// Encode an RGBA image as PNG.
pub fn encode_png(pixels: &RgbaImage) -> Result<Vec<u8>, CodecError> {
    encode(pixels, craft_image_codecs::Format::Png, craft_image_codecs::EncodeOptions::default())
}

/// Encode RGBA pixels as standard Base64 containing PNG bytes, for an inline document image.
pub fn encode_png_base64(pixels: &RgbaImage) -> Result<String, CodecError> {
    encode_png(pixels).map(|bytes| STANDARD.encode(bytes))
}

/// Decode standard Base64 containing one PNG into straight-alpha RGBA pixels.
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
    encode(pixels, craft_image_codecs::Format::Jpeg, craft_image_codecs::EncodeOptions { jpeg_quality: quality, ..Default::default() })
}

fn encode(pixels: &RgbaImage, format: craft_image_codecs::Format, options: craft_image_codecs::EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let image = craft_image_codecs::Image::from_u8(pixels.width(), pixels.height(), craft_image_codecs::ChannelLayout::Rgba, pixels.as_raw().clone())
        .map_err(map_codec_error)?;
    craft_image_codecs::encode(&image, format, &options).map_err(map_codec_error)
}

/// Encode pixels according to a destination filename's extension.
pub fn encode_for_path(path: impl AsRef<std::path::Path>, pixels: &RgbaImage) -> Result<Vec<u8>, CodecError> {
    let extension = path.as_ref().extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default().to_ascii_lowercase();
    match craft_image_codecs::from_extension(&extension) {
        Some(craft_image_codecs::Format::Png) => encode_png(pixels),
        Some(craft_image_codecs::Format::Jpeg) => encode_jpeg(pixels, 90),
        Some(
            format @ (craft_image_codecs::Format::Bmp
            | craft_image_codecs::Format::Gif
            | craft_image_codecs::Format::Tiff
            | craft_image_codecs::Format::WebP),
        ) => encode(pixels, format, craft_image_codecs::EncodeOptions::default()),
        _ => Err(CodecError::unsupported("image", format!("cannot encode files with the .{extension} extension"))),
    }
}

fn map_codec_error(error: craft_image_codecs::CodecError) -> CodecError {
    match error {
        craft_image_codecs::CodecError::UnknownFormat => CodecError::unsupported("image", "unrecognized image format"),
        craft_image_codecs::CodecError::Unsupported { format, reason } => CodecError::unsupported(format.name(), reason),
        craft_image_codecs::CodecError::Malformed { format, message } => CodecError::malformed(format.name(), message),
        craft_image_codecs::CodecError::LimitExceeded(message) => CodecError::LimitExceeded(message),
        craft_image_codecs::CodecError::Encode { format, message } => CodecError::Encode { format: format.name(), message },
        craft_image_codecs::CodecError::InvalidImage(message) => CodecError::InvalidImage(message),
    }
}
