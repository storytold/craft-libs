//! One video object plane: mode decision, motion estimation, transform and quantisation, the
//! reconstruction a decoder makes, and the macroblock syntax (ISO/IEC 14496-2 §6.2.6–6.2.8,
//! §7.4–7.7).

use craft_media::bitstream::BitWriter;

use super::dct::{fdct, idct};
use super::headers::{self, VopCodingType};
use super::tables::{
    self, ALT_HORIZONTAL_SCAN, ALT_VERTICAL_SCAN, B_MB_TYPE, CBPY, DC_SIZE_CHROMA, DC_SIZE_LUMA, ESCAPE, MCBPC_INTER, MCBPC_INTRA, MVD, Tcoef, ZIGZAG,
};

/// Table 7-6: sixteen-division rounding table for deriving a 4:2:0 chrominance motion vector
/// from the sum of four 8×8 luminance motion vectors in 4MV mode (§7.6.1.6).
const ROUNDTAB_16: [i32; 16] = [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2];

/// An 8-bit sample plane, `w` samples a row.
#[derive(Clone, Debug)]
pub(super) struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, data: vec![0; w * h] }
    }

    /// The sample at `(x, y)`, coordinates clamped to the plane.
    #[inline]
    fn at(&self, x: i32, y: i32) -> i32 {
        let x = x.clamp(0, self.w as i32 - 1) as usize;
        let y = y.clamp(0, self.h as i32 - 1) as usize;
        i32::from(self.data[y * self.w + x])
    }
}

/// Per-macroblock metadata retained on reference frames for B-VOP direct mode (§7.6.4).
#[derive(Clone, Copy, Debug)]
pub(super) struct RefMb {
    pub not_coded: bool,
    pub intra: bool,
    pub mvs: [(i32, i32); 4],
}

impl Default for RefMb {
    fn default() -> Self {
        Self { not_coded: true, intra: false, mvs: [(0, 0); 4] }
    }
}

/// A picture at macroblock-aligned size, with macroblock motion vectors for B-VOP direct mode.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
    pub mbs: Vec<RefMb>,
}

impl Frame {
    pub fn new(mb_w: usize, mb_h: usize) -> Frame {
        Frame {
            y: Plane::new(mb_w * 16, mb_h * 16),
            u: Plane::new(mb_w * 8, mb_h * 8),
            v: Plane::new(mb_w * 8, mb_h * 8),
            mbs: vec![RefMb::default(); mb_w * mb_h],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Intra,
    Inter,
    Inter4v,
    /// `not_coded`: the co-located macroblock of the reference, no residual.
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BMode {
    /// Co-located macroblock in the backward reference was `not_coded`: no bits written.
    ColocatedSkip,
    /// Direct mode (`modb = 1` when `cbp == 0 && dbmv == (0, 0)`, else `mb_type = 1`).
    Direct {
        dbmv: (i32, i32),
    },
    Interpolate,
    Backward,
    Forward,
}

/// Scan order selected for each of the 6 blocks of an intra macroblock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScanKind {
    Zigzag,
    AltHorizontal,
    AltVertical,
}

impl ScanKind {
    pub fn table(self) -> &'static [usize; 64] {
        match self {
            ScanKind::Zigzag => &ZIGZAG,
            ScanKind::AltHorizontal => &ALT_HORIZONTAL_SCAN,
            ScanKind::AltVertical => &ALT_VERTICAL_SCAN,
        }
    }
}

/// A coded macroblock of an I-, P-, or B-VOP.
#[derive(Clone, Debug)]
pub(super) struct Mb {
    pub mode: Mode,
    pub b_mode: Option<BMode>,
    /// Motion vectors of the 4 8×8 luminance blocks (for 1MV, all 4 entries are identical).
    pub mvs: [(i32, i32); 4],
    /// Their spatial predictors (§7.6.5).
    pub preds: [(i32, i32); 4],
    /// Backward motion vector and predictor in B-VOPs.
    pub bwd_mv: (i32, i32),
    pub bwd_pred: (i32, i32),
    /// Intra AC prediction flag (`ac_pred_flag`).
    pub ac_pred: bool,
    /// Per-block scan selection for intra macroblocks.
    pub scans: [ScanKind; 6],
    /// Video packet slice identifier (`0, 1, ...`).
    pub slice_id: usize,
    /// Coded block pattern, block `i` in bit `5 - i` (Y0–Y3, Cb, Cr). Intra: the AC coefficients.
    pub cbp: u8,
    /// Intra: `dct_dc_differential` of each block.
    pub dc_diff: [i32; 6],
    /// Quantised levels in raster order (intra: index 0 is unused).
    pub levels: [[i16; 64]; 6],
}

impl Mb {
    pub fn skip() -> Mb {
        Mb {
            mode: Mode::Skip,
            b_mode: None,
            mvs: [(0, 0); 4],
            preds: [(0, 0); 4],
            bwd_mv: (0, 0),
            bwd_pred: (0, 0),
            ac_pred: false,
            scans: [ScanKind::Zigzag; 6],
            slice_id: 0,
            cbp: 0,
            dc_diff: [0; 6],
            levels: [[0; 64]; 6],
        }
    }
}

/// Table 7-1: `dc_scaler` of luminance and chrominance blocks.
pub(super) fn dc_scaler(qp: i32, luma: bool) -> i32 {
    match (luma, qp) {
        (_, ..=4) => 8,
        (true, 5..=8) => 2 * qp,
        (true, 9..=24) => qp + 8,
        (true, _) => 2 * qp - 16,
        (false, 5..=24) => (qp + 13) / 2,
        (false, _) => qp - 6,
    }
}

/// The second inverse quantisation method (H.263 style, §7.4.2.2), saturated to [-2048, 2047].
#[inline]
fn dequant_h263(level: i32, qp: i32) -> i32 {
    if level == 0 {
        return 0;
    }
    let mag = qp * (2 * level.abs() + 1) - if qp & 1 == 0 { 1 } else { 0 };
    (if level < 0 { -mag } else { mag }).clamp(-2048, 2047)
}

/// Inverse-quantise an intra block into `rec` (ISO/IEC 14496-2 §7.4.2.1 and §7.4.2.2).
fn dequant_intra_block(dc_unquant: i32, levels: &[i16; 64], qp: i32, mpeg_quant: bool, mat: &[u8; 64], rec: &mut [i32; 64]) {
    rec[0] = dc_unquant.clamp(-2048, 2047);
    if !mpeg_quant {
        for k in 1..64 {
            rec[k] = dequant_h263(i32::from(levels[k]), qp);
        }
    } else {
        for k in 1..64 {
            let lv = i32::from(levels[k]);
            if lv == 0 {
                rec[k] = 0;
            } else {
                let w = i64::from(mat[k]);
                let mag = (2 * i64::from(lv.unsigned_abs()) * w * i64::from(qp)) >> 4;
                rec[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047) as i32;
            }
        }
        let sum: i32 = rec.iter().sum();
        if sum & 1 == 0 {
            rec[63] ^= 1;
        }
    }
}

/// Inverse-quantise an inter block into `rec` (ISO/IEC 14496-2 §7.4.2.1 and §7.4.2.2).
fn dequant_inter_block(levels: &[i16; 64], qp: i32, mpeg_quant: bool, mat: &[u8; 64], rec: &mut [i32; 64]) {
    if !mpeg_quant {
        for k in 0..64 {
            rec[k] = dequant_h263(i32::from(levels[k]), qp);
        }
    } else {
        for k in 0..64 {
            let lv = i32::from(levels[k]);
            if lv == 0 {
                rec[k] = 0;
            } else {
                let w = i64::from(mat[k]);
                let mag = ((2 * i64::from(lv.unsigned_abs()) + 1) * w * i64::from(qp)) >> 4;
                rec[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047) as i32;
            }
        }
        let sum: i32 = rec.iter().sum();
        if sum & 1 == 0 {
            rec[63] ^= 1;
        }
    }
}

/// Forward quantise one AC coefficient of an intra block.
#[inline]
fn quant_intra_ac(c: i32, qp: i32, mpeg_quant: bool, weight: u8) -> i32 {
    if !mpeg_quant {
        let l = (c.abs() / (2 * qp)).min(MAX_LEVEL);
        if c < 0 { -l } else { l }
    } else {
        let w = i32::from(weight);
        let denom = 2 * w * qp;
        let l = ((c.abs() * 16 + denom / 2) / denom).min(MAX_LEVEL);
        if c < 0 { -l } else { l }
    }
}

/// Forward quantise one coefficient of an inter block.
#[inline]
fn quant_inter_coef(c: i32, qp: i32, mpeg_quant: bool, weight: u8) -> i32 {
    if !mpeg_quant {
        let l = ((c.abs() - qp / 2) / (2 * qp)).clamp(0, MAX_LEVEL);
        if c < 0 { -l } else { l }
    } else {
        let w = i32::from(weight);
        let scaled = c.abs() * 16;
        let bias = (w * qp) / 2;
        let denom = 2 * w * qp;
        let l = ((scaled - bias) / denom).clamp(0, MAX_LEVEL);
        if c < 0 { -l } else { l }
    }
}

#[inline]
fn div_round_signed(num: i32, den: i32) -> i32 {
    if num >= 0 { (num + den / 2) / den } else { -((-num + den / 2) / den) }
}

/// The largest level the fixed-length escape carries.
const MAX_LEVEL: i32 = 2047;

/// The value of an intra DC prediction neighbour outside the VOP or not intra (`2^(8+2)`).
const DC_DEFAULT: i32 = 1024;

#[derive(Clone, Copy)]
struct BlockPred {
    dc: i32,
    ac_row: [i32; 7],
    ac_col: [i32; 7],
    qp: i32,
    slice_id: usize,
}

impl Default for BlockPred {
    fn default() -> Self {
        Self { dc: DC_DEFAULT, ac_row: [0; 7], ac_col: [0; 7], qp: 0, slice_id: usize::MAX }
    }
}

/// Reconstructed intra DC values (`F[0][0]`) and quantised AC rows/columns (`QF[0][u]`, `QF[v][0]`)
/// of the blocks coded so far, for adaptive DC and AC prediction (§7.4.3).
struct DcAcStore {
    /// Luminance, one per 8×8 block.
    y: Vec<BlockPred>,
    yw: usize,
    /// Chrominance, one per macroblock.
    c: [Vec<BlockPred>; 2],
    cw: usize,
}

impl DcAcStore {
    fn new(mb_w: usize, mb_h: usize) -> DcAcStore {
        let n = mb_w * mb_h;
        DcAcStore { y: vec![BlockPred::default(); 4 * n], yw: 2 * mb_w, c: [vec![BlockPred::default(); n], vec![BlockPred::default(); n]], cw: mb_w }
    }

