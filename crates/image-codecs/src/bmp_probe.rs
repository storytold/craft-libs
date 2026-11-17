//! Header-only BMP dimensions and pixel-data coverage, without allocating a canvas.

/// Recognize a BMP file header with a supported DIB header kind.
///
/// This is a format-routing check, not full validation: dimensions and pixel data may still
/// be invalid. Unlike the broad signature detector, it requires the minimum 26-byte core
/// header and a known DIB size, preserving callers that reject arbitrary `BM` prefixes.
pub fn is_supported_header(b: &[u8]) -> bool {
    if b.len() >= 26 && b.starts_with(b"BM") {
        let dib = u32::from_le_bytes([b[14], b[15], b[16], b[17]]);
        return matches!(dib, 12 | 16 | 40 | 52 | 56 | 64 | 108 | 124);
    }
    false
}

/// Coverage of the declared pixel array. Row byte counts include row padding; RLE coverage
/// counts the pixels drawn by commands and may be smaller than the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub pixels: u64,
    pub row_bytes: Option<(u64, u64)>,
}

/// Inspect pixel data for the given dimensions without decoding a bitmap. The row tuple is
/// `(available, needed)`; callers choose their own sparse-canvas and allocation policy.
pub fn coverage(bytes: &[u8], w: u32, h: u32) -> Coverage {
    match bmp_layout(bytes) {
        BmpLayout::Rle { offset, rle4 } => Coverage { pixels: bmp_rle_pixels(bytes, offset, rle4, w, h), row_bytes: None },
        BmpLayout::Rows { offset, bits } => {
            let needed = u64::from(w).saturating_mul(u64::from(bits)).div_ceil(32).saturating_mul(4).saturating_mul(u64::from(h));
            let held = bytes.len().saturating_sub(offset) as u64;
            Coverage { pixels: u64::from(w) * u64::from(h), row_bytes: Some((held, needed)) }
        }
    }
}

enum BmpLayout {
    /// RLE8 or RLE4 commands.
    Rle { offset: usize, rle4: bool },
    /// Rows of `bits` per pixel, each padded to 4 bytes.
    Rows { offset: usize, bits: u16 },
}

/// The layout the decoder reads, from the file header and the bitmap header: a 12-byte core header
/// (bits per pixel at 24, never compressed) or a 40-byte or larger one (bits at 28, compression at
/// 30). Fields the decoder would have refused are read as 1 bit per pixel at offset 0, the most
/// lenient layout.
fn bmp_layout(bytes: &[u8]) -> BmpLayout {
    let u16_at = |i: usize| bytes.get(i..i.checked_add(2)?).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let u32_at = |i: usize| bytes.get(i..i.checked_add(4)?).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let offset = u32_at(10).and_then(|o| usize::try_from(o).ok()).unwrap_or(0);
    let (bits, compression) = match u32_at(14) {
        Some(12) => (u16_at(24), Some(0)),
        _ => (u16_at(28), u32_at(30)),
    };
    match compression {
        Some(1) => BmpLayout::Rle { offset, rle4: false },
        Some(2) => BmpLayout::Rle { offset, rle4: true },
        _ => BmpLayout::Rows { offset, bits: bits.filter(|&b| b > 0).unwrap_or(1) },
    }
}

