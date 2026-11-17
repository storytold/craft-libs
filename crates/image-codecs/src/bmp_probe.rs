//! Header-only BMP dimensions and pixel-data coverage, without allocating a canvas.

use crate::error::CodecError;

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

/// Wrap a DIB (a BMP without its 14-byte `BITMAPFILEHEADER`, as the Windows clipboard holds one
/// for `CF_DIB` and `CF_DIBV5`) as a BMP file the decoders here read. The pixel offset is computed
/// from the header with saturating arithmetic, so a hostile header cannot overflow; whether the
/// pixel data exists is left to the decoder. Apply [`dib_layout`] first for clipboard DIBs.
///
/// # Errors
///
/// Returns [`CodecError::LimitExceeded`] when the output (`dib.len() + 14` bytes) cannot be allocated.
pub fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, CodecError> {
    let u16_at = |at: usize| dib.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let u32_at = |at: usize| dib.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let bi_size = u32_at(0).unwrap_or(40);
    let offset = if bi_size == 12 {
        let bpp = u16_at(10).unwrap_or(24);
        let palette = if bpp <= 8 { (1u32 << bpp) * 3 } else { 0 };
        14 + bi_size + palette
    } else {
        let bpp = u16_at(14).unwrap_or(24);
        let compression = u32_at(16).unwrap_or(0);
        let clr_used = u32_at(32).unwrap_or(0);
        let palette_entries = if clr_used != 0 {
            clr_used
        } else if bpp <= 8 {
            1u32 << bpp
        } else {
            0
        };
        let masks = if compression == 3 && bi_size >= 40 { 12 } else { 0 };
        14u32.saturating_add(bi_size).saturating_add(masks).saturating_add(palette_entries.saturating_mul(4))
    };
    let total = dib.len().saturating_add(14);
    let file_size = u32::try_from(total).unwrap_or(u32::MAX);
    let mut bmp = Vec::new();
    bmp.try_reserve_exact(total).map_err(|_| CodecError::LimitExceeded(format!("a {total}-byte BMP")))?;
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0u8; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

/// Put a clipboard DIB with a V4 or V5 header (108 bytes or more) in the layout the decoder reads.
/// A 32-bit one whose alpha mask is set but whose compression says `BI_RGB` is marked
/// `BI_BITFIELDS` (with the usual colour masks when it gives none), so its alpha is kept as
/// Windows programs mean it; arboard makes the same change. Bit fields then need the colour masks
/// after the header too, where Windows puts a copy and the decoder skips them; a DIB without that
/// copy gets it. Any other DIB comes back unchanged, as does one whose 12 extra bytes cannot be
/// allocated.
pub fn dib_layout(mut dib: Vec<u8>) -> Vec<u8> {
    let u32_at = |d: &[u8], at: usize| d.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let Some(size) = u32_at(&dib, 0).filter(|&s| s >= 108).and_then(|s| usize::try_from(s).ok()) else { return dib };
    let put = |d: &mut Vec<u8>, at: usize, v: u32| {
        if let Some(b) = d.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    };
    if dib.get(14..16) == Some(&[32, 0]) && u32_at(&dib, 16) == Some(0) && u32_at(&dib, 52) == Some(0xFF00_0000) {
        put(&mut dib, 16, 3);
        if (40..52).step_by(4).all(|at| u32_at(&dib, at) == Some(0)) {
            put(&mut dib, 40, 0x00FF_0000);
            put(&mut dib, 44, 0x0000_FF00);
            put(&mut dib, 48, 0x0000_00FF);
        }
    }
    if u32_at(&dib, 16) == Some(3)
        && size <= dib.len()
        && let Some(masks) = dib.get(40..52).map(<[u8]>::to_vec)
        && dib.get(size..size.saturating_add(12)) != Some(masks.as_slice())
        && dib.try_reserve(12).is_ok()
    {
        dib.splice(size..size, masks);
    }
    dib
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

    // The WordCraft originals, kept verbatim as the oracle.
    fn oracle_dib_to_bmp(dib: &[u8]) -> Vec<u8> {
        let u16_at = |at: usize| dib.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
        let u32_at = |at: usize| dib.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
        let bi_size = u32_at(0).unwrap_or(40);
        let offset = if bi_size == 12 {
            let bpp = u16_at(10).unwrap_or(24);
            let palette = if bpp <= 8 { (1u32 << bpp) * 3 } else { 0 };
            14 + bi_size + palette
        } else {
            let bpp = u16_at(14).unwrap_or(24);
            let compression = u32_at(16).unwrap_or(0);
            let clr_used = u32_at(32).unwrap_or(0);
            let palette_entries = if clr_used != 0 {
                clr_used
            } else if bpp <= 8 {
                1u32 << bpp
            } else {
                0
            };
            let masks = if compression == 3 && bi_size >= 40 { 12 } else { 0 };
            14u32.saturating_add(bi_size).saturating_add(masks).saturating_add(palette_entries.saturating_mul(4))
        };
        let file_size = u32::try_from(14 + dib.len()).unwrap_or(u32::MAX);
        let mut bmp = Vec::with_capacity(14 + dib.len());
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&file_size.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 4]);
        bmp.extend_from_slice(&offset.to_le_bytes());
        bmp.extend_from_slice(dib);
        bmp
    }

    fn oracle_dib_layout(mut dib: Vec<u8>) -> Vec<u8> {
        let u32_at = |d: &[u8], at: usize| d.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
        let Some(size) = u32_at(&dib, 0).filter(|&s| s >= 108).and_then(|s| usize::try_from(s).ok()) else { return dib };
        let put = |d: &mut Vec<u8>, at: usize, v: u32| {
            if let Some(b) = d.get_mut(at..at + 4) {
                b.copy_from_slice(&v.to_le_bytes());
            }
        };
        if dib.get(14..16) == Some(&[32, 0]) && u32_at(&dib, 16) == Some(0) && u32_at(&dib, 52) == Some(0xFF00_0000) {
            put(&mut dib, 16, 3);
            if (40..52).step_by(4).all(|at| u32_at(&dib, at) == Some(0)) {
                put(&mut dib, 40, 0x00FF_0000);
                put(&mut dib, 44, 0x0000_FF00);
                put(&mut dib, 48, 0x0000_00FF);
            }
        }
        if u32_at(&dib, 16) == Some(3)
            && size <= dib.len()
            && let Some(masks) = dib.get(40..52).map(<[u8]>::to_vec)
            && dib.get(size..size + 12) != Some(masks.as_slice())
        {
            dib.splice(size..size, masks);
        }
        dib
    }

    fn core(bpp: u16) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bpp.to_le_bytes());
        d.resize(12 + 64, 7);
        d
    }

    #[allow(clippy::too_many_arguments)]
    fn info(size: u32, bpp: u16, comp: u32, clr_used: u32, masks: [u32; 4], tail: usize) -> Vec<u8> {
        let mut d = vec![0u8; size.clamp(40, 124) as usize];
        d[0..4].copy_from_slice(&size.to_le_bytes());
        d[4..8].copy_from_slice(&2i32.to_le_bytes());
        d[8..12].copy_from_slice(&2i32.to_le_bytes());
        d[12..14].copy_from_slice(&1u16.to_le_bytes());
        d[14..16].copy_from_slice(&bpp.to_le_bytes());
        d[16..20].copy_from_slice(&comp.to_le_bytes());
        d[32..36].copy_from_slice(&clr_used.to_le_bytes());
        for (i, m) in masks.iter().enumerate() {
            if let Some(b) = d.get_mut(40 + 4 * i..44 + 4 * i) {
                b.copy_from_slice(&m.to_le_bytes());
            }
        }
        d.extend((0..tail).map(|i| (i * 31 + 5) as u8));
        d
    }

    fn corpus() -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        for bpp in [1, 4, 8, 24, 0, 99] {
            v.push(core(bpp));
        }
        for bpp in [1u16, 4, 8, 16, 24, 32, 0, 255] {
            for clr in [0u32, 1, 5, 256, u32::MAX] {
                v.push(info(40, bpp, 0, clr, [0; 4], 40));
            }
        }
        for (size, comp, masks) in [
            (40, 3, [0xFF0000, 0xFF00, 0xFF, 0]),
            (40, 3, [0; 4]),
            (52, 3, [0xFF0000, 0xFF00, 0xFF, 0]),
            (108, 3, [0xFF0000, 0xFF00, 0xFF, 0xFF00_0000]),
            (108, 0, [0xFF0000, 0xFF00, 0xFF, 0xFF00_0000]),
            (108, 0, [0, 0, 0, 0xFF00_0000]),
            (108, 0, [0; 4]),
            (124, 3, [0xFF0000, 0xFF00, 0xFF, 0xFF00_0000]),
            (124, 0, [0, 0, 0, 0xFF00_0000]),
            (124, 0, [1, 0, 0, 0xFF00_0000]),
            (124, 0, [0xFF0000, 0xFF00, 0xFF, 0]),
            (124, 1, [0, 0, 0, 0xFF00_0000]),
            (124, 3, [1, 2, 3, 4]),
            (0, 0, [0; 4]),
            (12, 0, [0; 4]),
            (u32::MAX, 3, [0; 4]),
            (u32::MAX, 0, [0; 4]),
            (200, 3, [9; 4]),
        ] {
            for bpp in [8u16, 16, 32] {
                for clr in [0u32, 2, u32::MAX] {
                    v.push(info(size, bpp, comp, clr, masks, 48));
                }
            }
        }
        v
    }

    #[test]
    fn dib_helpers_match_the_wordcraft_originals_byte_for_byte() {
        for dib in corpus() {
            for n in 0..=dib.len() {
                let cut = &dib[..n];
                assert_eq!(dib_layout(cut.to_vec()), oracle_dib_layout(cut.to_vec()), "layout len {n}");
                assert_eq!(dib_to_bmp(cut).unwrap(), oracle_dib_to_bmp(cut), "bmp len {n}");
                let both = dib_layout(cut.to_vec());
                assert_eq!(dib_to_bmp(&both).unwrap(), oracle_dib_to_bmp(&oracle_dib_layout(cut.to_vec())));
            }
        }
    }

    #[test]
    fn dib_helpers_survive_pseudo_random_bytes() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        for len in 0..300usize {
            let d: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            assert_eq!(dib_to_bmp(&dib_layout(d.clone())).unwrap(), oracle_dib_to_bmp(&oracle_dib_layout(d)));
        }
    }

    #[test]
    fn huge_palette_count_saturates_the_offset() {
        let b = dib_to_bmp(&info(40, 8, 0, u32::MAX, [0; 4], 8)).unwrap();
        assert_eq!(u32::from_le_bytes([b[10], b[11], b[12], b[13]]), u32::MAX);
        let b = dib_to_bmp(&info(u32::MAX, 8, 3, u32::MAX, [0; 4], 8)).unwrap();
        assert_eq!(u32::from_le_bytes([b[10], b[11], b[12], b[13]]), u32::MAX);
    }

    #[test]
    fn wrapped_dibs_decode_and_keep_alpha() {
        // 2x2 32-bit V5, BI_RGB with an alpha mask, bottom-up BGRA pixels.
        let mut d = info(124, 32, 0, 0, [0, 0, 0, 0xFF00_0000], 0);
        let px = [[1u8, 2, 3, 255], [4, 5, 6, 128], [7, 8, 9, 64], [10, 11, 12, 0]];
        for p in px {
            d.extend_from_slice(&p);
        }
        let bmp = dib_to_bmp(&dib_layout(d)).unwrap();
        let img = crate::decode(&bmp).unwrap();
        let alphas: Vec<u16> = img.to_rgba8().chunks(4).map(|p| u16::from(p[3])).collect();
        assert_eq!(alphas, [64, 0, 255, 128]);
        // 24-bit plain DIB and a 12-byte core 8-bit one open too.
        let mut d = info(40, 24, 0, 0, [0; 4], 0);
        d.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(crate::decode(&dib_to_bmp(&d).unwrap()).is_ok());
        let mut c = core(8);
        c.truncate(12);
        c.extend((0..256 * 3).map(|i| i as u8));
        c.extend_from_slice(&[0, 1, 0, 0, 2, 3, 0, 0]);
        assert!(crate::decode(&dib_to_bmp(&c).unwrap()).is_ok());
    }
}
