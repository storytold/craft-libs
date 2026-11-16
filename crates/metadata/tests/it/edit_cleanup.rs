//! Stable IFD0 compaction under repeated malformed resolution tags.
//! Fixtures retain original value offsets and compare the entire metadata block.

use std::borrow::Cow;

use craft_metadata::edit::exif_with_resolution;

#[derive(Clone, Copy)]
struct Layout {
    big_tiff: bool,
    big_endian: bool,
}

impl Layout {
    /// Encode a fixture integer at the selected width and byte order.
    fn number(self, value: u64, width: usize) -> Vec<u8> {
        if self.big_endian { value.to_be_bytes()[8 - width..].to_vec() } else { value.to_le_bytes()[..width].to_vec() }
    }

    /// Build source and expected blocks separately, leaving out-of-line data at its original
    /// offsets. Cases select unrelated, malformed or rewritable entries in physical order.
    fn fixture(self, cases: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (header, count_bytes, entry_bytes, offset_bytes) = if self.big_tiff { (16, 8, 20, 8) } else { (8, 2, 12, 4) };
        let first = header + count_bytes;
        let table_end = first + cases.len() * entry_bytes;
        let mut data = b"unrelated out-of-line value\0".to_vec();
        let mut source_table = Vec::new();
        let mut kept_table = Vec::new();
        for &case in cases {
            let rational = |n, d| [self.number(n, 4), self.number(d, 4)].concat();
            let (tag, ty, count, value, replacement) = match case {
                0 => (270, 2, data.len() as u64, data.clone(), Some(data.clone())),
                1 => (282, 3, 1, self.number(300, 2), None), // wrong type
                2 => (283, 5, 1, rational(300, 0), None),    // zero denominator
                3 => (296, 4, 1, self.number(3, 4), None),   // wrong unit type
                4 => (282, 5, 1, rational(300, 1), Some(rational(240, 1))),
                5 => (283, 5, 1, rational(300, 1), Some(rational(240, 1))),
                6 => (296, 3, 1, self.number(3, 2), Some(self.number(2, 2))),
                7 => (282, 5, 2, [rational(300, 1), rational(300, 1)].concat(), None), // wrong count
                9 => (282, 5, u64::MAX, vec![0; 16], None),                            // oversized value span
                _ => (283, 1, 1, vec![99], None),                                      // unknown value type
            };
            let mut entry = [self.number(tag, 2), self.number(ty, 2), self.number(count, offset_bytes)].concat();
            let mut rewritten = entry.clone();
            if value.len() > offset_bytes {
                let at = table_end + offset_bytes + data.len();
                entry.extend(self.number(at as u64, offset_bytes));
                rewritten.extend(self.number(at as u64, offset_bytes));
                // These values occupy their original slots even if their entry is removed.
                data.extend_from_slice(replacement.as_deref().unwrap_or(&value));
            } else {
                let mut inline = value.clone();
                inline.resize(offset_bytes, 0);
                entry.extend(inline);
                let mut inline = replacement.clone().unwrap_or_else(|| value.clone());
                inline.resize(offset_bytes, 0);
                rewritten.extend(inline);
            }
            source_table.extend_from_slice(&entry);
            if replacement.is_some() {
                kept_table.extend_from_slice(&rewritten);
            }
        }
        let mut prefix = if self.big_endian { b"MM".to_vec() } else { b"II".to_vec() };
        prefix.extend(self.number(if self.big_tiff { 43 } else { 42 }, 2));
        if self.big_tiff {
            prefix.extend(self.number(8, 2));
            prefix.extend(self.number(0, 2));
        }
        prefix.extend(self.number(header as u64, offset_bytes));
        let next = self.number(0x10203040, offset_bytes);
        let mut source = prefix.clone();
        source.extend(self.number(cases.len() as u64, count_bytes));
        source.extend_from_slice(&source_table);
        source.extend_from_slice(&next);
        source.extend_from_slice(&data);
        // Replace the rewritten rational payloads in the source with their original values.
        // Locate them using source entry offsets, never offsets from the compacted table.
        for (case, entry) in cases.iter().zip(source_table.chunks_exact(entry_bytes)) {
            if !self.big_tiff && matches!(case, 4 | 5) {
                let bytes: [u8; 4] = entry[8..12].try_into().unwrap();
                let at = if self.big_endian { u32::from_be_bytes(bytes) } else { u32::from_le_bytes(bytes) } as usize;
                source[at..at + 4].copy_from_slice(&self.number(300, 4));
            }
        }
        let mut expected = prefix;
        expected.extend(self.number((kept_table.len() / entry_bytes) as u64, count_bytes));
        expected.extend_from_slice(&kept_table);
        expected.extend_from_slice(&next);
        expected.resize(table_end + offset_bytes, 0);
        expected.extend_from_slice(&data);
        (source, expected)
    }
}

