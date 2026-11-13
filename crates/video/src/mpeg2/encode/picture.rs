//! Slices and macroblocks of one frame picture (H.262 §6.2.4-§6.2.6), with a reconstruction
//! that matches the decoder exactly. One slice per macroblock row; slices only depend on the
//! reference picture, so rows can be coded in parallel.

use crate::mpeg2::headers::SCAN;
use crate::mpeg2::mc::{Block, Frame, predict};
use crate::mpeg2::slice::NON_LINEAR_Q;
use crate::mpeg2::vlc::{MB_FWD, MB_INTRA, MB_PATTERN, MB_QUANT};

use super::bitwriter::BitWriter;
use super::motion::Search;
use super::tables::{EncTables, enc_tables, wrap_delta};
use super::transform::{fdct, quant_inter, quant_intra, reconstruct};

/// Intra coding is chosen when its cost (the luma's deviation from its block means) undercuts
/// the best prediction's SAD by this much.
const INTRA_BIAS: u32 = 256;

/// quantiser_scale for quantiser_scale_code `code` (Table 7-6 when `nonlinear`).
pub(crate) fn qscale(nonlinear: bool, code: u8) -> i32 {
    if nonlinear { i32::from(NON_LINEAR_Q[usize::from(code & 31)]) } else { 2 * i32::from(code & 31) }
}

/// The quantiser_scale_code (1..=31) whose quantiser_scale is closest to `qs`.
pub(crate) fn qcode_for(nonlinear: bool, qs: f64) -> u8 {
    let mut best = (1u8, f64::MAX);
    for code in 1..=31u8 {
        let d = (f64::from(qscale(nonlinear, code)) - qs).abs();
        if d < best.1 {
            best = (code, d);
        }
    }
    best.0
}

/// What every slice of one picture shares (read-only).
pub(crate) struct PicCtx<'a> {
    /// The padded source picture.
    pub src: &'a Frame,
    /// The reconstructed previous picture (P pictures); `None` codes an I picture.
    pub reference: Option<&'a Frame>,
    /// Vectors of the previous picture (motion search candidates), one per macroblock.
    pub prev_mvs: &'a [[i32; 2]],
    pub mb_w: usize,
    pub shx: u32,
    pub shy: u32,
    pub intra_matrix: &'a [u8; 64],
    pub inter_matrix: &'a [u8; 64],
    pub nonlinear: bool,
    /// intra_dc_precision (0..=2: 8..=10 bits).
    pub dc_prec: u8,
    pub f_code: u8,
    pub search: i32,
    /// quantiser_scale_code per macroblock.
    pub mb_qcode: &'a [u8],
}

impl PicCtx<'_> {
    fn cmb(&self) -> (usize, usize) {
        (16 >> self.shx, 16 >> self.shy)
    }
    fn blocks(&self) -> usize {
        if self.shy == 1 { 6 } else { 8 }
    }
    fn dc_reset(&self) -> i32 {
        1 << (7 + self.dc_prec)
    }
}

/// The part of the reconstructed picture one macroblock row covers.
pub(crate) struct RowPlanes<'b> {
    pub y: &'b mut [u8],
    pub cb: &'b mut [u8],
    pub cr: &'b mut [u8],
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RowStats {
    pub intra: u32,
    pub skipped: u32,
    /// Sum of the quantiser_scale of every macroblock.
    pub qsum: u64,
}

pub(crate) struct RowOut {
    /// The slice, from its start code, byte aligned.
    pub bytes: Vec<u8>,
    /// The vector each macroblock used (zero for intra and skipped ones).
    pub mvs: Vec<[i32; 2]>,
    pub stats: RowStats,
}

/// One macroblock's samples: luma 16×16, chroma rows of `16 >> shx`.
#[derive(Clone)]
struct MbPix {
    y: [u8; 256],
    c: [[u8; 256]; 2],
}

impl MbPix {
    fn load(f: &Frame, mbx: usize, mby: usize, (cbw, cbh): (usize, usize)) -> MbPix {
        let mut m = MbPix { y: [0; 256], c: [[0; 256]; 2] };
        for r in 0..16 {
            let at = (mby * 16 + r) * f.w + mbx * 16;
            m.y[r * 16..r * 16 + 16].copy_from_slice(&f.planes[0][at..at + 16]);
        }
        for c in 0..2 {
            for r in 0..cbh {
                let at = (mby * cbh + r) * f.cw + mbx * cbw;
                m.c[c][r * cbw..r * cbw + cbw].copy_from_slice(&f.planes[1 + c][at..at + cbw]);
            }
        }
        m
    }

