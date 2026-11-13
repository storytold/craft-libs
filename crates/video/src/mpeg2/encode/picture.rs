//! Slices and macroblocks of one frame picture (H.262 §6.2.4-§6.2.6), with a reconstruction
//! that matches the decoder exactly. One slice per macroblock row; slices only depend on the
//! reference pictures, so rows can be coded in parallel.

use crate::mpeg2::headers::{PictureType, SCAN};
use crate::mpeg2::mc::{Block, Frame, predict};
use crate::mpeg2::slice::NON_LINEAR_Q;
use crate::mpeg2::vlc::{MB_BWD, MB_FWD, MB_INTRA, MB_PATTERN, MB_QUANT};

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
    /// I, P or B.
    pub ptype: PictureType,
    /// The padded source picture.
    pub src: &'a Frame,
    /// The reconstructed reference pictures: forward (P and B pictures) and backward (B
    /// pictures). A B picture without a forward reference (the leading B pictures of a closed
    /// GOP) predicts backward only.
    pub fwd: Option<&'a Frame>,
    pub bwd: Option<&'a Frame>,
    /// Motion search candidates per macroblock, forward and backward (empty: none).
    pub cand: [&'a [[i32; 2]]; 2],
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
    /// Motion vector predictors, forward and backward.
    pmv: [[i32; 2]; 2],
    /// Skipped macroblocks not yet covered by an address increment.
    skipped: usize,
    /// The vectors of the macroblock to the left (motion search candidates).
    left_mv: [[i32; 2]; 2],
    /// Prediction directions (`MB_FWD` / `MB_BWD`) of the previous macroblock, or `MB_INTRA`:
    /// a skipped macroblock in a B picture repeats them.
    last_dirs: i16,
}

/// A B macroblock's prediction: its directions and a vector per direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BPred {
    dirs: i16,
    mvs: [[i32; 2]; 2],
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
    let mut st = SliceSt { qcode: q0, dc: [ctx.dc_reset(); 3], pmv: [[0; 2]; 2], skipped: 0, left_mv: [[0; 2]; 2], last_dirs: MB_INTRA };
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
    let lambda = ((qs * 3 + 4) / 8).max(1) as u32;
    let rec = match (ctx.ptype, ctx.fwd) {
        (PictureType::P, Some(reference)) => {
            let search = Search { reference, src: &src.y, mbx, mby, f_code: ctx.f_code, range: ctx.search, pmv: st.pmv[0], lambda };
            let found = search.run(&candidates(ctx, st, addr, 0));
            if intra_cost(&src.y).saturating_add(INTRA_BIAS) < found.sad {
                None
            } else {
                let (rec, skipped) = inter_mb(ctx, st, w, &mb, reference, found.mv);
                ro.stats.skipped += skipped as u32;
                ro.mvs[mbx] = found.mv;
                st.left_mv[0] = found.mv;
                Some(rec)
            }
        }
        (PictureType::B, _) if ctx.bwd.is_some() => b_decision(ctx, st, &mb, lambda).map(|pred| {
            let (rec, skipped) = b_mb(ctx, st, w, &mb, pred);
            ro.stats.skipped += skipped as u32;
            st.left_mv = pred.mvs;
            rec
        }),
        _ => None,
    };
    let rec = rec.unwrap_or_else(|| {
        ro.stats.intra += 1;
        st.left_mv = [[0; 2]; 2];
        intra_mb(ctx, st, w, &mb)
    });
    rec.put(out, mbx, ctx.cmb(), planes);
}

/// Motion search candidates for direction `s` of macroblock `addr`: the left neighbour's vector,
/// the predictor, and the co-located and neighbouring candidates of the picture.
fn candidates(ctx: &PicCtx, st: &SliceSt, addr: usize, s: usize) -> Vec<[i32; 2]> {
    let mut c = vec![st.left_mv[s], st.pmv[s]];
    for a in [Some(addr), addr.checked_sub(ctx.mb_w), Some(addr + 1), Some(addr + ctx.mb_w)].into_iter().flatten() {
        if let Some(&v) = ctx.cand[s].get(a) {
            c.push(v);
        }
    }
    c
}

