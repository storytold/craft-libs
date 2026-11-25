//! VOP macroblock and block decoding, adaptive DC/AC prediction, inverse quantisation, and IDCT
//! reconstruction (ISO/IEC 14496-2 §6.2.6–6.2.8, §7.4–7.7, Annex A).

use std::sync::OnceLock;

use craft_media::bitstream::BitReader;

use super::DecodeError;
use super::headers::{VolConfig, VopHeader, VopType, try_read_video_packet_header};
use super::mc::{MbMeta, RefFrame, average_pred, chroma_mv_1mv, chroma_mv_4mv, mv_predictor, predict_halfpel, predict_luma};
use super::vlc::{
    ALT_HORIZONTAL_SCAN, ALT_VERTICAL_SCAN, BMbType, ZIGZAG, inter_tcoef, intra_tcoef, read_b_mb_type, read_block_coeffs, read_cbpy, read_dc_diff,
    read_mcbpc_inter, read_mcbpc_intra, read_mv_component,
};

/// `dquant` table for `mb_type` 1 (`inter+q`) and 4 (`intra+q`) (§6.3.6).
const DQUANT_TABLE: [i32; 4] = [-1, -2, 1, 2];

/// `M[x][u] = C(u)/2 · cos((2x+1)uπ/16)`, `C(0) = 1/√2`, `C(u>0) = 1`.
fn idct_basis() -> &'static [[f64; 8]; 8] {
    static B: OnceLock<[[f64; 8]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut m = [[0f64; 8]; 8];
        for (x, row) in m.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                *v = c / 2.0 * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos();
            }
        }
        m
    })
}

/// In-place 8×8 inverse DCT of dequantised coefficients (ISO/IEC 14496-2 Annex A), saturated to `[-256, 255]`.
pub(crate) fn idct(blk: &mut [i32; 64]) {
    if blk[1..].iter().all(|&c| c == 0) {
        let d = blk[0];
        let v = if d >= 0 { (d + 4) >> 3 } else { -((-d + 4) >> 3) };
        blk.fill(v.clamp(-256, 255));
        return;
    }
    let m = idct_basis();
    let mut tmp = [[0f64; 8]; 8];
    for v in 0..8 {
        let r = &blk[v * 8..v * 8 + 8];
        if r.iter().all(|&c| c == 0) {
            continue;
        }
        for x in 0..8 {
            tmp[v][x] = (0..8).map(|u| m[x][u] * f64::from(r[u])).sum();
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            let s: f64 = (0..8).map(|v| m[y][v] * tmp[v][x]).sum();
            blk[y * 8 + x] = (s.round() as i32).clamp(-256, 255);
        }
    }
}

/// Table 7-1: `dc_scaler` of luminance and chrominance blocks.
#[inline]
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

#[inline]
fn div_round_signed(num: i32, den: i32) -> i32 {
    if num >= 0 { (num + den / 2) / den } else { -((-num + den / 2) / den) }
}

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

/// Per-block intra DC and AC predictors (§7.4.3).
struct DcAcStore {
    y: Vec<BlockPred>,
    yw: usize,
    c: [Vec<BlockPred>; 2],
    cw: usize,
}

impl DcAcStore {
    fn new(mb_w: usize, mb_h: usize) -> Self {
        let n = mb_w * mb_h;
        Self { y: vec![BlockPred::default(); 4 * n], yw: 2 * mb_w, c: [vec![BlockPred::default(); n], vec![BlockPred::default(); n]], cw: mb_w }
    }

    fn grid_and_pos(&mut self, b: usize, mx: usize, my: usize) -> (&mut [BlockPred], usize, usize, usize) {
        match b {
            0..=3 => (&mut self.y, self.yw, 2 * mx + (b & 1), 2 * my + (b >> 1)),
            _ => (&mut self.c[b - 4], self.cw, mx, my),
        }
    }

    /// Mark all 6 blocks of macroblock `(mx, my)` as non-intra in `slice_id`.
    fn set_non_intra(&mut self, mx: usize, my: usize, slice_id: usize) {
        for b in 0..6 {
            let (g, w, bx, by) = self.grid_and_pos(b, mx, my);
            g[by * w + bx] = BlockPred { dc: DC_DEFAULT, ac_row: [0; 7], ac_col: [0; 7], qp: 0, slice_id };
        }
    }

