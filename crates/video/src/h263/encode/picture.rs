//! One H.263 picture: mode decision, motion estimation, transform and quantisation, the
//! reconstruction a decoder makes, and the macroblock syntax (ITU-T H.263 §5.3, §6).

use craft_media::bitstream::BitWriter;

use crate::mpeg4::encode::dct::{fdct, idct};
use crate::mpeg4::encode::picture::{Frame, block_origin, block_origin_mut, chroma_mv, dequant, events, median, mvd_bits, predict, put, write_mvd};
use crate::mpeg4::encode::tables::{self, CBPY, ESCAPE, MCBPC_INTER, MCBPC_INTRA, Tcoef};

/// The largest vector component the search tries, in half samples. Without Annex D a vector
/// component lies in [-16, 15.5] (§6.1.1).
const SEARCH_LIMIT: i32 = 31;

/// The largest coefficient level: the escape code carries eight bits and forbids -128 (§5.4.2).
const LEVEL_LIMIT: i32 = 127;

/// Inter codings after which a macroblock is coded intra again: a mismatch between the
/// encoder's and a decoder's inverse transform must not build up over more than 132 (§4.4).
const REFRESH: u8 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Intra,
    Inter,
    /// COD = 1: the co-located macroblock of the reference, no residual.
    Skip,
}

/// A coded macroblock.
#[derive(Clone, Debug)]
pub(super) struct Mb {
    pub mode: Mode,
    /// Motion vector in half samples (zero unless inter).
    pub mv: (i32, i32),
    /// Its predictor (§6.1.1).
    pub pred: (i32, i32),
    /// Coded block pattern, block `i` in bit `5 - i` (Y0–Y3, Cb, Cr). Intra: the AC coefficients.
    pub cbp: u8,
    /// Intra: `INTRADC` of each block.
    pub dc: [u8; 6],
    /// Quantised levels in raster order (intra: index 0 is unused).
    pub levels: [[i16; 64]; 6],
}

impl Mb {
    fn skip() -> Mb {
        Mb { mode: Mode::Skip, mv: (0, 0), pred: (0, 0), cbp: 0, dc: [0; 6], levels: [[0; 64]; 6] }
    }
}

/// `INTRADC` for a DCT DC coefficient (eight times the block mean): a step of eight, and neither
/// 0 nor 128, which a decoder must not see; 255 stands for 1024 and is never chosen (§5.4.1).
fn intra_dc(coef: i32) -> u8 {
    let level = ((coef.max(0) + 4) / 8).clamp(1, 254);
    (if level == 128 { if coef < 1024 { 127 } else { 129 } } else { level }) as u8
}

/// Quantise `coef` from index `first`, with a dead zone of `dead`, into `levels` and the
/// dequantised values a decoder will transform; whether any level is non-zero.
fn quantise(coef: &[i32; 64], first: usize, qp: i32, dead: i32, levels: &mut [i16; 64], deq: &mut [i32; 64]) -> bool {
    let mut any = false;
    for (k, &c) in coef.iter().enumerate().skip(first) {
        let l = ((c.abs() - dead).max(0) / (2 * qp)).min(LEVEL_LIMIT);
        let l = if c < 0 { -l } else { l };
        levels[k] = l as i16;
        deq[k] = dequant(l, qp);
        any |= l != 0;
    }
    any
}

/// Encoder-side state for one picture.
pub(super) struct PictureCoder<'a> {
    pub src: &'a Frame,
    pub reference: Option<&'a Frame>,
    /// Picture size in samples.
    pub width: usize,
    pub height: usize,
    pub mb_w: usize,
    pub mb_h: usize,
    pub qp: i32,
    /// Macroblock rows to a GOB when GOB headers are written; vector prediction starts afresh
    /// at each of them (§6.1.1).
    pub gob_rows: Option<usize>,
    /// Per macroblock: inter codings since its last intra coding.
    pub refresh: &'a mut [u8],
}

/// What [`PictureCoder::code`] made.
pub(super) struct Coded {
    pub mbs: Vec<Mb>,
    pub recon: Frame,
}

