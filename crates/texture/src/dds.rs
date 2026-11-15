//! First-surface DDS decoding to unmodified RGBA floats.
//!
//! Adapted from MaterialCraft `dds_image.rs` (originally chasse20, cc052aeaa8c61fbe6975674b6d5e28bb3c597b9d).
//! Uses the same `dds` backend and first-surface semantics, including the first cube face or
//! volume slice. This is not a whole-resource decoder. Transfer functions and image policy
//! belong to callers. [`Limits`] bounds the output float plane, not backend scratch buffers.

use crate::{ImageError, Kind, Limits};

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("DDS decode failed: {0}")]
    Decode(String),
}

/// Unmodified backend output. Alpha and out-of-range float values are not clamped.
#[derive(Debug)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    /// Exactly the backend's `Precision::F32` classification, as used by MaterialCraft.
    pub source_precision_is_f32: bool,
}

/// Decode the first surface after validating dimensions, pixels and output-byte limits.
/// The output reservation is fallible; no second full-sized float plane is created.
pub fn decode_first(bytes: &[u8], limits: &Limits) -> Result<Surface, DecodeError> {
    let error = |e: ::dds::DecodingError| DecodeError::Decode(e.to_string());
    let mut decoder = ::dds::Decoder::new(std::io::Cursor::new(bytes)).map_err(error)?;
    let size = decoder.main_size();
    let len = limits.check(size.width, size.height, Kind::Color)?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len).map_err(|_| ImageError::OutOfMemory(size.width, size.height))?;
    rgba.resize(len, 0.0_f32);
    let view = ::dds::ImageViewMut::new(bytemuck::cast_slice_mut(&mut rgba), size, ::dds::ColorFormat::RGBA_F32)
        .ok_or_else(|| DecodeError::Decode("invalid DDS output dimensions".into()))?;
    decoder.read_surface(view).map_err(error)?;
    Ok(Surface { width: size.width, height: size.height, rgba, source_precision_is_f32: decoder.format().precision() == ::dds::Precision::F32 })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn bytes(format: ::dds::Format, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        ::dds::Header::new_image(1, 1, format).write(&mut bytes).unwrap();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn rgba8_keeps_transfer_and_alpha_unmodified() {
        let data = bytes(::dds::Format::R8G8B8A8_UNORM, &[128, 64, 255, 32]);
        let image = decode_first(&data, &Limits::default()).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert!(!image.source_precision_is_f32);
        for (actual, expected) in image.rgba.iter().zip([128.0 / 255.0, 64.0 / 255.0, 1.0, 32.0 / 255.0]) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn float_hdr_and_negative_values_survive() {
        let values = [-0.5_f32, 2.0, 0.25, 0.75];
        let payload: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let image = decode_first(&bytes(::dds::Format::R32G32B32A32_FLOAT, &payload), &Limits::default()).unwrap();
        assert!(image.source_precision_is_f32);
        assert_eq!(image.rgba, values);
    }

    #[test]
    fn checks_output_budget_and_truncation() {
        let data = bytes(::dds::Format::R8G8B8A8_UNORM, &[1, 2, 3, 4]);
        let mut limits = Limits::default();
        limits.max_bytes = 15;
        assert!(matches!(decode_first(&data, &limits), Err(DecodeError::Image(ImageError::TooLarge(1, 1)))));
        limits.max_bytes = 16;
        assert!(decode_first(&data, &limits).is_ok());
        assert!(decode_first(&data[..data.len() - 1], &limits).is_err());
        assert!(decode_first(&[], &limits).is_err());
    }
}
