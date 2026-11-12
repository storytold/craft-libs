//! Orientation in Exif and XMP blocks: read, and rewritten to 1 in place.

use craft_metadata::edit::*;
use proptest::prelude::{any, prop_assert, prop_assert_eq, proptest};

/// A minimal EXIF (TIFF-structured) block: a filler tag, then Orientation.
fn exif(o: u32, big: bool, long: bool) -> Vec<u8> {
    let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut v = if big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
    v.extend_from_slice(&u32b(8));
    v.extend_from_slice(&u16b(2));
    // ImageDescription-ish filler (ASCII, 4 bytes inline).
    v.extend_from_slice(&u16b(0x010E));
    v.extend_from_slice(&u16b(2));
    v.extend_from_slice(&u32b(4));
    v.extend_from_slice(b"abc\0");
    v.extend_from_slice(&u16b(0x0112));
    if long {
        v.extend_from_slice(&u16b(4));
        v.extend_from_slice(&u32b(1));
        v.extend_from_slice(&u32b(o));
    } else {
        v.extend_from_slice(&u16b(3));
        v.extend_from_slice(&u32b(1));
        v.extend_from_slice(&u16b(o as u16));
        v.extend_from_slice(&[0, 0]);
    }
    v.extend_from_slice(&u32b(0));
    v
}

