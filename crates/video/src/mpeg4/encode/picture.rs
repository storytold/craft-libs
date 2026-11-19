//! One video object plane: mode decision, motion estimation, transform and quantisation, the
//! reconstruction a decoder makes, and the macroblock syntax (ISO/IEC 14496-2 §6.2.6–6.2.8,
//! §7.4–7.6).

use craft_media::bitstream::BitWriter;

use super::dct::{fdct, idct};
use super::tables::{self, CBPY, DC_SIZE_CHROMA, DC_SIZE_LUMA, ESCAPE, MCBPC_INTER, MCBPC_INTRA, MVD, Tcoef, ZIGZAG};

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

/// A picture at macroblock-aligned size.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
}

impl Frame {
    pub fn new(mb_w: usize, mb_h: usize) -> Frame {
        Frame { y: Plane::new(mb_w * 16, mb_h * 16), u: Plane::new(mb_w * 8, mb_h * 8), v: Plane::new(mb_w * 8, mb_h * 8) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Intra,
    Inter,
    /// `not_coded`: the co-located macroblock of the reference, no residual.
    Skip,
}

/// A coded macroblock.
#[derive(Clone, Debug)]
pub(super) struct Mb {
    pub mode: Mode,
    /// Motion vector in half samples (zero unless inter).
    pub mv: (i32, i32),
    /// Its predictor (§7.6.5).
    pub pred: (i32, i32),
    /// Coded block pattern, block `i` in bit `5 - i` (Y0–Y3, Cb, Cr). Intra: the AC coefficients.
    pub cbp: u8,
    /// Intra: `dct_dc_differential` of each block.
    pub dc_diff: [i32; 6],
    /// Quantised levels in raster order (intra: index 0 is unused).
    pub levels: [[i16; 64]; 6],
}

impl Mb {
    pub fn skip() -> Mb {
        Mb { mode: Mode::Skip, mv: (0, 0), pred: (0, 0), cbp: 0, dc_diff: [0; 6], levels: [[0; 64]; 6] }
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
fn dequant(level: i32, qp: i32) -> i32 {
    if level == 0 {
        return 0;
    }
    let mag = qp * (2 * level.abs() + 1) - if qp & 1 == 0 { 1 } else { 0 };
    (if level < 0 { -mag } else { mag }).clamp(-2048, 2047)
}

/// The largest level the fixed-length escape carries.
const MAX_LEVEL: i32 = 2047;

/// The value of an intra DC prediction neighbour outside the VOP or not intra (`2^(8+2)`).
const DC_DEFAULT: i32 = 1024;

/// Reconstructed intra DC values (`F[0][0]`) of the blocks coded so far, for DC prediction.
struct DcStore {
    /// Luminance, one per 8×8 block.
    y: Vec<i32>,
    yw: usize,
    /// Chrominance, one per macroblock.
    c: [Vec<i32>; 2],
    cw: usize,
}

impl DcStore {
    fn new(mb_w: usize, mb_h: usize) -> DcStore {
        let n = mb_w * mb_h;
        DcStore { y: vec![DC_DEFAULT; 4 * n], yw: 2 * mb_w, c: [vec![DC_DEFAULT; n], vec![DC_DEFAULT; n]], cw: mb_w }
    }

    /// The grid, its width and the block's position for block `b` of macroblock `(mx, my)`.
    fn locate(&mut self, b: usize, mx: usize, my: usize) -> (&mut Vec<i32>, usize, usize, usize) {
        match b {
            0..=3 => (&mut self.y, self.yw, 2 * mx + (b & 1), 2 * my + (b >> 1)),
            _ => (&mut self.c[b - 4], self.cw, mx, my),
        }
    }

    /// The prediction `F_P` of block `b` (§7.4.3.1): from the block above when the horizontal
    /// gradient is the smaller, else from the block to the left.
    fn predict(&mut self, b: usize, mx: usize, my: usize) -> i32 {
        let (g, w, bx, by) = self.locate(b, mx, my);
        let get = |x: usize, y: usize, dx: bool, dy: bool| -> i32 {
            if (dx && x == 0) || (dy && y == 0) {
                return DC_DEFAULT;
            }
            g[(y - usize::from(dy)) * w + x - usize::from(dx)]
        };
        let a = get(bx, by, true, false);
        let bb = get(bx, by, true, true);
        let c = get(bx, by, false, true);
        if (a - bb).abs() < (bb - c).abs() { c } else { a }
    }

    fn set(&mut self, b: usize, mx: usize, my: usize, v: i32) {
        let (g, w, bx, by) = self.locate(b, mx, my);
        g[by * w + bx] = v;
    }
}

/// Motion-compensated prediction of a `size`×`size` block at `(x, y)` with a half-sample vector
/// (§7.6.2), samples outside the plane taken from its edge.
fn predict(plane: &Plane, x: i32, y: i32, mv: (i32, i32), size: usize, rounding: bool, out: &mut [u8]) {
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

/// The chrominance vector of a 16×16 luminance vector: halved, with every fraction to a half
/// sample (§7.6.2.2).
#[inline]
fn chroma_mv(mv: (i32, i32)) -> (i32, i32) {
    ((mv.0 >> 1) | (mv.0 & 1), (mv.1 >> 1) | (mv.1 & 1))
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

/// The largest vector component the search tries, in half samples (`fcode` 2 covers it).
const SEARCH_LIMIT: i32 = 63;

/// Encoder-side state for one VOP.
pub(super) struct PictureCoder<'a> {
    pub src: &'a Frame,
    pub reference: Option<&'a Frame>,
    /// Picture size in samples (even).
    pub width: usize,
    pub height: usize,
    pub mb_w: usize,
    pub mb_h: usize,
    pub qp: i32,
    pub rounding: bool,
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
    pub fcode: u8,
}

impl PictureCoder<'_> {
    /// Decide, transform and reconstruct every macroblock.
    pub fn code(&mut self) -> Coded {
        let mut recon = Frame::new(self.mb_w, self.mb_h);
        let mut dc = DcStore::new(self.mb_w, self.mb_h);
        let mut mbs: Vec<Mb> = Vec::with_capacity(self.mb_w * self.mb_h);
        for my in 0..self.mb_h {
            for mx in 0..self.mb_w {
                let i = my * self.mb_w + mx;
                let mb = match self.reference {
                    None => self.intra(mx, my, &mut dc, &mut recon),
                    Some(r) => {
                        let pred = mv_pred(&mbs, self.mb_w, mx, my);
                        self.inter_or_intra(r, mx, my, pred, &mbs, &mut dc, &mut recon)
                    }
                };
                match mb.mode {
                    Mode::Intra => self.refresh[i] = 0,
                    Mode::Inter => self.refresh[i] = self.refresh[i].saturating_add(1),
                    Mode::Skip => {}
                }
                mbs.push(mb);
            }
        }
        let reach = mbs.iter().map(|m| m.mv.0.max(-m.mv.0 - 1).max(m.mv.1).max(-m.mv.1 - 1)).max().unwrap_or(0);
        let fcode = if reach < 32 { 1 } else { 2 };
        Coded { mbs, recon, fcode }
    }

    /// 8×8 source block `b` of macroblock `(mx, my)`.
    fn source_block(&self, b: usize, mx: usize, my: usize) -> [i32; 64] {
        let (plane, x, y) = block_origin(self.src, b, mx, my);
        std::array::from_fn(|k| i32::from(plane.data[(y + k / 8) * plane.w + x + k % 8]))
    }

    fn intra(&self, mx: usize, my: usize, dc: &mut DcStore, recon: &mut Frame) -> Mb {
        let qp = self.qp;
        let mut mb = Mb { mode: Mode::Intra, ..Mb::skip() };
        for b in 0..6 {
            let px = self.source_block(b, mx, my);
            let mut coef = [0; 64];
            fdct(&px, &mut coef);
            let scaler = dc_scaler(qp, b < 4);
            let qf = ((coef[0].max(0) + scaler / 2) / scaler).min(2047 / scaler);
            let fp = dc.predict(b, mx, my);
            mb.dc_diff[b] = qf - (fp + scaler / 2) / scaler;
            dc.set(b, mx, my, qf * scaler);
            let mut rec = [0; 64];
            rec[0] = qf * scaler;
            for k in 1..64 {
                let c = coef[k];
                let l = (c.abs() / (2 * qp)).min(MAX_LEVEL);
                let l = if c < 0 { -l } else { l };
                mb.levels[b][k] = l as i16;
                rec[k] = dequant(l, qp);
                if l != 0 {
                    mb.cbp |= 1 << (5 - b);
                }
            }
            idct(&mut rec);
            let (plane, x, y) = block_origin_mut(recon, b, mx, my);
            for k in 0..64 {
                plane.data[(y + k / 8) * plane.w + x + k % 8] = rec[k].clamp(0, 255) as u8;
            }
        }
        mb
    }

    /// Whether a vector reads only samples inside the picture for the part of the macroblock that
    /// is inside it, so the prediction does not depend on how a decoder pads its reference.
    fn mv_ok(&self, mx: usize, my: usize, mv: (i32, i32)) -> bool {
        let range = -SEARCH_LIMIT - 1..=SEARCH_LIMIT;
        if !range.contains(&mv.0) || !range.contains(&mv.1) {
            return false;
        }
        let fits = |x0: usize, size: usize, extent: usize, v: i32| {
            let inside = size.min(extent - x0) as i32;
            let start = x0 as i32 + (v >> 1);
            start >= 0 && start + inside - 1 + (v & 1) < extent as i32
        };
        let c = chroma_mv(mv);
        fits(mx * 16, 16, self.width, mv.0)
            && fits(my * 16, 16, self.height, mv.1)
            && fits(mx * 8, 8, self.width / 2, c.0)
            && fits(my * 8, 8, self.height / 2, c.1)
    }

    /// Sum of absolute differences of the 16×16 luminance prediction with `mv`.
    fn sad(&self, r: &Frame, mx: usize, my: usize, mv: (i32, i32)) -> u32 {
        let mut p = [0u8; 256];
        predict(&r.y, (mx * 16) as i32, (my * 16) as i32, mv, 16, self.rounding, &mut p);
        let s = &self.src.y;
        let mut sad = 0u32;
        for j in 0..16 {
            let row = &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
            for i in 0..16 {
                sad += u32::from(row[i].abs_diff(p[j * 16 + i]));
            }
        }
        sad
    }

    /// Motion search: the best of the predictor, the neighbours' vectors and zero, a small
    /// diamond search in whole samples, then the eight half-sample positions around it.
    fn search(&self, r: &Frame, mx: usize, my: usize, pred: (i32, i32), cands: &[(i32, i32)]) -> ((i32, i32), u32) {
        let lambda = self.qp as u32;
        let cost = |mv: (i32, i32)| -> Option<(u32, u32)> {
            if !self.mv_ok(mx, my, mv) {
                return None;
            }
            let sad = self.sad(r, mx, my, mv);
            Some((sad + lambda * (mvd_bits(mv.0 - pred.0) + mvd_bits(mv.1 - pred.1)), sad))
        };
        let mut best = ((0, 0), cost((0, 0)).unwrap_or((u32::MAX, u32::MAX)));
        for &c in cands {
            let c = (c.0 & !1, c.1 & !1);
            if let Some(k) = cost(c).filter(|k| k.0 < best.1.0) {
                best = (c, k);
            }
        }
        for _ in 0..32 {
            let centre = best.0;
            for d in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
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
        for d in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let mv = (centre.0 + d.0, centre.1 + d.1);
            if let Some(k) = cost(mv).filter(|k| k.0 < best.1.0) {
                best = (mv, k);
            }
        }
        (best.0, best.1.1)
    }

    #[allow(clippy::too_many_arguments)]
    fn inter_or_intra(&self, r: &Frame, mx: usize, my: usize, pred: (i32, i32), mbs: &[Mb], dc: &mut DcStore, recon: &mut Frame) -> Mb {
        let i = my * self.mb_w + mx;
        let mut cands = vec![pred];
        if mx > 0 {
            cands.push(mbs[i - 1].mv);
        }
        if my > 0 {
            cands.push(mbs[i - self.mb_w].mv);
        }
        let (mv, sad) = self.search(r, mx, my, pred, &cands);
        // Intra when the macroblock's own variation is clearly below the prediction error
        // (the usual H.263 test), or when it is due for refresh.
        let s = &self.src.y;
        let row = |j: usize| &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
        let sum: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v)).sum::<u32>()).sum();
        let mean = (sum + 128) / 256;
        let dev: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v).abs_diff(mean)).sum::<u32>()).sum();
        if self.refresh[i] >= REFRESH || dev + 500 < sad {
            return self.intra(mx, my, dc, recon);
        }
        for b in 0..6 {
            dc.set(b, mx, my, DC_DEFAULT);
        }
        let mut mb = Mb { mode: Mode::Inter, mv, pred, ..Mb::skip() };
        let qp = self.qp;
        let cmv = chroma_mv(mv);
        let mut py = [0u8; 256];
        predict(&r.y, (mx * 16) as i32, (my * 16) as i32, mv, 16, self.rounding, &mut py);
        let mut pc = [[0u8; 64]; 2];
        predict(&r.u, (mx * 8) as i32, (my * 8) as i32, cmv, 8, self.rounding, &mut pc[0]);
        predict(&r.v, (mx * 8) as i32, (my * 8) as i32, cmv, 8, self.rounding, &mut pc[1]);
        let mut rec = [[0i32; 64]; 6];
        for b in 0..6 {
            let p = |k: usize| -> i32 {
                match b {
                    0..=3 => i32::from(py[((b >> 1) * 8 + k / 8) * 16 + (b & 1) * 8 + k % 8]),
                    _ => i32::from(pc[b - 4][k]),
                }
            };
            let src = self.source_block(b, mx, my);
            let resid: [i32; 64] = std::array::from_fn(|k| src[k] - p(k));
            let mut coef = [0; 64];
            fdct(&resid, &mut coef);
            let mut deq = [0; 64];
            for k in 0..64 {
                let c = coef[k];
                let l = ((c.abs() - qp / 2) / (2 * qp)).clamp(0, MAX_LEVEL);
                let l = if c < 0 { -l } else { l };
                mb.levels[b][k] = l as i16;
                deq[k] = dequant(l, qp);
                if l != 0 {
                    mb.cbp |= 1 << (5 - b);
                }
            }
            if mb.cbp & (1 << (5 - b)) != 0 {
                idct(&mut deq);
            }
            rec[b] = std::array::from_fn(|k| (p(k) + deq[k]).clamp(0, 255));
        }
        if mv == (0, 0) && mb.cbp == 0 {
            mb = Mb::skip();
        }
        for (b, rec) in rec.iter().enumerate() {
            let (plane, x, y) = block_origin_mut(recon, b, mx, my);
            for k in 0..64 {
                plane.data[(y + k / 8) * plane.w + x + k % 8] = rec[k] as u8;
            }
        }
        mb
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

