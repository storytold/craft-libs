//! First-surface DDS decoding to unmodified RGBA floats.
//!
//! Adapted from MaterialCraft `dds_image.rs` (originally chasse20, cc052aeaa8c61fbe6975674b6d5e28bb3c597b9d).
//! Uses the same `dds` backend and first-surface semantics, including the first cube face or
//! volume slice. This is not a whole-resource decoder. Transfer functions and image policy
//! belong to callers. [`Limits`] bounds the output float plane, not backend scratch buffers.

use crate::{ImageError, Kind, Limits};

/// A malformed DDS/backend failure or rejected output allocation.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// Output size or allocation failed shared image limits.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// The decoder rejected input or output geometry.
    #[error("DDS decode failed: {0}")]
    Decode(String),
}

/// Unmodified backend output. Alpha and out-of-range float values are not clamped.
#[derive(Debug)]
pub struct Surface {
    /// Width of the first surface in pixels.
    pub width: u32,
    /// Height of the first surface in pixels.
    pub height: u32,
    /// Row-major RGBA float samples without transfer conversion.
    pub rgba: Vec<f32>,
    /// Exactly the backend's `Precision::F32` classification, as used by MaterialCraft.
    pub source_precision_is_f32: bool,
}

struct Bc3Prepared {
    bytes: Vec<u8>,
    restore_rgb: Vec<(usize, usize)>,
}

