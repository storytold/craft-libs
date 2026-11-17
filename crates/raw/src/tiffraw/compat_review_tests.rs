use super::*;
use craft_tiff::image::Layout;

fn image(width: u32, height: u32, bits: u16, cpp: u16, compression: u16, len: usize) -> ImageInfo {
    ImageInfo {
        width,
        height,
        bits_per_sample: vec![bits; cpp as usize],
        samples_per_pixel: cpp,
        compression,
        photometric: 34892,
        planar: 1,
        predictor: 1,
        sample_format: 1,
        new_subfile_type: 0,
        layout: Layout::Strips { rows_per_strip: height },
        offsets: vec![0],
        byte_counts: vec![len as u64],
    }
}

#[test]
fn truncated_uncompressed_rows_are_rejected_in_full_and_header_modes() {
    for (bits, packing, float, bytes) in [
        (8, Packing::Msb, false, 8),
        (12, Packing::Msb, false, 12),
        (12, Packing::Word16, false, 16),
        (16, Packing::Msb, false, 16),
        (32, Packing::Msb, true, 32),
    ] {
        let data = vec![0u8; bytes];
        let mut info = image(4, 2, bits, 1, comp::NONE, bytes);
        info.sample_format = if float { 3 } else { 1 };
        assert!(read_image_in(crate::Mode::Full, &data, &info, ByteOrder::Little, packing, &Limits::default()).is_ok());
        info.byte_counts[0] -= 1;
        for mode in [crate::Mode::Full, crate::Mode::Header] {
            assert!(
                matches!(read_image_in(mode, &data[..bytes - 1], &info, ByteOrder::Little, packing, &Limits::default()), Err(RawError::Corrupt(message)) if message.contains("uncompressed chunk")),
                "{bits}/{float}/{mode:?}"
            );
        }
    }
}

#[test]
fn edge_padding_may_be_absent_but_visible_uncompressed_samples_must_all_exist() {
    let data = vec![31u8; 16 * 7 + 8];
    let mut info = image(8, 8, 8, 1, comp::NONE, data.len());
    info.layout = Layout::Tiles { tile_width: 16, tile_height: 16 };
    let RawData::U16(values) = read_image(&data, &info, ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap() else { panic!("integer") };
    assert_eq!(values, vec![31; 64]);
    info.byte_counts[0] -= 1;
    assert!(read_image(&data[..data.len() - 1], &info, ByteOrder::Little, Packing::Msb, &Limits::default()).is_err());
}

#[test]
fn packed_predictor_two_wraps_at_declared_twelve_bits() {
    // Samples [250, 10, 4090, 20]; differences modulo4096 are 0FA,F10,FF0,01A.
    let packed = [0x0f, 0xaf, 0x10, 0xff, 0x00, 0x1a];
    let ChunkPx::U16(values) = unpack_chunk(&packed, ByteOrder::Little, Packing::Msb, 4, 1, 1, 12, false, 2).unwrap() else { panic!("integer") };
    assert_eq!(values, vec![250, 10, 4090, 20]);
}

#[allow(clippy::unwrap_used)] // Test-only fixture encoder; failed construction must fail the test.
fn jpeg(cpp: usize, width: u16, height: u16) -> Vec<u8> {
    let mut data = Vec::new();
    let color = if cpp == 1 { jpeg_encoder::ColorType::Luma } else { jpeg_encoder::ColorType::Rgb };
    jpeg_encoder::Encoder::new(&mut data, 100).encode(&vec![90; width as usize * height as usize * cpp], width, height, color).unwrap();
    data
}

#[test]
fn baseline_jpeg_components_are_checked_before_forced_colorspace_conversion() {
    for (actual, expected) in [(3, 1), (1, 3)] {
        let data = jpeg(actual, 8, 8);
        let error = lossy_jpeg(&data, expected, 1024).unwrap_err();
        assert!(matches!(error, RawError::Corrupt(message) if message.contains(&format!("{actual} components"))));
    }
    assert!(matches!(lossy_jpeg(&jpeg(3, 8, 8), 4, 1024), Err(RawError::Unsupported(message)) if message.contains("4 samples")));
    for cpp in [1, 3] {
        assert_eq!(lossy_jpeg(&jpeg(cpp, 8, 8), cpp, 1024).unwrap().3, cpp);
    }
}

#[test]
fn jpeg_tile_geometry_and_limits_are_checked_at_the_header_with_valid_edge_controls() {
    let data = jpeg(3, 8, 8);
    assert!(lossy_jpeg_with_tile(&data, 3, 1024, Some((16, 16, 8, 8))).is_ok());
    assert!(
        matches!(lossy_jpeg_with_tile(&data, 3, 1024, Some((16, 16, 9, 8))), Err(RawError::Corrupt(message)) if message.contains("does not cover"))
    );
    assert!(matches!(lossy_jpeg_with_tile(&data, 3, 32, Some((16, 16, 8, 8))), Err(RawError::Corrupt(message)) if message.contains("would exceed")));
    // Mutate only SOF dimensions, retaining a valid header but no entropy for the claimed frame.
    // The geometry error proves refusal precedes decode's short/corrupt entropy failure.
    for (width, height) in [(33u16, 8u16), (8, 33), (256, 256)] {
        let mut oversized = data.clone();
        let sof = oversized.windows(2).position(|bytes| bytes == [0xff, 0xc0]).unwrap();
        oversized[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
        oversized[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
        assert!(
            matches!(lossy_jpeg_with_tile(&oversized, 3, 1 << 20, Some((16, 16, 8, 8))), Err(RawError::Corrupt(message)) if message.contains("exceeds its"))
        );
    }
    // A genuine full MCU remains valid even when only a 4x4 part of the nominal16 tile is visible.
    let padded = jpeg(3, 32, 32);
    assert!(lossy_jpeg_with_tile(&padded, 3, 32 * 32 * 3, Some((16, 16, 4, 4))).is_ok());
    assert!(
        matches!(lossy_jpeg_with_tile(&data, 3, 1 << 20, Some((usize::MAX, 16, 8, 8))), Err(RawError::Corrupt(message)) if message.contains("overflow MCU padding"))
    );
    let mut info = image(8, 8, 8, 3, comp::LOSSY_JPEG, data.len());
    info.layout = Layout::Tiles { tile_width: 16, tile_height: 16 };
    let RawData::U16(values) = read_image(&data, &info, ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap() else { panic!("integer") };
    assert_eq!(values.len(), 8 * 8 * 3);
    assert!(values.iter().all(|sample| (i32::from(*sample) - 90).abs() <= 3));
}