impl PictureCoder<'_> {
    /// Decide, transform and reconstruct every macroblock.
    pub fn code(&mut self) -> Coded {
        let mut recon = Frame::new(self.mb_w, self.mb_h);
        let mut mbs: Vec<Mb> = Vec::with_capacity(self.mb_w * self.mb_h);
        for my in 0..self.mb_h {
            for mx in 0..self.mb_w {
                let i = my * self.mb_w + mx;
                let mb = match self.reference {
                    None => self.intra(mx, my, &mut recon),
                    Some(r) => {
                        let pred = self.mv_pred(&mbs, mx, my);
                        self.inter_or_intra(r, mx, my, pred, &mbs, &mut recon)
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
        Coded { mbs, recon }
    }

    /// The vector predictor of macroblock `(mx, my)` (§6.1.1): the median of the vectors to the
    /// left, above and above right. Left outside the picture is zero; above, and above right, are
    /// the left vector when above is outside (the top row, or the top of a GOB with a header);
    /// above right beyond the right edge is zero. Intra and skipped macroblocks count as zero.
    fn mv_pred(&self, mbs: &[Mb], mx: usize, my: usize) -> (i32, i32) {
        let i = my * self.mb_w + mx;
        let v1 = if mx == 0 { (0, 0) } else { mbs[i - 1].mv };
        let top = self.gob_rows.map_or(0, |k| my - my % k);
        let above = my > top;
        let v2 = if above { mbs[i - self.mb_w].mv } else { v1 };
        let v3 = if mx + 1 >= self.mb_w {
            (0, 0)
        } else if above {
            mbs[i - self.mb_w + 1].mv
        } else {
            v1
        };
        (median(v1.0, v2.0, v3.0), median(v1.1, v2.1, v3.1))
    }

    /// 8×8 source block `b` of macroblock `(mx, my)`.
    fn source_block(&self, b: usize, mx: usize, my: usize) -> [i32; 64] {
        let (plane, x, y) = block_origin(self.src, b, mx, my);
        std::array::from_fn(|k| i32::from(plane.data[(y + k / 8) * plane.w + x + k % 8]))
    }

    fn intra(&self, mx: usize, my: usize, recon: &mut Frame) -> Mb {
        let qp = self.qp;
        let mut mb = Mb { mode: Mode::Intra, ..Mb::skip() };
        for b in 0..6 {
            let mut coef = [0; 64];
            fdct(&self.source_block(b, mx, my), &mut coef);
            mb.dc[b] = intra_dc(coef[0]);
            let mut rec = [0; 64];
            rec[0] = i32::from(mb.dc[b]) * 8;
            if quantise(&coef, 1, qp, 0, &mut mb.levels[b], &mut rec) {
                mb.cbp |= 1 << (5 - b);
            }
            idct(&mut rec);
            store(recon, b, mx, my, |k| rec[k].clamp(0, 255) as u8);
        }
        mb
    }

    /// Whether a vector keeps the whole macroblock, luminance and chrominance, inside the
    /// picture, as it must without Annex D, so the prediction never depends on edge padding.
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
        predict(&r.y, (mx * 16) as i32, (my * 16) as i32, mv, 16, false, &mut p);
        let s = &self.src.y;
        (0..16)
            .map(|j| {
                let row = &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
                row.iter().zip(&p[j * 16..][..16]).map(|(&a, &b)| u32::from(a.abs_diff(b))).sum::<u32>()
            })
            .sum()
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

    fn inter_or_intra(&self, r: &Frame, mx: usize, my: usize, pred: (i32, i32), mbs: &[Mb], recon: &mut Frame) -> Mb {
        let i = my * self.mb_w + mx;
        let mut cands = vec![pred];
        if mx > 0 {
            cands.push(mbs[i - 1].mv);
        }
        if my > 0 {
            cands.push(mbs[i - self.mb_w].mv);
        }
        let (mv, sad) = self.search(r, mx, my, pred, &cands);
        // Intra when the macroblock's own variation is clearly below the prediction error,
        // or when it is due for refresh.
        let s = &self.src.y;
        let row = |j: usize| &s.data[(my * 16 + j) * s.w + mx * 16..][..16];
        let sum: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v)).sum::<u32>()).sum();
        let mean = (sum + 128) / 256;
        let dev: u32 = (0..16).map(|j| row(j).iter().map(|&v| u32::from(v).abs_diff(mean)).sum::<u32>()).sum();
        if self.refresh[i] >= REFRESH || dev + 500 < sad {
            return self.intra(mx, my, recon);
        }
        let mut mb = Mb { mode: Mode::Inter, mv, pred, ..Mb::skip() };
        let qp = self.qp;
        let cmv = chroma_mv(mv);
        let mut py = [0u8; 256];
        predict(&r.y, (mx * 16) as i32, (my * 16) as i32, mv, 16, false, &mut py);
        let mut pc = [[0u8; 64]; 2];
        predict(&r.u, (mx * 8) as i32, (my * 8) as i32, cmv, 8, false, &mut pc[0]);
        predict(&r.v, (mx * 8) as i32, (my * 8) as i32, cmv, 8, false, &mut pc[1]);
        let mut rec = [[0u8; 64]; 6];
        for (b, out) in rec.iter_mut().enumerate() {
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
            let coded = quantise(&coef, 0, qp, qp / 2, &mut mb.levels[b], &mut deq);
            if coded {
                mb.cbp |= 1 << (5 - b);
                idct(&mut deq);
            }
            *out = std::array::from_fn(|k| (p(k) + deq[k]).clamp(0, 255) as u8);
        }
        if mv == (0, 0) && mb.cbp == 0 {
            mb = Mb::skip();
        }
        for (b, block) in rec.iter().enumerate() {
            store(recon, b, mx, my, |k| block[k]);
        }
        mb
    }
}