/// Luma SAD of a prediction against the source.
fn sad(a: &[u8; 256], b: &[u8; 256]) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(*y))).sum()
}

/// Choose a B macroblock's prediction: forward, backward or interpolated, from a motion search
/// per direction, or the skip prediction (the previous macroblock's directions with the
/// predictors). `None`: code it intra.
fn b_decision(ctx: &PicCtx, st: &SliceSt, mb: &MbIn, lambda: u32) -> Option<BPred> {
    let addr = mb.mby * ctx.mb_w + mb.mbx;
    let mut found = [None, None];
    for (s, r) in [ctx.fwd, ctx.bwd].into_iter().enumerate() {
        if let Some(reference) = r {
            let search =
                Search { reference, src: &mb.src.y, mbx: mb.mbx, mby: mb.mby, f_code: ctx.f_code, range: ctx.search, pmv: st.pmv[s], lambda };
            let f = search.run(&candidates(ctx, st, addr, s));
            found[s] = Some((f.mv, f.sad, search.mv_cost(f.mv)));
        }
    }
    let mut best: Option<(BPred, u32)> = None;
    let mut consider = |pred: BPred, cost: u32| {
        if best.is_none_or(|(_, c)| cost < c) {
            best = Some((pred, cost));
        }
    };
    for (s, dir) in [(0, MB_FWD), (1, MB_BWD)] {
        if let Some((mv, sad, bits)) = found[s] {
            let mut mvs = [[0; 2]; 2];
            mvs[s] = mv;
            consider(BPred { dirs: dir, mvs }, sad.saturating_add(bits));
        }
    }
    if let (Some((f, _, bits_f)), Some((b, _, bits_b))) = (found[0], found[1]) {
        let pred = BPred { dirs: MB_FWD | MB_BWD, mvs: [f, b] };
        let p = predict_b(ctx, pred, mb.mbx, mb.mby);
        consider(pred, sad(&p.y, &mb.src.y).saturating_add(bits_f).saturating_add(bits_b));
    }
    // the skip prediction costs no vector bits; the predictors came from the macroblock to the
    // left, so they must be checked to stay inside the reference from here
    let edge = mb.mbx == 0 || mb.mbx + 1 == ctx.mb_w;
    if !edge && st.last_dirs & MB_INTRA == 0 && st.last_dirs != 0 {
        let pred = BPred { dirs: st.last_dirs, mvs: st.pmv };
        let usable = [(0, MB_FWD, ctx.fwd), (1, MB_BWD, ctx.bwd)].into_iter().all(|(s, dir, r)| {
            st.last_dirs & dir == 0
                || r.is_some_and(|reference| {
                    let search =
                        Search { reference, src: &mb.src.y, mbx: mb.mbx, mby: mb.mby, f_code: ctx.f_code, range: ctx.search, pmv: st.pmv[s], lambda };
                    search.valid(st.pmv[s])
                })
        });
        if usable {
            consider(pred, sad(&predict_b(ctx, pred, mb.mbx, mb.mby).y, &mb.src.y));
        }
    }
    let (pred, cost) = best?;
    (intra_cost(&mb.src.y).saturating_add(INTRA_BIAS) >= cost).then_some(pred)
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

fn put_mb_type(w: &mut BitWriter, t: &EncTables, ptype: PictureType, flags: i16) {
    let table = match ptype {
        PictureType::P => &t.mbtype_p,
        PictureType::B => &t.mbtype_b,
        _ => &t.mbtype_i,
    };
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
    put_mb_type(w, t, ctx.ptype, if quant { MB_INTRA | MB_QUANT } else { MB_INTRA });
    if quant {
        w.put(u32::from(mb.qcode), 5);
        st.qcode = mb.qcode;
    }
    st.pmv = [[0; 2]; 2];
    st.last_dirs = MB_INTRA;
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
    let mut rec = predict_mb(ctx, Some(reference), mv, mb.mbx, mb.mby);
    let qs = qscale(ctx.nonlinear, mb.qcode);
    let (levels, cbp) = residual(ctx, mb, &rec, qs);
    st.dc = [ctx.dc_reset(); 3];
    st.last_dirs = MB_FWD;
    // the first and last macroblock of a slice are always coded
    let edge = mb.mbx == 0 || mb.mbx + 1 == ctx.mb_w;
    if mv == [0, 0] && cbp == 0 && !edge {
        st.skipped += 1;
        st.pmv = [[0; 2]; 2];
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
    put_mb_type(w, t, PictureType::P, flags);
    if quant {
        w.put(u32::from(mb.qcode), 5);
        st.qcode = mb.qcode;
    }
    if flags & MB_FWD != 0 {
        for c in 0..2 {
            t.put_motion(w, wrap_delta(mv[c], st.pmv[0][c], ctx.f_code), ctx.f_code);
        }
        st.pmv[0] = mv;
    } else {
        st.pmv = [[0; 2]; 2]; // "No MC" resets the predictors
    }
    put_residual(ctx, w, &levels, cbp, &mut rec, qs);
    (rec, false)
}

/// Quantised residual of every block of `mb` against `pred`, and the coded_block_pattern.
fn residual(ctx: &PicCtx, mb: &MbIn, pred: &MbPix, qs: i32) -> ([[i32; 64]; 8], u32) {
    let (cbw, _) = ctx.cmb();
    let n = ctx.blocks();
    let mut levels = [[0i32; 64]; 8];
    let mut cbp = 0u32;
    for i in 0..n {
        let (s, p) = (mb.src.block(i, cbw), pred.block(i, cbw));
        let f = fdct(&std::array::from_fn(|k| s[k] - p[k]));
        for pos in 0..64 {
            levels[i][pos] = quant_inter(f[pos], ctx.inter_matrix[pos], qs);
        }
        if levels[i].iter().any(|&l| l != 0) {
            cbp |= 1 << (n - 1 - i);
        }
    }
    (levels, cbp)
}

/// Write coded_block_pattern and the coded blocks, adding their reconstruction to `rec`.
fn put_residual(ctx: &PicCtx, w: &mut BitWriter, levels: &[[i32; 64]; 8], cbp: u32, rec: &mut MbPix, qs: i32) {
    if cbp == 0 {
        return;
    }
    let t = enc_tables();
    let (cbw, _) = ctx.cmb();
    let n = ctx.blocks();
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

/// Code a B macroblock with prediction `pred` (or skip it); returns its reconstruction and
/// whether it was skipped.
fn b_mb(ctx: &PicCtx, st: &mut SliceSt, w: &mut BitWriter, mb: &MbIn, pred: BPred) -> (MbPix, bool) {
    let t = enc_tables();
    let mut rec = predict_b(ctx, pred, mb.mbx, mb.mby);
    let qs = qscale(ctx.nonlinear, mb.qcode);
    let (levels, cbp) = residual(ctx, mb, &rec, qs);
    st.dc = [ctx.dc_reset(); 3];
    // a skipped B macroblock repeats the previous one's directions with the predictors as its
    // vectors (§7.6.6.3); the first and last macroblock of a slice are always coded
    let edge = mb.mbx == 0 || mb.mbx + 1 == ctx.mb_w;
    let same = |s: usize, dir: i16| pred.dirs & dir == 0 || pred.mvs[s] == st.pmv[s];
    if cbp == 0 && !edge && st.last_dirs & MB_INTRA == 0 && pred.dirs == st.last_dirs && same(0, MB_FWD) && same(1, MB_BWD) {
        st.skipped += 1;
        return (rec, true);
    }
    put_increment(w, t, st.skipped + 1);
    st.skipped = 0;
    let quant = cbp != 0 && mb.qcode != st.qcode;
    let mut flags = pred.dirs;
    if cbp != 0 {
        flags |= MB_PATTERN;
    }
    if quant {
        flags |= MB_QUANT;
    }
    put_mb_type(w, t, PictureType::B, flags);
    if quant {
        w.put(u32::from(mb.qcode), 5);
        st.qcode = mb.qcode;
    }
    for (s, dir) in [(0, MB_FWD), (1, MB_BWD)] {
        if pred.dirs & dir != 0 {
            for c in 0..2 {
                t.put_motion(w, wrap_delta(pred.mvs[s][c], st.pmv[s][c], ctx.f_code), ctx.f_code);
            }
            st.pmv[s] = pred.mvs[s];
        }
    }
    st.last_dirs = pred.dirs;
    put_residual(ctx, w, &levels, cbp, &mut rec, qs);
    (rec, false)
}

/// The prediction of a B macroblock: forward, backward, or the average of both, formed in the
/// decoder's order (forward first, then the backward prediction averaged in).
fn predict_b(ctx: &PicCtx, pred: BPred, mbx: usize, mby: usize) -> MbPix {
    let mut p = MbPix { y: [0; 256], c: [[0; 256]; 2] };
    let mut avg = false;
    for (s, dir, r) in [(0, MB_FWD, ctx.fwd), (1, MB_BWD, ctx.bwd)] {
        if pred.dirs & dir != 0 {
            if let Some(f) = r {
                predict_into(ctx, &mut p, f, pred.mvs[s], (mbx, mby), avg);
            }
            avg = true;
        }
    }
    p
}

/// The frame prediction of a macroblock from `f` with vector `mv` (gray without a reference).
fn predict_mb(ctx: &PicCtx, f: Option<&Frame>, mv: [i32; 2], mbx: usize, mby: usize) -> MbPix {
    let mut p = MbPix { y: [128; 256], c: [[128; 256]; 2] };
    if let Some(f) = f {
        predict_into(ctx, &mut p, f, mv, (mbx, mby), false);
    }
    p
}

/// Form the frame prediction from `f` with vector `mv` into `p`, or average it with what `p`
/// holds, as the decoder forms it (§7.6.4: chroma vectors are the luma ones halved towards zero).
fn predict_into(ctx: &PicCtx, p: &mut MbPix, f: &Frame, mv: [i32; 2], (mbx, mby): (usize, usize), avg: bool) {
    let (cbw, cbh) = ctx.cmb();
    let blk =
        Block { x: (mbx * 16) as i32 + (mv[0] >> 1), y: (mby * 16) as i32 + (mv[1] >> 1), hx: mv[0] & 1 != 0, hy: mv[1] & 1 != 0, bw: 16, bh: 16 };
    predict(&mut p.y, 0, 16, f.plane(0, None), blk, avg);
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
        predict(&mut p.c[c], 0, cbw, f.plane(1 + c, None), cb, avg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skip_prediction_never_leaves_the_reference() {
        // three macroblocks across; the left neighbour's forward vector (+17 samples) fits its
        // own position but points past the right edge from the middle macroblock
        let mut f = Frame::new(48, 16, 24, 8);
        for (i, v) in f.planes[0].iter_mut().enumerate() {
            *v = ((i % 48) * 5 + (i / 48) * 11) as u8;
        }
        let ctx = PicCtx {
            ptype: PictureType::B,
            src: &f,
            fwd: Some(&f),
            bwd: Some(&f),
            cand: [&[], &[]],
            mb_w: 3,
            shx: 1,
            shy: 1,
            intra_matrix: &crate::mpeg2::headers::DEFAULT_INTRA,
            inter_matrix: &[16; 64],
            nonlinear: true,
            dc_prec: 1,
            f_code: 2,
            search: 15,
            mb_qcode: &[4; 3],
        };
        let bad = BPred { dirs: MB_FWD, mvs: [[34, 0], [0, 0]] };
        // a source that the out-of-range (edge-clamped) prediction would match exactly
        let src = predict_b(&ctx, bad, 1, 0);
        let st = SliceSt { qcode: 4, dc: [256; 3], pmv: bad.mvs, skipped: 0, left_mv: bad.mvs, last_dirs: MB_FWD };
        let mb = MbIn { src: &src, mbx: 1, mby: 0, qcode: 4 };
        let pred = b_decision(&ctx, &st, &mb, 1);
        assert_ne!(pred, Some(bad));
    }
}