    /// Return `(dc_pred_quantised, predict_from_top, neighbor_pred)` for intra block `b` at `(mx, my)`.
    ///
    /// With `field_dct`, the lower two luma blocks hold the bottom field, so their upper and
    /// diagonal neighbours are the bottom-field blocks of the macroblock above (§7.6.1), while
    /// the left neighbour stays in the lower block row of the macroblock to the left.
    fn predict(&mut self, b: usize, mb: (usize, usize), field_dct: bool, scaler: i32, slice_id: usize) -> (i32, bool, BlockPred) {
        let (mx, my) = mb;
        let (g, w, bx, by) = self.grid_and_pos(b, mx, my);
        let upper_row = if field_dct && (b == 2 || b == 3) { 2 * my } else { by };
        let get = |dx: bool, dy: bool| -> BlockPred {
            if (dx && bx == 0) || (dy && upper_row == 0) {
                return BlockPred::default();
            }
            let row = if dy { upper_row - 1 } else { by };
            let p = g[row * w + (bx - usize::from(dx))];
            if p.slice_id == slice_id { p } else { BlockPred::default() }
        };
        let a = get(true, false);
        let bb = get(true, true);
        let c = get(false, true);
        let from_top = (a.dc - bb.dc).abs() < (bb.dc - c.dc).abs();
        let chosen = if from_top { c } else { a };
        let dc_pred = div_round_signed(chosen.dc, scaler);
        (dc_pred, from_top, chosen)
    }

    /// Save the reconstructed intra DC and quantised AC levels for block `b` at `(mx, my)`.
    fn store_intra(&mut self, b: usize, mb: (usize, usize), dc_unquant: i32, levels: &[i32; 64], qp: i32, slice_id: usize) {
        let (g, w, bx, by) = self.grid_and_pos(b, mb.0, mb.1);
        let ac_row = std::array::from_fn(|i| levels[i + 1]);
        let ac_col = std::array::from_fn(|i| levels[(i + 1) * 8]);
        g[by * w + bx] = BlockPred { dc: dc_unquant, ac_row, ac_col, qp, slice_id };
    }
}

/// Inverse-quantise an intra 8×8 block into `deq` (ISO/IEC 14496-2 §7.4.2).
pub(super) fn dequant_intra(levels: &[i32; 64], qp: i32, scaler: i32, vol: &VolConfig, deq: &mut [i32; 64]) {
    deq[0] = (levels[0] * scaler).clamp(-2048, 2047);
    if !vol.quant_type {
        let odd_sub = if qp & 1 == 0 { 1 } else { 0 };
        for k in 1..64 {
            let lv = levels[k];
            if lv == 0 {
                deq[k] = 0;
            } else {
                let mag = qp * (2 * lv.abs() + 1) - odd_sub;
                deq[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047);
            }
        }
    } else {
        for k in 1..64 {
            let lv = levels[k];
            if lv == 0 {
                deq[k] = 0;
            } else {
                let w = i32::from(vol.intra_quant_mat[k]);
                let mag = (2 * i64::from(lv.unsigned_abs()) * i64::from(w) * i64::from(qp)) >> 4;
                deq[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047) as i32;
            }
        }
        let sum: i32 = deq.iter().sum();
        if sum & 1 == 0 {
            deq[63] ^= 1;
        }
    }
}

/// Inverse-quantise a non-intra 8×8 block into `deq` (ISO/IEC 14496-2 §7.4.2).
fn dequant_inter(levels: &[i32; 64], qp: i32, vol: &VolConfig, deq: &mut [i32; 64]) {
    if !vol.quant_type {
        let odd_sub = if qp & 1 == 0 { 1 } else { 0 };
        for k in 0..64 {
            let lv = levels[k];
            if lv == 0 {
                deq[k] = 0;
            } else {
                let mag = qp * (2 * lv.abs() + 1) - odd_sub;
                deq[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047);
            }
        }
    } else {
        for k in 0..64 {
            let lv = levels[k];
            if lv == 0 {
                deq[k] = 0;
            } else {
                let w = i32::from(vol.nonintra_quant_mat[k]);
                let mag = ((2 * i64::from(lv.unsigned_abs()) + 1) * i64::from(w) * i64::from(qp)) >> 4;
                deq[k] = (if lv < 0 { -mag } else { mag }).clamp(-2048, 2047) as i32;
            }
        }
        let sum: i32 = deq.iter().sum();
        if sum & 1 == 0 {
            deq[63] ^= 1;
        }
    }
}

