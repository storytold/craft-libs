//! Variable-length code tables and scan tables of ISO/IEC 14496-2 Annex B and §7.4.

use std::sync::OnceLock;

use craft_media::bitstream::BitReader;

use super::DecodeError;

/// Table B-6, `mcbpc` of an I-VOP macroblock, indexed by `(mb_type - 3) * 4 + cbpc`.
pub(super) const MCBPC_INTRA: [(u16, u8); 8] = [(1, 1), (1, 3), (2, 3), (3, 3), (1, 4), (1, 6), (2, 6), (3, 6)];

/// Table B-7, `mcbpc` of a P-VOP macroblock, indexed by `mb_type * 4 + cbpc`
/// (`mb_type` 0 inter, 1 inter+q, 2 inter4v, 3 intra, 4 intra+q).
pub(super) const MCBPC_INTER: [(u16, u8); 20] = [
    (1, 1),
    (3, 4),
    (2, 4),
    (5, 6),
    (3, 3),
    (7, 7),
    (6, 7),
    (5, 9),
    (2, 3),
    (5, 7),
    (4, 7),
    (5, 8),
    (3, 5),
    (4, 8),
    (3, 8),
    (3, 7),
    (4, 6),
    (4, 9),
    (3, 9),
    (2, 9),
];

/// Table B-8, `cbpy` indexed by the coded block pattern of an intra macroblock (block 0 in the
/// most significant of the four bits). An inter macroblock sends the code of the inverted pattern.
pub(super) const CBPY: [(u16, u8); 16] =
    [(3, 4), (5, 5), (4, 5), (9, 4), (3, 5), (7, 4), (2, 6), (11, 4), (2, 5), (3, 6), (5, 4), (10, 4), (4, 4), (8, 4), (6, 4), (3, 2)];

/// Table B-12, `motion_code` by magnitude 0–32; a sign bit (1 = negative) follows every code but 0.
pub(super) const MVD_TABLE: [(u16, u8); 33] = [
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (3, 6),
    (5, 7),
    (4, 7),
    (3, 7),
    (11, 9),
    (10, 9),
    (9, 9),
    (17, 10),
    (16, 10),
    (15, 10),
    (14, 10),
    (13, 10),
    (12, 10),
    (11, 10),
    (10, 10),
    (9, 10),
    (8, 10),
    (7, 10),
    (6, 10),
    (5, 10),
    (4, 10),
    (7, 11),
    (6, 11),
    (5, 11),
    (4, 11),
    (3, 11),
    (2, 11),
    (3, 12),
    (2, 12),
];

/// Table B-13, `dct_dc_size_luminance` 0–12.
pub(super) const DC_SIZE_LUMA: [(u16, u8); 13] =
    [(3, 3), (3, 2), (2, 2), (2, 3), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7), (1, 8), (1, 9), (1, 10), (1, 11)];

/// Table B-14, `dct_dc_size_chrominance` 0–12.
pub(super) const DC_SIZE_CHROMA: [(u16, u8); 13] =
    [(3, 2), (2, 2), (1, 2), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7), (1, 8), (1, 9), (1, 10), (1, 11), (1, 12)];

/// The TCOEF escape code (`0000 011`), shared by both coefficient tables.
pub(super) const ESCAPE: (u16, u8) = (3, 7);

