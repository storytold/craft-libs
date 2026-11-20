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

/// A little-endian TIFF with one IFD of (tag, type, count, value) entries and `tail` after it.
fn ifd(entries: &[(u16, u16, u32, [u8; 4])], tail: &[u8]) -> Vec<u8> {
    let mut b = b"II\x2a\0\x08\0\0\0".to_vec();
    b.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, ty, count, value) in entries {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(value);
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(tail);
    b
}

#[test]
fn sony_lossless_ycbcr_needs_a_sony_make_and_every_layout_tag() {
    let short = |v: u16| [v.to_le_bytes()[0], v.to_le_bytes()[1], 0, 0];
    let layout = [(0x828e, 1, 4, [0, 1, 1, 2]), (0x0103, 3, 1, short(7)), (0x0106, 3, 1, short(6)), (0x0115, 3, 1, short(3))];
    // Make stored after the IFD (ASCII, 5 bytes): 8 + 2 + 5 * 12 + 4 = 74.
    let with_make = |make: (u16, u16, u32, [u8; 4]), tail: &[u8]| ifd(&[&[make], &layout[..]].concat(), tail);
    assert!(legacy_tiff_raw_hint(&with_make((0x010f, 2, 5, 74u32.to_le_bytes()), b"SONY\0")));
    assert!(legacy_tiff_raw_hint(&with_make((0x010f, 7, 4, *b"sony"), b"")));
    assert!(!legacy_tiff_raw_hint(&with_make((0x010f, 2, 6, 74u32.to_le_bytes()), b"Nikon\0")));
    assert!(!legacy_tiff_raw_hint(&with_make((0x010f, 99, 4, *b"SONY"), b"")));
    assert!(!legacy_tiff_raw_hint(&with_make((0x010f, 2, 5, 9999u32.to_le_bytes()), b"")));
    // A Sony file without the full lossless layout is not a RAW by this rule.
    let partial = ifd(&[(0x010f, 7, 4, *b"SONY"), layout[0], layout[1], layout[2]], b"");
    assert!(!legacy_tiff_raw_hint(&partial));
    // Repeated tags: the first value wins.
    let repeated = ifd(&[&[(0x010f, 7, 4, *b"SONY")], &layout[..], &[(0x0103, 3, 1, short(1))]].concat(), b"");
    assert!(legacy_tiff_raw_hint(&repeated));
}

#[test]
fn oversized_or_cut_directories_are_skipped() {
    let mut huge = b"II\x2a\0\x08\0\0\0".to_vec();
    huge.extend_from_slice(&5000u16.to_le_bytes());
    assert!(!legacy_tiff_raw_hint(&huge));
    let mut cut = ifd(&[(0x010f, 7, 4, *b"SONY"), (0x0103, 3, 1, [7, 0, 0, 0])], b"");
    cut.truncate(20);
    assert!(!legacy_tiff_raw_hint(&cut));
}

#[test]
fn a_subifds_array_without_its_offset_is_skipped() {
    // IFD0 at 64 holds one SubIFDs entry (LONG, count 3) whose offset field is cut off. A DNG
    // directory sits at 32, and the bytes at 8 point to it. Reading the missing offset as 0
    // would treat the TIFF header as the pointer array and follow 8 -> 32.
    let mut b = vec![0u8; 64];
    b[..8].copy_from_slice(b"II\x2a\0\x40\0\0\0");
    b[8..12].copy_from_slice(&32u32.to_le_bytes());
    b[32..34].copy_from_slice(&1u16.to_le_bytes());
    b[34..46].copy_from_slice(&[0x12, 0xc6, 1, 0, 4, 0, 0, 0, 1, 4, 0, 0]);
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&[0x4a, 0x01, 4, 0, 3, 0, 0, 0]);
    assert_eq!(b.len(), 74);
    assert!(!legacy_tiff_raw_hint(&b));
    // With the offset present (the array at 8), the same SubIFD is followed.
    let mut whole = b.clone();
    whole.extend_from_slice(&8u32.to_le_bytes());
    assert!(legacy_tiff_raw_hint(&whole));
}