/// Write an 8×8 block into `frame` for block `b` of `(mx, my)`.
fn write_block_samples(frame: &mut RefFrame, b: usize, mx: usize, my: usize, field_dct: bool, samples: &[u8; 64]) {
    match b {
        0..=3 if !field_dct => {
            let x0 = mx * 16 + (b & 1) * 8;
            let y0 = my * 16 + (b >> 1) * 8;
            for r in 0..8 {
                let dst = &mut frame.y.data[(y0 + r) * frame.y.w + x0..][..8];
                dst.copy_from_slice(&samples[r * 8..r * 8 + 8]);
            }
        }
        0..=3 => {
            // Interlaced field DCT: blocks 0,1 are top field (even rows), blocks 2,3 are bottom field (odd rows).
            let x0 = mx * 16 + (b & 1) * 8;
            let y_base = my * 16 + (b >> 1);
            for r in 0..8 {
                let dst = &mut frame.y.data[(y_base + r * 2) * frame.y.w + x0..][..8];
                dst.copy_from_slice(&samples[r * 8..r * 8 + 8]);
            }
        }
        4 => {
            let (x0, y0) = (mx * 8, my * 8);
            for r in 0..8 {
                let dst = &mut frame.u.data[(y0 + r) * frame.u.w + x0..][..8];
                dst.copy_from_slice(&samples[r * 8..r * 8 + 8]);
            }
        }
        _ => {
            let (x0, y0) = (mx * 8, my * 8);
            for r in 0..8 {
                let dst = &mut frame.v.data[(y0 + r) * frame.v.w + x0..][..8];
                dst.copy_from_slice(&samples[r * 8..r * 8 + 8]);
            }
        }
    }
}

/// Copy a co-located 16×16 macroblock from `src` to `dst` at `(mx, my)`.
fn copy_macroblock(src: &RefFrame, dst: &mut RefFrame, mx: usize, my: usize) {
    let (yx, yy) = (mx * 16, my * 16);
    for r in 0..16 {
        let s = &src.y.data[(yy + r) * src.y.w + yx..][..16];
        let d = &mut dst.y.data[(yy + r) * dst.y.w + yx..][..16];
        d.copy_from_slice(s);
    }
    let (cx, cy) = (mx * 8, my * 8);
    for r in 0..8 {
        dst.u.data[(cy + r) * dst.u.w + cx..][..8].copy_from_slice(&src.u.data[(cy + r) * src.u.w + cx..][..8]);
        dst.v.data[(cy + r) * dst.v.w + cx..][..8].copy_from_slice(&src.v.data[(cy + r) * src.v.w + cx..][..8]);
    }
}

/// Predicted luminance (`16×16`) and chrominance (`2 × 8×8`) samples for one macroblock.
struct MbPred {
    y: [u8; 256],
    c: [[u8; 64]; 2],
}

impl Default for MbPred {
    fn default() -> Self {
        Self { y: [0u8; 256], c: [[0u8; 64]; 2] }
    }
}