    fn locate(&mut self, b: usize, mx: usize, my: usize) -> (&mut Vec<BlockPred>, usize, usize, usize) {
        match b {
            0..=3 => (&mut self.y, self.yw, 2 * mx + (b & 1), 2 * my + (b >> 1)),
            _ => (&mut self.c[b - 4], self.cw, mx, my),
        }
    }

    /// Return `(dc_unquant_pred, from_top, chosen_neighbor)` for block `b` (§7.4.3.1): from the
    /// block above when the horizontal gradient is smaller, else from the block to the left.
    fn predict(&mut self, b: usize, mx: usize, my: usize, slice_id: usize) -> (i32, bool, BlockPred) {
        let (g, w, bx, by) = self.locate(b, mx, my);
        let get = |x: usize, y: usize, dx: bool, dy: bool| -> BlockPred {
            if (dx && x == 0) || (dy && y == 0) {
                return BlockPred::default();
            }
            let p = g[(y - usize::from(dy)) * w + x - usize::from(dx)];
            if p.slice_id == slice_id { p } else { BlockPred::default() }
        };
        let a = get(bx, by, true, false);
        let bb = get(bx, by, true, true);
        let c = get(bx, by, false, true);
        let from_top = (a.dc - bb.dc).abs() < (bb.dc - c.dc).abs();
        let chosen = if from_top { c } else { a };
        (chosen.dc, from_top, chosen)
    }

    fn set_intra(&mut self, b: usize, mb: (usize, usize), dc: i32, qf: &[i32; 64], qp: i32, slice_id: usize) {
        let (g, w, bx, by) = self.locate(b, mb.0, mb.1);
        let ac_row = std::array::from_fn(|i| qf[i + 1]);
        let ac_col = std::array::from_fn(|i| qf[(i + 1) * 8]);
        g[by * w + bx] = BlockPred { dc, ac_row, ac_col, qp, slice_id };
    }

    fn set_non_intra(&mut self, mx: usize, my: usize, slice_id: usize) {
        for b in 0..6 {
            let (g, w, bx, by) = self.locate(b, mx, my);
            g[by * w + bx] = BlockPred { dc: DC_DEFAULT, ac_row: [0; 7], ac_col: [0; 7], qp: 0, slice_id };
        }
    }
}

/// Half-sample bilinear motion-compensated prediction of a `size`×`size` block at `(x, y)`
/// (§7.6.2.1), samples outside the plane taken from its edge.
pub(super) fn predict_halfpel(plane: &Plane, x: i32, y: i32, mv: (i32, i32), size: usize, rounding: bool, out: &mut [u8]) {
    let (ix, iy) = (x + (mv.0 >> 1), y + (mv.1 >> 1));
    let (hx, hy) = (mv.0 & 1, mv.1 & 1);
    let r = i32::from(rounding);
    for j in 0..size {
        for i in 0..size {
            let (sx, sy) = (ix + i as i32, iy + j as i32);
            let a = plane.at(sx, sy);
            let v = match (hx, hy) {
                (0, 0) => a,
                (1, 0) => (a + plane.at(sx + 1, sy) + 1 - r) >> 1,
                (0, _) => (a + plane.at(sx, sy + 1) + 1 - r) >> 1,
                _ => (a + plane.at(sx + 1, sy) + plane.at(sx, sy + 1) + plane.at(sx + 1, sy + 1) + 2 - r) >> 2,
            };
            out[j * size + i] = v as u8;
        }
    }
}

/// Block-edge mirror index for the 8-tap quarter-sample filter over `0..=limit` (§7.6.2.3).
#[inline]
fn mirror_idx(idx: isize, limit: isize) -> usize {
    if idx < 0 {
        (-idx - 1).min(limit) as usize
    } else if idx > limit {
        (2 * limit - idx + 1).max(0) as usize
    } else {
        idx as usize
    }
}

/// 1D 8-tap FIR half-sample interpolation `[-1, 3, -6, 20, 20, -6, 3, -1]` with block-edge mirroring (§7.6.2.3).
#[inline]
fn fir_halfpel_1d(samples: &[i32], count: usize, rounding: i32, out: &mut [i32]) {
    let limit = count as isize;
    for k in 0..count {
        let ik = k as isize;
        let s0 = samples[k] + samples[k + 1];
        let s1 = samples[mirror_idx(ik - 1, limit)] + samples[mirror_idx(ik + 2, limit)];
        let s2 = samples[mirror_idx(ik - 2, limit)] + samples[mirror_idx(ik + 3, limit)];
        let s3 = samples[mirror_idx(ik - 3, limit)] + samples[mirror_idx(ik + 4, limit)];
        out[k] = ((20 * s0 - 6 * s1 + 3 * s2 - s3 + 16 - rounding) >> 5).clamp(0, 255);
    }
}

/// Quarter-sample luminance motion-compensated prediction of a `size`×`size` (`16` or `8`) block
/// at `(x, y)` with quarter-pel motion vector `mv` (ISO/IEC 14496-2 §7.6.2.3).
pub(super) fn predict_qpel(plane: &Plane, x: i32, y: i32, mv: (i32, i32), size: usize, rounding: bool, out: &mut [u8]) {
    let (ix, iy) = (x + (mv.0 >> 2), y + (mv.1 >> 2));
    let (qx, qy) = (mv.0 & 3, mv.1 & 3);
    let r = i32::from(rounding);

    if qx == 0 && qy == 0 {
        for j in 0..size {
            for i in 0..size {
                out[j * size + i] = plane.at(ix + i as i32, iy + j as i32) as u8;
            }
        }
        return;
    }

    let stride = size + 1;
    let mut grid = [0i32; 17 * 17];
    for j in 0..=size {
        for i in 0..=size {
            grid[j * stride + i] = plane.at(ix + i as i32, iy + j as i32);
        }
    }

    let mut h_rows = [0i32; 17 * 16];
    let mut half_row = [0i32; 16];
    for j in 0..=size {
        let row = &grid[j * stride..j * stride + stride];
        match qx {
            0 => {
                for i in 0..size {
                    h_rows[j * size + i] = row[i];
                }
            }
            2 => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                h_rows[j * size..j * size + size].copy_from_slice(&half_row[..size]);
            }
            1 => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                for i in 0..size {
                    h_rows[j * size + i] = (row[i] + half_row[i] + 1 - r) >> 1;
                }
            }
            _ => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                for i in 0..size {
                    h_rows[j * size + i] = (row[i + 1] + half_row[i] + 1 - r) >> 1;
                }
            }
        }
    }

    if qy == 0 {
        for j in 0..size {
            for i in 0..size {
                out[j * size + i] = h_rows[j * size + i] as u8;
            }
        }
        return;
    }

    let mut col = [0i32; 17];
    let mut half_col = [0i32; 16];
    for i in 0..size {
        for j in 0..=size {
            col[j] = h_rows[j * size + i];
        }
        fir_halfpel_1d(&col[..=size], size, r, &mut half_col[..size]);
        for j in 0..size {
            let v = match qy {
                2 => half_col[j],
                1 => (col[j] + half_col[j] + 1 - r) >> 1,
                _ => (col[j + 1] + half_col[j] + 1 - r) >> 1,
            };
            out[j * size + i] = v as u8;
        }
    }
}

