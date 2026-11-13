//! Shared EXIF ColorSpace tag parsing.

pub use craft_metadata::edit::{EXIF_COLOR_SPACE_SRGB, exif_color_space};

#[cfg(test)]
mod tests {
    use super::*;

    const TAG_EXIF_IFD: u16 = 0x8769;
    const TAG_COLOR_SPACE: u16 = 0xA001;
    const TYPE_SHORT: u16 = 3;
    const TYPE_LONG: u16 = 4;

    /// A little- or big-endian EXIF block: IFD0 holding only the Exif pointer, then an Exif IFD
    /// holding `entries` (tag, type, count, value).
    fn exif(big_endian: bool, entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let u16b = |v: u16| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let u32b = |v: u32| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut b = if big_endian { b"MM\0\x2a".to_vec() } else { b"II\x2a\0".to_vec() };
        b.extend(u32b(8));
        // IFD0: one entry, then the next-IFD pointer. The Exif IFD starts right after (8 + 2 + 12 + 4).
        b.extend(u16b(1));
        b.extend(u16b(TAG_EXIF_IFD));
        b.extend(u16b(TYPE_LONG));
        b.extend(u32b(1));
        b.extend(u32b(26));
        b.extend(u32b(0));
        b.extend(u16b(entries.len() as u16));
        for &(tag, ty, count, value) in entries {
            b.extend(u16b(tag));
            b.extend(u16b(ty));
            b.extend(u32b(count));
            // SHORT values are left-justified in the 4-byte field.
            if ty == TYPE_SHORT {
                b.extend(u16b(value as u16));
                b.extend([0, 0]);
            } else {
                b.extend(u32b(value));
            }
        }
        b.extend(u32b(0));
        b
    }

    #[test]
    fn reads_the_color_space_in_either_byte_order() {
        for big in [false, true] {
            let srgb = exif(big, &[(0x9000, 7, 4, 0x3032_3330), (TAG_COLOR_SPACE, TYPE_SHORT, 1, 1)]);
            assert_eq!(exif_color_space(&srgb), Some(EXIF_COLOR_SPACE_SRGB), "big endian: {big}");
            let mut prefixed = b"Exif\0\0".to_vec();
            prefixed.extend(&srgb);
            assert_eq!(exif_color_space(&prefixed), Some(1));
            assert_eq!(exif_color_space(&exif(big, &[(TAG_COLOR_SPACE, TYPE_SHORT, 1, 0xFFFF)])), Some(0xFFFF));
            assert_eq!(exif_color_space(&exif(big, &[(TAG_COLOR_SPACE, TYPE_LONG, 1, 1)])), Some(1), "LONG is accepted");
        }
    }

    #[test]
    fn missing_or_malformed_reads_as_no_tag() {
        assert_eq!(exif_color_space(&exif(false, &[(0x9000, 7, 4, 0)])), None, "no ColorSpace entry");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, TYPE_SHORT, 2, 1)])), None, "count ≠ 1");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, 2, 1, 1)])), None, "ASCII type");
        assert_eq!(exif_color_space(&exif(false, &[(TAG_COLOR_SPACE, TYPE_LONG, 1, 0x1_0000)])), None, "LONG over u16");
        let good = exif(false, &[(TAG_COLOR_SPACE, TYPE_SHORT, 1, 1)]);
        // Every truncation is rejected without panicking.
        for len in 0..good.len() - 4 {
            assert_eq!(exif_color_space(&good[..len]), None, "truncated to {len}");
        }
        // An Exif pointer past the end, into the header, or of the wrong type.
        for (at, value) in [(18usize, 0xFFFF_FFF0u32), (18, 4), (18, u32::MAX)] {
            let mut b = good.clone();
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(exif_color_space(&b), None, "pointer {value:#x}");
        }
        let mut b = good.clone();
        b[12..14].copy_from_slice(&TYPE_SHORT.to_le_bytes());
        assert_eq!(exif_color_space(&b), None, "pointer type SHORT");
        // An Exif IFD claiming more entries than the block holds.
        let mut b = good;
        b[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(exif_color_space(&b), None);
        assert_eq!(exif_color_space(b"not exif"), None);
    }
}
