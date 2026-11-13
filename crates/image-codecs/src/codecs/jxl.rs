//! Pure-Rust lossless JPEG XL (Modular) writer.
//!
//! Currently writes a bare JPEG XL codestream. The encoder accepts 8- or 16-bit
//! gray, RGB and RGBA samples, but does not write ICC/EXIF/XMP boxes.
//! Gray+alpha sources are expanded losslessly to RGBA by the export plan.
//! The flat export pipeline converts tagged RGB documents to sRGB first.

use std::panic::{AssertUnwindSafe, catch_unwind};

use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions as ZuneOptions};
use zune_jpegxl::JxlSimpleEncoder;

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};

const F: Format = Format::Jxl;

// zune-jpegxl 0.5.2 emits ExtraChannelInfo(all_default=true) for every alpha
// channel. In JPEG XL that flag means *8-bit* alpha, even when the modular
// payload was encoded from 16-bit samples. Replace only that metadata bundle,
// leaving the entire encoded frame and its TOC untouched. The replacement is
// exactly 16 bits longer, so subsequent byte-aligned data stays aligned.
//
// This deliberately verifies the pinned encoder's header layout rather than
// trying to parse or rewrite arbitrary JPEG XL codestreams. A different header
// must fail closed, not produce a file that silently corrupts transparency.
fn correct_16_bit_alpha_header(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CodecError> {
    fn read(bytes: &[u8], pos: &mut usize, count: usize) -> Option<u32> {
        let mut value = 0u32;
        for bit in 0..count {
            let offset = *pos;
            let byte = *bytes.get(offset / 8)?;
            value |= u32::from((byte >> (offset % 8)) & 1) << bit;
            *pos += 1;
        }
        Some(value)
    }

    fn read_dimension(bytes: &[u8], pos: &mut usize) -> Option<u32> {
        let bits = match read(bytes, pos, 2)? {
            0 => 9,
            1 => 13,
            2 => 18,
            _ => 30,
        };
        read(bytes, pos, bits)?.checked_add(1)
    }

    fn write_bits(out: &mut Vec<u8>, pos: &mut usize, count: usize, value: u32) {
        for bit in 0..count {
            if (*pos).is_multiple_of(8) {
                out.push(0);
            }
            out[*pos / 8] |= (((value >> bit) & 1) as u8) << (*pos % 8);
            *pos += 1;
        }
    }

    let mut pos = 0;
    let header_matches = read(bytes, &mut pos, 16) == Some(0x0aff)
        && read(bytes, &mut pos, 1) == Some(0)
        && read_dimension(bytes, &mut pos) == Some(height)
        && read(bytes, &mut pos, 3) == Some(0)
        && read_dimension(bytes, &mut pos) == Some(width)
        && read(bytes, &mut pos, 1) == Some(0) // metadata not all-default
        && read(bytes, &mut pos, 1) == Some(0) // no extra fields
        && read(bytes, &mut pos, 1) == Some(0) // integer samples
        && read(bytes, &mut pos, 2) == Some(3) // custom bit depth
        && read(bytes, &mut pos, 6) == Some(15) // 16 bits
        && read(bytes, &mut pos, 1) == Some(0) // uses >14 bits
        && read(bytes, &mut pos, 2) == Some(1); // one extra channel
    let alpha_flag_pos = pos;
    let header_matches = header_matches
        && read(bytes, &mut pos, 1) == Some(1) // incorrect default 8-bit alpha
        && read(bytes, &mut pos, 1) == Some(0) // not XYB
        && read(bytes, &mut pos, 1) == Some(1) // default sRGB encoding (RGBA)
        && read(bytes, &mut pos, 2) == Some(0) // no extensions
        && read(bytes, &mut pos, 1) == Some(1); // default transform

    let frame_offset = pos.div_ceil(8);
    let header_matches = header_matches && frame_offset <= bytes.len() && (pos..frame_offset * 8).all(|bit| bytes[bit / 8] & (1 << (bit % 8)) == 0);
    if !header_matches {
        return Err(CodecError::encode(F, "unexpected zune-jpegxl header for 16-bit alpha"));
    }

    let mut fixed = Vec::with_capacity(bytes.len() + 2);
    let mut cursor = 0;
    for bit in 0..alpha_flag_pos {
        write_bits(&mut fixed, &mut cursor, 1, u32::from((bytes[bit / 8] >> (bit % 8)) & 1));
    }

    // JPEG XL ExtraChannelInfo, non-default, straight 16-bit alpha:
    // type=kAlpha (Enum=0), bit_depth=uint16, dim_shift=0, name_len=0,
    // alpha_associated=false. Adds exactly 16 bits versus all_default=true.
    write_bits(&mut fixed, &mut cursor, 1, 0);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 1, 0);
    write_bits(&mut fixed, &mut cursor, 2, 3);
    write_bits(&mut fixed, &mut cursor, 6, 15);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 2, 0);
    write_bits(&mut fixed, &mut cursor, 1, 0);

    for bit in alpha_flag_pos + 1..frame_offset * 8 {
        write_bits(&mut fixed, &mut cursor, 1, u32::from((bytes[bit / 8] >> (bit % 8)) & 1));
    }
    if cursor != (frame_offset + 2) * 8 {
        return Err(CodecError::encode(F, "JPEG XL alpha header alignment mismatch"));
    }
    fixed.extend_from_slice(&bytes[frame_offset..]);
    Ok(fixed)
}

pub(crate) fn encode(image: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let image = image.converted(plan.layout, plan.sample);
    let space = match image.layout() {
        ChannelLayout::Gray => ColorSpace::Luma,
        ChannelLayout::GrayA => ColorSpace::LumaA,
        ChannelLayout::Rgb => ColorSpace::RGB,
        ChannelLayout::Rgba => ColorSpace::RGBA,
        _ => return Err(CodecError::unsupported(F, "JPEG XL export supports grayscale and RGB, with optional alpha")),
    };
    let depth = match image.sample_type() {
        SampleType::U8 => BitDepth::Eight,
        SampleType::U16 => BitDepth::Sixteen,
        _ => return Err(CodecError::unsupported(F, "JPEG XL lossless export supports 8 or 16-bit integer pixels")),
    };
    let (width, height) = image.dimensions();
    let samples = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(image.layout().channels()))
        .and_then(|n| n.checked_mul(image.sample_type().bytes()))
        .ok_or_else(|| CodecError::encode(F, "image dimensions overflow the buffer size"))?;
    if image.data().len() != samples {
        return Err(CodecError::InvalidImage("incorrect JPEG XL pixel buffer length".into()));
    }
    // A panic in an external image encoder must not terminate the host application.
    catch_unwind(AssertUnwindSafe(|| {
        let options = ZuneOptions::new(width as usize, height as usize, space, depth);
        let mut output = Vec::new();
        JxlSimpleEncoder::new(image.data(), options).encode(&mut output).map_err(|e| CodecError::encode(F, e.to_string()))?;
        if depth == BitDepth::Sixteen && image.layout() == ChannelLayout::Rgba { correct_16_bit_alpha_header(&output, width, height) } else { Ok(output) }
    }))
    .unwrap_or_else(|_| Err(CodecError::encode(F, "the JPEG XL encoder failed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_header_correction_rejects_unexpected_or_truncated_headers() {
        for bytes in [&[][..], &[0xff, 0x0a][..], &[0; 32][..]] {
            assert!(correct_16_bit_alpha_header(bytes, 2, 2).is_err());
        }
    }
}