/// The motion vector predictor of macroblock `(mx, my)` (§7.6.5): the median of the vectors to
/// the left, above and above right. A candidate outside the VOP is zero when it is the only one;
/// when two are outside, the third is the predictor. Intra and skipped macroblocks count as zero.
pub(super) fn mv_pred(mbs: &[Mb], mb_w: usize, mx: usize, my: usize) -> (i32, i32) {
    let i = my * mb_w + mx;
    let a = (mx > 0).then(|| mbs[i - 1].mv);
    let b = (my > 0).then(|| mbs[i - mb_w].mv);
    let c = (my > 0 && mx + 1 < mb_w).then(|| mbs[i - mb_w + 1].mv);
    match (a, b, c) {
        (None, None, None) => (0, 0),
        (Some(v), None, None) | (None, Some(v), None) | (None, None, Some(v)) => v,
        _ => {
            let [a, b, c] = [a, b, c].map(|v| v.unwrap_or((0, 0)));
            (median(a.0, b.0, c.0), median(a.1, b.1, c.1))
        }
    }
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

/// The run-length events of a block from scan position `start`, as `(run, level)`.
fn events(levels: &[i16; 64], start: usize) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    let mut run = 0;
    for &z in &ZIGZAG[start..] {
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

fn write_coeffs(w: &mut BitWriter, levels: &[i16; 64], start: usize, t: &Tcoef, escape_only: bool) {
    let ev = events(levels, start);
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
            write_coeffs(w, &mb.levels[b], 1, tables::intra(), escape_only);
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
        w.write_bit(false); // ac_pred_flag
        put(w, CBPY[cbpy]);
        write_intra_blocks(w, mb, escape_only);
        return;
    }
    match mb.mode {
        Mode::Skip => w.write_bit(true),
        Mode::Intra => {
            w.write_bit(false);
            put(w, MCBPC_INTER[3 * 4 + cbpc]);
            w.write_bit(false); // ac_pred_flag
            put(w, CBPY[cbpy]);
            write_intra_blocks(w, mb, escape_only);
        }
        Mode::Inter => {
            w.write_bit(false);
            put(w, MCBPC_INTER[cbpc]);
            put(w, CBPY[cbpy ^ 15]);
            write_mvd(w, mb.mv.0 - mb.pred.0, fcode);
            write_mvd(w, mb.mv.1 - mb.pred.1, fcode);
            for b in 0..6 {
                if mb.cbp & (1 << (5 - b)) != 0 {
                    write_coeffs(w, &mb.levels[b], 0, tables::inter(), escape_only);
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
        assert_eq!(dequant(1, 3), 9);
        assert_eq!(dequant(-1, 4), -11);
        assert_eq!(dequant(2000, 31), 2047);
        assert_eq!(dequant(-2000, 31), -2048);
    }

    #[test]
    fn chroma_vectors_round_to_half_samples() {
        let c = |v| chroma_mv((v, 0)).0;
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
        mbs[0].mv = (4, -2);
        mbs[1].mv = (6, 2);
        mbs[2].mv = (8, 0);
        // first row: the left vector
        assert_eq!(mv_pred(&mbs, 3, 1, 0), (4, -2));
        assert_eq!(mv_pred(&mbs, 3, 0, 0), (0, 0));
        // left column: left is zero, median of 0, above, above right
        assert_eq!(mv_pred(&mbs, 3, 0, 1), (4, 0));
        // right column: above right is zero
        mbs[4].mv = (2, 2);
        assert_eq!(mv_pred(&mbs, 3, 2, 1), (2, 0));
    }
}