/// Predict a macroblock from `reference` using 1MV (`mvs[0]`) or 4MV (`mvs[0..4]`).
fn predict_macroblock(
    reference: &RefFrame,
    vol: &VolConfig,
    rounding: bool,
    mb: (usize, usize),
    is_4mv: bool,
    mvs: &[(i32, i32); 4],
    pred: &mut MbPred,
) {
    let (mx, my) = mb;
    let (yx, yy) = ((mx * 16) as i32, (my * 16) as i32);
    let (cx, cy) = ((mx * 8) as i32, (my * 8) as i32);
    if !is_4mv {
        predict_luma(&reference.y, (yx, yy), mvs[0], 16, vol.quarter_sample, rounding, &mut pred.y);
        let cmv = chroma_mv_1mv(mvs[0], vol.quarter_sample);
        predict_halfpel(&reference.u, cx, cy, cmv, 8, rounding, &mut pred.c[0]);
        predict_halfpel(&reference.v, cx, cy, cmv, 8, rounding, &mut pred.c[1]);
    } else {
        let mut sub = [0u8; 64];
        for (b, &mv) in mvs.iter().enumerate() {
            let bx = (b & 1) * 8;
            let by = (b >> 1) * 8;
            predict_luma(&reference.y, (yx + bx as i32, yy + by as i32), mv, 8, vol.quarter_sample, rounding, &mut sub);
            for r in 0..8 {
                pred.y[(by + r) * 16 + bx..][..8].copy_from_slice(&sub[r * 8..r * 8 + 8]);
            }
        }
        let cmv = chroma_mv_4mv(mvs, vol.quarter_sample);
        predict_halfpel(&reference.u, cx, cy, cmv, 8, rounding, &mut pred.c[0]);
        predict_halfpel(&reference.v, cx, cy, cmv, 8, rounding, &mut pred.c[1]);
    }
}

struct VopCtx<'a> {
    vol: &'a VolConfig,
    vop: &'a mut VopHeader,
    mb_w: usize,
    mb_h: usize,
    qp: i32,
    slice_id: usize,
    /// Quantiser of the previous coded macroblock, which selects the intra DC syntax (Table 6-12).
    running_qp: i32,
    dc_store: DcAcStore,
    out: RefFrame,
}

