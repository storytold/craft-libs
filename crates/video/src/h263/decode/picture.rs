//! Macroblock parsing and reconstruction (ITU-T H.263 §5.3, §5.4, §6 and Annexes D, F, J).
//!
//! The syntax tables, the inverse transform and the half-sample interpolation are the ones the
//! MPEG-4 Part 2 decoder shares: H.263's TCOEF, MCBPC (I), CBPY and MVD codes are the codes of
//! its short video header.

use craft_media::bitstream::BitReader;

use super::DecodeError;
use super::headers::{PictureHeader, PictureType, VectorLimit, read_gob_header};
use crate::mpeg4::decode::mc::{Plane, chroma_mv_1mv, chroma_mv_4mv, predict_halfpel};
use crate::mpeg4::decode::picture::idct;
use crate::mpeg4::decode::vlc::{MVD_TABLE, ZIGZAG, decode_small_vlc, inter_tcoef, read_block_coeffs, read_cbpy, read_mcbpc_intra};

/// `DQUANT` (§5.3.6): the change of QUANT for each 2-bit code.
const DQUANT: [i32; 4] = [-1, -2, 1, 2];

/// Table 8: `(code, length)` of MCBPC in P-pictures, indexed by `4 * type + CBPC` for the
/// types 0 to 4, then the four type-5 codes.
pub(super) const MCBPC_INTER: [(u16, u8); 24] = [
    (0b1, 1),
    (0b0011, 4),
    (0b0010, 4),
    (0b000101, 6),
    (0b011, 3),
    (0b0000111, 7),
    (0b0000110, 7),
    (0b000000101, 9),
    (0b010, 3),
    (0b0000101, 7),
    (0b0000100, 7),
    (0b00000101, 8),
    (0b00011, 5),
    (0b00000100, 8),
    (0b00000011, 8),
    (0b0000011, 7),
    (0b000100, 6),
    (0b000000100, 9),
    (0b000000011, 9),
    (0b000000010, 9),
    (0b00000000010, 11),
    (0b0000000001100, 13),
    (0b0000000001110, 13),
    (0b0000000001111, 13),
];

/// The macroblock stuffing code `0000 0000 1` (Table 8).
const MCBPC_STUFFING: (u16, u8) = (1, 9);

/// Table J.2: `STRENGTH` of the deblocking filter for QUANT 1 to 31.
const STRENGTH: [i32; 32] = [0, 1, 1, 2, 2, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 11, 12, 12, 12];

/// Figures F.2, F.3 and F.4: the weights of the three predictions of overlapped motion
/// compensation. Each pixel's weights add up to 8.
const H0: [[u8; 8]; 8] = [
    [4, 5, 5, 5, 5, 5, 5, 4],
    [5, 5, 5, 5, 5, 5, 5, 5],
    [5, 5, 6, 6, 6, 6, 5, 5],
    [5, 5, 6, 6, 6, 6, 5, 5],
    [5, 5, 6, 6, 6, 6, 5, 5],
    [5, 5, 6, 6, 6, 6, 5, 5],
    [5, 5, 5, 5, 5, 5, 5, 5],
    [4, 5, 5, 5, 5, 5, 5, 4],
];
const H1: [[u8; 8]; 8] = [
    [2, 2, 2, 2, 2, 2, 2, 2],
    [1, 1, 2, 2, 2, 2, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 2, 2, 2, 2, 1, 1],
    [2, 2, 2, 2, 2, 2, 2, 2],
];
const H2: [[u8; 8]; 8] = [
    [2, 1, 1, 1, 1, 1, 1, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 2, 1, 1, 1, 1, 2, 2],
    [2, 1, 1, 1, 1, 1, 1, 2],
];

/// A decoded picture at macroblock-aligned size.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub width: u32,
    pub height: u32,
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Self {
        let (mb_w, mb_h) = (width.div_ceil(16) as usize, height.div_ceil(16) as usize);
        Self { width, height, y: Plane::new(mb_w * 16, mb_h * 16, 0), u: Plane::new(mb_w * 8, mb_h * 8, 128), v: Plane::new(mb_w * 8, mb_h * 8, 128) }
    }
}

