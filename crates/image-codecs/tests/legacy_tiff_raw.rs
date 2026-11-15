use craft_image_codecs::legacy_tiff_raw_hint;

fn tiff(tag: u16, ty: u16, value: u32) -> Vec<u8> {
    let mut b = b"II\x2a\0\x08\0\0\0\x01\0".to_vec();
    b.extend_from_slice(&tag.to_le_bytes());
    b.extend_from_slice(&ty.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&value.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

#[test]
fn recognizes_legacy_raw_compression_and_broad_cr_marker() {
    assert!(legacy_tiff_raw_hint(&tiff(0x0103, 3, 34713)));
    assert!(!legacy_tiff_raw_hint(&tiff(0x0103, 3, 1)));
    assert!(legacy_tiff_raw_hint(b"II*\0\x08\0\0\0CR"));
    let mut cut = tiff(0xc612, 1, 1);
    cut.truncate(17);
    assert!(!legacy_tiff_raw_hint(&cut));
}