/// Predict a luminance block (`size` = 16 or 8) using either quarter-pel or half-pel interpolation.
#[inline]
fn predict_luma(plane: &Plane, origin: (i32, i32), mv: (i32, i32), size: usize, quarter_sample: bool, rounding: bool, out: &mut [u8]) {
    if quarter_sample {
        predict_qpel(plane, origin.0, origin.1, mv, size, rounding, out);
    } else {
        predict_halfpel(plane, origin.0, origin.1, mv, size, rounding, out);
    }
}

/// The chrominance vector of a 16×16 luminance vector: halved, with every fraction to a half
/// sample (§7.6.2.2, §7.6.2.3).
#[inline]
fn chroma_mv(mv: (i32, i32), quarter_sample: bool) -> (i32, i32) {
    let h = if quarter_sample { (mv.0 / 2, mv.1 / 2) } else { mv };
    ((h.0 >> 1) | (h.0 & 1), (h.1 >> 1) | (h.1 & 1))
}

#[inline]
fn div_round_16(sum: i32) -> i32 {
    if sum == 0 {
        return 0;
    }
    let abs = sum.abs();
    let mag = (abs >> 4) * 2 + ROUNDTAB_16[(abs & 0xF) as usize];
    if sum < 0 { -mag } else { mag }
}

/// Convert four 8×8 luminance motion vectors to a 4:2:0 chrominance motion vector in half-sample
/// units (Table 7-6, §7.6.1.6, §7.6.2.3).
#[inline]
fn chroma_mv_4mv(mvs: &[(i32, i32); 4], quarter_sample: bool) -> (i32, i32) {
    let mut sx = 0i32;
    let mut sy = 0i32;
    for &(vx, vy) in mvs {
        let hx = if quarter_sample { vx / 2 } else { vx };
        let hy = if quarter_sample { vy / 2 } else { vy };
        sx += hx;
        sy += hy;
    }
    (div_round_16(sx), div_round_16(sy))
}

/// Predicted 16×16 luma and two 8×8 chroma planes for one macroblock.
struct MbPred {
    y: [u8; 256],
    c: [[u8; 64]; 2],
}

impl Default for MbPred {
    fn default() -> Self {
        Self { y: [0u8; 256], c: [[0u8; 64]; 2] }
    }
}

/// Predict a full macroblock (16×16 Y and two 8×8 chroma planes) for 1MV or 4MV.
fn predict_mb(r: &Frame, mb: (usize, usize), mvs: &[(i32, i32); 4], is_4mv: bool, quarter_sample: bool, rounding: bool, pred: &mut MbPred) {
    let (mx, my) = mb;
    let (yx, yy) = ((mx * 16) as i32, (my * 16) as i32);
    let (cx, cy) = ((mx * 8) as i32, (my * 8) as i32);
    if !is_4mv {
        predict_luma(&r.y, (yx, yy), mvs[0], 16, quarter_sample, rounding, &mut pred.y);
        let cmv = chroma_mv(mvs[0], quarter_sample);
        predict_halfpel(&r.u, cx, cy, cmv, 8, rounding, &mut pred.c[0]);
        predict_halfpel(&r.v, cx, cy, cmv, 8, rounding, &mut pred.c[1]);
    } else {
        let mut sub = [0u8; 64];
        for (b, &mv) in mvs.iter().enumerate() {
            let bx = (b & 1) * 8;
            let by = (b >> 1) * 8;
            predict_luma(&r.y, (yx + bx as i32, yy + by as i32), mv, 8, quarter_sample, rounding, &mut sub);
            for row in 0..8 {
                pred.y[(by + row) * 16 + bx..][..8].copy_from_slice(&sub[row * 8..row * 8 + 8]);
            }
        }
        let cmv = chroma_mv_4mv(mvs, quarter_sample);
        predict_halfpel(&r.u, cx, cy, cmv, 8, rounding, &mut pred.c[0]);
        predict_halfpel(&r.v, cx, cy, cmv, 8, rounding, &mut pred.c[1]);
    }
}

/// Average two predicted sample slices (`(a + b + 1) >> 1`, ISO/IEC 14496-2 §7.6.9).
#[inline]
fn average_pred(a: &[u8], b: &[u8], out: &mut [u8]) {
    for ((o, &va), &vb) in out.iter_mut().zip(a).zip(b) {
        *o = ((u16::from(va) + u16::from(vb) + 1) >> 1) as u8;
    }
}

/// The median of three.
#[inline]
fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// Bits of a motion vector difference at `fcode` 1 (an estimate for the motion search).
fn mvd_bits(d: i32) -> u32 {
    let a = d.unsigned_abs().min(32) as usize;
    u32::from(MVD[a].1) + u32::from(a != 0)
}

/// The largest vector component the search tries, in half-sample or quarter-sample units (`fcode` 2 covers ±63).
const SEARCH_LIMIT: i32 = 63;

/// Encoder-side coding tools and parameters for one VOP.
pub(super) struct PictureCoder<'a> {
    pub src: &'a Frame,
    pub reference: Option<&'a Frame>,
    /// Future reference and `(trb, trd)` when coding a B-VOP.
    pub bwd_reference: Option<(&'a Frame, i64, i64)>,
    /// Picture size in samples (even).
    pub width: usize,
    pub height: usize,
    pub mb_w: usize,
    pub mb_h: usize,
    pub qp: i32,
    pub rounding: bool,
    pub ac_prediction: bool,
    pub four_mv: bool,
    pub mpeg_quant: bool,
    pub intra_matrix: &'a [u8; 64],
    pub non_intra_matrix: &'a [u8; 64],
    pub quarter_sample: bool,
    /// Start a new video packet slice every `slice_mbs` macroblocks (`None` = single slice).
    pub slice_mbs: Option<usize>,
    /// Per macroblock: inter codings since its last intra coding (refresh before 132, §7.4.5).
    pub refresh: &'a mut [u8],
}

/// Inter codings after which a macroblock is coded intra again: IDCT mismatch between encoder and
/// decoder must not build up over more than 132 (§7.4.5).
const REFRESH: u8 = 128;

/// What [`PictureCoder::code`] made.
pub(super) struct Coded {
    pub mbs: Vec<Mb>,
    pub recon: Frame,
    pub fcode_forward: u8,
    pub fcode_backward: u8,
}