/// What the later macroblocks and the deblocking filter need to know about a macroblock.
#[derive(Clone, Copy, Debug, Default)]
struct MbInfo {
    /// INTRA macroblock: its vectors read as zero to the predictors.
    intra: bool,
    /// COD was 0, or the macroblock is INTRA (the "coded macroblock" of Annex J).
    coded: bool,
    quant: u8,
    /// The four luminance vectors in half samples; all equal for one vector per macroblock.
    mvs: [(i32, i32); 4],
}

/// The transform output of one macroblock: samples for INTRA, a residual for INTER.
struct MbBlocks {
    data: [[i16; 64]; 6],
    present: [bool; 6],
}

impl MbBlocks {
    fn new() -> Self {
        Self { data: [[0; 64]; 6], present: [false; 6] }
    }
}

/// Parse and reconstruct one picture; `reference` is the previous picture for an INTER picture.
pub(super) fn decode_picture(r: &mut BitReader<'_>, hdr: &PictureHeader, reference: Option<&Frame>) -> Result<Frame, DecodeError> {
    let (mb_w, mb_h) = hdr.mb_dims();
    let inter = hdr.picture_type == PictureType::Inter;
    let reference = if inter {
        let f = reference.ok_or_else(|| DecodeError::Invalid("INTER picture without a reference picture".into()))?;
        if f.width != hdr.width || f.height != hdr.height {
            return Err(DecodeError::Invalid("INTER picture does not have its reference picture's size".into()));
        }
        Some(f)
    } else {
        None
    };
    let mut pic = Picture {
        hdr,
        mb_w,
        mb_h,
        reference,
        out: Frame::new(hdr.width, hdr.height),
        info: vec![MbInfo::default(); mb_w * mb_h],
        quant: i32::from(hdr.quant),
    };
    let rows = hdr.rows_per_gob();
    let mut blocks: Vec<MbBlocks> = (0..mb_w).map(|_| MbBlocks::new()).collect();
    for gob in 0..mb_h.div_ceil(rows) {
        let header = if gob == 0 { None } else { read_gob_header(r, gob)? };
        if let Some(q) = header {
            pic.quant = i32::from(q);
        }
        for row in gob * rows..((gob + 1) * rows).min(mb_h) {
            // Vectors above the GOB are out of reach when the GOB header is not empty (§6.1.1).
            let top_limit = if header.is_some() { 2 * gob * rows } else { 0 };
            for (mx, mb) in blocks.iter_mut().enumerate() {
                pic.parse_macroblock(r, mx, row, top_limit, mb)?;
            }
            for (mx, mb) in blocks.iter().enumerate() {
                pic.reconstruct(mx, row, mb);
            }
        }
    }
    if hdr.modes.deblocking {
        deblock(&mut pic.out, &pic.info, mb_w, mb_h);
    }
    Ok(pic.out)
}

struct Picture<'a> {
    hdr: &'a PictureHeader,
    mb_w: usize,
    mb_h: usize,
    reference: Option<&'a Frame>,
    out: Frame,
    info: Vec<MbInfo>,
    quant: i32,
}

/// Table 14 (or its MPEG-4 spelling: the magnitude code and a sign bit): a vector difference in
/// half samples.
fn read_table14(r: &mut BitReader<'_>) -> Result<i32, DecodeError> {
    let magnitude = decode_small_vlc(r, &MVD_TABLE, "invalid motion vector difference code")? as i32;
    if magnitude != 0 && r.read_bit()? { Ok(-magnitude) } else { Ok(magnitude) }
}

/// Table D.3: a vector difference in half samples, interleaved with continuation bits.
pub(super) fn read_table_d3(r: &mut BitReader<'_>) -> Result<i32, DecodeError> {
    if r.read_bit()? {
        return Ok(0);
    }
    let mut value = 1i32;
    for _ in 0..12 {
        let bit = r.read_bit()?;
        if !r.read_bit()? {
            // The second bit ended the code: the first was the sign.
            return Ok(if bit { -value } else { value });
        }
        value = (value << 1) | i32::from(bit);
    }
    Err(DecodeError::Invalid("motion vector difference code is too long".into()))
}