/// Write the samples of block `b` of macroblock `(mx, my)` into `frame`.
fn store(frame: &mut Frame, b: usize, mx: usize, my: usize, sample: impl Fn(usize) -> u8) {
    let (plane, x, y) = block_origin_mut(frame, b, mx, my);
    for k in 0..64 {
        plane.data[(y + k / 8) * plane.w + x + k % 8] = sample(k);
    }
}

/// One TCOEF event (§5.4.2 and Table 16): the table code and a sign, or the escape with the
/// last flag, a six-bit run and an eight-bit level.
fn write_event(w: &mut BitWriter, t: &Tcoef, last: bool, run: usize, level: i32) {
    if let Some((code, len)) = t.code(last, run, level.unsigned_abs()) {
        w.write_bits(code, len);
        w.write_bit(level < 0);
        return;
    }
    put(w, ESCAPE);
    w.write_bit(last);
    w.write_bits(run as u32, 6);
    w.write_bits((level as u32) & 0xFF, 8);
}

/// The TCOEF events of a block from scan position `start`.
fn write_coeffs(w: &mut BitWriter, levels: &[i16; 64], start: usize) {
    let ev = events(levels, start);
    let n = ev.len();
    for (k, &(run, level)) in ev.iter().enumerate() {
        write_event(w, tables::inter(), k + 1 == n, run, level);
    }
}

/// One `macroblock()` of an intra picture (`intra_picture`) or an inter picture (§5.3). H.263
/// has one coefficient table for intra and inter blocks, and no AC prediction.
pub(super) fn write_mb(w: &mut BitWriter, mb: &Mb, intra_picture: bool) {
    let cbpc = usize::from(mb.cbp & 3);
    let cbpy = usize::from(mb.cbp >> 2);
    let intra_blocks = |w: &mut BitWriter| {
        for (b, levels) in mb.levels.iter().enumerate() {
            w.write_bits(u32::from(mb.dc[b]), 8);
            if mb.cbp & (1 << (5 - b)) != 0 {
                write_coeffs(w, levels, 1);
            }
        }
    };
    match (mb.mode, intra_picture) {
        (Mode::Intra, true) => {
            put(w, MCBPC_INTRA[cbpc]);
            put(w, CBPY[cbpy]);
            intra_blocks(w);
        }
        (Mode::Skip, _) => w.write_bit(true),
        (Mode::Intra, false) => {
            w.write_bit(false);
            put(w, MCBPC_INTER[3 * 4 + cbpc]);
            put(w, CBPY[cbpy]);
            intra_blocks(w);
        }
        (Mode::Inter, _) => {
            w.write_bit(false);
            put(w, MCBPC_INTER[cbpc]);
            put(w, CBPY[cbpy ^ 15]);
            write_mvd(w, mb.mv.0 - mb.pred.0, 1);
            write_mvd(w, mb.mv.1 - mb.pred.1, 1);
            for (b, levels) in mb.levels.iter().enumerate() {
                if mb.cbp & (1 << (5 - b)) != 0 {
                    write_coeffs(w, levels, 0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intra_dc_avoids_the_forbidden_values() {
        for coef in -50..2200 {
            let dc = intra_dc(coef);
            assert!(dc != 0 && dc != 128 && dc != 255, "coef {coef} gave {dc}");
        }
    }

    #[test]
    fn intra_dc_follows_the_coefficient() {
        let d = |c| i32::from(intra_dc(c));
        assert!(d(200) < d(800) && d(800) < d(1500));
    }
}