impl PictureCoder<'_> {
    /// Decide, transform and reconstruct every macroblock.
    pub fn code(&mut self) -> Coded {
        if let Some((bwd, trb, trd)) = self.bwd_reference
            && let Some(fwd) = self.reference
        {
            return self.code_b_vop(fwd, bwd, trb, trd);
        }
        let mut recon = Frame::new(self.mb_w, self.mb_h);
        let mut dc = DcAcStore::new(self.mb_w, self.mb_h);
        let mut mbs: Vec<Mb> = Vec::with_capacity(self.mb_w * self.mb_h);
        for my in 0..self.mb_h {
            for mx in 0..self.mb_w {
                let i = my * self.mb_w + mx;
                let slice_id = self.slice_mbs.map_or(0, |s| i / s.max(1));
                let mut mb = match self.reference {
                    None => self.intra(mx, my, slice_id, &mut dc, &mut recon),
                    Some(r) => self.inter_or_intra(r, mx, my, slice_id, &mbs, &mut dc, &mut recon),
                };
                mb.slice_id = slice_id;
                match mb.mode {
                    Mode::Intra => {
                        self.refresh[i] = 0;
                        recon.mbs[i] = RefMb { not_coded: false, intra: true, mvs: [(0, 0); 4] };
                    }
                    Mode::Inter | Mode::Inter4v => {
                        self.refresh[i] = self.refresh[i].saturating_add(1);
                        recon.mbs[i] = RefMb { not_coded: false, intra: false, mvs: mb.mvs };
                    }
                    Mode::Skip => {
                        recon.mbs[i] = RefMb { not_coded: true, intra: false, mvs: [(0, 0); 4] };
                    }
                }
                mbs.push(mb);
            }
        }
        let reach = mbs.iter().flat_map(|m| m.mvs.iter()).map(|mv| mv.0.max(-mv.0 - 1).max(mv.1).max(-mv.1 - 1)).max().unwrap_or(0);
        let fcode_forward = if reach < 32 { 1 } else { 2 };
        Coded { mbs, recon, fcode_forward, fcode_backward: 1 }
    }

    /// 8×8 source block `b` of macroblock `(mx, my)`.
    fn source_block(&self, b: usize, mx: usize, my: usize) -> [i32; 64] {
        let (plane, x, y) = block_origin(self.src, b, mx, my);
        std::array::from_fn(|k| i32::from(plane.data[(y + k / 8) * plane.w + x + k % 8]))
    }

    fn intra(&self, mx: usize, my: usize, slice_id: usize, dc: &mut DcAcStore, recon: &mut Frame) -> Mb {
        let qp = self.qp;
        let mut mb = Mb { mode: Mode::Intra, slice_id, ..Mb::skip() };

        // Forward DCT and DC quantization for all 6 blocks, plus neighbor lookup.
        let mut raw_qf = [[0i32; 64]; 6];
        let mut dc_unquant = [0i32; 6];
        let mut neighbors = [(false, BlockPred::default()); 6];
        let mut benefit = 0i32;

        let mut temp_dc = DcAcStore { y: dc.y.clone(), yw: dc.yw, c: [dc.c[0].clone(), dc.c[1].clone()], cw: dc.cw };

        for b in 0..6 {
            let px = self.source_block(b, mx, my);
            let mut coef = [0; 64];
            fdct(&px, &mut coef);
            let scaler = dc_scaler(qp, b < 4);
            let qf0 = ((coef[0].max(0) + scaler / 2) / scaler).min(2047 / scaler);
            let (fp, from_top, neighbor) = temp_dc.predict(b, mx, my, slice_id);
            mb.dc_diff[b] = qf0 - div_round_signed(fp, scaler);
            dc_unquant[b] = qf0 * scaler;
            neighbors[b] = (from_top, neighbor);

            raw_qf[b][0] = qf0;
            for k in 1..64 {
                raw_qf[b][k] = quant_intra_ac(coef[k], qp, self.mpeg_quant, self.intra_matrix[k]);
            }

            if self.ac_prediction && neighbor.qp > 0 {
                if from_top {
                    for u in 1..8 {
                        let pred = div_round_signed(neighbor.ac_row[u - 1] * neighbor.qp, qp);
                        benefit += raw_qf[b][u].abs() - (raw_qf[b][u] - pred).abs();
                    }
                } else {
                    for v in 1..8 {
                        let pred = div_round_signed(neighbor.ac_col[v - 1] * neighbor.qp, qp);
                        benefit += raw_qf[b][v * 8].abs() - (raw_qf[b][v * 8] - pred).abs();
                    }
                }
            }
            temp_dc.set_intra(b, (mx, my), dc_unquant[b], &raw_qf[b], qp, slice_id);
        }

        let use_ac_pred = self.ac_prediction && benefit > 0;
        mb.ac_pred = use_ac_pred;

        for b in 0..6 {
            let (_fp, from_top, neighbor) = dc.predict(b, mx, my, slice_id);
            let mut qf = raw_qf[b];
            let mut coded = raw_qf[b];
            if use_ac_pred {
                mb.scans[b] = if from_top { ScanKind::AltHorizontal } else { ScanKind::AltVertical };
                if neighbor.qp > 0 {
                    if from_top {
                        for u in 1..8 {
                            let pred = div_round_signed(neighbor.ac_row[u - 1] * neighbor.qp, qp);
                            let diff = (qf[u] - pred).clamp(-MAX_LEVEL, MAX_LEVEL);
                            coded[u] = diff;
                            qf[u] = diff + pred;
                        }
                    } else {
                        for v in 1..8 {
                            let pred = div_round_signed(neighbor.ac_col[v - 1] * neighbor.qp, qp);
                            let diff = (qf[v * 8] - pred).clamp(-MAX_LEVEL, MAX_LEVEL);
                            coded[v * 8] = diff;
                            qf[v * 8] = diff + pred;
                        }
                    }
                }
            } else {
                mb.scans[b] = ScanKind::Zigzag;
            }

            for k in 1..64 {
                let l = coded[k];
                mb.levels[b][k] = l as i16;
                if l != 0 {
                    mb.cbp |= 1 << (5 - b);
                }
            }

            dc.set_intra(b, (mx, my), dc_unquant[b], &qf, qp, slice_id);

            let recon_levels: [i16; 64] = std::array::from_fn(|k| qf[k] as i16);
            let mut rec = [0; 64];
            dequant_intra_block(dc_unquant[b], &recon_levels, qp, self.mpeg_quant, self.intra_matrix, &mut rec);
            idct(&mut rec);
            let (plane, x, y) = block_origin_mut(recon, b, mx, my);
            for k in 0..64 {
                plane.data[(y + k / 8) * plane.w + x + k % 8] = rec[k].clamp(0, 255) as u8;
            }
        }
        mb
    }

    /// Whether a motion vector for a `size`×`size` luma block at `(x0, y0)` reads only samples
    /// inside the picture for the part of the block that is inside it.
    fn luma_mv_ok(&self, x0: usize, y0: usize, size: usize, mv: (i32, i32)) -> bool {
        let range = -SEARCH_LIMIT - 1..=SEARCH_LIMIT;
        if !range.contains(&mv.0) || !range.contains(&mv.1) {
            return false;
        }
        let shift = if self.quarter_sample { 2 } else { 1 };
        let mask = (1 << shift) - 1;
        let fits = |p0: usize, extent: usize, v: i32| {
            if p0 >= extent {
                return true;
            }
            let inside = size.min(extent - p0) as i32;
            let start = p0 as i32 + (v >> shift);
            let frac = i32::from((v & mask) != 0);
            start >= 0 && start + inside - 1 + frac < extent as i32
        };
        fits(x0, self.width, mv.0) && fits(y0, self.height, mv.1)
    }

    fn mv_ok(&self, mx: usize, my: usize, mv: (i32, i32)) -> bool {
        if !self.luma_mv_ok(mx * 16, my * 16, 16, mv) {
            return false;
        }
        let fits_c = |x0: usize, size: usize, extent: usize, v: i32| {
            let inside = size.min(extent - x0) as i32;
            let start = x0 as i32 + (v >> 1);
            start >= 0 && start + inside - 1 + (v & 1) < extent as i32
        };
        let c = chroma_mv(mv, self.quarter_sample);
        fits_c(mx * 8, 8, self.width / 2, c.0) && fits_c(my * 8, 8, self.height / 2, c.1)
    }

    /// Sum of absolute differences of a `size`×`size` luminance prediction at `(x0, y0)` with `mv`.
    fn sad_block(&self, r: &Frame, x0: usize, y0: usize, size: usize, mv: (i32, i32), rounding: bool) -> u32 {
        let mut p = [0u8; 256];
        predict_luma(&r.y, (x0 as i32, y0 as i32), mv, size, self.quarter_sample, rounding, &mut p[..size * size]);
        let s = &self.src.y;
        let mut sad = 0u32;
        for j in 0..size {
            let row = &s.data[(y0 + j) * s.w + x0..][..size];
            for i in 0..size {
                sad += u32::from(row[i].abs_diff(p[j * size + i]));
            }
        }
        sad
    }

    /// Motion search for a `16×16` macroblock or `8×8` block: whole-sample diamond search, half-sample
    /// refinement, and optional quarter-sample refinement.
    fn search_region(
        &self,
        r: &Frame,
        origin: (usize, usize),
        size: usize,
        pred: (i32, i32),
        cands: &[(i32, i32)],
        rounding: bool,
    ) -> ((i32, i32), u32) {
        let (x0, y0) = origin;
        let (mx, my) = (x0 / 16, y0 / 16);
        let lambda = self.qp as u32;
        let full_step = if self.quarter_sample { 4 } else { 2 };
        let half_step = if self.quarter_sample { 2 } else { 1 };
        let cost = |mv: (i32, i32)| -> Option<(u32, u32)> {
            let ok = if size == 16 { self.mv_ok(mx, my, mv) } else { self.luma_mv_ok(x0, y0, size, mv) };
            if !ok {
                return None;
            }
            let sad = self.sad_block(r, x0, y0, size, mv, rounding);
            Some((sad + lambda * (mvd_bits(mv.0 - pred.0) + mvd_bits(mv.1 - pred.1)), sad))
        };
        let mut best = ((0, 0), cost((0, 0)).unwrap_or((u32::MAX, u32::MAX)));
        for &c in cands {
            let aligned = (c.0 & !(full_step - 1), c.1 & !(full_step - 1));
            for cand in [c, aligned] {
                if let Some(k) = cost(cand).filter(|k| k.0 < best.1.0) {
                    best = (cand, k);
                }
            }
        }
        for _ in 0..32 {
            let centre = best.0;
            for d in [(full_step, 0), (-full_step, 0), (0, full_step), (0, -full_step)] {
                let mv = (centre.0 + d.0, centre.1 + d.1);
                if let Some(k) = cost(mv).filter(|k| k.0 < best.1.0) {
                    best = (mv, k);
                }
            }
            if best.0 == centre {
                break;
            }
        }
        let centre = best.0;
        for d in [
            (-half_step, -half_step),
            (0, -half_step),
            (half_step, -half_step),
            (-half_step, 0),
            (half_step, 0),
            (-half_step, half_step),
            (0, half_step),
            (half_step, half_step),
        ] {
            let mv = (centre.0 + d.0, centre.1 + d.1);
            if let Some(k) = cost(mv).filter(|k| k.0 < best.1.0) {
                best = (mv, k);
            }
        }
        if self.quarter_sample {
            let centre = best.0;
            for d in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let mv = (centre.0 + d.0, centre.1 + d.1);
                if let Some(k) = cost(mv).filter(|k| k.0 < best.1.0) {
                    best = (mv, k);
                }
            }
        }
        (best.0, best.1.1)
    }

    #[allow(clippy::too_many_arguments)]
    fn inter_or_intra(&self, r: &Frame, mx: usize, my: usize, slice_id: usize, mbs: &[Mb], dc: &mut DcAcStore, recon: &mut Frame) -> Mb {
        let i = my * self.mb_w + mx;
        let pred = mv_pred_block(mbs, self.mb_w, (mx, my), 0, [(0, 0); 4], slice_id);
        let mut cands = vec![pred];
        if mx > 0 && mbs[i - 1].slice_id == slice_id {
            cands.push(mbs[i - 1].mvs[1]);
        }
        if my > 0 && mbs[i - self.mb_w].slice_id == slice_id {
            cands.push(mbs[i - self.mb_w].mvs[2]);
        }
        let (mv_1mv, mut sad) = self.search_region(r, (mx * 16, my * 16), 16, pred, &cands, self.rounding);
        let mut mode = Mode::Inter;
        let mut mvs = [mv_1mv; 4];
        let mut preds = [pred; 4];

        if self.four_mv {
            let mut mvs_4 = [(0, 0); 4];
            let mut preds_4 = [(0, 0); 4];
            let mut sad_4 = 0u32;
            let mut bits_4 = 0u32;
            for b in 0..4 {
                let bp = mv_pred_block(mbs, self.mb_w, (mx, my), b, mvs_4, slice_id);
                let bx = mx * 16 + (b & 1) * 8;
                let by = my * 16 + (b >> 1) * 8;
                let (bmv, bsad) = self.search_region(r, (bx, by), 8, bp, &[bp, mv_1mv], self.rounding);
                preds_4[b] = bp;
                mvs_4[b] = bmv;
                sad_4 = sad_4.saturating_add(bsad);
                bits_4 = bits_4.saturating_add(mvd_bits(bmv.0 - bp.0) + mvd_bits(bmv.1 - bp.1));
            }
            let fits_c = |x0: usize, size: usize, extent: usize, v: i32| {
                let inside = size.min(extent - x0) as i32;
                let start = x0 as i32 + (v >> 1);
                start >= 0 && start + inside - 1 + (v & 1) < extent as i32
            };
            let cmv = chroma_mv_4mv(&mvs_4, self.quarter_sample);
            let chroma_ok = fits_c(mx * 8, 8, self.width / 2, cmv.0) && fits_c(my * 8, 8, self.height / 2, cmv.1);
            let bits_1 = mvd_bits(mv_1mv.0 - pred.0) + mvd_bits(mv_1mv.1 - pred.1);
            let lambda = self.qp as u32;
            if chroma_ok && mvs_4.iter().any(|&v| v != mvs_4[0]) && sad_4 + lambda * (bits_4 + 4) + 32 < sad + lambda * bits_1 {
                mode = Mode::Inter4v;
                mvs = mvs_4;
                preds = preds_4;
                sad = sad_4;
            }
        }

        // Intra when the macroblock's own variation is clearly below the prediction error
        // (the usual H.263 test), or when it is due for refresh.
        let s = &self.src.y;
        let row = |j: usize| &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
        let sum: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v)).sum::<u32>()).sum();
        let mean = (sum + 128) / 256;
        let dev: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v).abs_diff(mean)).sum::<u32>()).sum();
        if self.refresh[i] >= REFRESH || dev + 500 < sad {
            return self.intra(mx, my, slice_id, dc, recon);
        }
        dc.set_non_intra(mx, my, slice_id);
        let mut mb = Mb { mode, mvs, preds, slice_id, ..Mb::skip() };
        let mut p = MbPred::default();
        predict_mb(r, (mx, my), &mvs, mode == Mode::Inter4v, self.quarter_sample, self.rounding, &mut p);
        self.quant_inter_residual((mx, my), &p, &mut mb, recon);
        if mode == Mode::Inter && mvs[0] == (0, 0) && mb.cbp == 0 {
            mb = Mb { slice_id, ..Mb::skip() };
        }
        mb
    }

    fn quant_inter_residual(&self, (mx, my): (usize, usize), pred: &MbPred, mb: &mut Mb, recon: &mut Frame) {
        let qp = self.qp;
        let mut rec = [[0i32; 64]; 6];
        for b in 0..6 {
            let p = |k: usize| -> i32 {
                match b {
                    0..=3 => i32::from(pred.y[((b >> 1) * 8 + k / 8) * 16 + (b & 1) * 8 + k % 8]),
                    _ => i32::from(pred.c[b - 4][k]),
                }
            };
            let src = self.source_block(b, mx, my);
            let resid: [i32; 64] = std::array::from_fn(|k| src[k] - p(k));
            let mut coef = [0; 64];
            fdct(&resid, &mut coef);
            for k in 0..64 {
                let l = quant_inter_coef(coef[k], qp, self.mpeg_quant, self.non_intra_matrix[k]);
                mb.levels[b][k] = l as i16;
                if l != 0 {
                    mb.cbp |= 1 << (5 - b);
                }
            }
            let mut deq = [0; 64];
            if mb.cbp & (1 << (5 - b)) != 0 {
                dequant_inter_block(&mb.levels[b], qp, self.mpeg_quant, self.non_intra_matrix, &mut deq);
                idct(&mut deq);
            }
            rec[b] = std::array::from_fn(|k| (p(k) + deq[k]).clamp(0, 255));
        }
        for (b, rec) in rec.iter().enumerate() {
            let (plane, x, y) = block_origin_mut(recon, b, mx, my);
            for k in 0..64 {
                plane.data[(y + k / 8) * plane.w + x + k % 8] = rec[k] as u8;
            }
        }
    }

    /// Encode a B-VOP between `fwd` (past reference) and `bwd` (future reference) using direct,
    /// interpolate, forward, and backward modes (ISO/IEC 14496-2 §7.6.4, §7.6.9).
    fn code_b_vop(&self, fwd: &Frame, bwd: &Frame, trb: i64, trd: i64) -> Coded {
        let mut recon = Frame::new(self.mb_w, self.mb_h);
        let mut mbs = Vec::with_capacity(self.mb_w * self.mb_h);
        let trd = trd.max(2);
        let trb = trb.clamp(1, trd - 1);

        let mut prev_slice = 0usize;

        for my in 0..self.mb_h {
            let mut pmv_f = (0i32, 0i32);
            let mut pmv_b = (0i32, 0i32);
            for mx in 0..self.mb_w {
                let idx = my * self.mb_w + mx;
                let slice_id = self.slice_mbs.map_or(0, |s| idx / s.max(1));
                if slice_id != prev_slice {
                    pmv_f = (0, 0);
                    pmv_b = (0, 0);
                    prev_slice = slice_id;
                }

                let col = bwd.mbs.get(idx).copied().unwrap_or_default();
                if col.not_coded {
                    // Co-located not_coded in backward reference: copy from forward reference, emit no bits.
                    for b in 0..6 {
                        let (src_p, sx, sy) = block_origin(fwd, b, mx, my);
                        let (dst_p, dx, dy) = block_origin_mut(&mut recon, b, mx, my);
                        for r in 0..8 {
                            dst_p.data[(dy + r) * dst_p.w + dx..][..8].copy_from_slice(&src_p.data[(sy + r) * src_p.w + sx..][..8]);
                        }
                    }
                    let mut mb = Mb::skip();
                    mb.b_mode = Some(BMode::ColocatedSkip);
                    mb.slice_id = slice_id;
                    mbs.push(mb);
                    continue;
                }

                // 1. Direct mode prediction (delta = (0, 0))
                let col_mvs = if col.intra { [(0, 0); 4] } else { col.mvs };
                let mut dir_f = [(0, 0); 4];
                let mut dir_b = [(0, 0); 4];
                for b in 0..4 {
                    let (cx, cy) = col_mvs[b];
                    dir_f[b] = (((trb * i64::from(cx)) / trd) as i32, ((trb * i64::from(cy)) / trd) as i32);
                    dir_b[b] = ((((trb - trd) * i64::from(cx)) / trd) as i32, (((trb - trd) * i64::from(cy)) / trd) as i32);
                }
                let dir_4mv = dir_f.iter().any(|&m| m != dir_f[0]) || dir_b.iter().any(|&m| m != dir_b[0]);
                let mut dir_fp = MbPred::default();
                let mut dir_bp = MbPred::default();
                predict_mb(fwd, (mx, my), &dir_f, dir_4mv, self.quarter_sample, false, &mut dir_fp);
                predict_mb(bwd, (mx, my), &dir_b, dir_4mv, self.quarter_sample, false, &mut dir_bp);
                let mut dir_p = MbPred::default();
                average_pred(&dir_fp.y, &dir_bp.y, &mut dir_p.y);
                average_pred(&dir_fp.c[0], &dir_bp.c[0], &mut dir_p.c[0]);
                average_pred(&dir_fp.c[1], &dir_bp.c[1], &mut dir_p.c[1]);
                let sad_dir = self.luma_sad_buf(mx, my, &dir_p.y);

                // 2. Forward and backward searches
                let (mv_f, sad_f) = self.search_region(fwd, (mx * 16, my * 16), 16, pmv_f, &[pmv_f, dir_f[0]], false);
                let (mv_b, sad_b) = self.search_region(bwd, (mx * 16, my * 16), 16, pmv_b, &[pmv_b, dir_b[0]], false);

                // 3. Interpolate prediction from (mv_f, mv_b)
                let mut fwd_p = MbPred::default();
                let mut bwd_p = MbPred::default();
                predict_mb(fwd, (mx, my), &[mv_f; 4], false, self.quarter_sample, false, &mut fwd_p);
                predict_mb(bwd, (mx, my), &[mv_b; 4], false, self.quarter_sample, false, &mut bwd_p);
                let mut int_p = MbPred::default();
                average_pred(&fwd_p.y, &bwd_p.y, &mut int_p.y);
                average_pred(&fwd_p.c[0], &bwd_p.c[0], &mut int_p.c[0]);
                average_pred(&fwd_p.c[1], &bwd_p.c[1], &mut int_p.c[1]);
                let sad_int = self.luma_sad_buf(mx, my, &int_p.y);

                let lambda = self.qp as u32;
                let cost_dir = sad_dir;
                let cost_f = sad_f + lambda * (4 + mvd_bits(mv_f.0 - pmv_f.0) + mvd_bits(mv_f.1 - pmv_f.1));
                let cost_b = sad_b + lambda * (3 + mvd_bits(mv_b.0 - pmv_b.0) + mvd_bits(mv_b.1 - pmv_b.1));
                let cost_int = sad_int
                    + lambda
                        * (2 + mvd_bits(mv_f.0 - pmv_f.0) + mvd_bits(mv_f.1 - pmv_f.1) + mvd_bits(mv_b.0 - pmv_b.0) + mvd_bits(mv_b.1 - pmv_b.1));

                let mut mb = Mb { mode: Mode::Inter, slice_id, ..Mb::skip() };
                if cost_dir <= cost_f && cost_dir <= cost_b && cost_dir <= cost_int {
                    mb.b_mode = Some(BMode::Direct { dbmv: (0, 0) });
                    self.quant_inter_residual((mx, my), &dir_p, &mut mb, &mut recon);
                } else if cost_int <= cost_f && cost_int <= cost_b {
                    mb.b_mode = Some(BMode::Interpolate);
                    mb.mvs = [mv_f; 4];
                    mb.preds = [pmv_f; 4];
                    mb.bwd_mv = mv_b;
                    mb.bwd_pred = pmv_b;
                    pmv_f = mv_f;
                    pmv_b = mv_b;
                    self.quant_inter_residual((mx, my), &int_p, &mut mb, &mut recon);
                } else if cost_b <= cost_f {
                    mb.b_mode = Some(BMode::Backward);
                    mb.bwd_mv = mv_b;
                    mb.bwd_pred = pmv_b;
                    pmv_b = mv_b;
                    self.quant_inter_residual((mx, my), &bwd_p, &mut mb, &mut recon);
                } else {
                    mb.b_mode = Some(BMode::Forward);
                    mb.mvs = [mv_f; 4];
                    mb.preds = [pmv_f; 4];
                    pmv_f = mv_f;
                    self.quant_inter_residual((mx, my), &fwd_p, &mut mb, &mut recon);
                }
                mbs.push(mb);
            }
        }

        let reach_f = mbs
            .iter()
            .filter(|m| matches!(m.b_mode, Some(BMode::Forward | BMode::Interpolate)))
            .map(|m| m.mvs[0].0.max(-m.mvs[0].0 - 1).max(m.mvs[0].1).max(-m.mvs[0].1 - 1))
            .max()
            .unwrap_or(0);
        let reach_b = mbs
            .iter()
            .filter(|m| matches!(m.b_mode, Some(BMode::Backward | BMode::Interpolate)))
            .map(|m| m.bwd_mv.0.max(-m.bwd_mv.0 - 1).max(m.bwd_mv.1).max(-m.bwd_mv.1 - 1))
            .max()
            .unwrap_or(0);
        let fcode_forward = if reach_f < 32 { 1 } else { 2 };
        let fcode_backward = if reach_b < 32 { 1 } else { 2 };
        Coded { mbs, recon, fcode_forward, fcode_backward }
    }

    fn luma_sad_buf(&self, mx: usize, my: usize, pred_y: &[u8; 256]) -> u32 {
        let s = &self.src.y;
        let mut sad = 0u32;
        for j in 0..16 {
            let row = &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
            for i in 0..16 {
                sad += u32::from(row[i].abs_diff(pred_y[j * 16 + i]));
            }
        }
        sad
    }
}

