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
