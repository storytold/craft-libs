#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use craft_image_codecs::{ChannelLayout, Format, Image};
use std::sync::Arc;

fn kind(format: Format, bytes: Vec<u8>) -> usvg::ImageKind {
    let bytes = Arc::new(bytes);
    match format {
        Format::Png => usvg::ImageKind::PNG(bytes),
        Format::Jpeg => usvg::ImageKind::JPEG(bytes),
        Format::Gif => usvg::ImageKind::GIF(bytes),
        Format::WebP => usvg::ImageKind::WEBP(bytes),
        _ => panic!("test raster format"),
    }
}

fn encoded(format: Format) -> Vec<u8> {
    let image = Image::from_u8(2, 2, ChannelLayout::Rgb, [17, 83, 149].repeat(4)).unwrap();
    craft_image_codecs::encode(&image, format, &Default::default()).unwrap()
}

fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(payload);
}

// ISO/IEC10918-1 framing, one gray 8x8 block. The single one-bit Huffman codes
// encode DC category 0 and AC EOB. No third-party asset or camera data is used.
fn gray_jpeg(bits: u8, entropy: &[u8]) -> Vec<u8> {
    let mut data = vec![0xff, 0xd8];
    let mut quantization = vec![0];
    quantization.extend_from_slice(&[1; 64]);
    segment(&mut data, 0xdb, &quantization);
    segment(&mut data, if bits == 8 { 0xc0 } else { 0xc1 }, &[bits, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    let mut tables = Vec::new();
    for class in [0x00, 0x10] {
        tables.extend_from_slice(&[class, 1]);
        tables.extend_from_slice(&[0; 15]);
        tables.push(0);
    }
    segment(&mut data, 0xc4, &tables);
    segment(&mut data, 0xda, &[1, 1, 0, 0, 63, 0]);
    data.extend_from_slice(entropy);
    data.extend_from_slice(&[0xff, 0xd9]);
    data
}

fn corrupt_jpeg() -> Vec<u8> {
    let mut data = gray_jpeg(8, &[0x3f]);
    let scan = data.windows(2).position(|marker| marker == b"\xff\xda").unwrap();
    // Keep the complete scan header and valid entropy, but reference table 1
    // while DHT defines only table 0. The renderer accepts these headers, then
    // fails pixel decoding when checking the selected Huffman tables. Random
    // all-one entropy is tolerated by its default policy and is not this guard.
    assert_eq!(data[scan + 6], 0);
    data[scan + 6] = 0x11;
    data
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn corrupt_png() -> Vec<u8> {
    let mut data = encoded(Format::Png);
    let mut at = 8;
    loop {
        let length = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        if &data[at + 4..at + 8] == b"IDAT" {
            // Keep the complete container and an honest CRC, but destroy the
            // zlib header in the pixel stream rather than the dimension header.
            assert!(length >= 2);
            data[at + 8..at + 10].fill(0);
            let crc = crc32(&data[at + 4..at + 8 + length]);
            data[at + 8 + length..at + 12 + length].copy_from_slice(&crc.to_be_bytes());
            return data;
        }
        at += length + 12;
    }
}

fn corrupt_gif() -> Vec<u8> {
    // Complete 2x2 GIF89a framing and palette, followed by a code7 when only
    // codes 0..5 are initially defined by an LZW minimum code size of 2.
    let mut data = b"GIF89a".to_vec();
    data.extend_from_slice(&[2, 0, 2, 0, 0x80, 0, 0]);
    data.extend_from_slice(&[0, 0, 0, 0, 255, 0]);
    data.extend_from_slice(&[0x2c, 0, 0, 0, 0, 2, 0, 2, 0, 0]);
    data.extend_from_slice(&[2, 1, 7, 0, 0x3b]);
    data
}

fn corrupt_webp() -> Vec<u8> {
    // Complete RIFF/VP8L dimensions, with no transform/Huffman/pixel bits.
    // Chunk length and odd-byte padding remain internally consistent.
    let mut data = b"RIFF".to_vec();
    data.extend_from_slice(&18u32.to_le_bytes());
    data.extend_from_slice(b"WEBPVP8L");
    data.extend_from_slice(&5u32.to_le_bytes());
    data.extend_from_slice(&[0x2f, 1, 0x40, 0, 0, 0]);
    data
}

#[test]
fn renderer_validator_accepts_complete_png_jpeg_gif_and_webp_at_exact_pixel_allowance() {
    for format in [Format::Png, Format::Jpeg, Format::Gif, Format::WebP] {
        let data = encoded(format);
        assert_eq!(craft_image_codecs::probe_allocation_size(&data).unwrap(), (2, 2));
        let mut remaining = 4;
        assert_eq!(validate_raster(&kind(format, data), 4, &mut remaining), Ok(()), "{format:?}");
        assert_eq!(remaining, 0, "{format:?}: exact area must be charged");
    }
}

#[test]
fn renderer_validator_rejects_corrupt_pixels_after_valid_dimensions_without_refunding_work() {
    let cases = [
        (Format::Png, corrupt_png(), (2, 2)),
        // Header-only admission cannot detect this missing scan-table dependency.
        (Format::Jpeg, corrupt_jpeg(), (8, 8)),
        (Format::Gif, corrupt_gif(), (2, 2)),
        (Format::WebP, corrupt_webp(), (2, 2)),
    ];
    for (format, data, dimensions) in cases {
        assert_eq!(craft_image_codecs::probe_allocation_size(&data).unwrap(), dimensions, "{format:?}");
        match format {
            Format::Gif => {
                gif::DecodeOptions::new().read_info(data.as_slice()).expect("intact GIF headers before corrupt LZW pixels");
            }
            Format::WebP => {
                image_webp::WebPDecoder::new(Cursor::new(&data)).expect("intact WebP headers before missing pixel bits");
            }
            _ => {}
        }
        let pixels = u64::from(dimensions.0) * u64::from(dimensions.1);
        let image = kind(format, data);
        let mut remaining = pixels * 2;
        for left in [pixels, 0] {
            assert_eq!(validate_raster(&image, pixels, &mut remaining), Err(RasterValidationFailure::Invalid), "{format:?}");
            assert_eq!(remaining, left, "{format:?}: failed decode work is nonrefundable");
        }
        assert_eq!(validate_raster(&image, pixels, &mut remaining), Err(RasterValidationFailure::Limit), "{format:?}");
        assert_eq!(remaining, 0);
    }
}

#[test]
fn twelve_bit_jpeg_is_valid_for_the_general_codec_but_not_the_svg_renderer() {
    for bits in [8, 12] {
        // DC=0, EOB=0, then six one-pad bits: a constant mid-gray block.
        let data = gray_jpeg(bits, &[0x3f]);
        let decoded = craft_image_codecs::decode(&data).unwrap();
        assert_eq!(decoded.dimensions(), (8, 8));
        let mut remaining = 64;
        let expected = if bits == 8 { Ok(()) } else { Err(RasterValidationFailure::Invalid) };
        assert_eq!(validate_raster(&kind(Format::Jpeg, data), 64, &mut remaining), expected, "{bits}-bit");
        assert_eq!(remaining, 0, "capability refusal still spends attempted decode work");
    }
}

#[test]
fn renderer_validator_refuses_unadmitted_headers_and_pixels_without_charging_unattempted_decode() {
    for format in [Format::Png, Format::Jpeg, Format::Gif, Format::WebP] {
        let image = kind(format, encoded(format));
        let mut remaining = 4;
        assert_eq!(validate_raster(&image, 3, &mut remaining), Err(RasterValidationFailure::Limit), "{format:?}: per-image ceiling");
        assert_eq!(remaining, 4);
        remaining = 3;
        assert_eq!(validate_raster(&image, 4, &mut remaining), Err(RasterValidationFailure::Limit), "{format:?}: cumulative allowance");
        assert_eq!(remaining, 3);
        assert_eq!(
            validate_raster(&kind(format, b"invalid raster header".to_vec()), 4, &mut remaining),
            Err(RasterValidationFailure::Invalid),
            "{format:?}"
        );
        assert_eq!(remaining, 3, "invalid dimensions cannot reserve an invented area");
    }
}