    /// Plane, offset and stride of block `i` (H.262 Figure 6-10 order: four luma blocks, then
    /// Cb, Cr, and for 4:2:2 the lower Cb, Cr).
    fn block_at(i: usize, cbw: usize) -> (usize, usize, usize) {
        if i < 4 { (0, (i >> 1) * 8 * 16 + (i & 1) * 8, 16) } else { (1 + (i & 1), ((i - 4) >> 1) * 8 * cbw, cbw) }
    }

    fn plane(&self, p: usize) -> &[u8; 256] {
        if p == 0 { &self.y } else { &self.c[p - 1] }
    }

    fn plane_mut(&mut self, p: usize) -> &mut [u8; 256] {
        if p == 0 { &mut self.y } else { &mut self.c[p - 1] }
    }

    fn block(&self, i: usize, cbw: usize) -> [i32; 64] {
        let (p, off, stride) = Self::block_at(i, cbw);
        let d = self.plane(p);
        std::array::from_fn(|k| i32::from(d[off + (k >> 3) * stride + (k & 7)]))
    }

    /// Write `v` (intra samples) or add it to what is there (inter residual), saturated.
    fn store(&mut self, i: usize, cbw: usize, v: &[i32; 64], add: bool) {
        let (p, off, stride) = Self::block_at(i, cbw);
        let d = self.plane_mut(p);
        for (k, &x) in v.iter().enumerate() {
            let at = off + (k >> 3) * stride + (k & 7);
            let base = if add { i32::from(d[at]) } else { 0 };
            d[at] = (base + x).clamp(0, 255) as u8;
        }
    }

    fn put(&self, out: &mut RowPlanes, mbx: usize, (cbw, cbh): (usize, usize), (w, cw): (usize, usize)) {
        for r in 0..16 {
            let at = r * w + mbx * 16;
            if let Some(d) = out.y.get_mut(at..at + 16) {
                d.copy_from_slice(&self.y[r * 16..r * 16 + 16]);
            }
        }
        for (c, plane) in [&mut *out.cb, &mut *out.cr].into_iter().enumerate() {
            for r in 0..cbh {
                let at = r * cw + mbx * cbw;
                if let Some(d) = plane.get_mut(at..at + cbw) {
                    d.copy_from_slice(&self.c[c][r * cbw..r * cbw + cbw]);
                }
            }
        }
    }
}

/// Slice coding state (what the decoder tracks too).
struct SliceSt {
    qcode: u8,
    dc: [i32; 3],
    pmv: [i32; 2],
    /// Skipped macroblocks not yet covered by an address increment.
    skipped: usize,
    /// The vector of the macroblock to the left (motion search candidate).
    left_mv: [i32; 2],
}

/// One macroblock being coded.
struct MbIn<'x> {
    src: &'x MbPix,
    mbx: usize,
    mby: usize,
    qcode: u8,
}

/// Code macroblock row `row` as one slice, writing its reconstruction into `out`.
pub(crate) fn encode_row(ctx: &PicCtx, row: usize, out: &mut RowPlanes) -> RowOut {
    let mut w = BitWriter::new();
    w.start_code(1 + row as u8);
    let q0 = ctx.mb_qcode.get(row * ctx.mb_w).copied().unwrap_or(1);
    w.put(u32::from(q0), 5);
    w.bit(false); // extra_bit_slice
    let mut st = SliceSt { qcode: q0, dc: [ctx.dc_reset(); 3], pmv: [0; 2], skipped: 0, left_mv: [0; 2] };
    let mut ro = RowOut { bytes: Vec::new(), mvs: vec![[0; 2]; ctx.mb_w], stats: RowStats::default() };
    for mbx in 0..ctx.mb_w {
        macroblock(ctx, &mut st, &mut w, (mbx, row), out, &mut ro);
    }
    ro.bytes = w.finish();
    ro
}

