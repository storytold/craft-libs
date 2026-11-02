//! Macroblock-layer syntax values produced by mode decision and consumed by the entropy coders.

use crate::mbinfo::{MbInfo, MbKind, Mv, Part};
use crate::nal::SliceType;

#[derive(Clone)]
pub struct MbCode {
    pub kind: MbKind,
    pub part: Part,
    /// Prediction direction per partition: 0 = L0, 1 = L1, 2 = Bi.
    pub bdir: [u8; 4],
    pub i16_mode: u8,
    pub chroma_mode: u8,
    /// Per 4x4 (or 8x8, first four entries) block in decoding order: -1 = use predicted mode, else rem_intra_pred_mode.
    pub ipred: [i8; 16],
    pub t8x8: bool,
    /// mvd per list per partition.
    pub mvd: [[Mv; 4]; 2],
    pub cbp: u8,
    pub qp_delta: i32,
    /// Intra16x16 DC levels in zig-zag order.
    pub luma_dc: [i16; 16],
    /// Luma 4x4 levels by luma4x4BlkIdx, zig-zag order (index 0 unused for Intra16x16 AC).
    pub luma4: [[i16; 16]; 16],
    /// Luma 8x8 levels by 8x8 block, zig-zag order.
    pub luma8: [[i16; 64]; 4],
    /// Chroma DC levels (Cb, Cr) in raster (c0..c3) order.
    pub cdc: [[i16; 4]; 2],
    /// Chroma AC levels, Cb blocks 0..3 then Cr 0..3, zig-zag order (index 0 unused).
    pub cac: [[i16; 16]; 8],
}

impl Default for MbCode {
    fn default() -> Self {
        MbCode {
            kind: MbKind::None,
            part: Part::P16x16,
            bdir: [0; 4],
            i16_mode: 0,
            chroma_mode: 0,
            ipred: [-1; 16],
            t8x8: false,
            mvd: [[Mv::ZERO; 4]; 2],
            cbp: 0,
            qp_delta: 0,
            luma_dc: [0; 16],
            luma4: [[0; 16]; 16],
            luma8: [[0; 64]; 4],
            cdc: [[0; 4]; 2],
            cac: [[0; 16]; 8],
        }
    }
}

impl MbCode {
    /// B macroblock type number (Table 7-14) for inter B macroblocks.
    pub fn b_type_number(&self) -> u32 {
        match self.kind {
            MbKind::BDirect => 0,
            _ => match self.part {
                Part::P16x16 => 1 + self.bdir[0] as u32,
                Part::P16x8 | Part::P8x16 => {
                    const PAIRS: [(u8, u8); 9] = [(0, 0), (1, 1), (0, 1), (1, 0), (0, 2), (1, 2), (2, 0), (2, 1), (2, 2)];
                    let k = PAIRS.iter().position(|&p| p == (self.bdir[0], self.bdir[1])).unwrap_or(0) as u32;
                    4 + 2 * k + (self.part == Part::P8x16) as u32
                }
                Part::P8x8 => 22,
            },
        }
    }
    pub fn uses_list(&self, part: usize, list: usize) -> bool {
        match self.kind {
            MbKind::PInter => list == 0,
            MbKind::BInter => self.bdir[part] == 2 || self.bdir[part] as usize == list,
            _ => false,
        }
    }
}

/// Neighbour context for entropy coding one macroblock.
pub struct NbCtx<'a> {
    pub left: Option<&'a MbInfo>,
    pub top: Option<&'a MbInfo>,
    pub cur: &'a MbInfo,
    pub slice_type: SliceType,
    pub prev_dqp_nz: bool,
    pub t8x8_mode: bool,
}

impl NbCtx<'_> {
    /// coded_block_flag-style condition for an unavailable neighbour.
    #[inline]
    pub fn unavail_cbf(&self) -> u32 {
        self.cur.kind.is_intra() as u32
    }
    /// Luma 4x4 neighbour non-zero count (A = left when `left`, else B = above). Returns None if unavailable.
    #[inline]
    pub fn luma_nnz(&self, raster: usize, left: bool) -> Option<u8> {
        let (x, y) = (raster & 3, raster >> 2);
        if left {
            if x > 0 { Some(self.cur.nnz[raster - 1]) } else { self.left.map(|m| m.nnz[y * 4 + 3]) }
        } else if y > 0 {
            Some(self.cur.nnz[raster - 4])
        } else {
            self.top.map(|m| m.nnz[12 + x])
        }
    }
    /// Chroma AC neighbour count for component `c`, block raster `b` (2x2).
    #[inline]
    pub fn chroma_nnz(&self, c: usize, b: usize, left: bool) -> Option<u8> {
        let (x, y) = (b & 1, b >> 1);
        if left {
            if x > 0 { Some(self.cur.nnz_c[c * 4 + b - 1]) } else { self.left.map(|m| m.nnz_c[c * 4 + y * 2 + 1]) }
        } else if y > 0 {
            Some(self.cur.nnz_c[c * 4 + b - 2])
        } else {
            self.top.map(|m| m.nnz_c[c * 4 + 2 + x])
        }
    }
}