impl Picture<'_> {
    /// Read one component pair `(MVD x, MVD y)` and add it to the predictor `pred` (§6.1.1, Annex D).
    fn read_vector(&self, r: &mut BitReader<'_>, pred: (i32, i32)) -> Result<(i32, i32), DecodeError> {
        let modes = self.hdr.modes;
        if modes.umv && self.hdr.plus {
            let (dx, dy) = (read_table_d3(r)?, read_table_d3(r)?);
            if dx == 1 && dy == 1 && !r.read_bit()? {
                // (0.5, 0.5) is followed by a 1 bit that prevents start code emulation (§D.2).
                return Err(DecodeError::Invalid("missing bit after the motion vector difference (0.5, 0.5)".into()));
            }
            let (mx, my) = (pred.0 + dx, pred.1 + dy);
            let (limit_x, limit_y) = self.vector_limits();
            if mx.abs() > limit_x || my.abs() > limit_y {
                return Err(DecodeError::Invalid("motion vector outside the allowed range".into()));
            }
            return Ok((mx, my));
        }
        let (sx, sy) = (read_table14(r)?, read_table14(r)?);
        Ok((self.resolve(pred.0, sx)?, self.resolve(pred.1, sy)?))
    }

    /// Largest vector magnitude, in half samples, per axis (Tables D.1 and D.2, or the bounds).
    fn vector_limits(&self) -> (i32, i32) {
        let hdr = self.hdr;
        match hdr.vector_limit {
            VectorLimit::ByFormat => {
                let x = match hdr.width {
                    0..=352 => 64,
                    353..=704 => 128,
                    705..=1408 => 256,
                    _ => 512,
                };
                let y = match hdr.height {
                    0..=288 => 64,
                    289..=576 => 128,
                    _ => 256,
                };
                (x, y)
            }
            VectorLimit::Unlimited => (2 * (hdr.width as i32 + 32), 2 * (hdr.height as i32 + 32)),
        }
    }

    /// Pick the vector component `pred + difference` that is in range (§6.1.1, §D.2). `s` is the
    /// signed difference of Table 14; the other reading of its code word is `s` shifted by 64.
    fn resolve(&self, pred: i32, s: i32) -> Result<i32, DecodeError> {
        let first = if s > 31 {
            s - 64
        } else if s < -32 {
            s + 64
        } else {
            s
        };
        let second = if first >= 0 { first - 64 } else { first + 64 };
        if !self.hdr.modes.umv {
            return Ok([first, second].into_iter().map(|d| pred + d).find(|mv| (-32..=31).contains(mv)).unwrap_or(pred + first));
        }
        if (-31..=32).contains(&pred) {
            return Ok(pred + first);
        }
        // The predictor is out of range: the vector has its sign and reaches at most 31.5 samples.
        let range = if pred > 0 { 0..=63 } else { -63..=0 };
        [first, second]
            .into_iter()
            .map(|d| pred + d)
            .find(|mv| range.contains(mv))
            .ok_or_else(|| DecodeError::Invalid("motion vector outside the allowed range".into()))
    }

    /// The vector of the luminance block `(gx, gy)` in the grid of 8x8 blocks, as the predictors
    /// see it: a vector of the current macroblock (`cur`) or of an earlier one.
    fn block_vector(&self, gx: usize, gy: usize, mb: (usize, usize), cur: &[(i32, i32); 4]) -> (i32, i32) {
        let (mx, my) = (gx / 2, gy / 2);
        let blk = (gx & 1) + 2 * (gy & 1);
        if (mx, my) == mb { cur[blk] } else { self.info[my * self.mb_w + mx].mvs[blk] }
    }

    /// The median predictor of block `b` (§6.1.1 with Figure 12 and Figure F.1).
    fn predict_vector(&self, mb: (usize, usize), b: usize, top_limit: usize, cur: &[(i32, i32); 4]) -> (i32, i32) {
        let gx = (2 * mb.0 + (b & 1)) as isize;
        let gy = (2 * mb.1 + (b >> 1)) as isize;
        let top_limit = top_limit as isize;
        let at = |x: isize, y: isize| self.block_vector(x as usize, y as usize, mb, cur);
        let v1 = if gx == 0 { (0, 0) } else { at(gx - 1, gy) };
        let above = gy - 1 < top_limit;
        let v2 = if above { v1 } else { at(gx, gy - 1) };
        // The third candidate: the macroblock above and to the right for blocks 0 and 1, the
        // block above and to the right inside this macroblock for block 2, the block above and
        // to the left for block 3 (Figure F.1).
        let (x3, y3) = match b {
            0 => (gx + 2, gy - 1),
            3 => (gx - 1, gy - 1),
            _ => (gx + 1, gy - 1),
        };
        let v3 = if x3 >= 2 * self.mb_w as isize {
            (0, 0)
        } else if above {
            v1
        } else {
            at(x3, y3)
        };
        (median(v1.0, v2.0, v3.0), median(v1.1, v2.1, v3.1))
    }

    fn parse_macroblock(&mut self, r: &mut BitReader<'_>, mx: usize, my: usize, top_limit: usize, out: &mut MbBlocks) -> Result<(), DecodeError> {
        out.present = [false; 6];
        let idx = my * self.mb_w + mx;
        let inter_picture = self.hdr.picture_type == PictureType::Inter;
        let (mb_type, cbpc) = if inter_picture {
            loop {
                if r.read_bit()? {
                    // COD: not coded, a zero vector and no residual.
                    self.info[idx] = MbInfo { intra: false, coded: false, quant: self.quant as u8, mvs: [(0, 0); 4] };
                    return Ok(());
                }
                if let Some(code) = read_mcbpc_inter(r)? {
                    break code;
                }
            }
        } else {
            read_mcbpc_intra(r)?
        };
        let intra = mb_type == 3 || mb_type == 4;
        let four_vectors = mb_type == 2 || mb_type == 5;
        if four_vectors && !self.hdr.four_vectors() {
            return Err(DecodeError::Invalid("four vectors per macroblock without Advanced Prediction or the Deblocking Filter".into()));
        }
        let cbpy = read_cbpy(r, intra)?;
        let cbp = (cbpy << 2) | cbpc;
        if matches!(mb_type, 1 | 4 | 5) {
            self.quant = (self.quant + DQUANT[r.read_bits(2)? as usize]).clamp(1, 31);
        }
        let quant = self.quant;
        let mut mvs = [(0, 0); 4];
        if !intra {
            if four_vectors {
                for b in 0..4 {
                    let pred = self.predict_vector((mx, my), b, top_limit, &mvs);
                    mvs[b] = self.read_vector(r, pred)?;
                }
            } else {
                let pred = self.predict_vector((mx, my), 0, top_limit, &mvs);
                mvs = [self.read_vector(r, pred)?; 4];
            }
        }
        self.info[idx] = MbInfo { intra, coded: true, quant: quant as u8, mvs };
        for b in 0..6 {
            let coded = cbp & (1 << (5 - b)) != 0;
            let mut levels = [0i32; 64];
            if intra {
                let code = r.read_bits(8)?;
                let dc = match code {
                    0 | 128 => return Err(DecodeError::Invalid(format!("forbidden INTRADC code {code}"))),
                    255 => 1024,
                    c => c as i32 * 8,
                };
                if coded {
                    read_block_coeffs(r, inter_tcoef(), &ZIGZAG, 1, true, &mut levels)?;
                }
                dequantize(&mut levels, quant, 1);
                levels[0] = dc;
            } else if coded {
                read_block_coeffs(r, inter_tcoef(), &ZIGZAG, 0, true, &mut levels)?;
                dequantize(&mut levels, quant, 0);
            } else {
                continue;
            }
            idct(&mut levels);
            let (lo, hi) = if intra { (0, 255) } else { (-256, 255) };
            out.data[b] = std::array::from_fn(|i| levels[i].clamp(lo, hi) as i16);
            out.present[b] = true;
        }
        Ok(())
    }

    /// The vector that stands in for the block at `(gx, gy)` in overlapped motion compensation
    /// (§F.3): the current block's vector where there is no usable neighbour.
    fn remote_vector(&self, gx: isize, gy: isize, mb: (usize, usize), cur: (i32, i32), own: &[(i32, i32); 4]) -> (i32, i32) {
        if gx < 0 || gy < 0 || gx >= 2 * self.mb_w as isize || gy >= 2 * self.mb_h as isize {
            return cur;
        }
        let (nx, ny) = (gx as usize / 2, gy as usize / 2);
        if (nx, ny) == mb {
            return own[(gx as usize & 1) + 2 * (gy as usize & 1)];
        }
        let n = &self.info[ny * self.mb_w + nx];
        if n.intra { cur } else { n.mvs[(gx as usize & 1) + 2 * (gy as usize & 1)] }
    }

    fn reconstruct(&mut self, mx: usize, my: usize, blocks: &MbBlocks) {
        let info = self.info[my * self.mb_w + mx];
        if info.intra {
            for b in 0..6 {
                let samples: [u8; 64] = std::array::from_fn(|i| blocks.data[b][i] as u8);
                write_block(&mut self.out, b, mx, my, &samples);
            }
            return;
        }
        let Some(reference) = self.reference else { return };
        let rounding = self.hdr.rounding;
        let (x0, y0) = ((mx * 16) as i32, (my * 16) as i32);
        let (cx0, cy0) = ((mx * 8) as i32, (my * 8) as i32);
        let one_vector = info.mvs.iter().all(|&v| v == info.mvs[0]);
        let mut luma = [0u8; 256];
        if self.hdr.modes.advanced_prediction {
            for b in 0..4 {
                let (bx, by) = ((b & 1) * 8, (b >> 1) * 8);
                let (gx, gy) = ((2 * mx + (b & 1)) as isize, (2 * my + (b >> 1)) as isize);
                let cur = info.mvs[b];
                // Upper half of the block borrows from the block above, lower half from the one below.
                let vertical = [(gx, gy - 1), (gx, gy + 1)];
                let horizontal = [(gx - 1, gy), (gx + 1, gy)];
                let mut pred = [[0u8; 64]; 5];
                let origin = (x0 + bx as i32, y0 + by as i32);
                predict_halfpel(&reference.y, origin.0, origin.1, cur, 8, rounding, &mut pred[0]);
                for (k, &(nx, ny)) in vertical.iter().chain(horizontal.iter()).enumerate() {
                    // Below the macroblock, the current vector stands in (§F.3).
                    let mv = if (b >> 1) == 1 && k == 1 { cur } else { self.remote_vector(nx, ny, (mx, my), cur, &info.mvs) };
                    predict_halfpel(&reference.y, origin.0, origin.1, mv, 8, rounding, &mut pred[k + 1]);
                }
                for j in 0..8 {
                    for i in 0..8 {
                        let (vert, horiz) = (usize::from(j >= 4) + 1, usize::from(i >= 4) + 3);
                        let sum = u32::from(pred[0][j * 8 + i]) * u32::from(H0[j][i])
                            + u32::from(pred[vert][j * 8 + i]) * u32::from(H1[j][i])
                            + u32::from(pred[horiz][j * 8 + i]) * u32::from(H2[j][i]);
                        luma[(by + j) * 16 + bx + i] = ((sum + 4) >> 3) as u8;
                    }
                }
            }
        } else if one_vector {
            predict_halfpel(&reference.y, x0, y0, info.mvs[0], 16, rounding, &mut luma);
        } else {
            let mut sub = [0u8; 64];
            for (b, &mv) in info.mvs.iter().enumerate() {
                let (bx, by) = ((b & 1) * 8, (b >> 1) * 8);
                predict_halfpel(&reference.y, x0 + bx as i32, y0 + by as i32, mv, 8, rounding, &mut sub);
                for j in 0..8 {
                    luma[(by + j) * 16 + bx..][..8].copy_from_slice(&sub[j * 8..j * 8 + 8]);
                }
            }
        }
        let chroma_mv = if one_vector { chroma_mv_1mv(info.mvs[0], false) } else { chroma_mv_4mv(&info.mvs, false) };
        let mut cb = [0u8; 64];
        let mut cr = [0u8; 64];
        predict_halfpel(&reference.u, cx0, cy0, chroma_mv, 8, rounding, &mut cb);
        predict_halfpel(&reference.v, cx0, cy0, chroma_mv, 8, rounding, &mut cr);
        for b in 0..6 {
            let mut samples = [0u8; 64];
            match b {
                0..=3 => {
                    let (bx, by) = ((b & 1) * 8, (b >> 1) * 8);
                    for j in 0..8 {
                        samples[j * 8..j * 8 + 8].copy_from_slice(&luma[(by + j) * 16 + bx..][..8]);
                    }
                }
                4 => samples = cb,
                _ => samples = cr,
            }
            if blocks.present[b] {
                for (s, &res) in samples.iter_mut().zip(blocks.data[b].iter()) {
                    *s = (i32::from(*s) + i32::from(res)).clamp(0, 255) as u8;
                }
            }
            write_block(&mut self.out, b, mx, my, &samples);
        }
    }
}