fn macroblock(ctx: &PicCtx, st: &mut SliceSt, w: &mut BitWriter, (mbx, mby): (usize, usize), out: &mut RowPlanes, ro: &mut RowOut) {
    let addr = mby * ctx.mb_w + mbx;
    let qcode = ctx.mb_qcode.get(addr).copied().unwrap_or(st.qcode);
    let src = MbPix::load(ctx.src, mbx, mby, ctx.cmb());
    let mb = MbIn { src: &src, mbx, mby, qcode };
    let qs = qscale(ctx.nonlinear, qcode);
    ro.stats.qsum += qs as u64;
    let planes = (ctx.src.w, ctx.src.cw);
    let Some(reference) = ctx.reference else {
        intra_mb(ctx, st, w, &mb).put(out, mbx, ctx.cmb(), planes);
        ro.stats.intra += 1;
        return;
    };
    let mut candidates = vec![st.left_mv, st.pmv];
    for a in [Some(addr), addr.checked_sub(ctx.mb_w), Some(addr + 1), Some(addr + ctx.mb_w)].into_iter().flatten() {
        if let Some(&v) = ctx.prev_mvs.get(a) {
            candidates.push(v);
        }
    }
    let lambda = ((qs * 3 + 4) / 8).max(1) as u32;
    let search = Search { reference, src: &src.y, mbx, mby, f_code: ctx.f_code, range: ctx.search, pmv: st.pmv, lambda };
    let found = search.run(&candidates);
    if intra_cost(&src.y).saturating_add(INTRA_BIAS) < found.sad {
        intra_mb(ctx, st, w, &mb).put(out, mbx, ctx.cmb(), planes);
        ro.stats.intra += 1;
        st.left_mv = [0; 2];
        return;
    }
    let (rec, skipped) = inter_mb(ctx, st, w, &mb, reference, found.mv);
    rec.put(out, mbx, ctx.cmb(), planes);
    ro.stats.skipped += skipped as u32;
    ro.mvs[mbx] = found.mv;
    st.left_mv = found.mv;
}

/// Σ |sample − block mean| over the four luma blocks: the cost of coding them intra.
fn intra_cost(y: &[u8; 256]) -> u32 {
    let mut cost = 0u32;
    for b in 0..4 {
        let off = (b >> 1) * 128 + (b & 1) * 8;
        let at = |k: usize| u32::from(y[off + (k >> 3) * 16 + (k & 7)]);
        let mean = ((0..64).map(at).sum::<u32>() + 32) / 64;
        cost += (0..64).map(|k| at(k).abs_diff(mean)).sum::<u32>();
    }
    cost
}

fn put_increment(w: &mut BitWriter, t: &EncTables, mut n: usize) {
    while n > 33 {
        t.mba_escape.put(w);
        n -= 33;
    }
    t.mba[n.max(1)].put(w);
}

/// Write the coefficients of a block (raster-order levels) in zigzag order from `start`, then
/// end of block.
fn put_coefficients(w: &mut BitWriter, t: &EncTables, levels: &[i32; 64], table: usize, start: usize) {
    let mut run = 0;
    let mut first = start == 0;
    for &pos in &SCAN[0][start..] {
        let l = levels[usize::from(pos)];
        if l == 0 {
            run += 1;
            continue;
        }
        t.put_coefficient(w, table, run, l, first);
        first = false;
        run = 0;
    }
    t.dct_eob[table].put(w);
}

fn put_mb_type(w: &mut BitWriter, t: &EncTables, predicted: bool, flags: i16) {
    let table = if predicted { &t.mbtype_p } else { &t.mbtype_i };
    if let Some(c) = table.get(flags as usize).copied().flatten() {
        c.put(w);
    }
}

/// Code an intra macroblock; returns its reconstruction.
fn intra_mb(ctx: &PicCtx, st: &mut SliceSt, w: &mut BitWriter, mb: &MbIn) -> MbPix {
    let t = enc_tables();
    let quant = mb.qcode != st.qcode;
    put_increment(w, t, st.skipped + 1);
    st.skipped = 0;
    put_mb_type(w, t, ctx.reference.is_some(), if quant { MB_INTRA | MB_QUANT } else { MB_INTRA });
    if quant {
        w.put(u32::from(mb.qcode), 5);
        st.qcode = mb.qcode;
    }
    st.pmv = [0; 2];
    let qs = qscale(ctx.nonlinear, st.qcode);
    let dc_mult = 8 >> ctx.dc_prec;
    let dc_max = (1 << (8 + ctx.dc_prec)) - 1;
    let (cbw, _) = ctx.cmb();
    let mut rec = mb.src.clone();
    for i in 0..ctx.blocks() {
        let cc = if i < 4 { 0 } else { 1 + (i & 1) };
        let f = fdct(&mb.src.block(i, cbw));
        let mut levels = [0i32; 64];
        levels[0] = ((f[0] / dc_mult as f32).round() as i32).clamp(0, dc_max);
        for pos in 1..64 {
            levels[pos] = quant_intra(f[pos], ctx.intra_matrix[pos], qs);
        }
        let diff = levels[0] - st.dc[cc];
        st.dc[cc] = levels[0];
        let size = 32 - diff.unsigned_abs().leading_zeros();
        let size_codes = if cc == 0 { &t.dc_luma } else { &t.dc_chroma };
        size_codes[size as usize].put(w);
        if size > 0 {
            let v = if diff > 0 { diff } else { diff + (1 << size) - 1 };
            w.put(v as u32, size);
        }
        put_coefficients(w, t, &levels, 1, 1);
        rec.store(i, cbw, &reconstruct(&levels, ctx.intra_matrix, qs, true, dc_mult), false);
    }
    rec
}