/// The plane and top-left sample of block `b` (0–3 luminance in raster order, 4 Cb, 5 Cr).
fn block_origin(f: &Frame, b: usize, mx: usize, my: usize) -> (&Plane, usize, usize) {
    match b {
        0..=3 => (&f.y, mx * 16 + (b & 1) * 8, my * 16 + (b >> 1) * 8),
        4 => (&f.u, mx * 8, my * 8),
        _ => (&f.v, mx * 8, my * 8),
    }
}

fn block_origin_mut(f: &mut Frame, b: usize, mx: usize, my: usize) -> (&mut Plane, usize, usize) {
    match b {
        0..=3 => (&mut f.y, mx * 16 + (b & 1) * 8, my * 16 + (b >> 1) * 8),
        4 => (&mut f.u, mx * 8, my * 8),
        _ => (&mut f.v, mx * 8, my * 8),
    }
}

/// Spatial motion vector predictor for block `block` (`0..4`) of macroblock `(mx, my)` (§7.6.5, Figure 7-16).
/// Candidates outside the VOP or in a different video packet slice (`slice_id`) are invalid.
pub(super) fn mv_pred_block(
    mbs: &[Mb],
    mb_w: usize,
    (mx, my): (usize, usize),
    block: usize,
    cur_mvs: [(i32, i32); 4],
    slice_id: usize,
) -> (i32, i32) {
    let i = my * mb_w + mx;
    let get = |idx: usize, cb: usize| -> Option<(i32, i32)> {
        let m = mbs.get(idx)?;
        (m.slice_id == slice_id).then_some(m.mvs[cb])
    };
    let (a, b, c) = match block {
        0 => (
            (mx > 0).then(|| get(i - 1, 1)).flatten(),
            (my > 0).then(|| get(i - mb_w, 2)).flatten(),
            (my > 0 && mx + 1 < mb_w).then(|| get(i - mb_w + 1, 2)).flatten(),
        ),
        1 => (Some(cur_mvs[0]), (my > 0).then(|| get(i - mb_w, 3)).flatten(), (my > 0 && mx + 1 < mb_w).then(|| get(i - mb_w + 1, 2)).flatten()),
        2 => ((mx > 0).then(|| get(i - 1, 3)).flatten(), Some(cur_mvs[0]), Some(cur_mvs[1])),
        _ => (Some(cur_mvs[2]), Some(cur_mvs[0]), Some(cur_mvs[1])),
    };
    match (a, b, c) {
        (None, None, None) => (0, 0),
        (Some(v), None, None) | (None, Some(v), None) | (None, None, Some(v)) => v,
        _ => {
            let [a, b, c] = [a, b, c].map(|v| v.unwrap_or((0, 0)));
            (median(a.0, b.0, c.0), median(a.1, b.1, c.1))
        }
    }
}