/// Table B-16, intra TCOEF codes, ordered by `last`, then `run`, then `level`.
const INTRA_VLC: [(u16, u8); 102] = [
    // last 0, run 0, levels 1–27
    (0x2, 2),
    (0x6, 3),
    (0xF, 4),
    (0xD, 5),
    (0xC, 5),
    (0x15, 6),
    (0x13, 6),
    (0x12, 6),
    (0x17, 7),
    (0x1F, 8),
    (0x1E, 8),
    (0x1D, 8),
    (0x25, 9),
    (0x24, 9),
    (0x23, 9),
    (0x21, 9),
    (0x21, 10),
    (0x20, 10),
    (0xF, 10),
    (0xE, 10),
    (0x7, 11),
    (0x6, 11),
    (0x20, 11),
    (0x21, 11),
    (0x50, 12),
    (0x51, 12),
    (0x52, 12),
    // last 0, run 1, levels 1–10
    (0xE, 4),
    (0x14, 6),
    (0x16, 7),
    (0x1C, 8),
    (0x20, 9),
    (0x1F, 9),
    (0xD, 10),
    (0x22, 11),
    (0x53, 12),
    (0x55, 12),
    // last 0, run 2, levels 1–5
    (0xB, 5),
    (0x15, 7),
    (0x1E, 9),
    (0xC, 10),
    (0x56, 12),
    // last 0, run 3, levels 1–4
    (0x11, 6),
    (0x1B, 8),
    (0x1D, 9),
    (0xB, 10),
    // last 0, runs 4–7, levels 1–3
    (0x10, 6),
    (0x22, 9),
    (0xA, 10),
    (0xD, 6),
    (0x1C, 9),
    (0x8, 10),
    (0x12, 7),
    (0x1B, 9),
    (0x54, 12),
    (0x14, 7),
    (0x1A, 9),
    (0x57, 12),
    // last 0, runs 8–9, levels 1–2
    (0x19, 8),
    (0x9, 10),
    (0x18, 8),
    (0x23, 11),
    // last 0, runs 10–14, level 1
    (0x17, 8),
    (0x19, 9),
    (0x18, 9),
    (0x7, 10),
    (0x58, 12),
    // last 1, run 0, levels 1–8
    (0x7, 4),
    (0xC, 6),
    (0x16, 8),
    (0x17, 9),
    (0x6, 10),
    (0x5, 11),
    (0x4, 11),
    (0x59, 12),
    // last 1, run 1, levels 1–3
    (0xF, 6),
    (0x16, 9),
    (0x5, 10),
    // last 1, runs 2–6, levels 1–2
    (0xE, 6),
    (0x4, 10),
    (0x11, 7),
    (0x24, 11),
    (0x10, 7),
    (0x25, 11),
    (0x13, 7),
    (0x5A, 12),
    (0x15, 8),
    (0x5B, 12),
    // last 1, runs 7–20, level 1
    (0x14, 8),
    (0x13, 8),
    (0x1A, 8),
    (0x15, 9),
    (0x14, 9),
    (0x13, 9),
    (0x12, 9),
    (0x11, 9),
    (0x26, 11),
    (0x27, 11),
    (0x5C, 12),
    (0x5D, 12),
    (0x5E, 12),
    (0x5F, 12),
];

const INTRA_LMAX: [&[u8]; 2] = [&[27, 10, 5, 4, 3, 3, 3, 3, 2, 2, 1, 1, 1, 1, 1], &[8, 3, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]];