/// Code a predicted macroblock with vector `mv` (or skip it); returns its reconstruction and
/// whether it was skipped.
fn inter_mb(ctx: &PicCtx, st: &mut SliceSt, w: &mut BitWriter, mb: &MbIn, reference: &Frame, mv: [i32; 2]) -> (MbPix, bool) {
    let t = enc_tables();
    let mut rec = predict_mb(ctx, reference, mv, mb.mbx, mb.mby);
    let qs = qscale(ctx.nonlinear, mb.qcode);
    let (cbw, _) = ctx.cmb();
    let n = ctx.blocks();
    let mut levels = [[0i32; 64]; 8];
    let mut cbp = 0u32;
    for i in 0..n {
        let (s, p) = (mb.src.block(i, cbw), rec.block(i, cbw));
        let f = fdct(&std::array::from_fn(|k| s[k] - p[k]));
        for pos in 0..64 {
            levels[i][pos] = quant_inter(f[pos], ctx.inter_matrix[pos], qs);
        }
        if levels[i].iter().any(|&l| l != 0) {
            cbp |= 1 << (n - 1 - i);
        }
    }
    st.dc = [ctx.dc_reset(); 3];
    // the first and last macroblock of a slice are always coded
    let edge = mb.mbx == 0 || mb.mbx + 1 == ctx.mb_w;
    if mv == [0, 0] && cbp == 0 && !edge {
        st.skipped += 1;
        st.pmv = [0; 2];
        return (rec, true);
    }
    put_increment(w, t, st.skipped + 1);
    st.skipped = 0;
    let quant = cbp != 0 && mb.qcode != st.qcode;
    let mut flags = if cbp == 0 {
        MB_FWD
    } else if mv == [0, 0] {
        MB_PATTERN
    } else {
        MB_FWD | MB_PATTERN
    };
    if quant {
        flags |= MB_QUANT;
    }
    put_mb_type(w, t, true, flags);
    if quant {
        w.put(u32::from(mb.qcode), 5);
        st.qcode = mb.qcode;
    }
    if flags & MB_FWD != 0 {
        for c in 0..2 {
            t.put_motion(w, wrap_delta(mv[c], st.pmv[c], ctx.f_code), ctx.f_code);
        }
        st.pmv = mv;
    } else {
        st.pmv = [0; 2]; // "No MC" resets the predictors
    }
    if cbp != 0 {
        if n == 8 {
            t.cbp[(cbp >> 2) as usize].put(w);
            w.put(cbp & 3, 2);
        } else {
            t.cbp[cbp as usize].put(w);
        }
        for i in 0..n {
            if cbp & (1 << (n - 1 - i)) != 0 {
                put_coefficients(w, t, &levels[i], 0, 0);
                rec.store(i, cbw, &reconstruct(&levels[i], ctx.inter_matrix, qs, false, 0), true);
            }
        }
    }
    (rec, false)
}

/// The frame prediction of a macroblock from `f` with vector `mv`, formed as the decoder forms
/// it (§7.6.4: chroma vectors are the luma ones halved towards zero).
fn predict_mb(ctx: &PicCtx, f: &Frame, mv: [i32; 2], mbx: usize, mby: usize) -> MbPix {
    let (cbw, cbh) = ctx.cmb();
    let mut p = MbPix { y: [0; 256], c: [[0; 256]; 2] };
    let blk =
        Block { x: (mbx * 16) as i32 + (mv[0] >> 1), y: (mby * 16) as i32 + (mv[1] >> 1), hx: mv[0] & 1 != 0, hy: mv[1] & 1 != 0, bw: 16, bh: 16 };
    predict(&mut p.y, 0, 16, f.plane(0, None), blk, false);
    let cmv = [if ctx.shx == 1 { mv[0] / 2 } else { mv[0] }, if ctx.shy == 1 { mv[1] / 2 } else { mv[1] }];
    let cb = Block {
        x: (mbx * cbw) as i32 + (cmv[0] >> 1),
        y: (mby * cbh) as i32 + (cmv[1] >> 1),
        hx: cmv[0] & 1 != 0,
        hy: cmv[1] & 1 != 0,
        bw: cbw,
        bh: cbh,
    };
    for c in 0..2 {
        predict(&mut p.c[c], 0, cbw, f.plane(1 + c, None), cb, false);
    }
    p
}