/// 1MV spatial predictor of macroblock `(mx, my)` in slice 0.
#[cfg(test)]
pub(super) fn mv_pred(mbs: &[Mb], mb_w: usize, mx: usize, my: usize) -> (i32, i32) {
    mv_pred_block(mbs, mb_w, (mx, my), 0, [(0, 0); 4], 0)
}

fn put(w: &mut BitWriter, (code, len): (u16, u8)) {
    w.write_bits(u32::from(code), u32::from(len));
}

/// One `motion_code` and `motion_residual` (§7.6.3), the difference wrapped into the range of
/// `fcode`.
fn write_mvd(w: &mut BitWriter, d: i32, fcode: u8) {
    let r_size = u32::from(fcode - 1);
    let f = 1i32 << r_size;
    let (low, high, range) = (-32 * f, 32 * f - 1, 64 * f);
    let d = if d < low {
        d + range
    } else if d > high {
        d - range
    } else {
        d
    };
    if d == 0 {
        put(w, MVD[0]);
        return;
    }
    let a = d.abs() - 1;
    let code = (a >> r_size) + 1;
    put(w, MVD[code as usize]);
    w.write_bit(d < 0);
    if r_size > 0 {
        w.write_bits((a & (f - 1)) as u32, r_size);
    }
}

/// The run-length events of a block from scan position `start` in `scan`, as `(run, level)`.
fn events(levels: &[i16; 64], scan: &[usize; 64], start: usize) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    let mut run = 0;
    for &z in &scan[start..] {
        let l = i32::from(levels[z]);
        if l == 0 {
            run += 1;
        } else {
            out.push((run, l));
            run = 0;
        }
    }
    out
}