impl VopCtx<'_> {
    fn check_resync(&mut self, r: &mut BitReader<'_>, idx: usize) -> Result<bool, DecodeError> {
        if idx > 0
            && let Some((_mb_num, new_q)) = try_read_video_packet_header(r, self.vol, self.vop)?
        {
            self.qp = i32::from(new_q);
            self.running_qp = self.qp;
            self.slice_id += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn decode_intra_mb(
        &mut self,
        r: &mut BitReader<'_>,
        mb: (usize, usize),
        cbp: u8,
        ac_pred_flag: bool,
        field_dct: bool,
    ) -> Result<(), DecodeError> {
        let (mx, my) = mb;
        let qp = self.qp;
        let use_dc_vlc = self.running_qp < self.vop.intra_dc_vlc_thr_qp;
        self.running_qp = qp;
        for b in 0..6 {
            let luma = b < 4;
            let scaler = if self.vol.short_header { 8 } else { dc_scaler(qp, luma) };
            let (dc_pred, from_top, neighbor) =
                if self.vol.short_header { (0, false, BlockPred::default()) } else { self.dc_store.predict(b, mb, field_dct, scaler, self.slice_id) };

            let mut levels = [0i32; 64];
            let dc_diff = if self.vol.short_header {
                let code = r.read_bits(8)? as u8;
                match code {
                    0 => return Err(DecodeError::Invalid("forbidden H.263 intrADC code 0".into())),
                    128 | 255 => 128,
                    v => i32::from(v),
                }
            } else if use_dc_vlc {
                read_dc_diff(r, luma)?
            } else {
                0
            };

            let scan = if !ac_pred_flag {
                if self.vop.alternate_vertical_scan { &ALT_VERTICAL_SCAN } else { &ZIGZAG }
            } else if from_top {
                &ALT_HORIZONTAL_SCAN
            } else {
                &ALT_VERTICAL_SCAN
            };

            if (cbp & (1 << (5 - b))) != 0 {
                let start = usize::from(use_dc_vlc || self.vol.short_header);
                let table = if self.vol.short_header { inter_tcoef() } else { intra_tcoef() };
                read_block_coeffs(r, table, scan, start, self.vol.short_header, &mut levels)?;
            }

            if use_dc_vlc || self.vol.short_header {
                levels[0] = dc_pred + dc_diff;
            } else {
                levels[0] += dc_pred;
            }

            if ac_pred_flag && neighbor.qp > 0 {
                if from_top {
                    for (u, item) in levels.iter_mut().enumerate().take(8).skip(1) {
                        *item += div_round_signed(neighbor.ac_row[u - 1] * neighbor.qp, qp);
                    }
                } else {
                    for v in 1..8 {
                        levels[v * 8] += div_round_signed(neighbor.ac_col[v - 1] * neighbor.qp, qp);
                    }
                }
            }

            let mut deq = [0i32; 64];
            dequant_intra(&levels, qp, scaler, self.vol, &mut deq);
            if !self.vol.short_header {
                self.dc_store.store_intra(b, (mx, my), deq[0], &levels, qp, self.slice_id);
            }
            idct(&mut deq);
            let samples: [u8; 64] = std::array::from_fn(|k| deq[k].clamp(0, 255) as u8);
            write_block_samples(&mut self.out, b, mx, my, field_dct, &samples);
        }
        Ok(())
    }

    fn decode_inter_residual(
        &mut self,
        r: &mut BitReader<'_>,
        mb: (usize, usize),
        cbp: u8,
        field_dct: bool,
        pred: &MbPred,
    ) -> Result<(), DecodeError> {
        let (mx, my) = mb;
        let scan = if self.vop.alternate_vertical_scan { &ALT_VERTICAL_SCAN } else { &ZIGZAG };
        for b in 0..6 {
            let mut pred_blk = [0u8; 64];
            match b {
                0..=3 if !field_dct => {
                    let x_off = (b & 1) * 8;
                    let y_off = (b >> 1) * 8;
                    for row in 0..8 {
                        pred_blk[row * 8..row * 8 + 8].copy_from_slice(&pred.y[(y_off + row) * 16 + x_off..][..8]);
                    }
                }
                0..=3 => {
                    let x_off = (b & 1) * 8;
                    let y_base = b >> 1;
                    for row in 0..8 {
                        pred_blk[row * 8..row * 8 + 8].copy_from_slice(&pred.y[(y_base + row * 2) * 16 + x_off..][..8]);
                    }
                }
                _ => {
                    pred_blk.copy_from_slice(&pred.c[b - 4]);
                }
            }

            if (cbp & (1 << (5 - b))) == 0 {
                write_block_samples(&mut self.out, b, mx, my, field_dct, &pred_blk);
                continue;
            }

            let mut levels = [0i32; 64];
            read_block_coeffs(r, inter_tcoef(), scan, 0, self.vol.short_header, &mut levels)?;
            let mut deq = [0i32; 64];
            dequant_inter(&levels, self.qp, self.vol, &mut deq);
            idct(&mut deq);
            let samples: [u8; 64] = std::array::from_fn(|k| (i32::from(pred_blk[k]) + deq[k]).clamp(0, 255) as u8);
            write_block_samples(&mut self.out, b, mx, my, field_dct, &samples);
        }
        Ok(())
    }

    fn decode_i_vop(&mut self, r: &mut BitReader<'_>) -> Result<(), DecodeError> {
        let total_mbs = self.mb_w * self.mb_h;
        for idx in 0..total_mbs {
            self.check_resync(r, idx)?;
            let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
            let (mb_type, cbpc) = read_mcbpc_intra(r)?;
            let ac_pred_flag = if self.vol.short_header { false } else { r.read_bit()? };
            let cbpy = read_cbpy(r, true)?;
            let cbp = (cbpy << 2) | cbpc;
            if mb_type == 4 {
                let dq = r.read_bits(2)? as usize;
                self.qp = (self.qp + DQUANT_TABLE[dq]).clamp(1, 31);
            }
            let field_dct = if self.vol.interlaced { r.read_bit()? } else { false };
            self.decode_intra_mb(r, (mx, my), cbp, ac_pred_flag, field_dct)?;
            self.out.mbs[idx] = MbMeta { not_coded: false, intra: true, mvs: [(0, 0); 4], slice_id: self.slice_id };
        }
        Ok(())
    }

    fn decode_ps_vop(&mut self, r: &mut BitReader<'_>, reference: Option<&RefFrame>) -> Result<(), DecodeError> {
        let total_mbs = self.mb_w * self.mb_h;
        let is_svop = self.vop.coding_type == VopType::S;
        'mb: for idx in 0..total_mbs {
            self.check_resync(r, idx)?;
            let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
            let (mb_type, cbpc) = loop {
                if r.read_bit()? {
                    let gmc_used = is_svop;
                    self.dc_store.set_non_intra(mx, my, self.slice_id);
                    let mv = if gmc_used { self.vop.gmc_mv } else { (0, 0) };
                    self.out.mbs[idx] = MbMeta { not_coded: !gmc_used, intra: false, mvs: [mv; 4], slice_id: self.slice_id };
                    if let Some(ref_frame) = reference {
                        if mv == (0, 0) {
                            copy_macroblock(ref_frame, &mut self.out, mx, my);
                        } else {
                            let mut pred = MbPred::default();
                            predict_macroblock(ref_frame, self.vol, self.vop.rounding, (mx, my), false, &[mv; 4], &mut pred);
                            self.decode_inter_residual(r, (mx, my), 0, false, &pred)?;
                        }
                    }
                    continue 'mb;
                }
                if let Some(v) = read_mcbpc_inter(r)? {
                    break v;
                }
            };
            let intra = mb_type >= 3;
            let mcsel = if is_svop && (mb_type == 0 || mb_type == 1) { r.read_bit()? } else { false };
            let ac_pred_flag = if intra && !self.vol.short_header { r.read_bit()? } else { false };
            let cbpy = read_cbpy(r, intra)?;
            let cbp = (cbpy << 2) | cbpc;
            if mb_type == 1 || mb_type == 4 {
                let dq = r.read_bits(2)? as usize;
                self.qp = (self.qp + DQUANT_TABLE[dq]).clamp(1, 31);
            }
            let mut field_dct = false;
            if self.vol.interlaced {
                if intra || cbp != 0 {
                    field_dct = r.read_bit()?;
                }
                if (mb_type == 0 || mb_type == 1) && !mcsel && r.read_bit()? {
                    return Err(DecodeError::Unsupported("interlaced field motion compensation".into()));
                }
            }

            if intra {
                self.decode_intra_mb(r, (mx, my), cbp, ac_pred_flag, field_dct)?;
                self.out.mbs[idx] = MbMeta { not_coded: false, intra: true, mvs: [(0, 0); 4], slice_id: self.slice_id };
                continue;
            }

            self.running_qp = self.qp;
            self.dc_store.set_non_intra(mx, my, self.slice_id);
            self.out.mbs[idx].not_coded = false;
            self.out.mbs[idx].intra = false;
            self.out.mbs[idx].slice_id = self.slice_id;

            let is_4mv = mb_type == 2;
            let grid = (self.mb_w, self.mb_h);
            let mut mvs = [(0, 0); 4];
            if mcsel {
                mvs = [self.vop.gmc_mv; 4];
                self.out.mbs[idx].mvs = mvs;
            } else if !is_4mv {
                let pred_mv = mv_predictor(&self.out.mbs, grid, (mx, my), 0, self.slice_id);
                let mvx = read_mv_component(r, pred_mv.0, self.vop.fcode_forward)?;
                let mvy = read_mv_component(r, pred_mv.1, self.vop.fcode_forward)?;
                mvs = [(mvx, mvy); 4];
                self.out.mbs[idx].mvs = mvs;
            } else {
                for (b, slot) in mvs.iter_mut().enumerate() {
                    let pred_mv = mv_predictor(&self.out.mbs, grid, (mx, my), b, self.slice_id);
                    let mvx = read_mv_component(r, pred_mv.0, self.vop.fcode_forward)?;
                    let mvy = read_mv_component(r, pred_mv.1, self.vop.fcode_forward)?;
                    *slot = (mvx, mvy);
                    self.out.mbs[idx].mvs[b] = (mvx, mvy);
                }
            }

            let mut pred = MbPred::default();
            if let Some(ref_frame) = reference {
                predict_macroblock(ref_frame, self.vol, self.vop.rounding, (mx, my), is_4mv, &mvs, &mut pred);
            }
            self.decode_inter_residual(r, (mx, my), cbp, field_dct, &pred)?;
        }
        Ok(())
    }

    fn decode_b_vop(&mut self, r: &mut BitReader<'_>, fwd_ref: &RefFrame, bwd_ref: &RefFrame, time_ticks: i64) -> Result<(), DecodeError> {
        let total_mbs = self.mb_w * self.mb_h;
        let trd = (bwd_ref.time_ticks - fwd_ref.time_ticks).max(2);
        let trb = (time_ticks - fwd_ref.time_ticks).clamp(1, trd - 1);

        let mut pmv_f = (0i32, 0i32);
        let mut pmv_b = (0i32, 0i32);

        for idx in 0..total_mbs {
            let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
            if mx == 0 {
                pmv_f = (0, 0);
                pmv_b = (0, 0);
            }
            if self.check_resync(r, idx)? {
                pmv_f = (0, 0);
                pmv_b = (0, 0);
            }

            let col = bwd_ref.mbs.get(idx).copied().unwrap_or_default();
            if col.not_coded && self.vol.gmc.is_none() {
                copy_macroblock(fwd_ref, &mut self.out, mx, my);
                self.out.mbs[idx] = MbMeta { not_coded: true, intra: false, mvs: [(0, 0); 4], slice_id: self.slice_id };
                continue;
            }

            let modb_1 = r.read_bit()?;
            let (mb_type, cbpb, is_modb_skip) = if modb_1 {
                (BMbType::Direct, 0u8, true)
            } else {
                let has_cbpb = !r.read_bit()?;
                let t = read_b_mb_type(r)?;
                let cbpb = if has_cbpb { r.read_bits(6)? as u8 } else { 0 };
                (t, cbpb, false)
            };

            if cbpb != 0 && mb_type != BMbType::Direct && r.read_bit()? {
                let dq = if r.read_bit()? { 2 } else { -2 };
                self.qp = (self.qp + dq).clamp(1, 31);
            }

            let mut field_dct = false;
            if self.vol.interlaced {
                if cbpb != 0 {
                    field_dct = r.read_bit()?;
                }
                if mb_type != BMbType::Direct && r.read_bit()? {
                    return Err(DecodeError::Unsupported("interlaced B-VOP field prediction".into()));
                }
            }

            let mut pred = MbPred::default();
            match mb_type {
                BMbType::Direct => {
                    let delta = if is_modb_skip { (0, 0) } else { (read_mv_component(r, 0, 1)?, read_mv_component(r, 0, 1)?) };
                    let col_mvs = if col.intra { [(0, 0); 4] } else { col.mvs };
                    let mut mvs_f = [(0, 0); 4];
                    let mut mvs_b = [(0, 0); 4];
                    for b in 0..4 {
                        let (cx, cy) = col_mvs[b];
                        let fx = ((trb * i64::from(cx)) / trd) as i32 + delta.0;
                        let fy = ((trb * i64::from(cy)) / trd) as i32 + delta.1;
                        let bx = if delta.0 == 0 { (((trb - trd) * i64::from(cx)) / trd) as i32 } else { fx - cx };
                        let by = if delta.1 == 0 { (((trb - trd) * i64::from(cy)) / trd) as i32 } else { fy - cy };
                        mvs_f[b] = (fx, fy);
                        mvs_b[b] = (bx, by);
                    }
                    let is_4mv = mvs_f.iter().any(|m| *m != mvs_f[0]) || mvs_b.iter().any(|m| *m != mvs_b[0]);
                    let mut fwd_p = MbPred::default();
                    let mut bwd_p = MbPred::default();
                    predict_macroblock(fwd_ref, self.vol, false, (mx, my), is_4mv, &mvs_f, &mut fwd_p);
                    predict_macroblock(bwd_ref, self.vol, false, (mx, my), is_4mv, &mvs_b, &mut bwd_p);
                    average_pred(&fwd_p.y, &bwd_p.y, &mut pred.y);
                    average_pred(&fwd_p.c[0], &bwd_p.c[0], &mut pred.c[0]);
                    average_pred(&fwd_p.c[1], &bwd_p.c[1], &mut pred.c[1]);
                }
                BMbType::Interpolate => {
                    let fx = read_mv_component(r, pmv_f.0, self.vop.fcode_forward)?;
                    let fy = read_mv_component(r, pmv_f.1, self.vop.fcode_forward)?;
                    pmv_f = (fx, fy);
                    let bx = read_mv_component(r, pmv_b.0, self.vop.fcode_backward)?;
                    let by = read_mv_component(r, pmv_b.1, self.vop.fcode_backward)?;
                    pmv_b = (bx, by);
                    let mut fwd_p = MbPred::default();
                    let mut bwd_p = MbPred::default();
                    predict_macroblock(fwd_ref, self.vol, false, (mx, my), false, &[pmv_f; 4], &mut fwd_p);
                    predict_macroblock(bwd_ref, self.vol, false, (mx, my), false, &[pmv_b; 4], &mut bwd_p);
                    average_pred(&fwd_p.y, &bwd_p.y, &mut pred.y);
                    average_pred(&fwd_p.c[0], &bwd_p.c[0], &mut pred.c[0]);
                    average_pred(&fwd_p.c[1], &bwd_p.c[1], &mut pred.c[1]);
                }
                BMbType::Backward => {
                    let bx = read_mv_component(r, pmv_b.0, self.vop.fcode_backward)?;
                    let by = read_mv_component(r, pmv_b.1, self.vop.fcode_backward)?;
                    pmv_b = (bx, by);
                    predict_macroblock(bwd_ref, self.vol, false, (mx, my), false, &[pmv_b; 4], &mut pred);
                }
                BMbType::Forward => {
                    let fx = read_mv_component(r, pmv_f.0, self.vop.fcode_forward)?;
                    let fy = read_mv_component(r, pmv_f.1, self.vop.fcode_forward)?;
                    pmv_f = (fx, fy);
                    predict_macroblock(fwd_ref, self.vol, false, (mx, my), false, &[pmv_f; 4], &mut pred);
                }
            }

            self.decode_inter_residual(r, (mx, my), cbpb, field_dct, &pred)?;
            self.out.mbs[idx] = MbMeta { not_coded: false, intra: false, mvs: [(0, 0); 4], slice_id: self.slice_id };
        }
        Ok(())
    }
}