/// All layout combinations share the same compaction contract, including JPEG-prefixed EXIF.
fn layouts() -> impl Iterator<Item = Layout> {
    [false, true].into_iter().flat_map(|big_tiff| [false, true].into_iter().map(move |big_endian| Layout { big_tiff, big_endian }))
}

/// Every removal mask exercises first/middle/last entries, adjacent runs, all and none removed.
/// Mixed valid duplicates must still be rewritten and unrelated payload offsets must not move.
#[test]
fn compaction_preserves_complete_blocks_for_removal_patterns() {
    for layout in layouts() {
        for mask in 0..256 {
            let cases: Vec<u8> = (0..8).map(|i| if mask & (1 << i) != 0 { 1 + (i % 3) as u8 } else { [0, 4, 5, 6][i % 4] }).collect();
            let (source, expected) = layout.fixture(&cases);
            for prefix in [b"".as_slice(), b"Exif\0\0".as_slice()] {
                let source = [prefix, &source].concat();
                let expected = [prefix, &expected].concat();
                assert_eq!(exif_with_resolution(&source, (240.0, 240.0)).as_ref(), expected, "mask {mask}");
            }
        }
        let (source, expected) = layout.fixture(&[7, 4, 1, 4, 8, 5, 2, 5, 3, 6, 9, 0]);
        assert_eq!(exif_with_resolution(&source, (240.0, 240.0)).as_ref(), expected);
    }
}

/// Invalid output resolution, empty/missing metadata and already-correct first duplicates keep
/// the borrowed fast path; truncated or invalid table layouts cannot be partially compacted.
#[test]
fn cleanup_preserves_noop_and_invalid_layout_behavior() {
    for layout in layouts() {
        let (source, _) = layout.fixture(&[1, 4, 2, 5, 3, 6, 0]);
        for ppi in [(0.0, 240.0), (-1.0, 240.0), (f32::NAN, 240.0), (240.0, f32::INFINITY), (1_000_001.0, 240.0)] {
            assert!(matches!(exif_with_resolution(&source, ppi), Cow::Borrowed(_)));
        }
        let table_end = if layout.big_tiff { 24 + 7 * 20 + 8 } else { 10 + 7 * 12 + 4 };
        for n in 0..table_end {
            assert!(matches!(exif_with_resolution(&source[..n], (240.0, 240.0)), Cow::Borrowed(_)), "length {n}");
        }
        let mut invalid = source;
        invalid[0] = 0;
        assert!(matches!(exif_with_resolution(&invalid, (240.0, 240.0)), Cow::Borrowed(_)));
        for cases in [&[][..], &[0][..], &[4, 5, 4, 1][..]] {
            let (source, _) = layout.fixture(cases);
            assert!(matches!(exif_with_resolution(&source, (300.0, 300.0)), Cow::Borrowed(_)));
        }
    }
    assert!(matches!(exif_with_resolution(&[], (240.0, 240.0)), Cow::Borrowed(_)));
}