/// One TCOEF event (§6.3.8 / Table B-16, B-17, with the three escape modes of §7.4.1.3).
fn write_event(w: &mut BitWriter, t: &Tcoef, last: bool, run: usize, level: i32, escape_only: bool) {
    let mag = level.unsigned_abs();
    let sign = level < 0;
    if !escape_only {
        if let Some((code, len)) = t.code(last, run, mag) {
            w.write_bits(code, len);
            w.write_bit(sign);
            return;
        }
        put(w, ESCAPE);
        let lmax = t.lmax(last, run);
        if let Some((code, len)) = mag.checked_sub(lmax).filter(|_| lmax > 0).and_then(|m| t.code(last, run, m)) {
            w.write_bit(false);
            w.write_bits(code, len);
            w.write_bit(sign);
            return;
        }
        if let Some((code, len)) = t.rmax(last, mag).and_then(|rmax| run.checked_sub(rmax + 1)).and_then(|r| t.code(last, r, mag)) {
            w.write_bits(0b10, 2);
            w.write_bits(code, len);
            w.write_bit(sign);
            return;
        }
    } else {
        put(w, ESCAPE);
    }
    w.write_bits(0b11, 2);
    w.write_bit(last);
    w.write_bits(run as u32, 6);
    w.write_bit(true);
    w.write_bits((level as u32) & 0xFFF, 12);
    w.write_bit(true);
}

fn write_coeffs(w: &mut BitWriter, levels: &[i16; 64], scan: &[usize; 64], start: usize, t: &Tcoef, escape_only: bool) {
    let ev = events(levels, scan, start);
    let n = ev.len();
    for (k, &(run, level)) in ev.iter().enumerate() {
        write_event(w, t, k + 1 == n, run, level, escape_only);
    }
}

fn write_intra_blocks(w: &mut BitWriter, mb: &Mb, escape_only: bool) {
    for b in 0..6 {
        let d = mb.dc_diff[b];
        let size = 32 - d.unsigned_abs().leading_zeros();
        put(w, if b < 4 { DC_SIZE_LUMA[size as usize] } else { DC_SIZE_CHROMA[size as usize] });
        if size > 0 {
            let bits = if d > 0 { d } else { d + (1 << size) - 1 };
            w.write_bits(bits as u32, size);
            if size > 8 {
                w.write_bit(true);
            }
        }
        if mb.cbp & (1 << (5 - b)) != 0 {
            write_coeffs(w, &mb.levels[b], mb.scans[b].table(), 1, tables::intra(), escape_only);
        }
    }
}

/// Write all macroblocks of a VOP, inserting video packet headers at slice boundaries when
/// `slice_mbs` is configured.
pub(super) fn write_vop_mbs(
    w: &mut BitWriter,
    mbs: &[Mb],
    coding_type: VopCodingType,
    fcode_forward: u8,
    fcode_backward: u8,
    quant: u8,
    escape_only: bool,
) {
    let total_mbs = mbs.len();
    let mut prev_slice = 0usize;
    for (idx, mb) in mbs.iter().enumerate() {
        if idx > 0 && mb.slice_id != prev_slice {
            headers::write_video_packet_header(w, coding_type, fcode_forward, fcode_backward, idx, total_mbs, quant);
            prev_slice = mb.slice_id;
        }
        match coding_type {
            VopCodingType::I => write_mb(w, mb, true, fcode_forward, escape_only),
            VopCodingType::P | VopCodingType::N => write_mb(w, mb, false, fcode_forward, escape_only),
            VopCodingType::B => write_b_mb(w, mb, fcode_forward, fcode_backward, escape_only),
        }
    }
}