/// Decode one complete `VideoObjectPlane` (`I`, `P`, `S(GMC)`, `B`, or `N`) into a [`RefFrame`].
pub(super) fn decode_vop(
    r: &mut BitReader<'_>,
    vol: &VolConfig,
    vop: &mut VopHeader,
    refs: (Option<&RefFrame>, Option<&RefFrame>),
    timing: (i64, i64),
) -> Result<RefFrame, DecodeError> {
    let (past_ref, future_ref) = refs;
    let (time_ticks, pts) = timing;
    let mb_w = vol.mb_width();
    let mb_h = vol.mb_height();
    let mut out = RefFrame::new(vol.width, vol.height, mb_w, mb_h, vop.coding_type, time_ticks, pts);
    out.top_field_first = vop.top_field_first;

    if !vop.vop_coded || vop.coding_type == VopType::N {
        if let Some(prev) = future_ref.or(past_ref) {
            out.y = prev.y.clone();
            out.u = prev.u.clone();
            out.v = prev.v.clone();
            out.mbs = prev.mbs.clone();
        }
        return Ok(out);
    }

    let qp = i32::from(vop.quant);
    let mut ctx = VopCtx { vol, vop, mb_w, mb_h, qp, slice_id: 0, running_qp: qp, dc_store: DcAcStore::new(mb_w, mb_h), out };

    match ctx.vop.coding_type {
        VopType::I => ctx.decode_i_vop(r)?,
        VopType::P | VopType::S => ctx.decode_ps_vop(r, future_ref.or(past_ref))?,
        VopType::B => {
            let (Some(fwd_ref), Some(bwd_ref)) = (past_ref, future_ref) else {
                return Err(DecodeError::Invalid("B-VOP without both forward and backward reference VOPs".into()));
            };
            ctx.decode_b_vop(r, fwd_ref, bwd_ref, time_ticks)?;
        }
        VopType::N => {}
    }

    Ok(ctx.out)
}