/// Read MCBPC of a P-picture macroblock (Table 8); `None` after a stuffing code.
pub(super) fn read_mcbpc_inter(r: &mut BitReader<'_>) -> Result<Option<(u8, u8)>, DecodeError> {
    let peek = r.peek(13) as u16;
    let left = r.bits_left();
    let matches = |(code, len): (u16, u8)| usize::from(len) <= left && (peek >> (13 - len)) == code;
    if matches(MCBPC_STUFFING) {
        r.skip(usize::from(MCBPC_STUFFING.1))?;
        return Ok(None);
    }
    let Some(index) = MCBPC_INTER.iter().position(|&c| matches(c)) else {
        return Err(DecodeError::Invalid("invalid MCBPC code in a P-picture".into()));
    };
    r.skip(usize::from(MCBPC_INTER[index].1))?;
    Ok(Some((index as u8 / 4, index as u8 % 4)))
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// Inverse quantisation of the levels from index `first` on (§6.2.1, §6.2.2).
fn dequantize(levels: &mut [i32; 64], quant: i32, first: usize) {
    for level in &mut levels[first..] {
        if *level == 0 {
            continue;
        }
        let magnitude = quant * (2 * level.abs() + 1) - i32::from(quant % 2 == 0);
        *level = if *level < 0 { -magnitude } else { magnitude }.clamp(-2048, 2047);
    }
}

/// Store the 8x8 `samples` of block `b` (0 to 3 luminance, 4 and 5 chrominance) of macroblock `(mx, my)`.
fn write_block(frame: &mut Frame, b: usize, mx: usize, my: usize, samples: &[u8; 64]) {
    let (plane, x0, y0) = match b {
        0..=3 => (&mut frame.y, mx * 16 + (b & 1) * 8, my * 16 + (b >> 1) * 8),
        4 => (&mut frame.u, mx * 8, my * 8),
        _ => (&mut frame.v, mx * 8, my * 8),
    };
    for j in 0..8 {
        plane.data[(y0 + j) * plane.w + x0..][..8].copy_from_slice(&samples[j * 8..j * 8 + 8]);
    }
}

/// The deblocking edge filter of Annex J (§J.3): every horizontal edge, then every vertical one.
fn deblock(frame: &mut Frame, info: &[MbInfo], mb_w: usize, mb_h: usize) {
    for vertical in [false, true] {
        for (plane, mb_size) in [(&mut frame.y, 16usize), (&mut frame.u, 8), (&mut frame.v, 8)] {
            let (w, h) = (plane.w, plane.h);
            let (across, along) = if vertical { (w, h) } else { (h, w) };
            for edge in (8..across).step_by(8) {
                for pos in 0..along {
                    let (x, y) = if vertical { (edge, pos) } else { (pos, edge) };
                    let mb1 = if vertical { (x - 1) / mb_size + y / mb_size * mb_w } else { x / mb_size + (y - 1) / mb_size * mb_w };
                    let mb2 = x / mb_size + y / mb_size * mb_w;
                    debug_assert!(mb1 < mb_w * mb_h && mb2 < mb_w * mb_h);
                    let (a, b) = (info[mb1], info[mb2]);
                    if !a.coded && !b.coded {
                        continue;
                    }
                    let quant = if b.coded { b.quant } else { a.quant };
                    let step = if vertical { 1 } else { w };
                    let base = y * w + x;
                    filter_edge(&mut plane.data, base, step, STRENGTH[usize::from(quant).min(31)]);
                }
            }
        }
    }
}

/// Filter the four samples `A B | C D` across an edge; `base` is `C` and `step` the distance between samples.
pub(super) fn filter_edge(data: &mut [u8], base: usize, step: usize, strength: i32) {
    let [a, b, c, d] = [data[base - 2 * step], data[base - step], data[base], data[base + step]].map(i32::from);
    let delta = (a - 4 * b + 4 * c - d) / 8;
    let ramp = delta.signum() * (delta.abs() - (2 * (delta.abs() - strength)).max(0)).max(0);
    let limit = (ramp / 2).abs();
    let d2 = ((a - d) / 4).clamp(-limit, limit);
    let clip = |v: i32| v.clamp(0, 255) as u8;
    data[base - 2 * step] = clip(a - d2);
    data[base - step] = clip(b + ramp);
    data[base] = clip(c - ramp);
    data[base + step] = clip(d + d2);
}