/// Make dds 0.2 decode BC3 colour in the four-colour mode required by the format. Microsoft's
/// *Textures with alpha channels* says BC2/BC3 never use BC1's colour comparison:
/// https://learn.microsoft.com/en-us/windows/uwp/graphics-concepts/textures-with-alpha-channels
///
/// Reversed endpoints can be exchanged while swapping indices 0↔1 and 2↔3 without changing the
/// four-colour table. Equal endpoints need index 3 restored from an endpoint-colour texel after
/// decoding. Only the bounded first surface is copied; alpha bytes are never changed.
fn bc3_four_colour_surface(bytes: &[u8], payload_start: usize, size: ::dds::Size) -> Option<Bc3Prepared> {
    let width = size.width as usize;
    let height = size.height as usize;
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let surface_bytes = blocks_x.checked_mul(blocks_y)?.checked_mul(16)?;
    let end = payload_start.checked_add(surface_bytes)?;
    if end > bytes.len() {
        return None;
    }
    let mut transformed = bytes[..end].to_vec();
    let mut repairs = Vec::new();
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let block_start = payload_start + (by * blocks_x + bx) * 16;
            let block = &mut transformed[block_start..block_start + 16];
            let color0 = u16::from_le_bytes([block[8], block[9]]);
            let color1 = u16::from_le_bytes([block[10], block[11]]);
            if color0 < color1 {
                block[8..10].copy_from_slice(&color1.to_le_bytes());
                block[10..12].copy_from_slice(&color0.to_le_bytes());
                for indices in &mut block[12..16] {
                    *indices ^= 0x55;
                }
                continue;
            }
            if color0 > color1 {
                continue;
            }
            let mut source = None;
            let mut targets = Vec::new();
            for ly in 0..4 {
                for lx in 0..4 {
                    let (x, y) = (bx * 4 + lx, by * 4 + ly);
                    if x >= width || y >= height {
                        continue;
                    }
                    let texel = ly * 4 + lx;
                    let index = (block[12 + texel / 4] >> (2 * (texel % 4))) & 3;
                    let output = y * width + x;
                    if index == 3 {
                        targets.push(output);
                    } else if source.is_none() {
                        source = Some(output);
                    }
                }
            }
            if targets.is_empty() {
                continue;
            }
            let source = source.unwrap_or_else(|| {
                // Every valid texel used index 3. Decode one as index 0 instead; all four
                // choices have the same endpoint colour, and its alpha is independent.
                let first = targets[0];
                let (x, y) = (first % width, first / width);
                let texel = (y % 4) * 4 + x % 4;
                block[12 + texel / 4] &= !(3 << (2 * (texel % 4)));
                first
            });
            repairs.extend(targets.into_iter().map(|target| (source, target)));
        }
    }
    Some(Bc3Prepared { bytes: transformed, restore_rgb: repairs })
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
    let mut repairs = Vec::new();
    let mut transformed: Option<Vec<u8>> = None;
    if decoder.format() == ::dds::Format::BC3_UNORM {
        let mut header_reader = std::io::Cursor::new(bytes);
        let _ = ::dds::header::Header::read(&mut header_reader, &Default::default()).map_err(|e| DecodeError::Decode(e.to_string()))?;
        if let Some(prepared) = bc3_four_colour_surface(bytes, header_reader.position() as usize, size) {
            repairs = prepared.restore_rgb;
            transformed = Some(prepared.bytes);
        }
    }
    if let Some(data) = transformed.as_deref() {
        let mut reader = std::io::Cursor::new(data);
        let header = ::dds::header::Header::read(&mut reader, &Default::default()).map_err(|e| DecodeError::Decode(e.to_string()))?;
        decoder = ::dds::Decoder::from_header(reader, header).map_err(error)?;
    }
    let view = ::dds::ImageViewMut::new(bytemuck::cast_slice_mut(&mut rgba), size, ::dds::ColorFormat::RGBA_F32)
        .ok_or_else(|| DecodeError::Decode("invalid DDS output dimensions".into()))?;
    decoder.read_surface(view).map_err(error)?;
    for (source, target) in repairs {
        let rgb = rgba.get(source * 4..source * 4 + 3).and_then(|samples| <[f32; 3]>::try_from(samples).ok());
        if let (Some(rgb), Some(target)) = (rgb, rgba.get_mut(target * 4..target * 4 + 3)) {
            target.copy_from_slice(&rgb);
        }
    }
    Ok(Surface { width: size.width, height: size.height, rgba, source_precision_is_f32: decoder.format().precision() == ::dds::Precision::F32 })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn bytes(format: ::dds::Format, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        ::dds::header::Header::new_image(1, 1, format).write(&mut bytes).unwrap();
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
    fn half_float_and_compressed_pixels_preserve_backend_values() {
        let half: Vec<u8> = [0xb800u16, 0x4000, 0x3400, 0x3a00].iter().flat_map(|v| v.to_le_bytes()).collect();
        let image = decode_first(&bytes(::dds::Format::R16G16B16A16_FLOAT, &half), &Limits::default()).unwrap();
        assert_eq!(image.rgba, [-0.5, 2.0, 0.25, 0.75]);
        assert_eq!(image.source_precision_is_f32, ::dds::Format::R16G16B16A16_FLOAT.precision() == ::dds::Precision::F32);
        let mut compressed = Vec::new();
        ::dds::header::Header::new_image(4, 4, ::dds::Format::BC1_UNORM).write(&mut compressed).unwrap();
        compressed.extend_from_slice(&[0, 0xf8, 0, 0, 0, 0, 0, 0]);
        let image = decode_first(&compressed, &Limits::default()).unwrap();
        assert!(!image.source_precision_is_f32);
        assert!(image.rgba.as_chunks::<4>().0.iter().all(|p| *p == [1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn cube_keeps_first_surface_only_like_materialcraft() {
        let mut data = Vec::new();
        ::dds::header::Header::new_cube_map(1, 1, ::dds::Format::R8G8B8A8_UNORM).write(&mut data).unwrap();
        data.extend_from_slice(&[255, 0, 0, 255]);
        data.extend_from_slice(&[0; 20]);
        let image = decode_first(&data, &Limits::default()).unwrap();
        assert_eq!(image.rgba, [1.0, 0.0, 0.0, 1.0]);
    }

    fn bc3_sized(width: u32, height: u32, color0: u16, color1: u16, indices: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        ::dds::header::Header::new_image(width, height, ::dds::Format::BC3_UNORM).write(&mut data).unwrap();
        for _ in 0..width.div_ceil(4) * height.div_ceil(4) {
            // Nontrivial alpha endpoints and indices: the color repair must leave these alone.
            data.extend_from_slice(&[255, 0, 0xe4, 0x93, 0x3b, 0x62, 0x24, 0x49]);
            data.extend_from_slice(&color0.to_le_bytes());
            data.extend_from_slice(&color1.to_le_bytes());
            data.extend_from_slice(&indices);
        }
        data
    }

    fn bc3(color0: u16, color1: u16, indices: [u8; 4]) -> Vec<u8> {
        bc3_sized(4, 4, color0, color1, indices)
    }

    #[test]
    fn bc3_always_uses_four_color_interpolation() {
        // Reversing both endpoints and swapping color indices 0↔1 and 2↔3 is exactly
        // equivalent in four-color mode. dds 0.2 instead selects BC1's transparent mode here.
        let indices = [0b11_10_01_00, 0b00_01_10_11, 0b10_11_00_01, 0b01_00_11_10];
        let broken = decode_first(&bc3(0x001f, 0xf800, indices), &Limits::default()).unwrap().rgba;
        let expected_indices = indices.map(|byte| byte ^ 0x55);
        let expected = decode_first(&bc3(0xf800, 0x001f, expected_indices), &Limits::default()).unwrap().rgba;
        assert_eq!(broken, expected, "BC3 color must not use BC1's transparent mode");

        // Equal endpoints still have four identical color choices, including index 3.
        let broken = decode_first(&bc3(0x07e0, 0x07e0, [0xff; 4]), &Limits::default()).unwrap().rgba;
        let expected = decode_first(&bc3(0x07e0, 0x07e0, [0x00; 4]), &Limits::default()).unwrap().rgba;
        assert_eq!(broken, expected, "index 3 is the endpoint color, not transparent black");

        // A partial right/bottom block follows the same rule, including its cropped texels.
        let broken = decode_first(&bc3_sized(5, 3, 0x001f, 0xf800, [0xff; 4]), &Limits::default()).unwrap().rgba;
        let expected = decode_first(&bc3_sized(5, 3, 0xf800, 0x001f, [0xaa; 4]), &Limits::default()).unwrap().rgba;
        assert_eq!(broken, expected, "partial BC3 blocks also use four-color interpolation");
        let broken = decode_first(&bc3_sized(5, 3, 0x07e0, 0x07e0, [0xff; 4]), &Limits::default()).unwrap().rgba;
        let expected = decode_first(&bc3_sized(5, 3, 0x07e0, 0x07e0, [0x00; 4]), &Limits::default()).unwrap().rgba;
        assert_eq!(broken, expected, "partial equal-endpoint blocks keep index 3's color and alpha");
    }

    #[test]
    fn checks_output_budget_and_truncation() {
        let data = bytes(::dds::Format::R8G8B8A8_UNORM, &[1, 2, 3, 4]);
        let mut limits = Limits { max_bytes: 15, ..Limits::default() };
        assert!(matches!(decode_first(&data, &limits), Err(DecodeError::Image(ImageError::TooLarge(1, 1)))));
        limits.max_bytes = 16;
        assert!(decode_first(&data, &limits).is_ok());
        assert!(decode_first(&data[..data.len() - 1], &limits).is_err());
        assert!(decode_first(&[], &limits).is_err());
    }
}