/// One `macroblock()` of a B-VOP (§6.2.6).
fn write_b_mb(w: &mut BitWriter, mb: &Mb, fcode_forward: u8, fcode_backward: u8, escape_only: bool) {
    let Some(b_mode) = mb.b_mode else {
        return;
    };
    if b_mode == BMode::ColocatedSkip {
        return;
    }
    if b_mode == (BMode::Direct { dbmv: (0, 0) }) && mb.cbp == 0 {
        w.write_bit(true); // modb = '1' (no mb_type, no cbpb, no mv_data)
        return;
    }
    w.write_bit(false); // modb first bit '0'
    let has_cbp = mb.cbp != 0;
    w.write_bit(!has_cbp); // modb = '00' when cbpb is present, '01' when cbpb is 0
    let type_idx = match b_mode {
        BMode::Direct { .. } | BMode::ColocatedSkip => 0,
        BMode::Interpolate => 1,
        BMode::Backward => 2,
        BMode::Forward => 3,
    };
    put(w, B_MB_TYPE[type_idx]);
    if has_cbp {
        w.write_bits(u32::from(mb.cbp), 6);
    }
    if has_cbp && !matches!(b_mode, BMode::Direct { .. }) {
        w.write_bit(false); // dbquant = '0' (no quantiser change)
    }
    match b_mode {
        BMode::Direct { dbmv } => {
            write_mvd(w, dbmv.0, 1);
            write_mvd(w, dbmv.1, 1);
        }
        BMode::Interpolate => {
            write_mvd(w, mb.mvs[0].0 - mb.preds[0].0, fcode_forward);
            write_mvd(w, mb.mvs[0].1 - mb.preds[0].1, fcode_forward);
            write_mvd(w, mb.bwd_mv.0 - mb.bwd_pred.0, fcode_backward);
            write_mvd(w, mb.bwd_mv.1 - mb.bwd_pred.1, fcode_backward);
        }
        BMode::Backward => {
            write_mvd(w, mb.bwd_mv.0 - mb.bwd_pred.0, fcode_backward);
            write_mvd(w, mb.bwd_mv.1 - mb.bwd_pred.1, fcode_backward);
        }
        BMode::Forward => {
            write_mvd(w, mb.mvs[0].0 - mb.preds[0].0, fcode_forward);
            write_mvd(w, mb.mvs[0].1 - mb.preds[0].1, fcode_forward);
        }
        BMode::ColocatedSkip => {}
    }
    for b in 0..6 {
        if mb.cbp & (1 << (5 - b)) != 0 {
            write_coeffs(w, &mb.levels[b], &ZIGZAG, 0, tables::inter(), escape_only);
        }
    }
}

/// One `macroblock()` of an I-VOP (`intra_vop`) or a P-VOP. `escape_only` codes every coefficient
/// with the fixed-length escape (to check the code tables against a decoder).
pub(super) fn write_mb(w: &mut BitWriter, mb: &Mb, intra_vop: bool, fcode: u8, escape_only: bool) {
    let cbpc = usize::from(mb.cbp & 3);
    let cbpy = usize::from(mb.cbp >> 2);
    if intra_vop {
        put(w, MCBPC_INTRA[cbpc]);
        w.write_bit(mb.ac_pred);
        put(w, CBPY[cbpy]);
        write_intra_blocks(w, mb, escape_only);
        return;
    }
    match mb.mode {
        Mode::Skip => w.write_bit(true),
        Mode::Intra => {
            w.write_bit(false);
            put(w, MCBPC_INTER[3 * 4 + cbpc]);
            w.write_bit(mb.ac_pred);
            put(w, CBPY[cbpy]);
            write_intra_blocks(w, mb, escape_only);
        }
        Mode::Inter => {
            w.write_bit(false);
            put(w, MCBPC_INTER[cbpc]);
            put(w, CBPY[cbpy ^ 15]);
            write_mvd(w, mb.mvs[0].0 - mb.preds[0].0, fcode);
            write_mvd(w, mb.mvs[0].1 - mb.preds[0].1, fcode);
            for b in 0..6 {
                if mb.cbp & (1 << (5 - b)) != 0 {
                    write_coeffs(w, &mb.levels[b], &ZIGZAG, 0, tables::inter(), escape_only);
                }
            }
        }
        Mode::Inter4v => {
            w.write_bit(false);
            put(w, MCBPC_INTER[2 * 4 + cbpc]);
            put(w, CBPY[cbpy ^ 15]);
            for b in 0..4 {
                write_mvd(w, mb.mvs[b].0 - mb.preds[b].0, fcode);
                write_mvd(w, mb.mvs[b].1 - mb.preds[b].1, fcode);
            }
            for b in 0..6 {
                if mb.cbp & (1 << (5 - b)) != 0 {
                    write_coeffs(w, &mb.levels[b], &ZIGZAG, 0, tables::inter(), escape_only);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_scaler_follows_table_7_1() {
        assert_eq!((1..=31).map(|q| dc_scaler(q, true)).collect::<Vec<_>>()[..10], [8, 8, 8, 8, 10, 12, 14, 16, 17, 18]);
        assert_eq!(dc_scaler(24, true), 32);
        assert_eq!(dc_scaler(25, true), 34);
        assert_eq!(dc_scaler(31, true), 46);
        assert_eq!(dc_scaler(5, false), 9);
        assert_eq!(dc_scaler(24, false), 18);
        assert_eq!(dc_scaler(25, false), 19);
        assert_eq!(dc_scaler(31, false), 25);
    }

    #[test]
    fn dequant_is_h263_style() {
        assert_eq!(dequant_h263(1, 3), 9);
        assert_eq!(dequant_h263(-1, 4), -11);
        assert_eq!(dequant_h263(2000, 31), 2047);
        assert_eq!(dequant_h263(-2000, 31), -2048);
    }

    #[test]
    fn chroma_vectors_round_to_half_samples() {
        let c = |v| chroma_mv((v, 0), false).0;
        assert_eq!([c(0), c(1), c(2), c(3), c(4), c(5)], [0, 1, 1, 1, 2, 3]);
        assert_eq!([c(-1), c(-2), c(-3), c(-4), c(-5)], [-1, -1, -1, -2, -3]);
    }

    #[test]
    fn mvd_wraps_and_splits_residual() {
        // fcode 2: difference 5 -> motion_code 3 (|5|-1 = 4 = 2·(3-1) + 0), residual 0.
        let mut w = BitWriter::new();
        write_mvd(&mut w, 5, 2);
        let mut want = BitWriter::new();
        put(&mut want, MVD[3]);
        want.write_bit(false);
        want.write_bits(0, 1);
        assert_eq!(w.finish(), want.finish());
        // fcode 1: 40 wraps to -24.
        let mut w = BitWriter::new();
        write_mvd(&mut w, 40, 1);
        let mut want = BitWriter::new();
        put(&mut want, MVD[24]);
        want.write_bit(true);
        assert_eq!(w.finish(), want.finish());
    }

    #[test]
    fn predictor_rules_at_the_edges() {
        let mut mbs = vec![Mb::skip(); 6];
        mbs[0].mvs = [(4, -2); 4];
        mbs[1].mvs = [(6, 2); 4];
        mbs[2].mvs = [(8, 0); 4];
        // first row: the left vector
        assert_eq!(mv_pred(&mbs, 3, 1, 0), (4, -2));
        assert_eq!(mv_pred(&mbs, 3, 0, 0), (0, 0));
        // left column: left is zero, median of 0, above, above right
        assert_eq!(mv_pred(&mbs, 3, 0, 1), (4, 0));
        // right column: above right is zero
        mbs[4].mvs = [(2, 2); 4];
        assert_eq!(mv_pred(&mbs, 3, 2, 1), (2, 0));
    }
}