/// The pixels an RLE8 or RLE4 BMP's commands draw on a `w` × `h` canvas, read the way the decoder
/// paints them: a run is clipped at the end of its row, an end-of-row code moves to the next row, a
/// delta skips pixels (left blank), and an end-of-bitmap code, a truncated command or the last row
/// ends the picture. Every command takes at least 2 bytes, so the walk is linear in the file.
fn bmp_rle_pixels(bytes: &[u8], offset: usize, rle4: bool, w: u32, h: u32) -> u64 {
    let (w, h) = (u64::from(w), u64::from(h));
    let (mut pos, mut x, mut y, mut covered) = (offset, 0u64, 0u64, 0u64);
    while y < h {
        let (Some(&count), Some(&value)) = (bytes.get(pos), bytes.get(pos.saturating_add(1))) else {
            break;
        };
        pos = pos.saturating_add(2);
        match (count, value) {
            // end of row
            (0, 0) => (x, y) = (0, y.saturating_add(1)),
            // end of bitmap
            (0, 1) => break,
            // delta: right and down
            (0, 2) => {
                let (Some(&dx), Some(&dy)) = (bytes.get(pos), bytes.get(pos.saturating_add(1))) else {
                    break;
                };
                pos = pos.saturating_add(2);
                (x, y) = (x.saturating_add(u64::from(dx)), y.saturating_add(u64::from(dy)));
            }
            // absolute: `value` pixels stored one per byte (RLE8) or nibble (RLE4), padded to 2 bytes
            (0, n) => {
                let len = if rle4 { usize::from(n).div_ceil(2) } else { usize::from(n) };
                pos = pos.saturating_add(len + (len & 1));
                if pos > bytes.len() {
                    break;
                }
                covered = covered.saturating_add(u64::from(n).min(w.saturating_sub(x)));
                x = x.saturating_add(u64::from(n));
            }
            // a run of `count` pixels
            (n, _) => {
                covered = covered.saturating_add(u64::from(n).min(w.saturating_sub(x)));
                x = x.saturating_add(u64::from(n));
            }
        }
    }
    covered
}

/// Reads BMP `(width, height)` from the DIB header without allocating.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(0..2) != Some(b"BM") {
        return None;
    }
    let dib = u32::from_le_bytes(bytes.get(14..18)?.try_into().ok()?);
    let (w, h) = if dib == 12 {
        (u16::from_le_bytes(bytes.get(18..20)?.try_into().ok()?) as u32, u16::from_le_bytes(bytes.get(20..22)?.try_into().ok()?) as u32)
    } else {
        let w = i32::from_le_bytes(bytes.get(18..22)?.try_into().ok()?);
        let h = i32::from_le_bytes(bytes.get(22..26)?.try_into().ok()?);
        (u32::try_from(w).ok()?, h.unsigned_abs())
    };
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp(compression: u32, pixels: &[u8]) -> Vec<u8> {
        let mut b = vec![0; 54];
        b[..2].copy_from_slice(b"BM");
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&4i32.to_le_bytes());
        b[22..26].copy_from_slice(&2i32.to_le_bytes());
        b[28..30].copy_from_slice(&8u16.to_le_bytes());
        b[30..34].copy_from_slice(&compression.to_le_bytes());
        b.extend_from_slice(pixels);
        b
    }

    #[test]
    fn dimensions_and_padded_rows() {
        let b = bmp(0, &[0; 7]);
        assert_eq!(dimensions(&b), Some((4, 2)));
        assert_eq!(coverage(&b, 4, 2), Coverage { pixels: 8, row_bytes: Some((7, 8)) });
        assert_eq!(dimensions(&b[..20]), None);
    }

    #[test]
    fn rle_counts_clipped_runs_deltas_and_complete_absolute_blocks() {
        let b = bmp(1, &[3, 7, 0, 0, 0, 2, 1, 0, 0, 3, 1, 2, 3, 0, 0, 1]);
        assert_eq!(coverage(&b, 4, 2).pixels, 6);
        let truncated = bmp(1, &[0, 4, 1, 2]);
        assert_eq!(coverage(&truncated, 4, 2).pixels, 0);
        let nibble = bmp(2, &[0, 3, 0x12, 0x30, 0, 1]);
        assert_eq!(coverage(&nibble, 4, 2).pixels, 3);
    }
    #[test]
    fn supported_header_recognition_preserves_length_signature_and_dib_rules() {
        let mut b = [0_u8; 26];
        b[..2].copy_from_slice(b"BM");
        for dib in [12_u32, 16, 40, 52, 56, 64, 108, 124] {
            b[14..18].copy_from_slice(&dib.to_le_bytes());
            assert!(is_supported_header(&b), "DIB {dib}");
            for n in 0..26 {
                assert!(!is_supported_header(&b[..n]), "prefix {n}");
            }
        }
        for dib in [0_u32, 13, 39, 41, 125, u32::MAX] {
            b[14..18].copy_from_slice(&dib.to_le_bytes());
            assert!(!is_supported_header(&b), "DIB {dib}");
        }
        b[14..18].copy_from_slice(&40_u32.to_le_bytes());
        b[0] = b'X';
        assert!(!is_supported_header(&b));
    }
}