#[test]
fn reads_orientation_in_every_encoding() {
    for o in 1..=8 {
        for big in [false, true] {
            for long in [false, true] {
                let e = exif(o, big, long);
                assert_eq!(exif_orientation(&e), o as u16, "o={o} big={big} long={long}");
                let mut prefixed = b"Exif\0\0".to_vec();
                prefixed.extend_from_slice(&e);
                assert_eq!(exif_orientation(&prefixed), o as u16);
                let fixed = upright_exif(&prefixed);
                assert_eq!(exif_orientation(&fixed), 1);
                assert_eq!(fixed.len(), prefixed.len(), "rewritten in place");
                assert_eq!(&fixed[..6], b"Exif\0\0");
                // Rewritten in the entry's own type and width: exactly `exif(1, ..)`.
                assert_eq!(&fixed[6..], &exif(1, big, long)[..], "o={o} big={big} long={long}");
            }
        }
    }
    // Already upright, no tag, or garbage: borrowed unchanged.
    assert!(matches!(upright_exif(&exif(1, false, false)), std::borrow::Cow::Borrowed(_)));
    assert!(matches!(upright_exif(&ifd(false, &[(0x0131, 2, 4, *b"PC1\0")], &[])), std::borrow::Cow::Borrowed(_)));
    assert!(matches!(upright_exif(b"nonsense"), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn malformed_or_out_of_range_orientation_reads_as_1() {
    for bad in [0, 9, 255, 0xFFFF, 0x1_0006] {
        assert_eq!(exif_orientation(&exif(bad, false, true)), 1, "{bad}");
    }
    let good = exif(6, false, false);
    assert_eq!(exif_orientation(&good), 6);
    for n in 0..good.len() {
        // Any cut, even one that keeps the Orientation entry but loses the next-IFD pointer.
        assert_eq!(exif_orientation(&good[..n]), 1, "truncated at {n}");
        assert!(matches!(upright_exif(&good[..n]), std::borrow::Cow::Borrowed(_)), "truncated at {n}");
    }
    // IFD offset past the end, a huge entry count, a wrong field type.
    let mut far = good.clone();
    far[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(exif_orientation(&far), 1);
    let mut many = good.clone();
    many[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(exif_orientation(&many), 1, "an entry count past the end is a truncated table");
    let mut typed = good.clone();
    typed[24..26].copy_from_slice(&2u16.to_le_bytes()); // ASCII
    assert_eq!(exif_orientation(&typed), 1);
    assert_eq!(exif_orientation(&[]), 1);
    assert_eq!(exif_orientation(b"II*\0"), 1);
}

proptest! {
    #[test]
    fn random_exif_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256), le in any::<bool>()) {
        let mut b = if le { b"II*\0\x08\0\0\0".to_vec() } else { b"MM\0*\0\0\0\x08".to_vec() };
        b.extend_from_slice(&bytes);
        let o = exif_orientation(&b);
        prop_assert!((1..=8).contains(&o));
        prop_assert_eq!(exif_orientation(&upright_exif(&b)), 1);
        let _ = exif_orientation(&bytes);
        let _ = upright_exif(&bytes);
    }

    #[test]
    fn random_xmp_never_panics(s in ".{0,64}", tail in ".{0,16}") {
        let x = format!("{s}tiff:Orientation{tail}");
        let _ = upright_xmp(&x);
    }
}

#[test]
fn xmp_orientation_is_rewritten() {
    let attr = r#"<rdf:Description tiff:Orientation="6" tiff:Make="X"/>"#;
    assert_eq!(upright_xmp(attr), r#"<rdf:Description tiff:Orientation="1" tiff:Make="X"/>"#);
    let elem = "<tiff:Orientation>8</tiff:Orientation><tiff:Orientation>3</tiff:Orientation>";
    assert_eq!(upright_xmp(elem), "<tiff:Orientation>1</tiff:Orientation><tiff:Orientation>1</tiff:Orientation>");
    assert_eq!(upright_xmp("tiff:Orientation='12'"), "tiff:Orientation='1'");
    for same in [r#"tiff:Orientation="1""#, "no orientation", "tiff:Orientation", "tiff:Orientation=\"\"", "tiff:Orientation: 6"] {
        assert!(matches!(upright_xmp(same), std::borrow::Cow::Borrowed(_)), "{same}");
    }
}

/// A TIFF-structured EXIF block: IFD0 at offset 8 holding `entries`
/// (`tag, type, count, 4-byte value field`), a zero next-IFD pointer, then `tail`.
fn ifd(big: bool, entries: &[(u16, u16, u32, [u8; 4])], tail: &[u8]) -> Vec<u8> {
    let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut v = if big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
    v.extend_from_slice(&u32b(8));
    v.extend_from_slice(&u16b(entries.len() as u16));
    for &(tag, ty, count, value) in entries {
        v.extend_from_slice(&u16b(tag));
        v.extend_from_slice(&u16b(ty));
        v.extend_from_slice(&u32b(count));
        v.extend_from_slice(&value);
    }
    v.extend_from_slice(&u32b(0));
    v.extend_from_slice(tail);
    v
}

/// The 4-byte value field of a SHORT.
fn short(v: u16, big: bool) -> [u8; 4] {
    let b = if big { v.to_be_bytes() } else { v.to_le_bytes() };
    [b[0], b[1], 0, 0]
}

#[test]
fn duplicate_orientation_entries_first_wins() {
    for big in [false, true] {
        let o = |v| (0x0112, 3, 1, short(v, big));
        assert_eq!(exif_orientation(&ifd(big, &[o(6), o(3)], &[])), 6);
        assert_eq!(exif_orientation(&ifd(big, &[o(3), o(6)], &[])), 3);
        assert_eq!(exif_orientation(&ifd(big, &[o(1), o(8)], &[])), 1);
        // A malformed first entry is ignored as a whole: a later good one does not count.
        let bad_first = ifd(big, &[(0x0112, 3, 2, short(6, big)), o(8)], &[]);
        assert_eq!(exif_orientation(&bad_first), 1);
        let long = |v: u32| (0x0112, 4, 1, if big { v.to_be_bytes() } else { v.to_le_bytes() });
        let long_first = ifd(big, &[long(6), o(8)], &[]);
        assert_eq!(exif_orientation(&long_first), 6, "a LONG first entry counts");
        assert_eq!(&upright_exif(&long_first)[..], &ifd(big, &[long(1), o(1)], &[])[..], "each in its own width");
        let rational_first = ifd(big, &[(0x0112, 5, 1, short(6, big)), o(8)], &[]);
        assert_eq!(exif_orientation(&rational_first), 1);
        // Upright: every well-formed duplicate becomes 1, so no reader rotates twice.
        let dup = ifd(big, &[o(6), o(3)], &[]);
        let fixed = upright_exif(&dup);
        assert_eq!(fixed.len(), dup.len());
        assert_eq!(&fixed[..], &ifd(big, &[o(1), o(1)], &[])[..]);
        // Only a later duplicate disagrees: it is rewritten too.
        assert_eq!(&upright_exif(&ifd(big, &[o(1), o(8)], &[]))[..], &ifd(big, &[o(1), o(1)], &[])[..]);
        assert!(matches!(upright_exif(&ifd(big, &[o(1), o(1)], &[])), std::borrow::Cow::Borrowed(_)));
    }
}

#[test]
fn strict_ifd_parsing_ignores_the_tag() {
    for big in [false, true] {
        let o6 = (0x0112, 3, 1, short(6, big));
        let good = ifd(big, &[o6], &[]);
        assert_eq!(exif_orientation(&good), 6);
        // Truncated before (or inside) the next-IFD pointer.
        for cut in 1..=4 {
            assert_eq!(exif_orientation(&good[..good.len() - cut]), 1, "cut {cut}");
        }
        // IFD offsets inside the 8-byte header.
        for off in 0..8u32 {
            let mut b = good.clone();
            b[4..8].copy_from_slice(&if big { off.to_be_bytes() } else { off.to_le_bytes() });
            assert_eq!(exif_orientation(&b), 1, "offset {off}");
            assert!(matches!(upright_exif(&b), std::borrow::Cow::Borrowed(_)));
        }
        // Count must be exactly 1, type must be SHORT or LONG.
        let long6 = if big { 6u32.to_be_bytes() } else { 6u32.to_le_bytes() };
        for count in [0, 2, 3, u32::MAX] {
            assert_eq!(exif_orientation(&ifd(big, &[(0x0112, 3, count, short(6, big))], &[])), 1, "count {count}");
            assert_eq!(exif_orientation(&ifd(big, &[(0x0112, 4, count, long6)], &[])), 1, "LONG count {count}");
        }
        for ty in [1, 2, 5, 6, 7, 8, 9, 0, 0xFFFF] {
            let b = ifd(big, &[(0x0112, ty, 1, short(6, big))], &[]);
            assert_eq!(exif_orientation(&b), 1, "type {ty}");
            assert!(matches!(upright_exif(&b), std::borrow::Cow::Borrowed(_)), "type {ty}");
        }
    }
}

#[test]
fn upright_exif_keeps_every_other_byte() {
    for big in [false, true] {
        let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        // IFD0: Make (12-byte ASCII stored after the IFD), Orientation 6, Software (inline ASCII),
        // ExifIFD pointer to a sub-IFD with its own entries.
        let ifd_end = 8 + 2 + 4 * 12 + 4;
        let make_at = ifd_end as u32;
        let sub_at = make_at + 12;
        let mut tail = b"PhotoCraftCo".to_vec();
        // Sub-IFD: one DateTimeOriginal-ish SHORT entry, then next-IFD 0.
        tail.extend_from_slice(&if big { 1u16.to_be_bytes() } else { 1u16.to_le_bytes() });
        tail.extend_from_slice(&if big { 0x9000u16.to_be_bytes() } else { 0x9000u16.to_le_bytes() });
        tail.extend_from_slice(&if big { 7u16.to_be_bytes() } else { 7u16.to_le_bytes() });
        tail.extend_from_slice(&u32b(4));
        tail.extend_from_slice(b"0232");
        tail.extend_from_slice(&u32b(0));
        let e = ifd(
            big,
            &[(0x010F, 2, 12, u32b(make_at)), (0x0112, 3, 1, short(6, big)), (0x0131, 2, 4, *b"PC1\0"), (0x8769, 4, 1, u32b(sub_at))],
            &tail,
        );
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&e);
        let fixed = upright_exif(&prefixed).into_owned();
        assert_eq!(exif_orientation(&fixed), 1);
        assert_eq!(fixed.len(), prefixed.len());
        // Only the Orientation value (2 bytes in the second entry) differs.
        let value = 6 + 8 + 2 + 12 + 8;
        let changed: Vec<usize> = (0..fixed.len()).filter(|&i| fixed[i] != prefixed[i]).collect();
        assert_eq!(changed, vec![if big { value + 1 } else { value }], "big={big}");
        assert_eq!(&fixed[..value], &prefixed[..value]);
        assert_eq!(&fixed[value + 2..], &prefixed[value + 2..]);
        // The offset-based Make string and the inline Software tag are intact.
        assert_eq!(&fixed[6 + make_at as usize..][..12], b"PhotoCraftCo");
        assert!(fixed.windows(4).any(|w| w == b"PC1\0"));
    }
}

#[test]
fn long_orientation_reads_and_rewrites_in_both_byte_orders() {
    for big in [false, true] {
        let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let long = |v: u32| (0x0112u16, 4u16, 1u32, u32b(v));
        for o in 1..=8u32 {
            let e = ifd(big, &[(0x0131, 2, 4, *b"PC1\0"), long(o)], b"tail");
            assert_eq!(exif_orientation(&e), o as u16, "o={o} big={big}");
            let fixed = upright_exif(&e);
            assert_eq!(exif_orientation(&fixed), 1);
            // Still a LONG, all 4 value bytes set to 1 in the file's byte order; nothing else moved.
            assert_eq!(&fixed[..], &ifd(big, &[(0x0131, 2, 4, *b"PC1\0"), long(1)], b"tail")[..], "o={o} big={big}");
        }
        // Out of range, including values that only fit in a LONG, reads as 1 and is left alone.
        for bad in [0, 9, 0x1_0006, u32::MAX] {
            let e = ifd(big, &[long(bad)], &[]);
            assert_eq!(exif_orientation(&e), 1, "{bad}");
        }
    }
}