/// Table B-17, inter TCOEF codes, in the order of [`INTER_LMAX`].
const INTER_VLC: [(u16, u8); 102] = [
    // last 0, run 0, levels 1–12
    (0x2, 2),
    (0xF, 4),
    (0x15, 6),
    (0x17, 7),
    (0x1F, 8),
    (0x25, 9),
    (0x24, 9),
    (0x21, 10),
    (0x20, 10),
    (0x7, 11),
    (0x6, 11),
    (0x20, 11),
    // last 0, run 1, levels 1–6
    (0x6, 3),
    (0x14, 6),
    (0x1E, 8),
    (0xF, 10),
    (0x21, 11),
    (0x50, 12),
    // last 0, run 2, levels 1–4
    (0xE, 4),
    (0x1D, 8),
    (0xE, 10),
    (0x51, 12),
    // last 0, runs 3–6, levels 1–3
    (0xD, 5),
    (0x23, 9),
    (0xD, 10),
    (0xC, 5),
    (0x22, 9),
    (0x52, 12),
    (0xB, 5),
    (0xC, 10),
    (0x53, 12),
    (0x13, 6),
    (0xB, 10),
    (0x54, 12),
    // last 0, runs 7–10, levels 1–2
    (0x12, 6),
    (0xA, 10),
    (0x11, 6),
    (0x9, 10),
    (0x10, 6),
    (0x8, 10),
    (0x16, 7),
    (0x55, 12),
    // last 0, runs 11–26, level 1
    (0x15, 7),
    (0x14, 7),
    (0x1C, 8),
    (0x1B, 8),
    (0x21, 9),
    (0x20, 9),
    (0x1F, 9),
    (0x1E, 9),
    (0x1D, 9),
    (0x1C, 9),
    (0x1B, 9),
    (0x1A, 9),
    (0x22, 11),
    (0x23, 11),
    (0x56, 12),
    (0x57, 12),
    // last 1, run 0, levels 1–3
    (0x7, 4),
    (0x19, 9),
    (0x5, 11),
    // last 1, run 1, levels 1–2
    (0xF, 6),
    (0x4, 11),
    // last 1, runs 2–40, level 1
    (0xE, 6),
    (0xD, 6),
    (0xC, 6),
    (0x13, 7),
    (0x12, 7),
    (0x11, 7),
    (0x10, 7),
    (0x1A, 8),
    (0x19, 8),
    (0x18, 8),
    (0x17, 8),
    (0x16, 8),
    (0x15, 8),
    (0x14, 8),
    (0x13, 8),
    (0x18, 9),
    (0x17, 9),
    (0x16, 9),
    (0x15, 9),
    (0x14, 9),
    (0x13, 9),
    (0x12, 9),
    (0x11, 9),
    (0x7, 10),
    (0x6, 10),
    (0x5, 10),
    (0x4, 10),
    (0x24, 11),
    (0x25, 11),
    (0x26, 11),
    (0x27, 11),
    (0x58, 12),
    (0x59, 12),
    (0x5A, 12),
    (0x5B, 12),
    (0x5C, 12),
    (0x5D, 12),
    (0x5E, 12),
    (0x5F, 12),
];

