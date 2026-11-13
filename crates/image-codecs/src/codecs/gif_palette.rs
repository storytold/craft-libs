//! The colour table of a GIF, read from its blocks without decoding pixels (GIF89a
//! specification: Logical Screen Descriptor, Global and Local Color Tables, Graphic Control
//! Extension).
//!
//! A GIF is always palette-based, so it opens in Indexed Color with the table of the frame that
//! [`crate::decode`] keeps, the first: its Local Color Table when it has one, else the Global
//! one, and the transparent entry that frame's Graphic Control Extension names.

/// Colours of the table the first frame uses, and its transparent entry. `None` for anything
/// that isn't a well-formed GIF up to its first image, or a first frame without a table.
pub(crate) fn palette(b: &[u8]) -> Option<(Vec<[u8; 3]>, Option<u8>)> {
    if !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return None;
    }
    // Logical Screen Descriptor: width, height, packed fields, background, aspect.
    let packed = *b.get(10)?;
    let mut at = 13;
    let table = |at: usize, packed: u8| -> Option<Vec<[u8; 3]>> {
        let n = 2usize << (packed & 0x07);
        let bytes = b.get(at..at.checked_add(n.checked_mul(3)?)?)?;
        Some(bytes.as_chunks::<3>().0.to_vec())
    };
    let global = if packed & 0x80 != 0 {
        let t = table(at, packed)?;
        at += t.len() * 3;
        Some(t)
    } else {
        None
    };
    let mut transparent = None;
    loop {
        match *b.get(at)? {
            // Extension: label, then data sub-blocks up to a zero-length one.
            0x21 => {
                let label = *b.get(at + 1)?;
                at += 2;
                // Graphic Control Extension: block size 4, packed (bit 0: transparent colour
                // given), delay (2 bytes), transparent colour index.
                if label == 0xF9 && *b.get(at)? == 4 {
                    let flags = *b.get(at + 1)?;
                    transparent = (flags & 0x01 != 0).then_some(*b.get(at + 4)?);
                }
                loop {
                    let size = usize::from(*b.get(at)?);
                    at = at.checked_add(1 + size)?;
                    if size == 0 {
                        break;
                    }
                }
            }
            // Image Descriptor: position and size (8 bytes), packed fields, then the Local Color
            // Table if bit 7 is set. This is the first frame: its table is the one used.
            0x2C => {
                let packed = *b.get(at + 9)?;
                let colors = if packed & 0x80 != 0 { table(at + 10, packed)? } else { global? };
                let transparent = transparent.filter(|i| usize::from(*i) < colors.len());
                return Some((colors, transparent));
            }
            // Trailer, or anything else: no image.
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::palette;

    /// A GIF up to its first image: `global` table entries, optional GCE transparency, optional
    /// local table.
    fn gif(global: &[[u8; 3]], transparent: Option<u8>, local: Option<&[[u8; 3]]>) -> Vec<u8> {
        let size_bits = |n: usize| (n.max(2).next_power_of_two().trailing_zeros() - 1) as u8;
        let mut b = b"GIF89a".to_vec();
        b.extend([4, 0, 4, 0]);
        b.push(if global.is_empty() { 0 } else { 0x80 | size_bits(global.len()) });
        b.extend([0, 0]);
        let pad = |b: &mut Vec<u8>, t: &[[u8; 3]]| {
            let n = 2usize << size_bits(t.len());
            for k in 0..n {
                b.extend(t.get(k).copied().unwrap_or([0, 0, 0]));
            }
        };
        if !global.is_empty() {
            pad(&mut b, global);
        }
        // A comment extension first, to skip.
        b.extend([0x21, 0xFE, 3, b'h', b'i', b'!', 0]);
        if let Some(i) = transparent {
            b.extend([0x21, 0xF9, 4, 0x01, 0, 0, i, 0]);
        }
        b.extend([0x2C, 0, 0, 0, 0, 4, 0, 4, 0]);
        match local {
            Some(t) => {
                b.push(0x80 | size_bits(t.len()));
                pad(&mut b, t);
            }
            None => b.push(0),
        }
        b.extend([2, 2, 0x4C, 0x01, 0, 0x3B]);
        b
    }

    #[test]
    fn the_first_frames_table_and_transparent_entry() {
        let g = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
        assert_eq!(palette(&gif(&g, None, None)), Some((g.to_vec(), None)));
        assert_eq!(palette(&gif(&g, Some(2), None)), Some((g.to_vec(), Some(2))));
        // A local table wins over the global one.
        let l = [[1, 2, 3], [4, 5, 6]];
        assert_eq!(palette(&gif(&g, Some(1), Some(&l))), Some((l.to_vec(), Some(1))));
        // A transparent index past the table is no transparent entry.
        assert_eq!(palette(&gif(&l, Some(9), None)), Some((l.to_vec(), None)));
    }

    #[test]
    fn no_table_or_not_a_gif_is_none() {
        assert_eq!(palette(&gif(&[], None, None)), None, "no table at all");
        assert_eq!(palette(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(palette(b""), None);
        let whole = gif(&[[9, 9, 9], [8, 8, 8]], Some(0), None);
        for cut in 0..whole.len() - 6 {
            // Cut short anywhere before the image descriptor's fields: never a panic.
            let _ = palette(&whole[..cut]);
        }
    }
}
