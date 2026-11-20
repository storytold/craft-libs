use craft_dds::{DdsCompression, DdsOptions, Layout, Texture, encode};
#[test]
fn metadata_requires_a_complete_header() {
    for (compression, layout, channels, header) in [(DdsCompression::None, Layout::Gray, 1, 128), (DdsCompression::Bc4, Layout::Gray, 1, 148)] {
        let data = encode(4, 4, layout, &vec![7; 16 * channels], DdsOptions { compression, mipmaps: false }).unwrap();
        for n in 0..header {
            assert!(Texture::parse(&data[..n]).is_err(), "accepted {n}/{header} header bytes");
        }
        assert!(Texture::parse(&data[..header]).is_ok());
    }
}
#[test]
fn empty_dimensions_are_not_textures() {
    for (w, h) in [(0, 0), (0, 4), (4, 0)] {
        assert!(encode(w, h, Layout::Gray, &[], DdsOptions::default()).is_err(), "encoded {w}x{h}");
    }
}
#[test]
fn decoded_output_respects_explicit_budget() {
    let data = encode(4, 4, Layout::Rgba, &[7; 64], DdsOptions { compression: DdsCompression::Bc1, mipmaps: false }).unwrap();
    let texture = Texture::parse(&data).unwrap();
    assert!(matches!(texture.decode_with_limit(63), Err(craft_dds::DdsError::LimitExceeded { required: 64, limit: 63 })));
    assert_eq!(texture.decode_with_limit(64).unwrap().len(), 64);
    assert_eq!(texture.decode().unwrap(), texture.decode_with_limit(u64::MAX).unwrap());
}
#[test]
fn parse_rejects_empty_dimensions() {
    let valid = encode(1, 1, Layout::Gray, &[7], DdsOptions::default()).unwrap();
    for (w, h) in [(0u32, 0u32), (0, 4), (4, 0)] {
        let mut data = valid.clone();
        data[12..16].copy_from_slice(&h.to_le_bytes());
        data[16..20].copy_from_slice(&w.to_le_bytes());
        assert!(Texture::parse(&data).is_err());
    }
}
#[test]
fn default_budget_rejects_oversized_header_before_payload_or_allocation() {
    let mut data = encode(1, 1, Layout::Gray, &[7], DdsOptions::default()).unwrap();
    data.truncate(128);
    data[12..16].copy_from_slice(&65536u32.to_le_bytes());
    data[16..20].copy_from_slice(&32768u32.to_le_bytes());
    data[20..24].copy_from_slice(&32768u32.to_le_bytes());
    let texture = Texture::parse(&data).unwrap();
    assert!(matches!(texture.decode(), Err(craft_dds::DdsError::LimitExceeded { limit: craft_dds::DEFAULT_MAX_DECODE_BYTES, .. })));
    assert!(matches!(texture.decode_with_limit(u64::MAX), Err(craft_dds::DdsError::Malformed(_))));
}

#[test]
fn mip_views_reuse_shared_decoders_and_keep_their_bounds() {
    for (compression, layout, channels) in [
        (DdsCompression::None, Layout::Gray, 1),
        (DdsCompression::Bc1, Layout::Rgba, 4),
        (DdsCompression::Bc4, Layout::Gray, 1),
        (DdsCompression::Bc5, Layout::Rgb, 3),
    ] {
        let bytes = encode(4, 4, layout, &vec![71; 16 * channels], DdsOptions { compression, mipmaps: true }).unwrap();
        let texture = Texture::parse(&bytes).unwrap();
        assert_eq!(texture.levels(), 3);
        assert_eq!(texture.compression(), compression);
        assert!(texture.mip_level(3).is_err());
        assert!(texture.mip_level(u32::MAX).is_err());
        for (index, side) in [(0, 4), (1, 2), (2, 1)] {
            let level = texture.mip_level(index).unwrap();
            assert_eq!((level.width(), level.height()), (side, side));
            let len = side as usize * side as usize * channels;
            assert!(matches!(level.decode_with_limit((len - 1) as u64), Err(craft_dds::DdsError::LimitExceeded { .. })));
            assert_eq!(level.decode_with_limit(len as u64).unwrap().len(), len);
        }
        let nested = texture.mip_level(1).unwrap().mip_level(1).unwrap();
        assert_eq!(nested.decode().unwrap(), texture.mip_level(2).unwrap().decode().unwrap());
    }
}

#[test]
fn sixteen_bit_mip_views_preserve_samples_and_stored_offsets() {
    for (layout, channels) in [(Layout::Gray, 1), (Layout::GrayA, 2), (Layout::Rgba, 4)] {
        // Two channels are stored as R16G16 and read back as red and green with blue 0.
        let read =
            |samples: Vec<u16>| -> Vec<u16> { if channels == 2 { samples.chunks(2).flat_map(|p| [p[0], p[1], 0]).collect() } else { samples } };
        let pixels: Vec<u16> = (0..16).flat_map(|p| (0..channels).map(move |c| (p * 400 + c * 31 + 7) as u16)).collect();
        let bytes = craft_dds::encode16(4, 4, layout, &pixels, DdsOptions { compression: DdsCompression::None, mipmaps: true }).unwrap();
        let texture = Texture::parse(&bytes).unwrap();
        assert_eq!(texture.compression(), DdsCompression::None);
        assert_eq!(texture.levels(), 3);
        assert_eq!(texture.decode16().unwrap(), read(pixels));
        for (index, averages) in [(1, vec![1000, 1800, 4200, 5000]), (2, vec![3000])] {
            let expected = read(averages.iter().flat_map(|p| (0..channels).map(move |c| (p + c * 31 + 7) as u16)).collect());
            let level = texture.mip_level(index).unwrap();
            let budget = (expected.len() * 2) as u64;
            assert!(matches!(level.decode16_with_limit(budget - 1), Err(craft_dds::DdsError::LimitExceeded { .. })));
            assert_eq!(level.decode16_with_limit(budget).unwrap(), expected);
            let rounded: Vec<u8> = expected.iter().map(|&v| ((u32::from(v) + 128) / 257) as u8).collect();
            assert_eq!(level.decode().unwrap(), rounded);
        }
        assert_eq!(texture.mip_level(1).unwrap().mip_level(1).unwrap().decode16().unwrap(), texture.mip_level(2).unwrap().decode16().unwrap());
        let truncated = Texture::parse(&bytes[..bytes.len() - 1]).unwrap();
        assert!(truncated.mip_level(2).unwrap().decode16().is_err());
    }
}