const INTER_LMAX: [&[u8]; 2] = [
    &[12, 6, 4, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    &[3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

/// Table 7-1: Zigzag scan (`ZIGZAG[i]` is the raster position of the `i`-th coefficient).
pub(super) const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Table 7-2: Alternate-Horizontal scan.
pub(super) const ALT_HORIZONTAL_SCAN: [usize; 64] = [
    0, 1, 2, 3, 8, 9, 16, 17, 10, 11, 4, 5, 6, 7, 15, 14, 13, 12, 19, 18, 24, 25, 32, 33, 26, 27, 20, 21, 22, 23, 28, 29, 30, 31, 34, 35, 40, 41, 48,
    49, 42, 43, 36, 37, 38, 39, 44, 45, 46, 47, 50, 51, 56, 57, 58, 59, 52, 53, 54, 55, 60, 61, 62, 63,
];

/// Table 7-3: Alternate-Vertical scan (the transpose of [`ALT_HORIZONTAL_SCAN`]).
pub(super) const ALT_VERTICAL_SCAN: [usize; 64] = [
    0, 8, 16, 24, 1, 9, 2, 10, 17, 25, 32, 40, 48, 56, 57, 49, 41, 33, 26, 18, 3, 11, 4, 12, 19, 27, 34, 42, 50, 58, 35, 43, 51, 59, 20, 28, 5, 13,
    6, 14, 21, 29, 36, 44, 52, 60, 37, 45, 53, 61, 22, 30, 7, 15, 23, 31, 38, 46, 54, 62, 39, 47, 55, 63,
];

/// Table 7-4: Default intra quantisation matrix (in raster order).
pub(super) const DEFAULT_INTRA_MAT: [u8; 64] = [
    8, 17, 18, 19, 21, 23, 25, 27, 17, 18, 19, 21, 23, 25, 27, 28, 20, 21, 22, 23, 24, 26, 28, 30, 21, 22, 23, 24, 26, 28, 30, 32, 22, 23, 24, 26,
    28, 30, 32, 35, 23, 24, 26, 28, 30, 32, 35, 38, 25, 26, 28, 30, 32, 35, 38, 41, 27, 28, 30, 32, 35, 38, 41, 45,
];

/// Table 7-5: Default non-intra quantisation matrix (in raster order).
pub(super) const DEFAULT_NONINTRA_MAT: [u8; 64] = [
    16, 17, 18, 19, 20, 21, 22, 23, 17, 18, 19, 20, 21, 22, 23, 24, 18, 19, 20, 21, 22, 23, 24, 25, 19, 20, 21, 22, 23, 24, 26, 27, 20, 21, 22, 23,
    25, 26, 27, 28, 21, 22, 23, 24, 26, 27, 28, 30, 22, 23, 24, 26, 27, 28, 30, 31, 23, 24, 25, 27, 28, 30, 31, 33,
];

/// Decode a small prefix-free VLC table (`max_len <= 12`).
#[inline]
fn decode_small_vlc(r: &mut BitReader<'_>, table: &[(u16, u8)], err: &'static str) -> Result<usize, DecodeError> {
    let peek = r.peek(12) as u16;
    let left = r.bits_left();
    for (idx, &(code, len)) in table.iter().enumerate() {
        let len_usize = usize::from(len);
        if len_usize <= left && (peek >> (12 - len)) == code {
            let _ = r.skip(len_usize);
            return Ok(idx);
        }
    }
    Err(DecodeError::Invalid(err.into()))
}

/// Read `mcbpc` in an I-VOP (Table B-6), skipping any preceding stuffing codes (`0000 0000 1`).
/// Returns `(mb_type, cbpc)` where `mb_type` is 3 (intra) or 4 (intra+q).
pub(super) fn read_mcbpc_intra(r: &mut BitReader<'_>) -> Result<(u8, u8), DecodeError> {
    for _ in 0..64 {
        if r.bits_left() >= 9 && r.peek(9) == 1 {
            let _ = r.skip(9);
            continue;
        }
        let idx = decode_small_vlc(r, &MCBPC_INTRA, "invalid intra mcbpc")?;
        return Ok((3 + (idx as u8 / 4), idx as u8 % 4));
    }
    Err(DecodeError::Invalid("excessive mcbpc stuffing".into()))
}

/// Read `mcbpc` in a P-VOP (Table B-7). Returns `(mb_type, cbpc)` where `mb_type` is 0..=4, or `None`
/// after consuming one stuffing code (`0000 0000 1`); the caller then restarts at `not_coded`.
pub(super) fn read_mcbpc_inter(r: &mut BitReader<'_>) -> Result<Option<(u8, u8)>, DecodeError> {
    if r.bits_left() >= 9 && r.peek(9) == 1 {
        // Stuffing: the macroblock header restarts at `not_coded`.
        let _ = r.skip(9);
        return Ok(None);
    }
    let idx = decode_small_vlc(r, &MCBPC_INTER, "invalid inter mcbpc")?;
    Ok(Some((idx as u8 / 4, idx as u8 % 4)))
}

/// Read `cbpy` (Table B-8). For inter macroblocks (`!intra`), the 4 bits are inverted.
pub(super) fn read_cbpy(r: &mut BitReader<'_>, intra: bool) -> Result<u8, DecodeError> {
    let idx = decode_small_vlc(r, &CBPY, "invalid cbpy")? as u8;
    Ok(if intra { idx } else { idx ^ 0xF })
}

/// Read `dct_dc_size_luminance` (Table B-13) or `dct_dc_size_chrominance` (Table B-14),
/// followed by `dct_dc_differential` and the marker bit when `size > 8`.
pub(super) fn read_dc_diff(r: &mut BitReader<'_>, luma: bool) -> Result<i32, DecodeError> {
    let size = if luma {
        decode_small_vlc(r, &DC_SIZE_LUMA, "invalid luma dc size")?
    } else {
        decode_small_vlc(r, &DC_SIZE_CHROMA, "invalid chroma dc size")?
    } as u32;
    if size == 0 {
        return Ok(0);
    }
    let raw = r.read_bits(size)? as i32;
    let diff = if (raw >> (size - 1)) == 0 { raw - ((1 << size) - 1) } else { raw };
    if size > 8 && !r.read_bit()? {
        return Err(DecodeError::Invalid("missing intra DC marker bit".into()));
    }
    Ok(diff)
}

/// Read one motion vector difference component (`motion_code` from Table B-12 + residual, §7.6.3)
/// and wrap `pred + diff` into the range of `fcode` (`1..=7`).
pub(super) fn read_mv_component(r: &mut BitReader<'_>, pred: i32, fcode: u8) -> Result<i32, DecodeError> {
    let code = decode_small_vlc(r, &MVD_TABLE, "invalid motion vector code")? as i32;
    if code == 0 {
        return Ok(pred);
    }
    let neg = r.read_bit()?;
    let r_size = u32::from(fcode.saturating_sub(1));
    let residual = if r_size > 0 { r.read_bits(r_size)? as i32 } else { 0 };
    let mag = ((code - 1) << r_size) + residual + 1;
    let diff = if neg { -mag } else { mag };
    let scale = 1i32 << r_size;
    let low = -32 * scale;
    let high = 32 * scale - 1;
    let range = 64 * scale;
    let mut mv = pred + diff;
    if mv < low {
        mv += range;
    } else if mv > high {
        mv -= range;
    }
    Ok(mv)
}

/// B-VOP macroblock prediction modes (Table B-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BMbType {
    Direct,
    Interpolate,
    Backward,
    Forward,
}

/// Read B-VOP `mb_type` (Table B-9: `1` Direct, `01` Interpolate, `001` Backward, `0001` Forward).
pub(super) fn read_b_mb_type(r: &mut BitReader<'_>) -> Result<BMbType, DecodeError> {
    if r.read_bit()? {
        return Ok(BMbType::Direct);
    }
    if r.read_bit()? {
        return Ok(BMbType::Interpolate);
    }
    if r.read_bit()? {
        return Ok(BMbType::Backward);
    }
    if r.read_bit()? {
        return Ok(BMbType::Forward);
    }
    Err(DecodeError::Invalid("invalid B-VOP mb_type".into()))
}

/// Fast 12-bit lookup table for TCOEF VLC decoding (Tables B-16 and B-17).
pub(super) struct TcoefDecode {
    /// Indexed by 12-bit peek value: `Some((last, run, level, code_len))` or `None`.
    lut: [Option<(bool, u8, u8, u8)>; 4096],
    /// `LMAX[last][run]`
    lmax: [[u8; 64]; 2],
    /// `RMAX[last][level]`
    rmax: [[u8; 28]; 2],
}

impl TcoefDecode {
    fn build(vlc: &[(u16, u8); 102], lmax_table: [&[u8]; 2]) -> Self {
        let mut lut = [None; 4096];
        let mut lmax = [[0u8; 64]; 2];
        let mut rmax = [[0u8; 28]; 2];
        let mut codes = vlc.iter();
        for (last_idx, runs) in lmax_table.iter().enumerate() {
            let last = last_idx == 1;
            for (run, &max_level) in runs.iter().enumerate() {
                lmax[last_idx][run] = max_level;
                for level in 1..=max_level {
                    let (code, len) = codes.next().copied().unwrap_or((0, 0));
                    rmax[last_idx][usize::from(level)] = rmax[last_idx][usize::from(level)].max(run as u8);
                    let shift = 12 - len;
                    let base = usize::from(code) << shift;
                    let count = 1usize << shift;
                    for slot in &mut lut[base..base + count] {
                        *slot = Some((last, run as u8, level, len));
                    }
                }
            }
        }
        Self { lut, lmax, rmax }
    }

    #[inline]
    fn read_vlc(&self, r: &mut BitReader<'_>) -> Result<(bool, usize, i32), DecodeError> {
        let peek = r.peek(12) as usize;
        if let Some((last, run, level, len)) = self.lut[peek]
            && usize::from(len) <= r.bits_left()
        {
            let _ = r.skip(usize::from(len));
            let sign = r.read_bit()?;
            let lv = i32::from(level);
            return Ok((last, usize::from(run), if sign { -lv } else { lv }));
        }
        Err(DecodeError::Invalid("invalid TCOEF VLC code".into()))
    }

    /// Read one `(last, run, level)` event, including the three MPEG-4 escape modes (§7.4.1.3)
    /// or the H.263 short-header fixed escape when `short_header` is set.
    pub(super) fn read_event(&self, r: &mut BitReader<'_>, short_header: bool) -> Result<(bool, usize, i32), DecodeError> {
        if r.bits_left() >= usize::from(ESCAPE.1) && r.peek(u32::from(ESCAPE.1)) == u32::from(ESCAPE.0) {
            let _ = r.skip(usize::from(ESCAPE.1));
            if short_header {
                let last = r.read_bit()?;
                let run = r.read_bits(6)? as usize;
                let raw = r.read_bits(8)? as u8;
                if raw == 0 {
                    return Err(DecodeError::Invalid("forbidden H.263 escape level 0".into()));
                }
                let level = if raw == 0x80 {
                    // H.263+ Annex T extended escape level (11-bit signed two's complement, cyclic code).
                    let low = r.read_bits(5)?;
                    let high = r.read_bits(6)?;
                    let v11 = ((high << 5) | low) as i32;
                    if v11 & 0x400 != 0 { v11 - 0x800 } else { v11 }
                } else {
                    i32::from(raw as i8)
                };
                return Ok((last, run, level));
            }
            // Escape mode 1: `0` + VLC(last, run, level - LMAX)
            if !r.read_bit()? {
                let (last, run, level) = self.read_vlc(r)?;
                let lmax = i32::from(self.lmax[usize::from(last)].get(run).copied().unwrap_or(0));
                if lmax == 0 {
                    return Err(DecodeError::Invalid("escape mode 1 run out of range".into()));
                }
                let adjusted = if level < 0 { level - lmax } else { level + lmax };
                return Ok((last, run, adjusted));
            }
            // Escape mode 2: `10` + VLC(last, run - (RMAX + 1), level)
            if !r.read_bit()? {
                let (last, run, level) = self.read_vlc(r)?;
                let mag = level.unsigned_abs() as usize;
                let rmax = usize::from(
                    self.rmax[usize::from(last)].get(mag).copied().ok_or_else(|| DecodeError::Invalid("escape mode 2 level out of range".into()))?,
                );
                return Ok((last, run + rmax + 1, level));
            }
            // Escape mode 3: `11` + `last` (1) + `run` (6) + marker (1) + `level` (12 signed) + marker (1)
            let last = r.read_bit()?;
            let run = r.read_bits(6)? as usize;
            if !r.read_bit()? {
                return Err(DecodeError::Invalid("missing escape mode 3 first marker bit".into()));
            }
            let raw_level = r.read_bits(12)? as i32;
            if !r.read_bit()? {
                return Err(DecodeError::Invalid("missing escape mode 3 second marker bit".into()));
            }
            let level = if raw_level & 0x800 != 0 { raw_level - 0x1000 } else { raw_level };
            if level == 0 || level == -2048 {
                return Err(DecodeError::Invalid("forbidden escape mode 3 level".into()));
            }
            return Ok((last, run, level));
        }
        self.read_vlc(r)
    }
}

/// The intra (Table B-16) TCOEF decoder.
pub(super) fn intra_tcoef() -> &'static TcoefDecode {
    static T: OnceLock<TcoefDecode> = OnceLock::new();
    T.get_or_init(|| TcoefDecode::build(&INTRA_VLC, INTRA_LMAX))
}

/// The inter (Table B-17) TCOEF decoder.
pub(super) fn inter_tcoef() -> &'static TcoefDecode {
    static T: OnceLock<TcoefDecode> = OnceLock::new();
    T.get_or_init(|| TcoefDecode::build(&INTER_VLC, INTER_LMAX))
}

/// Read the run-length coded coefficients of an 8×8 block from scan index `start` (`0` or `1`)
/// into `levels` (indexed by raster position `0..64` via `scan`).
pub(super) fn read_block_coeffs(
    r: &mut BitReader<'_>,
    table: &TcoefDecode,
    scan: &[usize; 64],
    start: usize,
    short_header: bool,
    levels: &mut [i32; 64],
) -> Result<(), DecodeError> {
    let mut idx = start;
    loop {
        let (last, run, level) = table.read_event(r, short_header)?;
        idx = idx.checked_add(run).filter(|&i| i < 64).ok_or_else(|| DecodeError::Invalid("DCT coefficient run exceeds 64 coefficients".into()))?;
        levels[scan[idx]] = level;
        idx += 1;
        if last {
            break;
        }
        if idx >= 64 {
            return Err(DecodeError::Invalid("unterminated DCT block after 64 coefficients".into()));
        }
    }
    Ok(())
}
