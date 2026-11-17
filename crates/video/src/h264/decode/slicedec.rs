//! Slice data decoding: macroblock parsing (CAVLC), motion vector derivation, and reconstruction.

use crate::h264::decode::cavlc;
use crate::h264::decode::deblock;
use crate::h264::decode::error::{Result, ensure, invalid};
use crate::h264::decode::inter::{self, McBlock, Weight};
use crate::h264::decode::intra::{self, Avail};
use crate::h264::decode::mbtypes::{CBP_ME, MbTypeInfo, Part, SubMbInfo, SubPart, b_mb_type, b_sub_mb_type, intra_mb_type, p_mb_type, p_sub_mb_type};
use crate::h264::decode::params::{Pps, Sps};
use crate::h264::decode::picture::{Frame, FrameRef, MbKind, MbState, Planes, RefPic, RefSrc};
use crate::h264::decode::slice::{SliceHeader, SliceType};
use crate::h264::decode::tables::{BLK4_XY, FIELD_SCAN4, FIELD_SCAN8, QPC_TABLE, RASTER_TO_BLK4, ZIGZAG4, ZIGZAG8};
use crate::h264::decode::transform::{self, Dequant, LevelScale};
use craft_media::bitstream::BitReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Per-slice data kept with the picture (deblocking, motion field export).
#[derive(Clone, Debug, Default)]
pub struct SliceInfo {
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    /// Unique ids of the pictures in RefPicList0/1.
    pub ref_ids: [Vec<u32>; 2],
    /// An MBAFF frame: the reference indices of field macroblocks index field lists.
    pub mbaff: bool,
}

impl SliceInfo {
    /// Unique id of the picture that reference index `ri` of list `l` of macroblock `st` refers
    /// to. A field macroblock of an MBAFF frame indexes a list of fields: entry `ri` is the field
    /// of list entry `ri / 2` of the macroblock's parity for even `ri`, of the other parity for odd
    /// `ri` (8.4.2.1).
    pub fn ref_id(&self, st: &MbState, l: usize, ri: i8) -> Option<u32> {
        let ri = usize::try_from(ri).ok()?;
        if self.mbaff && st.field {
            let key = *self.ref_ids[l].get(ri >> 1)?;
            Some((key & !3) | ((u32::from(st.bottom) ^ (ri as u32 & 1)) + 1))
        } else {
            self.ref_ids[l].get(ri).copied()
        }
    }
}

/// The picture under construction: private reconstruction buffers plus the shared [`Frame`] that
/// finished (deblocked) macroblock rows are published into.
pub struct PicState {
    pub planes: Planes,
    pub mbs: Vec<MbState>,
    pub slices: Vec<SliceInfo>,
    pub mb_w: usize,
    pub mb_h: usize,
    pub poc: i32,
    /// TopFieldOrderCnt and BottomFieldOrderCnt (an MBAFF field macroblock predicts from the
    /// field of its parity, 8.4.1.2.3, 8.4.2.3.1).
    pub field_poc: [i32; 2],
    pub decoded_mbs: usize,
    pub frame: FrameRef,
    /// Decoded macroblocks per MB row.
    row_count: Vec<u16>,
    /// Leading MB rows that are fully reconstructed / deblocked / published.
    rows_recon: usize,
    rows_deblocked: usize,
    rows_published: usize,
    /// Draft mode for a non-reference picture: rows are published without deblocking.
    pub skip_deblock: bool,
    /// An MBAFF frame: rows are deblocked and published in macroblock pair rows.
    pub mbaff: bool,
    /// Checked once per macroblock row: when set, slice decoding stops ([`Decoder::reset`]).
    ///
    /// [`Decoder::reset`]: crate::h264::decode::Decoder::reset
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Whether `decoded + add` macroblocks fit in a picture of `total`.
fn mbs_fit(decoded: usize, add: usize, total: usize) -> bool {
    decoded.checked_add(add).is_some_and(|n| n <= total)
}

impl PicState {
    pub fn new(frame: FrameRef) -> Self {
        let (mb_w, mb_h) = (frame.mb_w, frame.mb_h());
        PicState {
            planes: Planes::new(mb_w * 16, mb_h * 16),
            mbs: vec![MbState::default(); mb_w * mb_h],
            slices: Vec::new(),
            mb_w,
            mb_h,
            poc: frame.poc,
            field_poc: [frame.poc; 2],
            decoded_mbs: 0,
            frame,
            row_count: vec![0; mb_h],
            rows_recon: 0,
            rows_deblocked: 0,
            rows_published: 0,
            skip_deblock: false,
            mbaff: false,
            cancel: None,
        }
    }

    /// Reuse the buffers of a finished picture of the same size for a new frame.
    pub fn reset(&mut self, frame: FrameRef) {
        debug_assert_eq!((self.mb_w, self.mb_h), (frame.mb_w, frame.mb_h()));
        // start_mb() re-initialises each macroblock before use. Availability is reset, and the
        // kind too: a macroblock that a damaged slice leaves undecoded must not look like the
        // previous picture's (an intra kind would change the bS of an edge reaching it).
        for m in &mut self.mbs {
            m.slice_num = u32::MAX;
            m.kind = MbKind::None;
        }
        self.slices.clear();
        self.poc = frame.poc;
        self.field_poc = [frame.poc; 2];
        self.decoded_mbs = 0;
        self.frame = frame;
        self.row_count.fill(0);
        self.rows_recon = 0;
        self.rows_deblocked = 0;
        self.rows_published = 0;
        self.skip_deblock = false;
        self.mbaff = false;
    }

    /// Fail when decoding `n` more macroblocks would exceed the picture's own count: a conforming
    /// picture reconstructs each macroblock once, so overlapping slices (compact skip runs that
    /// repeat a whole picture of prediction work) are refused before the work is done.
    #[inline]
    pub fn reserve_mbs(&self, n: usize) -> Result<()> {
        ensure!(mbs_fit(self.decoded_mbs, n, self.mb_w * self.mb_h), "slice data overlaps macroblocks already decoded in this picture");
        Ok(())
    }

    /// Record a decoded macroblock and deblock / publish rows that became final. Fails when the
    /// picture was cancelled.
    #[inline]
    pub fn mb_done(&mut self, addr: usize) -> Result<()> {
        self.decoded_mbs += 1;
        let r = addr / self.mb_w;
        self.row_count[r] += 1;
        if self.row_count[r] as usize >= self.mb_w && r == self.rows_recon {
            return self.rows_done();
        }
        Ok(())
    }

    #[inline(never)]
    fn rows_done(&mut self) -> Result<()> {
        if self.cancelled() {
            return invalid("decoding cancelled");
        }
        while self.rows_recon < self.mb_h && self.row_count[self.rows_recon] as usize >= self.mb_w {
            self.rows_recon += 1;
        }
        self.flush_rows(false);
        Ok(())
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Deblock and publish rows. With `finished`, everything remaining is processed (also rows with
    /// missing macroblocks, so that waiting readers never block forever).
    pub fn flush_rows(&mut self, finished: bool) {
        // Deblocking row r must wait until row r + 1 is reconstructed (its intra prediction reads the
        // unfiltered bottom line of row r); row r is final once row r + 1 is deblocked.
        // In an MBAFF frame the unit is a pair row, deblocked in decoding order (top, then bottom
        // macroblock of each pair, 8.7); a field macroblock pair reaches up to its own height.
        let unit = if self.mbaff { 2 } else { 1 };
        let deblock_limit = if finished { self.mb_h } else { (self.rows_recon / unit).saturating_sub(1) * unit };
        while self.rows_deblocked < deblock_limit {
            let r = self.rows_deblocked;
            if !self.skip_deblock {
                let w = self.mb_w;
                for x in 0..w {
                    for y in r..(r + unit).min(self.mb_h) {
                        if self.mbaff {
                            deblock::deblock_mb_mbaff(self, y * w + x, w);
                        } else {
                            deblock::deblock_mb(self, y * w + x, w);
                        }
                    }
                }
            }
            self.rows_deblocked = (r + unit).min(self.mb_h);
        }
        let publish_limit = if finished { self.mb_h } else { self.rows_deblocked.saturating_sub(unit) };
        while self.rows_published < publish_limit {
            let r = self.rows_published;
            let slices = &self.slices;
            let ids = |st: &MbState, l: usize, ri: i8| slices.get(st.slice_num as usize).and_then(|s| s.ref_id(st, l, ri)).unwrap_or(u32::MAX);
            let row = Frame::make_row(&self.planes, r, &self.mbs, &ids);
            self.frame.publish(r, row);
            self.rows_published += 1;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WpMode {
    Default,
    Explicit,
    Implicit,
}

/// Prediction weight tables of a slice.
struct Weights {
    mode: WpMode,
    luma_denom: i32,
    chroma_denom: i32,
    /// [list][ref] -> (weight, offset) for luma and two chroma components.
    luma: [Vec<(i32, i32)>; 2],
    chroma: [Vec<[(i32, i32); 2]>; 2],
    /// Implicit weights (w0, w1) indexed by ref0 * n1 + ref1.
    implicit: Vec<(i32, i32)>,
    n1: usize,
    /// MBAFF: implicit weights of field macroblocks of each parity, over their field lists.
    implicit_field: [Vec<(i32, i32)>; 2],
    n1_field: [usize; 2],
}

/// Implicit bi-prediction weights (w0, w1) for every pair of `l0` and `l1`, indexed by
/// ref0 * l1.len() + ref1 (8.4.2.3.1, 8-201, 8-202).
fn implicit_weights(l0: &[RefPic], l1: &[RefPic], cur_poc: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::with_capacity(l0.len() * l1.len());
    for r0 in l0 {
        for r1 in l1 {
            let tb = cur_poc.saturating_sub(r0.poc()).clamp(-128, 127);
            let td = r1.poc().saturating_sub(r0.poc()).clamp(-128, 127);
            out.push(if td == 0 || r0.long_term || r1.long_term {
                (32, 32)
            } else {
                let tx = (16384 + (td / 2).abs()) / td;
                let dsf = ((tb * tx + 32) >> 6).clamp(-1024, 1023);
                if (dsf >> 2) < -64 || (dsf >> 2) > 128 { (32, 32) } else { (64 - (dsf >> 2), dsf >> 2) }
            });
        }
    }
    out
}

/// Temporal direct DistScaleFactor per entry of `l0` with `pic1` the first entry of list 1, and
/// whether the entry is long-term (8.4.1.2.3, 8-195 to 8-198).
fn direct_scale(l0: &[RefPic], pic1: &RefPic, cur_poc: i32) -> Vec<(i32, bool)> {
    l0.iter()
        .map(|r0| {
            let tb = cur_poc.saturating_sub(r0.poc()).clamp(-128, 127);
            let td = pic1.poc().saturating_sub(r0.poc()).clamp(-128, 127);
            if td == 0 || r0.long_term {
                (256, true)
            } else {
                let tx = (16384 + (td / 2).abs()) / td;
                (((tb * tx + 32) >> 6).clamp(-1024, 1023), false)
            }
        })
        .collect()
}

/// Current macroblock syntax (prediction part).
#[derive(Clone, Copy, Default)]
pub(crate) struct MbCur {
    pub info: MbTypeInfo,
    pub sub: [SubMbInfo; 4],
    /// rem_intra_pred_mode per block, or -1 when prev_intra_pred_mode_flag was set.
    pub rem_mode: [i8; 16],
    pub cbp: u8,
    pub ref_idx: [[i8; 4]; 2],
    /// mvd per (partition * 4 + sub-partition).
    pub mvd: [[[i16; 2]; 16]; 2],
}

/// Coefficient scratch for one macroblock.
pub(crate) struct Scratch {
    /// Luma: 16 4x4 blocks (raster block index * 16 + raster coefficient) or 4 8x8 blocks (b8 * 64 + raster).
    pub coef: [i32; 256],
    /// Chroma: [Cb, Cr][raster 2x2 block * 16 + raster coefficient].
    pub coef_c: [[i32; 64]; 2],
    /// Luma blocks with coefficients: raster 4x4 bits, or b8 bits in 8x8 mode.
    pub blk_nz: u16,
    /// Subset of `blk_nz` with a non-zero AC coefficient (the others take the DC-only transform).
    pub blk_ac: u16,
    /// Chroma blocks with coefficients (bit per raster 2x2 block).
    pub c_nz: [u8; 2],
    /// Subset of `c_nz` with a non-zero AC coefficient.
    pub c_ac: [u8; 2],
    pub pred: [[u8; 256]; 2],
    pub pred_c: [[[u8; 64]; 2]; 2],
}

impl Scratch {
    pub fn new() -> Box<Self> {
        Box::new(Scratch {
            coef: [0; 256],
            coef_c: [[0; 64]; 2],
            blk_nz: 0,
            blk_ac: 0,
            c_nz: [0; 2],
            c_ac: [0; 2],
            pred: [[0; 256]; 2],
            pred_c: [[[0; 64]; 2]; 2],
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Nb {
    pub avail: bool,
    pub ref_idx: i8,
    pub mv: [i16; 2],
}

const NB_NA: Nb = Nb { avail: false, ref_idx: -1, mv: [0, 0] };

/// The picture rows a macroblock works in. A frame macroblock uses the frame; a field macroblock
/// of an MBAFF frame uses its field: the frame lines of its parity, starting at line `off` with
/// `mul` = 2 times the frame stride, so that prediction and residual code address it like a
/// macroblock of a half-height picture.
#[derive(Clone, Copy)]
struct View {
    /// Row of the macroblock's top line in the view (luma samples).
    y: usize,
    /// First frame line of the view (the parity of a field macroblock).
    off: usize,
    /// Stride multiplier.
    mul: usize,
}

pub struct SliceDecoder<'a> {
    pub sh: &'a SliceHeader,
    pub pps: &'a Pps,
    pub sps: &'a Sps,
    pub pic: &'a mut PicState,
    pub refs: &'a [Vec<RefPic>; 2],
    pub ls: &'a LevelScale,
    pub slice_num: u32,
    pub mb_w: usize,
    pub mb_h: usize,
    pub qp: i32,
    pub chroma_qp_offset: [i32; 2],
    // current macroblock
    pub mb_addr: usize,
    pub mb_x: usize,
    pub mb_y: usize,
    /// Neighbouring macroblocks A (left), B (above), C (above right), D (above left) when available:
    /// the macroblocks covering luma locations (-1, 0), (0, -1), (16, -1) and (-1, -1) (6.4.11.1).
    pub nb: [Option<usize>; 4],
    /// MbaffFrameFlag (7.4.3): macroblocks come in pairs, each pair frame or field coded.
    pub mbaff: bool,
    /// The current macroblock is a field macroblock (every macroblock of a field picture, or one
    /// of a field macroblock pair).
    pub field: bool,
    /// Where the current macroblock's samples are in the picture buffers.
    view: View,
    /// MBAFF: RefPicList0/1 for field macroblocks of each parity, every frame or field pair split
    /// into its field of the macroblock's parity and then the other one (8.4.2.1).
    frefs: [[Vec<RefPic>; 2]; 2],
    pub mv_done: u16,
    pub cur: MbCur,
    pub s: Box<Scratch>,
    weights: Weights,
    /// Temporal direct: DistScaleFactor per refIdxL0, and whether pic0 is long-term.
    tdirect: Vec<(i32, bool)>,
    /// MBAFF: the same for field macroblocks of each parity over their field lists.
    tdirect_field: [Vec<(i32, bool)>; 2],
    /// CABAC: previous MB in decoding order had a non-zero mb_qp_delta.
    pub prev_qp_delta_nz: bool,
    /// Inverse scans of the slice (8.5.6, 8.5.7): zig-zag for frame macroblocks, field scan for
    /// field macroblocks.
    pub scan4: &'static [u8; 16],
    pub scan8: &'static [u8; 64],
}

/// Interpolate a bw x bh luma block at (px, py) with motion vector `mv` from reference `f` into `y`,
/// and the corresponding chroma blocks into `c` (stride `cs`).
#[inline]
fn predict_block(f: &RefPic, mv: [i16; 2], (px, py): (usize, usize), (bw, bh): (usize, usize), y: (&mut [u8], usize), c: [&mut [u8]; 2], cs: usize) {
    const LW: usize = 24;
    let mut win = [0u8; LW * 21];
    let ix = px as i32 + (mv[0] as i32 >> 2);
    let iy = py as i32 + (mv[1] as i32 >> 2);
    f.luma_window((ix - 2, iy - 2), bw + 5, bh + 5, &mut win, LW);
    inter::mc_luma_win(&win, LW, McBlock { fx: (mv[0] & 3) as u32, fy: (mv[1] & 3) as u32, bw, bh }, y.0, y.1);
    let cx = (px / 2) as i32 + (mv[0] as i32 >> 3);
    let cmv = mv[1].saturating_add(f.chroma_dy);
    let cy = (py / 2) as i32 + (cmv as i32 >> 3);
    let (cw, ch) = (bw / 2, bh / 2);
    let (fx, fy) = ((mv[0] & 7) as u32, (cmv & 7) as u32);
    for (comp, out) in c.into_iter().enumerate() {
        let mut w = [0u8; 16 * 9];
        f.chroma_window(comp, (cx, cy), cw + 1, ch + 1, &mut w, 16);
        inter::mc_chroma_win(&w, 16, McBlock { fx, fy, bw: cw, bh: ch }, out, cs);
    }
}

/// vertMvScale of the co-located block (8.4.1.2.1).
#[derive(Clone, Copy)]
enum VertMvScale {
    OneToOne,
    /// A field picture with a co-located frame.
    FrmToFld,
    /// A frame with a co-located field.
    FldToFrm,
}

/// Table 6-4: the macroblock holding the neighbouring luma (or chroma, `max_h` 8) location
/// (xN, yN) of a macroblock in an MBAFF frame, for a location outside the current macroblock
/// to the left, above left, above or above right. `frame` and `bottom` describe the current
/// macroblock; `pair(dx, dy)` says whether the pair at that offset from the current pair is
/// available and a frame macroblock pair. Returns the pair offset, whether the neighbour is
/// the pair's bottom macroblock (mbAddrX + 1), and yM.
fn mbaff_neighbour(
    xn: i32,
    yn: i32,
    max_h: i32,
    frame: bool,
    bottom: bool,
    pair: impl Fn(isize, isize) -> Option<bool>,
) -> Option<(isize, isize, bool, i32)> {
    let max_w = if max_h == 16 { 16 } else { 8 };
    if xn < 0 && yn < 0 {
        // D (A for the bottom frame macroblock)
        return match (frame, bottom) {
            (true, false) => pair(-1, -1).map(|_| (-1, -1, true, yn)),
            (true, true) => pair(-1, 0).map(|af| if af { (-1, 0, false, yn) } else { (-1, 0, true, (yn + max_h) >> 1) }),
            (false, false) => pair(-1, -1).map(|df| if df { (-1, -1, true, 2 * yn) } else { (-1, -1, false, yn) }),
            (false, true) => pair(-1, -1).map(|_| (-1, -1, true, yn)),
        };
    }
    if xn < 0 {
        if !(0..max_h).contains(&yn) {
            return None;
        }
        let af = pair(-1, 0)?;
        let (lower, ym) = match (frame, bottom, af) {
            (true, false, true) => (false, yn),
            (true, false, false) => (yn % 2 != 0, yn >> 1),
            (true, true, true) => (true, yn),
            (true, true, false) => (yn % 2 != 0, (yn + max_h) >> 1),
            (false, false, true) if yn < max_h / 2 => (false, yn << 1),
            (false, false, true) => (true, (yn << 1) - max_h),
            (false, true, true) if yn < max_h / 2 => (false, (yn << 1) + 1),
            (false, true, true) => (true, (yn << 1) + 1 - max_h),
            (false, false, false) => (false, yn),
            (false, true, false) => (true, yn),
        };
        return Some((-1, 0, lower, ym));
    }
    if yn >= 0 {
        return None;
    }
    // B (0..maxW - 1) or C (beyond)
    let dx = if xn < max_w { 0 } else { 1 };
    match (frame, bottom) {
        // B is the top macroblock of the current pair; C is not available
        (true, true) => (dx == 0).then_some((0, 0, false, yn)),
        (true, false) | (false, true) => pair(dx, -1).map(|_| (dx, -1, true, yn)),
        (false, false) => pair(dx, -1).map(|nf| if nf { (dx, -1, true, 2 * yn) } else { (dx, -1, false, yn) }),
    }
}

#[inline(always)]
fn median(a: i16, b: i16, c: i16) -> i16 {
    a.max(b).min(a.min(b).max(c))
}

impl<'a> SliceDecoder<'a> {
    pub fn new(
        sh: &'a SliceHeader,
        pps: &'a Pps,
        sps: &'a Sps,
        pic: &'a mut PicState,
        refs: &'a [Vec<RefPic>; 2],
        ls: &'a LevelScale,
    ) -> Result<Self> {
        let slice_num = pic.slices.len() as u32;
        let chroma_qp_offset = [pps.chroma_qp_index_offset, pps.second_chroma_qp_index_offset];
        pic.slices.push(SliceInfo {
            disable_deblocking_filter_idc: sh.disable_deblocking_filter_idc,
            alpha_offset: sh.slice_alpha_c0_offset_div2 * 2,
            beta_offset: sh.slice_beta_offset_div2 * 2,
            ref_ids: [refs[0].iter().map(|r| r.id()).collect(), refs[1].iter().map(|r| r.id()).collect()],
            mbaff: sps.mb_adaptive_frame_field && !sh.field_pic,
        });
        let mut weights = Self::build_weights(sh, pps, refs, pic.poc);
        let temporal = sh.slice_type == SliceType::B && !sh.direct_spatial_mv_pred;
        let tdirect = match refs[1].first() {
            Some(pic1) if temporal => direct_scale(&refs[0], pic1, pic.poc),
            _ => Vec::new(),
        };
        let (mb_w, mb_h) = (pic.mb_w, pic.mb_h);
        let mbaff = sps.mb_adaptive_frame_field && !sh.field_pic;
        pic.mbaff = mbaff;
        let frefs: [[Vec<RefPic>; 2]; 2] = if mbaff {
            std::array::from_fn(|cur| std::array::from_fn(|l| refs[l].iter().flat_map(|r| [r.field(cur, cur), r.field(cur ^ 1, cur)]).collect()))
        } else {
            Default::default()
        };
        let mut tdirect_field: [Vec<(i32, bool)>; 2] = Default::default();
        if mbaff {
            for p in 0..2 {
                let [l0, l1] = &frefs[p];
                if weights.mode == WpMode::Implicit {
                    weights.implicit_field[p] = implicit_weights(l0, l1, pic.field_poc[p]);
                    weights.n1_field[p] = l1.len();
                }
                if let (Some(pic1), true) = (l1.first(), temporal) {
                    tdirect_field[p] = direct_scale(l0, pic1, pic.field_poc[p]);
                }
            }
        }
        Ok(SliceDecoder {
            sh,
            pps,
            sps,
            pic,
            refs,
            ls,
            slice_num,
            mb_w,
            mb_h,
            qp: sh.qp(pps),
            chroma_qp_offset,
            mb_addr: 0,
            mb_x: 0,
            mb_y: 0,
            nb: [None; 4],
            mbaff,
            field: sh.field_pic,
            view: View { y: 0, off: 0, mul: 1 },
            frefs,
            mv_done: 0,
            cur: MbCur::default(),
            s: Scratch::new(),
            weights,
            tdirect,
            tdirect_field,
            prev_qp_delta_nz: false,
            scan4: if sh.field_pic { &FIELD_SCAN4 } else { &ZIGZAG4 },
            scan8: if sh.field_pic { &FIELD_SCAN8 } else { &ZIGZAG8 },
        })
    }

    fn build_weights(sh: &SliceHeader, pps: &Pps, refs: &[Vec<RefPic>; 2], cur_poc: i32) -> Weights {
        let mut w = Weights {
            mode: WpMode::Default,
            luma_denom: 0,
            chroma_denom: 0,
            luma: [Vec::new(), Vec::new()],
            chroma: [Vec::new(), Vec::new()],
            implicit: Vec::new(),
            n1: refs[1].len(),
            implicit_field: Default::default(),
            n1_field: [0; 2],
        };
        let explicit = (pps.weighted_pred && sh.slice_type.is_p()) || (pps.weighted_bipred_idc == 1 && sh.slice_type.is_b());
        if explicit {
            if let Some(t) = &sh.pred_weight_table {
                w.mode = WpMode::Explicit;
                w.luma_denom = t.luma_log2_denom as i32;
                w.chroma_denom = t.chroma_log2_denom as i32;
                for (l, entries) in [&t.l0, &t.l1].into_iter().enumerate() {
                    w.luma[l] = entries.iter().map(|e| (e.luma_weight, e.luma_offset)).collect();
                    w.chroma[l] =
                        entries.iter().map(|e| [(e.chroma_weight[0], e.chroma_offset[0]), (e.chroma_weight[1], e.chroma_offset[1])]).collect();
                }
            }
        } else if pps.weighted_bipred_idc == 2 && sh.slice_type.is_b() {
            w.mode = WpMode::Implicit;
            w.implicit = implicit_weights(&refs[0], &refs[1], cur_poc);
        }
        w
    }

    // ------------------------------------------------------------------------------------------
    // Neighbour helpers
    // ------------------------------------------------------------------------------------------

    /// Set up the current macroblock (raster address `addr`) and its neighbours. `field`: the
    /// macroblock is a field macroblock (ignored outside MBAFF frames, where it is the picture's
    /// structure).
    pub fn start_mb(&mut self, addr: usize, field: bool) {
        self.mb_addr = addr;
        self.mb_x = addr % self.mb_w;
        self.mb_y = addr / self.mb_w;
        let sn = self.slice_num;
        if self.mbaff {
            self.field = field;
            self.view = if field { View { y: (self.mb_y / 2) * 16, off: self.mb_y % 2, mul: 2 } } else { View { y: self.mb_y * 16, off: 0, mul: 1 } };
            let (scan4, scan8) = if field { (&FIELD_SCAN4, &FIELD_SCAN8) } else { (&ZIGZAG4, &ZIGZAG8) };
            self.scan4 = scan4;
            self.scan8 = scan8;
            self.pic.mbs[addr] = MbState { slice_num: sn, field, bottom: self.mb_y % 2 == 1, ..MbState::default() };
            let at = |s: &Self, x: i32, y: i32| s.locate_mbaff(x, y, 16).map(|(m, _)| m);
            self.nb = [at(self, -1, 0), at(self, 0, -1), at(self, 16, -1), at(self, -1, -1)];
        } else {
            self.view = View { y: self.mb_y * 16, off: 0, mul: 1 };
            let mbs = &self.pic.mbs;
            let w = self.mb_w;
            let (x, y) = (self.mb_x, self.mb_y);
            let ok = |a: usize| if mbs[a].slice_num == sn { Some(a) } else { None };
            self.nb = [
                if x > 0 { ok(addr - 1) } else { None },
                if y > 0 { ok(addr - w) } else { None },
                if y > 0 && x + 1 < w { ok(addr - w + 1) } else { None },
                if y > 0 && x > 0 { ok(addr - w - 1) } else { None },
            ];
            self.pic.mbs[addr] = MbState { slice_num: sn, field: self.field, ..MbState::default() };
        }
        self.mv_done = 0;
    }

    /// The macroblock covering luma location (xn, yn) relative to the current macroblock's
    /// upper-left sample, and the location (xW, yW) in it (6.4.12). The current macroblock for a
    /// location inside it.
    #[inline]
    pub(crate) fn locate(&self, xn: i32, yn: i32) -> Option<(usize, usize, usize)> {
        self.locate_in(xn, yn, 16, 16)
    }

    /// [`Self::locate`] for a chroma location (4:2:0, 8x8 samples per macroblock).
    #[inline]
    pub(crate) fn locate_c(&self, xn: i32, yn: i32) -> Option<(usize, usize, usize)> {
        self.locate_in(xn, yn, 8, 8)
    }

    #[inline]
    fn locate_in(&self, xn: i32, yn: i32, max_w: i32, max_h: i32) -> Option<(usize, usize, usize)> {
        if (0..max_w).contains(&xn) && (0..max_h).contains(&yn) {
            return Some((self.mb_addr, xn as usize, yn as usize));
        }
        if yn >= max_h || (xn >= max_w && yn >= 0) {
            return None;
        }
        let xw = xn.rem_euclid(max_w) as usize;
        if !self.mbaff {
            // 6.4.12.1
            let m = if xn < 0 {
                if yn < 0 { self.nb[3] } else { self.nb[0] }
            } else if xn < max_w {
                self.nb[1]
            } else {
                self.nb[2]
            };
            return m.map(|m| (m, xw, yn.rem_euclid(max_h) as usize));
        }
        let (m, ym) = self.locate_mbaff(xn, yn, max_h)?;
        Some((m, xw, ym.rem_euclid(max_h) as usize))
    }

    /// Table 6-4 (6.4.12.2): mbAddrN as a raster address and yM, for a location outside the current
    /// macroblock of an MBAFF frame (xn: < 0, inside, or > maxW - 1 of the macroblock; yn < maxH).
    fn locate_mbaff(&self, xn: i32, yn: i32, max_h: i32) -> Option<(usize, i32)> {
        let w = self.mb_w;
        let (px, pr) = (self.mb_x, self.mb_y / 2);
        // the top macroblock of an available pair (6.4.10) and whether it is a frame macroblock pair
        let top_of = |dx: isize, dy: isize| -> Option<usize> {
            let x = px.checked_add_signed(dx).filter(|&x| x < w)?;
            let top = 2 * pr.checked_add_signed(dy)? * w + x;
            self.pic.mbs.get(top).filter(|st| st.slice_num == self.slice_num).map(|_| top)
        };
        let pair = |dx: isize, dy: isize| top_of(dx, dy).and_then(|t| self.pic.mbs.get(t)).map(|st| !st.field);
        let (dx, dy, lower, ym) = mbaff_neighbour(xn, yn, max_h, !self.field, self.mb_y % 2 == 1, pair)?;
        Some((top_of(dx, dy)? + if lower { w } else { 0 }, ym))
    }

    #[inline]
    pub fn mb(&self) -> &MbState {
        &self.pic.mbs[self.mb_addr]
    }
    #[inline]
    pub fn mb_mut(&mut self) -> &mut MbState {
        &mut self.pic.mbs[self.mb_addr]
    }

    /// Neighbour MB usable for intra prediction (constrained_intra_pred aware).
    fn intra_nb(&self, n: usize) -> bool {
        match self.nb[n] {
            None => false,
            Some(a) => !self.pps.constrained_intra_pred || self.pic.mbs[a].kind.is_intra(),
        }
    }

    fn chroma_qp(&self, qp: i32, c: usize) -> i32 {
        QPC_TABLE[(qp + self.chroma_qp_offset[c]).clamp(0, 51) as usize] as i32
    }

    /// nC for a luma 4x4 block (raster index) - 9.2.1.
    fn nc_luma(&self, raster: usize) -> i32 {
        let (bx, by) = (raster & 3, raster >> 2);
        let cur = self.mb();
        let nb = |x: i32, y: i32| self.locate(x, y).map(|(m, xw, yw)| self.pic.mbs[m].nnz[(yw / 4) * 4 + xw / 4]);
        let a = if bx > 0 { Some(cur.nnz[raster - 1]) } else { nb(-1, by as i32 * 4) };
        let b = if by > 0 { Some(cur.nnz[raster - 4]) } else { nb(bx as i32 * 4, -1) };
        match (a, b) {
            (Some(a), Some(b)) => (a as i32 + b as i32 + 1) >> 1,
            (Some(a), None) => a as i32,
            (None, Some(b)) => b as i32,
            _ => 0,
        }
    }

    fn nc_chroma(&self, c: usize, raster: usize) -> i32 {
        let (bx, by) = (raster & 1, raster >> 1);
        let cur = self.mb();
        let nb = |x: i32, y: i32| self.locate_c(x, y).map(|(m, xw, yw)| self.pic.mbs[m].nnz_c[c][(yw / 4) * 2 + xw / 4]);
        let a = if bx > 0 { Some(cur.nnz_c[c][raster - 1]) } else { nb(-1, by as i32 * 4) };
        let b = if by > 0 { Some(cur.nnz_c[c][raster - 2]) } else { nb(bx as i32 * 4, -1) };
        match (a, b) {
            (Some(a), Some(b)) => (a as i32 + b as i32 + 1) >> 1,
            (Some(a), None) => a as i32,
            (None, Some(b)) => b as i32,
            _ => 0,
        }
    }

    /// Motion data of the 4x4 block covering luma location (xn, yn) relative to the current
    /// macroblock (8.4.1.3.2), scaled between field and frame macroblocks (8-217 to 8-220). A block
    /// of the current macroblock counts only once its motion is set.
    pub(crate) fn nb_motion(&self, list: usize, xn: i32, yn: i32) -> Nb {
        let Some((m, xw, yw)) = self.locate(xn, yn) else { return NB_NA };
        let (bx, by) = (xw / 4, yw / 4);
        if m == self.mb_addr && self.mv_done & (1 << (by * 4 + bx)) == 0 {
            return NB_NA;
        }
        let st = &self.pic.mbs[m];
        let r = st.ref_idx[list][(by >> 1) * 2 + (bx >> 1)];
        if r < 0 {
            return Nb { avail: true, ref_idx: -1, mv: [0, 0] };
        }
        let mv = st.mv[list][by * 4 + bx];
        if st.field == self.field {
            return Nb { avail: true, ref_idx: r, mv };
        }
        // only in MBAFF frames; `/` truncates toward zero as the standard's division does
        if self.field {
            Nb { avail: true, ref_idx: r.saturating_mul(2), mv: [mv[0], mv[1] / 2] }
        } else {
            Nb { avail: true, ref_idx: r / 2, mv: [mv[0], mv[1].saturating_mul(2)] }
        }
    }

    /// Returns (A, B, C) neighbour motion for a partition at (x, y) of width w (4x4 units), with C
    /// replaced by D when unavailable.
    fn nb_abc(&self, list: usize, x: usize, y: usize, w: usize) -> (Nb, Nb, Nb) {
        let (x, y, w) = (x as i32 * 4, y as i32 * 4, w as i32 * 4);
        let a = self.nb_motion(list, x - 1, y);
        let b = self.nb_motion(list, x, y - 1);
        let mut c = self.nb_motion(list, x + w, y - 1);
        if !c.avail {
            c = self.nb_motion(list, x - 1, y - 1);
        }
        (a, b, c)
    }

    /// Luma motion vector prediction (8.4.1.3). `shape`: 1 = 16x8, 2 = 8x16, 0 = other.
    fn mvp(&self, list: usize, (x, y): (usize, usize), w: usize, ref_idx: i8, shape: u8, part: usize) -> [i16; 2] {
        let (a, b, c) = self.nb_abc(list, x, y, w);
        if shape == 1 {
            if part == 0 && b.ref_idx == ref_idx {
                return b.mv;
            }
            if part == 1 && a.ref_idx == ref_idx {
                return a.mv;
            }
        } else if shape == 2 {
            if part == 0 && a.ref_idx == ref_idx {
                return a.mv;
            }
            if part == 1 && c.ref_idx == ref_idx {
                return c.mv;
            }
        }
        Self::median_pred(a, b, c, ref_idx)
    }

    fn median_pred(a: Nb, mut b: Nb, mut c: Nb, ref_idx: i8) -> [i16; 2] {
        if !b.avail && !c.avail && a.avail {
            b = a;
            c = a;
        }
        let ma = a.ref_idx == ref_idx;
        let mb = b.ref_idx == ref_idx;
        let mc = c.ref_idx == ref_idx;
        match (ma, mb, mc) {
            (true, false, false) => a.mv,
            (false, true, false) => b.mv,
            (false, false, true) => c.mv,
            _ => [median(a.mv[0], b.mv[0], c.mv[0]), median(a.mv[1], b.mv[1], c.mv[1])],
        }
    }

    /// Store motion for a rectangle (4x4 units) of the current MB and mark it decoded.
    fn set_motion(&mut self, x: usize, y: usize, w: usize, h: usize, refs: [i8; 2], mvs: [[i16; 2]; 2]) {
        let st = &mut self.pic.mbs[self.mb_addr];
        for yy in y..y + h {
            for xx in x..x + w {
                let r = yy * 4 + xx;
                for l in 0..2 {
                    st.mv[l][r] = if refs[l] >= 0 { mvs[l] } else { [0, 0] };
                }
                self.mv_done |= 1 << r;
            }
        }
        for l in 0..2 {
            // ref_idx is per 8x8; rectangles never straddle 8x8 blocks with different refs
            for yy in (y..y + h).step_by(1) {
                for xx in x..x + w {
                    st.ref_idx[l][(yy >> 1) * 2 + (xx >> 1)] = refs[l];
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Direct prediction (8.4.1.2)
    // ------------------------------------------------------------------------------------------

    /// Co-located motion for 4x4 block `raster` of the current MB (8.4.1.2.1): mvCol (unscaled),
    /// refIdxCol, the key that RefPicList0 must contain for MapColToList0 (8.4.1.2.3), and
    /// vertMvScale.
    fn colocated(&self, raster: usize) -> ([i16; 2], i8, u32, VertMvScale) {
        const NONE: ([i16; 2], i8, u32, VertMvScale) = ([0, 0], -1, u32::MAX, VertMvScale::OneToOne);
        let Some(col) = self.refs[1].first() else { return NONE };
        let (bx, by) = (raster & 3, raster >> 2);
        // (xCol, yCol): with direct_8x8_inference_flag the corner 4x4 block of each 8x8 quadrant
        let (x_col, y_col) =
            if self.sps.direct_8x8_inference { (if bx >= 2 { 12 } else { 0 }, if by >= 2 { 12 } else { 0 }) } else { (bx * 4, by * 4) };
        // colPic, the macroblock row of mbAddrCol in it, yM and vertMvScale (Tables 8-6, 8-8)
        let (colf, row, y_m, scale) = match &col.src {
            RefSrc::Whole(f) if self.mbaff => {
                // An MBAFF frame and a frame as colPic (Table 8-8, AFRM rows): the co-located
                // macroblock depends on whether it is a field macroblock (fieldDecodingFlagX).
                if f.mb_w != self.mb_w || self.mb_y >= f.mb_h() {
                    return NONE;
                }
                let col_field = f.row(self.mb_y).field.get(self.mb_x).copied().unwrap_or(false);
                let pair = self.mb_y & !1;
                match (self.field, col_field) {
                    (false, false) | (true, true) => (f, self.mb_y, y_col, VertMvScale::OneToOne),
                    (false, true) => {
                        // mbAddrCol6: the macroblock of the field closer to the current picture
                        let cur = self.pic.poc;
                        let top = col.field_poc[0].saturating_sub(cur).unsigned_abs() < col.field_poc[1].saturating_sub(cur).unsigned_abs();
                        (f, pair + usize::from(!top), 8 * (self.mb_y % 2) + 4 * (y_col / 8), VertMvScale::FldToFrm)
                    }
                    // mbAddrCol7
                    (true, false) => (f, pair + y_col / 8, (2 * y_col) % 16, VertMvScale::FrmToFld),
                }
            }
            RefSrc::Pair(t, b) if self.mbaff && self.field => {
                // a field macroblock and a field pair: the field of the macroblock's parity (Table 8-6),
                // mbAddrCol5 (Table 8-8)
                (if self.mb_y.is_multiple_of(2) { t } else { b }, self.mb_y / 2, y_col, VertMvScale::OneToOne)
            }
            RefSrc::Whole(f) => (f, self.mb_y, y_col, VertMvScale::OneToOne),
            RefSrc::FieldOf(f, _) => {
                // a field and a frame as colPic: mbAddrX is the top macroblock of the pair; a field
                // macroblock pair of an MBAFF frame gives the macroblock of the current parity
                // (mbAddrCol3), anything else mbAddrCol1 / mbAddrCol2 (Table 8-8)
                let top = 2 * self.mb_y;
                let col_field = top < f.mb_h() && f.row(top).field.get(self.mb_x).copied().unwrap_or(false);
                if col_field {
                    (f, top + usize::from(self.sh.bottom_field), y_col, VertMvScale::OneToOne)
                } else {
                    (f, top + y_col / 8, (2 * y_col) % 16, VertMvScale::FrmToFld)
                }
            }
            RefSrc::Pair(t, b) => {
                let cur = self.pic.poc;
                let top = (col.field_poc[0].saturating_sub(cur)).unsigned_abs() < (col.field_poc[1].saturating_sub(cur)).unsigned_abs();
                (if top { t } else { b }, self.mb_y / 2, 8 * (self.mb_y % 2) + 4 * (y_col / 8), VertMvScale::FldToFrm)
            }
        };
        if colf.mb_w != self.mb_w || row >= colf.mb_h() {
            return NONE;
        }
        let m = colf.row(row);
        let mb = self.mb_x;
        if m.intra[mb] {
            return NONE;
        }
        let b8 = mb * 4 + (y_m / 8) * 2 + x_col / 8;
        let l = if m.ref_idx[0][b8] >= 0 { 0 } else { 1 };
        let key = m.ref_id[l][b8];
        // refPicCol as the current macroblock's RefPicList0 holds it (MapColToList0, 8.4.1.2.3):
        // the field of the same parity as the current field or field macroblock, or the frame or
        // field pair containing the field. A field macroblock's list alternates parities, so the
        // field of the other parity is found at (refIdxL0Frm << 1) + 1 as the text requires.
        let parity = if self.mbaff { self.mb_y % 2 } else { usize::from(self.sh.bottom_field) };
        let key = match scale {
            VertMvScale::OneToOne => key,
            VertMvScale::FrmToFld => (key & !3) | (parity as u32 + 1),
            VertMvScale::FldToFrm => key & !3,
        };
        (m.mv[l][mb * 16 + (y_m / 4) * 4 + x_col / 4], m.ref_idx[l][b8], key, scale)
    }

    /// Spatial direct: MB-level reference indices, predicted mvs and directZeroPrediction (8.4.1.2.2).
    fn spatial_direct_base(&self) -> ([i8; 2], [[i16; 2]; 2], bool) {
        let mut refs = [-1i8; 2];
        let mut nbs = [(NB_NA, NB_NA, NB_NA); 2];
        let min_pos = |x: i8, y: i8| if x >= 0 && y >= 0 { x.min(y) } else { x.max(y) };
        for (l, nb) in nbs.iter_mut().enumerate() {
            *nb = self.nb_abc(l, 0, 0, 4);
            refs[l] = min_pos(nb.0.ref_idx, min_pos(nb.1.ref_idx, nb.2.ref_idx));
        }
        let mut mvs = [[0i16; 2]; 2];
        if refs[0] < 0 && refs[1] < 0 {
            return ([0, 0], mvs, true);
        }
        for l in 0..2 {
            if refs[l] >= 0 {
                let (a, b, c) = nbs[l];
                mvs[l] = Self::median_pred(a, b, c, refs[l]);
            }
        }
        (refs, mvs, false)
    }

    /// Temporal direct mvs (8.4.1.2.3) from the co-located mv and `DistScaleFactor` (clamped
    /// to ±1024 in `SliceDecoder`): mvL0 = clip_mv((mvCol · dsf + 128) >> 8), mvL1 =
    /// clip_mv(mvL0 − mvCol), both clipped to the spec's motion-vector range −2^15..2^15−1.
    /// The i32 maths cannot overflow; a truncating `as i16` used to wrap mvL0 past the range,
    /// after which the mvL1 subtraction overflowed a debug build. mvL1 is scaled from the
    /// unclamped mvL0 with only the results clipped: dsf 512, mvCol 32767 keeps mvL1 at 32767.
    fn temporal_direct_mvs(dsf: i32, mv_col: [i16; 2]) -> ([i16; 2], [i16; 2]) {
        let clamp = |v: i32| v.clamp(-(1 << 15), (1 << 15) - 1) as i16;
        let m0 = mv_col.map(|c| (dsf * i32::from(c) + 128) >> 8);
        (m0.map(clamp), [clamp(m0[0] - i32::from(mv_col[0])), clamp(m0[1] - i32::from(mv_col[1]))])
    }

    /// Derive direct-mode motion for the 8x8 blocks in `b8_mask` of the current MB.
    fn derive_direct(&mut self, b8_mask: u8) -> Result<()> {
        let spatial = self.sh.direct_spatial_mv_pred;
        let (base_refs, base_mvs, direct_zero) = if spatial { self.spatial_direct_base() } else { ([0, 0], [[0, 0]; 2], false) };
        let col_short_term = self.refs[1].first().map(|r| !r.long_term).unwrap_or(false);
        for b8 in 0..4 {
            if b8_mask & (1 << b8) == 0 {
                continue;
            }
            let (x8, y8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            let unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
            for sub in (0..4).filter(|s| unit == 1 || *s == 0) {
                let (x, y) = (x8 + (sub & 1), y8 + (sub >> 1));
                let raster = y * 4 + x;
                let (mv_col, ref_col, id_col, scale) = self.colocated(raster);
                let (refs, mvs) = if spatial {
                    let col_zero = col_short_term && ref_col == 0 && (-1..=1).contains(&mv_col[0]) && (-1..=1).contains(&mv_col[1]);
                    let mut mvs = base_mvs;
                    for l in 0..2 {
                        if direct_zero || base_refs[l] < 0 || (base_refs[l] == 0 && col_zero) {
                            mvs[l] = [0, 0];
                        }
                    }
                    (base_refs, mvs)
                } else {
                    let ref0 = if ref_col < 0 {
                        0
                    } else {
                        match self.cur_refs()[0].iter().position(|r| r.id() == id_col) {
                            Some(i) => i as i8,
                            None => 0,
                        }
                    };
                    // (8-193), (8-194)
                    let mv_col = match scale {
                        VertMvScale::OneToOne => mv_col,
                        VertMvScale::FrmToFld => [mv_col[0], mv_col[1] / 2],
                        VertMvScale::FldToFrm => [mv_col[0], mv_col[1].saturating_mul(2)],
                    };
                    let table = if self.mbaff && self.field { &self.tdirect_field[self.mb_y % 2] } else { &self.tdirect };
                    let (dsf, ltr) = table.get(ref0 as usize).copied().unwrap_or((256, true));
                    let (mv0, mv1) = if ltr { (mv_col, [0, 0]) } else { Self::temporal_direct_mvs(dsf, mv_col) };
                    ([ref0, 0], [mv0, mv1])
                };
                self.set_motion(x, y, unit, unit, refs, mvs);
            }
        }
        let st = &mut self.pic.mbs[self.mb_addr];
        st.direct8x8 |= b8_mask;
        Ok(())
    }

    // ------------------------------------------------------------------------------------------
    // Motion vector derivation for a parsed inter macroblock
    // ------------------------------------------------------------------------------------------

    /// P_Skip motion (8.4.1.1).
    pub fn derive_p_skip(&mut self) {
        let a = self.nb_motion(0, -1, 0);
        let b = self.nb_motion(0, 0, -1);
        let mv = if self.nb[0].is_none() || self.nb[1].is_none() || (a.ref_idx == 0 && a.mv == [0, 0]) || (b.ref_idx == 0 && b.mv == [0, 0]) {
            [0, 0]
        } else {
            self.mvp(0, (0, 0), 4, 0, 0, 0)
        };
        self.set_motion(0, 0, 4, 4, [0, -1], [mv, [0, 0]]);
    }

    /// Derive mvs of the current (non-skip) inter MB from `self.cur` (ref_idx, mvd).
    pub fn derive_inter_motion(&mut self) -> Result<()> {
        let info = self.cur.info;
        if info.kind == MbKind::BDirect16x16 || info.kind == MbKind::BSkip {
            return self.derive_direct(0xf);
        }
        let n = [self.cur_refs()[0].len(), self.cur_refs()[1].len()];
        if info.part != Part::P8x8 {
            let shape = match info.part {
                Part::P16x8 => 1,
                Part::P8x16 => 2,
                _ => 0,
            };
            for p in 0..info.part.num_parts() {
                let (x, y, w, h) = info.part.rect(p);
                let mut refs = [-1i8; 2];
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if info.pred[p] & (1 << l) != 0 {
                        let r = self.cur.ref_idx[l][p];
                        ensure!((r as usize) < n[l], "ref_idx {r} out of range (list {l}, size {})", n[l]);
                        let mvp = self.mvp(l, (x, y), w, r, shape, p);
                        let d = self.cur.mvd[l][p * 4];
                        refs[l] = r;
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
            return Ok(());
        }
        for p in 0..4 {
            let sub = self.cur.sub[p];
            let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
            if sub.direct {
                self.derive_direct(1 << p)?;
                continue;
            }
            let mut refs = [-1i8; 2];
            for (l, r) in refs.iter_mut().enumerate() {
                if sub.pred & (1 << l) != 0 {
                    let v = self.cur.ref_idx[l][p];
                    ensure!((v as usize) < n[l], "ref_idx {v} out of range (list {l}, size {})", n[l]);
                    *r = v;
                }
            }
            for j in 0..sub.shape.num_parts() {
                let (sx, sy, w, h) = sub.shape.rect(j);
                let (x, y) = (x8 + sx, y8 + sy);
                let mut mvs = [[0i16; 2]; 2];
                for l in 0..2 {
                    if refs[l] >= 0 {
                        let mvp = self.mvp(l, (x, y), w, refs[l], 0, 0);
                        let d = self.cur.mvd[l][p * 4 + j];
                        mvs[l] = [mvp[0].wrapping_add(d[0]), mvp[1].wrapping_add(d[1])];
                    }
                }
                self.set_motion(x, y, w, h, refs, mvs);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------
    // Inter prediction
    // ------------------------------------------------------------------------------------------

    fn weight_for(&self, refs: [i8; 2], comp: usize) -> Weight {
        let w = &self.weights;
        // an MBAFF field macroblock: field reference indices (8.4.2.3)
        let field_mb = self.mbaff && self.field;
        match w.mode {
            WpMode::Default => Weight::Default,
            WpMode::Implicit => {
                if refs[0] >= 0 && refs[1] >= 0 {
                    let (r0, r1) = (refs[0] as usize, refs[1] as usize);
                    let pair = if field_mb {
                        let p = self.mb_y % 2;
                        w.implicit_field[p].get(r0 * w.n1_field[p] + r1)
                    } else {
                        w.implicit.get(r0 * w.n1 + r1)
                    };
                    // the reference indices were range-checked when parsed
                    let (w0, w1) = pair.copied().unwrap_or((32, 32));
                    Weight::Weighted { log_wd: 5, w0, w1, o0: 0, o1: 0 }
                } else {
                    Weight::Default
                }
            }
            WpMode::Explicit => {
                let get = |l: usize| -> (i32, i32) {
                    if refs[l] < 0 {
                        return (0, 0);
                    }
                    // refIdxL0WP = refIdxL0 >> 1 for a field macroblock (8-284)
                    let r = if field_mb { refs[l] as usize >> 1 } else { refs[l] as usize };
                    if comp == 0 {
                        w.luma[l].get(r).copied().unwrap_or((1 << w.luma_denom, 0))
                    } else {
                        w.chroma[l].get(r).map(|c| c[comp - 1]).unwrap_or((1 << w.chroma_denom, 0))
                    }
                };
                let (w0, o0) = get(0);
                let (w1, o1) = get(1);
                let log_wd = if comp == 0 { w.luma_denom } else { w.chroma_denom };
                Weight::Weighted { log_wd, w0, w1, o0, o1 }
            }
        }
    }

    /// Motion-compensated prediction of a rectangle (4x4 units) written into the picture.
    fn mc_rect(&mut self, x4: usize, y4: usize, w4: usize, h4: usize) {
        let st = &self.pic.mbs[self.mb_addr];
        let r = y4 * 4 + x4;
        let b8 = (y4 >> 1) * 2 + (x4 >> 1);
        let refs = [st.ref_idx[0][b8], st.ref_idx[1][b8]];
        let mvs = [st.mv[0][r], st.mv[1][r]];
        let (bw, bh) = (w4 * 4, h4 * 4);
        let wts = [self.weight_for(refs, 0), self.weight_for(refs, 1), self.weight_for(refs, 2)];
        let px = self.mb_x * 16 + x4 * 4;
        let py = self.view.y + y4 * 4;
        // the field borrow (not `cur_refs`) leaves the picture buffers free to write
        let lists: &[Vec<RefPic>; 2] = if self.mbaff && self.field { &self.frefs[self.mb_y % 2] } else { self.refs };
        let use0 = refs[0] >= 0 && lists[0].get(refs[0] as usize).is_some();
        let use1 = refs[1] >= 0 && lists[1].get(refs[1] as usize).is_some();
        let (yo, ys) = self.luma_view();
        let (co, cs) = self.chroma_view();
        let (cx, cy) = (px / 2, py / 2);
        if use0 != use1 && wts.iter().all(|w| *w == Weight::Default) {
            // single-list unweighted prediction: interpolate straight into the picture
            let l = if use0 { 0 } else { 1 };
            let f = &lists[l][refs[l] as usize];
            let planes = &mut self.pic.planes;
            predict_block(
                f,
                mvs[l],
                (px, py),
                (bw, bh),
                (&mut planes.y[yo + py * ys + px..], ys),
                [&mut planes.cb[co + cy * cs + cx..], &mut planes.cr[co + cy * cs + cx..]],
                cs,
            );
            return;
        }
        let s = &mut *self.s;
        for l in 0..2 {
            if refs[l] < 0 {
                continue;
            }
            let Some(rp) = lists[l].get(refs[l] as usize) else { continue };
            let [pc0, pc1] = &mut s.pred_c[l];
            predict_block(rp, mvs[l], (px, py), (bw, bh), (&mut s.pred[l][..], 16), [&mut pc0[..], &mut pc1[..]], 8);
        }
        inter::weighted_store(
            &mut self.pic.planes.y[yo + py * ys + px..],
            ys,
            if use0 { Some(&s.pred[0][..]) } else { None },
            if use1 { Some(&s.pred[1][..]) } else { None },
            16,
            (bw, bh),
            wts[0],
        );
        for c in 0..2 {
            let wc = wts[c + 1];
            let a = if use0 { Some(&s.pred_c[0][c][..]) } else { None };
            let b = if use1 { Some(&s.pred_c[1][c][..]) } else { None };
            let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
            inter::weighted_store(&mut plane[co + cy * cs + cx..], cs, a, b, 8, (bw / 2, bh / 2), wc);
        }
    }

    /// RefPicList0/1 as the current macroblock indexes them: the slice's lists, or for a field
    /// macroblock of an MBAFF frame the field lists of its parity (8.4.2.1).
    #[inline]
    fn cur_refs(&self) -> &[Vec<RefPic>; 2] {
        if self.mbaff && self.field { &self.frefs[self.mb_y % 2] } else { self.refs }
    }

    /// All 16 blocks of the current MB share references and motion vectors.
    fn uniform_motion(&self) -> bool {
        let st = self.mb();
        (0..2).all(|l| {
            let r = st.ref_idx[l];
            r[1] == r[0] && r[2] == r[0] && r[3] == r[0] && st.mv[l].iter().all(|m| *m == st.mv[l][0])
        })
    }

    /// Inter prediction for the whole current MB, using the stored motion.
    pub fn predict_inter(&mut self) {
        let info = self.cur.info;
        let direct_unit = if self.sps.direct_8x8_inference { 2 } else { 1 };
        match info.kind {
            MbKind::PSkip => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 if self.uniform_motion() => self.mc_rect(0, 0, 4, 4),
            MbKind::BSkip | MbKind::BDirect16x16 => {
                for y in (0..4).step_by(direct_unit) {
                    for x in (0..4).step_by(direct_unit) {
                        self.mc_rect(x, y, direct_unit, direct_unit);
                    }
                }
            }
            _ => {
                if info.part != Part::P8x8 {
                    for p in 0..info.part.num_parts() {
                        let (x, y, w, h) = info.part.rect(p);
                        self.mc_rect(x, y, w, h);
                    }
                } else {
                    for p in 0..4 {
                        let sub = self.cur.sub[p];
                        let (x8, y8) = ((p & 1) * 2, (p >> 1) * 2);
                        if sub.direct {
                            for y in (0..2).step_by(direct_unit) {
                                for x in (0..2).step_by(direct_unit) {
                                    self.mc_rect(x8 + x, y8 + y, direct_unit, direct_unit);
                                }
                            }
                        } else {
                            for j in 0..sub.shape.num_parts() {
                                let (sx, sy, w, h) = sub.shape.rect(j);
                                self.mc_rect(x8 + sx, y8 + sy, w, h);
                            }
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Intra prediction mode derivation (8.3.1.1 / 8.3.2.1)
    // ------------------------------------------------------------------------------------------

    /// Intra4x4PredMode / Intra8x8PredMode of the neighbouring block covering luma location
    /// (xn, yn) outside the current macroblock, or None when dcPredModePredictedFlag must be set.
    fn intra_mode_nb(&self, xn: i32, yn: i32) -> Option<u8> {
        let (m, xw, yw) = self.locate(xn, yn)?;
        let (bx, by) = (xw / 4, yw / 4);
        let st = &self.pic.mbs[m];
        if self.pps.constrained_intra_pred && !st.kind.is_intra() {
            return None;
        }
        Some(match st.kind {
            MbKind::I4x4 | MbKind::I8x8 => st.intra_modes[by * 4 + bx],
            _ => 2,
        })
    }

    /// Resolve Intra4x4PredMode / Intra8x8PredMode for the current MB from `self.cur.rem_mode`.
    pub fn derive_intra_modes(&mut self, is8x8: bool) {
        let step = if is8x8 { 2 } else { 1 };
        let nblk = if is8x8 { 4 } else { 16 };
        for i in 0..nblk {
            let (bx, by) = if is8x8 {
                ((i & 1) * 2, (i >> 1) * 2)
            } else {
                let (x, y) = BLK4_XY[i];
                (x as usize, y as usize)
            };
            let a = if bx > 0 { Some(self.mb().intra_modes[by * 4 + bx - 1]) } else { self.intra_mode_nb(-1, by as i32 * 4) };
            let b = if by > 0 { Some(self.mb().intra_modes[(by - 1) * 4 + bx]) } else { self.intra_mode_nb(bx as i32 * 4, -1) };
            let pred = match (a, b) {
                (Some(a), Some(b)) => a.min(b),
                _ => 2,
            };
            let rem = self.cur.rem_mode[i];
            let mode = if rem < 0 {
                pred
            } else if (rem as u8) < pred {
                rem as u8
            } else {
                rem as u8 + 1
            };
            let st = self.mb_mut();
            for yy in by..by + step {
                for xx in bx..bx + step {
                    st.intra_modes[yy * 4 + xx] = mode;
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Reconstruction
    // ------------------------------------------------------------------------------------------

    fn add_luma_block4(&mut self, raster: usize) {
        if self.s.blk_nz & (1 << raster) == 0 {
            return;
        }
        let (base, stride) = self.luma_view();
        let px = self.mb_x * 16 + (raster & 3) * 4;
        let py = self.view.y + (raster >> 2) * 4;
        let Some(blk) = self.s.coef.get_mut(raster * 16..).and_then(<[i32]>::first_chunk_mut::<16>) else {
            return;
        };
        let dst = &mut self.pic.planes.y[base + py * stride + px..];
        if self.s.blk_ac & (1 << raster) == 0 {
            transform::idct_dc_add::<4>(blk[0], dst, stride);
            blk[0] = 0;
        } else {
            transform::idct4_add(blk, dst, stride);
            blk.fill(0);
        }
    }

    fn add_luma_block8(&mut self, b8: usize) {
        if self.s.blk_nz & (1 << b8) == 0 {
            return;
        }
        let (base, stride) = self.luma_view();
        let px = self.mb_x * 16 + (b8 & 1) * 8;
        let py = self.view.y + (b8 >> 1) * 8;
        let Some(blk) = self.s.coef.get_mut(b8 * 64..).and_then(<[i32]>::first_chunk_mut::<64>) else {
            return;
        };
        let dst = &mut self.pic.planes.y[base + py * stride + px..];
        if self.s.blk_ac & (1 << b8) == 0 {
            transform::idct_dc_add::<8>(blk[0], dst, stride);
            blk[0] = 0;
        } else {
            transform::idct8_add(blk, dst, stride);
            blk.fill(0);
        }
    }

    fn add_chroma(&mut self) {
        let (base, stride) = self.chroma_view();
        for c in 0..2 {
            if self.s.c_nz[c] == 0 {
                continue;
            }
            for b in 0..4 {
                if self.s.c_nz[c] & (1 << b) == 0 {
                    continue;
                }
                let px = self.mb_x * 8 + (b & 1) * 4;
                let py = self.view.y / 2 + (b >> 1) * 4;
                let Some(blk) = self.s.coef_c[c].get_mut(b * 16..).and_then(<[i32]>::first_chunk_mut::<16>) else {
                    continue;
                };
                let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
                let dst = &mut plane[base + py * stride + px..];
                if self.s.c_ac[c] & (1 << b) == 0 {
                    transform::idct_dc_add::<4>(blk[0], dst, stride);
                    blk[0] = 0;
                } else {
                    transform::idct4_add(blk, dst, stride);
                    blk.fill(0);
                }
            }
            self.s.c_nz[c] = 0;
            self.s.c_ac[c] = 0;
        }
    }

    /// Offset of the current macroblock's view in the luma plane and the view's stride.
    #[inline]
    fn luma_view(&self) -> (usize, usize) {
        let w = self.pic.planes.width;
        (self.view.off * w, w * self.view.mul)
    }

    /// [`Self::luma_view`] for the chroma planes.
    #[inline]
    fn chroma_view(&self) -> (usize, usize) {
        let w = self.pic.planes.cwidth;
        (self.view.off * w, w * self.view.mul)
    }

    /// The left neighbouring samples of luma rows y0..y0 + n are available for intra prediction.
    /// In an MBAFF frame they can come from both macroblocks of the left pair, which matters with
    /// constrained_intra_pred.
    fn left_ok(&self, y0: usize, n: usize) -> bool {
        if !self.mbaff || !self.pps.constrained_intra_pred {
            return self.intra_nb(0);
        }
        (y0..y0 + n).all(|y| self.locate(-1, y as i32).is_some_and(|(m, ..)| self.pic.mbs[m].kind.is_intra()))
    }

    /// Neighbour availability of the luma block of `n4` x `n4` 4x4 blocks at (bx, by).
    fn luma_avail4(&self, bx: usize, by: usize, n4: usize) -> Avail {
        let b = self.intra_nb(1);
        let c = self.intra_nb(2);
        let d = self.intra_nb(3);
        let left = if bx > 0 { true } else { self.left_ok(by * 4, n4 * 4) };
        let top = if by > 0 { true } else { b };
        let top_left = match (bx > 0, by > 0) {
            (true, true) => true,
            (false, true) => self.left_ok(by * 4 - 1, 1),
            (true, false) => b,
            (false, false) => d,
        };
        let top_right = if by == 0 {
            if bx < 3 { b } else { c }
        } else if bx == 3 {
            false
        } else {
            RASTER_TO_BLK4[(by - 1) * 4 + bx + 1] < RASTER_TO_BLK4[by * 4 + bx]
        };
        Avail { left, top, top_left, top_right }
    }

    /// Reconstruct an intra MB (prediction + residual) after its syntax has been parsed.
    pub fn reconstruct_intra(&mut self) {
        let kind = self.mb().kind;
        let (yo, stride) = self.luma_view();
        let (x0, y0) = (self.mb_x * 16, self.view.y);
        match kind {
            MbKind::I4x4 => {
                for &(bx, by) in BLK4_XY.iter() {
                    let (bx, by) = (bx as usize, by as usize);
                    let av = self.luma_avail4(bx, by, 1);
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    intra::pred4x4(&mut self.pic.planes.y[yo..], stride, x0 + bx * 4, y0 + by * 4, mode, av);
                    self.add_luma_block4(by * 4 + bx);
                }
            }
            MbKind::I8x8 => {
                for b8 in 0..4 {
                    let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
                    let mut av = self.luma_avail4(bx, by, 2);
                    av.top_right = match b8 {
                        0 => self.intra_nb(1),
                        1 => self.intra_nb(2),
                        2 => true,
                        _ => false,
                    };
                    let mode = self.mb().intra_modes[by * 4 + bx];
                    intra::pred8x8(&mut self.pic.planes.y[yo..], stride, x0 + bx * 4, y0 + by * 4, mode, av);
                    self.add_luma_block8(b8);
                }
            }
            MbKind::I16x16 => {
                let av = Avail { left: self.left_ok(0, 16), top: self.intra_nb(1), top_left: self.intra_nb(3), top_right: false };
                let mode = self.cur.info.i16_mode;
                intra::pred16x16(&mut self.pic.planes.y[yo..], stride, x0, y0, mode, av);
                for r in 0..16 {
                    self.add_luma_block4(r);
                }
            }
            _ => {}
        }
        // chroma rows 0..3 have the left neighbours of luma rows 0..7, rows 4..7 those of 8..15 (6.4.12)
        let av = Avail { left: self.left_ok(0, 8), top: self.intra_nb(1), top_left: self.intra_nb(3), top_right: false };
        let left_lower = self.left_ok(8, 8);
        let (co, cs) = self.chroma_view();
        let mode = self.mb().intra_chroma_mode;
        let (cx, cy) = (self.mb_x * 8, self.view.y / 2);
        intra::pred_chroma(&mut self.pic.planes.cb[co..], cs, cx, cy, mode, av, left_lower);
        intra::pred_chroma(&mut self.pic.planes.cr[co..], cs, cx, cy, mode, av, left_lower);
        self.add_chroma();
        self.s.blk_nz = 0;
        self.s.blk_ac = 0;
    }

    /// Add the residual of an inter MB (prediction already written).
    pub fn reconstruct_inter_residual(&mut self) {
        if self.mb().transform_8x8 {
            for b8 in 0..4 {
                self.add_luma_block8(b8);
            }
        } else if self.s.blk_nz != 0 {
            for r in 0..16 {
                self.add_luma_block4(r);
            }
        }
        self.add_chroma();
        self.s.blk_nz = 0;
        self.s.blk_ac = 0;
    }

    /// Write I_PCM samples.
    pub fn write_pcm(&mut self, samples: &[u8]) {
        let (yo, stride) = self.luma_view();
        let (x0, y0) = (self.mb_x * 16, self.view.y);
        for r in 0..16 {
            let o = yo + (y0 + r) * stride + x0;
            self.pic.planes.y[o..o + 16].copy_from_slice(&samples[r * 16..r * 16 + 16]);
        }
        let (co, cs) = self.chroma_view();
        let (cx, cy) = (self.mb_x * 8, self.view.y / 2);
        for c in 0..2 {
            let src = &samples[256 + c * 64..256 + c * 64 + 64];
            let plane = if c == 0 { &mut self.pic.planes.cb } else { &mut self.pic.planes.cr };
            for r in 0..8 {
                let o = co + (cy + r) * cs + cx;
                plane[o..o + 8].copy_from_slice(&src[r * 8..r * 8 + 8]);
            }
        }
    }

    /// Common state for an I_PCM MB.
    pub fn finish_pcm(&mut self) {
        let qpc = [self.chroma_qp(0, 0) as u8, self.chroma_qp(0, 1) as u8];
        let st = self.mb_mut();
        st.kind = MbKind::IPcm;
        st.qp = 0;
        st.qpc = qpc;
        st.nnz = [16; 16];
        st.nnz_c = [[16; 4]; 2];
        st.cbf_dc = 7;
        st.cbp = 0x2f;
        st.nz_mask = 0xffff;
        st.ref_idx = [[-1; 4]; 2];
    }

    /// Update QP after mb_qp_delta.
    pub fn apply_qp_delta(&mut self, delta: i32) -> Result<()> {
        ensure!((-26..=25).contains(&delta), "mb_qp_delta {delta} out of range");
        self.qp = (self.qp + delta + 52) % 52;
        Ok(())
    }

    pub fn store_qp(&mut self) {
        let qp = self.qp;
        let qpc = [self.chroma_qp(qp, 0) as u8, self.chroma_qp(qp, 1) as u8];
        let st = self.mb_mut();
        st.qp = qp as u8;
        st.qpc = qpc;
    }

    // ------------------------------------------------------------------------------------------
    // Residual storage helpers (shared by CAVLC and CABAC)
    // ------------------------------------------------------------------------------------------

    fn luma_list4(&self) -> usize {
        if self.mb().kind.is_intra() { 0 } else { 3 }
    }

    /// Coefficients, level scales and dequantiser of luma 4x4 block `raster`: the residual
    /// decoders write each coefficient straight to its raster position.
    pub fn luma4_dst(&mut self, raster: usize) -> (&mut [i32; 16], &'a [i32; 16], Dequant) {
        let qp = self.qp;
        let ls = &self.ls.ls4[self.luma_list4()][(qp % 6) as usize];
        let blk = &mut self.s.coef.as_chunks_mut::<16>().0[raster];
        (blk, ls, Dequant::new4(qp))
    }

    /// Luma 4x4 block `raster` has coefficients; `ac` adds an AC coefficient (the block then needs the
    /// full transform, not the DC-only path).
    pub fn mark_luma4(&mut self, raster: usize, ac: bool) {
        self.s.blk_nz |= 1 << raster;
        self.s.blk_ac |= (ac as u16) << raster;
        self.pic.mbs[self.mb_addr].nz_mask |= 1 << raster;
    }

    /// `luma4_dst` for luma 8x8 block `b8`.
    pub fn luma8_dst(&mut self, b8: usize) -> (&mut [i32; 64], &'a [i32; 64], Dequant) {
        let qp = self.qp;
        let list = if self.mb().kind.is_intra() { 0 } else { 1 };
        let ls = &self.ls.ls8[list][(qp % 6) as usize];
        let blk = &mut self.s.coef.as_chunks_mut::<64>().0[b8];
        (blk, ls, Dequant::new8(qp))
    }

    /// Luma 8x8 block `b8` has coefficients; `ac` adds an AC coefficient (the block then needs the
    /// full transform, not the DC-only path).
    pub fn mark_luma8(&mut self, b8: usize, ac: bool) {
        self.s.blk_nz |= 1 << b8;
        self.s.blk_ac |= (ac as u16) << b8;
        let (bx, by) = ((b8 & 1) * 2, (b8 >> 1) * 2);
        let m = (0b11u16 << (by * 4 + bx)) | (0b11u16 << ((by + 1) * 4 + bx));
        self.pic.mbs[self.mb_addr].nz_mask |= m;
    }

    /// Intra16x16 DC levels in scan order -> scaled DC of each 4x4 block.
    pub fn put_luma_dc(&mut self, levels: &[i32; 16]) {
        let mut c = [0i32; 16];
        for (k, &lv) in levels.iter().enumerate() {
            c[self.scan4[k] as usize] = lv;
        }
        let qp = self.qp;
        let ls00 = self.ls.ls4[0][(qp % 6) as usize][0];
        transform::luma_dc_dequant(&mut c, qp, ls00);
        for (r, &dc) in c.iter().enumerate() {
            if dc != 0 {
                self.s.coef[r * 16] = dc;
                self.s.blk_nz |= 1 << r;
            }
        }
    }

    /// Chroma DC levels (4:2:0: 4 values in raster order) for component `c`.
    pub fn put_chroma_dc(&mut self, c: usize, levels: &[i32; 4]) {
        let mut v = *levels;
        let qpc = self.chroma_qp(self.qp, c);
        let list = if self.mb().kind.is_intra() { 1 + c } else { 4 + c };
        let ls00 = self.ls.ls4[list][(qpc % 6) as usize][0];
        transform::chroma_dc_dequant_420(&mut v, qpc, ls00);
        for (b, &dc) in v.iter().enumerate() {
            if dc != 0 {
                self.s.coef_c[c][b * 16] = dc;
                self.s.c_nz[c] |= 1 << b;
            }
        }
    }

    /// `luma4_dst` for chroma AC block `b` of component `c`.
    pub fn chroma_ac_dst(&mut self, c: usize, b: usize) -> (&mut [i32; 16], &'a [i32; 16], Dequant) {
        let qpc = self.chroma_qp(self.qp, c);
        let list = if self.mb().kind.is_intra() { 1 + c } else { 4 + c };
        let ls = &self.ls.ls4[list][(qpc % 6) as usize];
        let blk = &mut self.s.coef_c[c].as_chunks_mut::<16>().0[b];
        (blk, ls, Dequant::new4(qpc))
    }

    /// Chroma AC block `b` of component `c` has coefficients; `ac` adds an AC coefficient (the block
    /// then needs the full transform, not the DC-only path).
    pub fn mark_chroma_ac(&mut self, c: usize, b: usize, ac: bool) {
        self.s.c_nz[c] |= 1 << b;
        self.s.c_ac[c] |= (ac as u8) << b;
    }

    // ------------------------------------------------------------------------------------------
    // CAVLC macroblock layer
    // ------------------------------------------------------------------------------------------

    /// Decode a P_Skip or B_Skip macroblock at the current address.
    pub fn decode_skip(&mut self) -> Result<()> {
        let b = self.sh.slice_type.is_b();
        self.cur = MbCur::default();
        self.cur.info.kind = if b { MbKind::BSkip } else { MbKind::PSkip };
        self.mb_mut().kind = self.cur.info.kind;
        self.store_qp();
        if b {
            self.derive_direct(0xf)?;
        } else {
            ensure!(!self.refs[0].is_empty(), "P_Skip with empty reference list");
            self.derive_p_skip();
        }
        self.predict_inter();
        self.prev_qp_delta_nz = false;
        Ok(())
    }

    /// num_ref_idx_lX_active for the current macroblock: doubled for a field macroblock of an MBAFF
    /// frame, whose lists hold fields (7.4.5.1).
    pub fn ref_range(&self) -> [u32; 2] {
        let k = if self.mbaff && self.field { 2 } else { 1 };
        self.sh.num_ref_idx_active.map(|n| n * k)
    }

    fn read_te(r: &mut BitReader, range: u32) -> Result<u32> {
        Ok(if range > 1 { r.read_ue()? } else { !r.read_bit()? as u32 })
    }

    /// Parse and reconstruct one macroblock_layer() with CAVLC.
    pub fn decode_mb_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        let mb_type = r.read_ue()?;
        let st = self.sh.slice_type;
        let info = match st {
            SliceType::I | SliceType::Si => intra_mb_type(mb_type),
            SliceType::P | SliceType::Sp => {
                if mb_type < 5 {
                    p_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 5)
                }
            }
            SliceType::B => {
                if mb_type < 23 {
                    b_mb_type(mb_type)
                } else {
                    intra_mb_type(mb_type - 23)
                }
            }
        };
        let Some(mut info) = info else { return invalid(format!("mb_type {mb_type} out of range")) };
        self.cur = MbCur { info, ..Default::default() };
        if info.kind == MbKind::IPcm {
            r.byte_align();
            let pos = r.byte_pos();
            let data = r.data();
            ensure!(data.len() >= pos + 384, "truncated I_PCM macroblock");
            let Some(&samples) = data.get(pos..).and_then(|d| d.first_chunk::<384>()) else {
                return invalid("truncated I_PCM macroblock");
            };
            r.skip(384 * 8)?;
            self.finish_pcm();
            self.write_pcm(&samples);
            return Ok(());
        }
        let mut transform_8x8 = false;
        if info.kind == MbKind::I4x4 && self.pps.transform_8x8_mode && r.read_flag()? {
            transform_8x8 = true;
            info.kind = MbKind::I8x8;
            self.cur.info.kind = MbKind::I8x8;
        }
        self.mb_mut().kind = info.kind;
        self.mb_mut().transform_8x8 = transform_8x8;
        let n_ref = self.ref_range();
        match info.kind {
            MbKind::I4x4 | MbKind::I8x8 => {
                let n = if info.kind == MbKind::I8x8 { 4 } else { 16 };
                for i in 0..n {
                    self.cur.rem_mode[i] = if r.read_flag()? { -1 } else { r.read_bits(3)? as i8 };
                }
                self.derive_intra_modes(info.kind == MbKind::I8x8);
                let m = r.read_ue()?;
                ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                self.mb_mut().intra_chroma_mode = m as u8;
            }
            MbKind::I16x16 => {
                let m = r.read_ue()?;
                ensure!(m <= 3, "intra_chroma_pred_mode out of range");
                self.mb_mut().intra_chroma_mode = m as u8;
            }
            MbKind::BDirect16x16 => {}
            _ => {
                if info.part == Part::P8x8 {
                    for p in 0..4 {
                        let t = r.read_ue()?;
                        let sub = if st.is_b() { b_sub_mb_type(t) } else { p_sub_mb_type(t) };
                        let Some(sub) = sub else { return invalid(format!("sub_mb_type {t} out of range")) };
                        self.cur.sub[p] = sub;
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            self.cur.ref_idx[l][p] = if n_ref[l] > 1 && !info.ref0 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                        }
                    }
                    for l in 0..2 {
                        for p in 0..4 {
                            let sub = self.cur.sub[p];
                            if sub.direct || sub.pred & (1 << l) == 0 {
                                continue;
                            }
                            for j in 0..sub.shape.num_parts() {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4 + j] = [x as i16, y as i16];
                            }
                        }
                    }
                } else {
                    let np = info.part.num_parts();
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                self.cur.ref_idx[l][p] = if n_ref[l] > 1 { Self::read_te(r, n_ref[l] - 1)? as i8 } else { 0 };
                            }
                        }
                    }
                    for l in 0..2 {
                        for p in 0..np {
                            if info.pred[p] & (1 << l) != 0 {
                                let x = r.read_se()?;
                                let y = r.read_se()?;
                                self.cur.mvd[l][p * 4] = [x as i16, y as i16];
                            }
                        }
                    }
                }
            }
        }
        // coded_block_pattern
        let cbp = if info.kind == MbKind::I16x16 {
            info.i16_cbp
        } else {
            let code = r.read_ue()?;
            ensure!(code < 48, "coded_block_pattern out of range");
            let (i, p) = CBP_ME[code as usize];
            if info.kind.is_intra() { i } else { p }
        };
        self.cur.cbp = cbp;
        self.mb_mut().cbp = cbp;
        if !info.kind.is_intra() && cbp & 15 != 0 && self.pps.transform_8x8_mode && self.no_sub_8x8_lt() {
            let t = r.read_flag()?;
            self.mb_mut().transform_8x8 = t;
        }
        if !info.kind.is_intra() {
            self.derive_inter_motion()?;
        }
        if cbp != 0 || info.kind == MbKind::I16x16 {
            let d = r.read_se()?;
            self.apply_qp_delta(d)?;
        }
        self.store_qp();
        self.residual_cavlc(r, cbp)?;
        if info.kind.is_intra() {
            self.reconstruct_intra();
        } else {
            self.predict_inter();
            self.reconstruct_inter_residual();
        }
        Ok(())
    }

    /// Whether transform_size_8x8_flag may be present for an inter MB (NoSubMbPartSizeLessThan8x8 etc.).
    pub fn no_sub_8x8_lt(&self) -> bool {
        let info = self.cur.info;
        match info.kind {
            MbKind::BDirect16x16 => self.sps.direct_8x8_inference,
            MbKind::Inter if info.part == Part::P8x8 => {
                self.cur.sub.iter().all(|s| if s.direct { self.sps.direct_8x8_inference } else { s.shape == SubPart::S8x8 })
            }
            _ => true,
        }
    }

    fn residual_cavlc(&mut self, r: &mut BitReader, cbp: u8) -> Result<()> {
        let kind = self.mb().kind;
        let t8 = self.mb().transform_8x8;
        let (scan4, scan8) = (self.scan4, self.scan8);
        if kind == MbKind::I16x16 {
            let nc = self.nc_luma(0);
            let mut lv = [0i32; 16];
            cavlc::residual_block(r, 0, 15, 16, nc, |k, v| lv[k] = v)?;
            self.put_luma_dc(&lv);
        }
        for b8 in 0..4 {
            let (bx8, by8) = ((b8 & 1) * 2, (b8 >> 1) * 2);
            if cbp & (1 << b8) == 0 {
                continue;
            }
            if t8 {
                let mut any = false;
                let mut ac = false;
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(raster);
                    let (blk, ls, dq) = self.luma8_dst(b8);
                    let tc = cavlc::residual_block(r, 0, 15, 16, nc, |k, v| {
                        let p = scan8[4 * k + i4] as usize;
                        ac |= p != 0;
                        blk[p] = dq.apply(v, ls[p]);
                    })?;
                    self.mb_mut().nnz[raster] = tc;
                    any |= tc > 0;
                }
                if any {
                    self.mark_luma8(b8, ac);
                }
            } else {
                for i4 in 0..4 {
                    let raster = (by8 + (i4 >> 1)) * 4 + bx8 + (i4 & 1);
                    let nc = self.nc_luma(raster);
                    let (blk, ls, dq) = self.luma4_dst(raster);
                    let mut ac = false;
                    let tc = if kind == MbKind::I16x16 {
                        cavlc::residual_block(r, 0, 14, 15, nc, |k, v| {
                            let p = scan4[k + 1] as usize;
                            ac = true;
                            blk[p] = dq.apply(v, ls[p]);
                        })?
                    } else {
                        cavlc::residual_block(r, 0, 15, 16, nc, |k, v| {
                            let p = scan4[k] as usize;
                            ac |= p != 0;
                            blk[p] = dq.apply(v, ls[p]);
                        })?
                    };
                    self.mb_mut().nnz[raster] = tc;
                    if tc > 0 {
                        self.mark_luma4(raster, ac);
                    }
                }
            }
        }
        let cbp_c = cbp >> 4;
        if cbp_c & 3 != 0 {
            for c in 0..2 {
                let mut lv = [0i32; 4];
                cavlc::residual_block(r, 0, 3, 4, -1, |k, v| lv[k] = v)?;
                self.put_chroma_dc(c, &lv);
            }
        }
        if cbp_c & 2 != 0 {
            for c in 0..2 {
                for b in 0..4 {
                    let nc = self.nc_chroma(c, b);
                    let (blk, ls, dq) = self.chroma_ac_dst(c, b);
                    let tc = cavlc::residual_block(r, 0, 14, 15, nc, |k, v| {
                        let p = scan4[k + 1] as usize;
                        blk[p] = dq.apply(v, ls[p]);
                    })?;
                    self.mb_mut().nnz_c[c][b] = tc;
                    if tc > 0 {
                        self.mark_chroma_ac(c, b, true);
                    }
                }
            }
        }
        Ok(())
    }

    /// Decode slice_data() with CAVLC. `r` is positioned after the slice header.
    pub fn decode_cavlc(&mut self, r: &mut BitReader) -> Result<()> {
        if self.mbaff {
            return self.decode_cavlc_mbaff(r);
        }
        let total = self.mb_w * self.mb_h;
        let mut addr = self.sh.first_mb_in_slice as usize;
        let intra_slice = self.sh.slice_type.is_intra();
        let field = self.field;
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            if !intra_slice {
                let run = r.read_ue()? as usize;
                ensure!(addr + run <= total, "mb_skip_run out of range");
                self.pic.reserve_mbs(run)?;
                for _ in 0..run {
                    self.start_mb(addr, field);
                    self.decode_skip()?;
                    self.pic.mb_done(addr)?;
                    addr += 1;
                }
                if run > 0 && !r.more_rbsp_data() {
                    break;
                }
                ensure!(addr < total, "macroblock address {addr} out of range");
            }
            self.pic.reserve_mbs(1)?;
            self.start_mb(addr, field);
            self.decode_mb_cavlc(r)?;
            self.pic.mb_done(addr)?;
            addr += 1;
            if !r.more_rbsp_data() {
                break;
            }
        }
        Ok(())
    }

    /// Raster address of macroblock `addr` (CurrMbAddr) of an MBAFF frame: pairs in raster order,
    /// the top macroblock first.
    pub fn mbaff_raster(&self, addr: usize) -> usize {
        let pair = addr / 2;
        (2 * (pair / self.mb_w) + addr % 2) * self.mb_w + pair % self.mb_w
    }

    /// mb_field_decoding_flag of a pair for which it is not present (7.4.4): the left pair's, else
    /// the pair above's (in the same slice), else frame.
    pub fn inferred_field(&self, addr: usize) -> bool {
        let w = self.mb_w;
        let (px, pr) = ((addr / 2) % w, (addr / 2) / w);
        let flag = |top: usize| self.pic.mbs.get(top).filter(|st| st.slice_num == self.slice_num).map(|st| st.field);
        let left = if px > 0 { flag(2 * pr * w + px - 1) } else { None };
        let above = if pr > 0 { flag(2 * (pr - 1) * w + px) } else { None };
        left.or(above).unwrap_or(false)
    }

    /// slice_data() of an MBAFF frame with CAVLC (7.3.4). The flag of a pair whose top
    /// macroblock is skipped but whose bottom one is not follows the skip run and applies to the
    /// skipped top macroblock too.
    fn decode_cavlc_mbaff(&mut self, r: &mut BitReader) -> Result<()> {
        let total = self.mb_w * self.mb_h;
        let first = self.sh.first_mb_in_slice as usize;
        ensure!(first < total / 2, "first_mb_in_slice {first} out of range");
        let mut addr = first * 2;
        let intra_slice = self.sh.slice_type.is_intra();
        let mut field = false;
        loop {
            ensure!(addr < total, "macroblock address {addr} out of range");
            if !intra_slice {
                let run = r.read_ue()? as usize;
                ensure!(addr + run <= total, "mb_skip_run out of range");
                self.pic.reserve_mbs(run)?;
                for i in 0..run {
                    if addr.is_multiple_of(2) {
                        field = if i + 1 < run {
                            self.inferred_field(addr)
                        } else {
                            // the bottom macroblock is coded: its flag comes next
                            ensure!(r.more_rbsp_data(), "slice ends inside a macroblock pair");
                            r.read_bit()?
                        };
                    }
                    let m = self.mbaff_raster(addr);
                    self.start_mb(m, field);
                    self.decode_skip()?;
                    self.pic.mb_done(m)?;
                    addr += 1;
                }
                if run > 0 && !r.more_rbsp_data() {
                    break;
                }
                ensure!(addr < total, "macroblock address {addr} out of range");
            }
            if addr.is_multiple_of(2) {
                field = r.read_bit()?;
            }
            self.pic.reserve_mbs(1)?;
            let m = self.mbaff_raster(addr);
            self.start_mb(m, field);
            self.decode_mb_cavlc(r)?;
            self.pic.mb_done(m)?;
            addr += 1;
            if !r.more_rbsp_data() {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod overlap_tests {
    use super::mbs_fit;

    #[test]
    fn a_picture_takes_each_macroblock_once() {
        assert!(mbs_fit(0, 99, 99));
        assert!(!mbs_fit(1, 99, 99));
        assert!(!mbs_fit(usize::MAX, 1, 99));
    }
}

#[cfg(test)]
mod temporal_direct_tests {
    use super::SliceDecoder;

    /// Temporal direct mvs stay inside the spec's motion-vector range −2^15..2^15−1 with a
    /// large `DistScaleFactor` and a co-located mv at an i16 extreme: the scale used to
    /// truncate past the range with `as i16`, and mvL0 − mvCol then overflowed a debug build
    /// ("attempt to subtract with overflow"). `dsf` 256 is the identity scale.
    #[test]
    fn temporal_direct_mvs_at_i16_extremes_stay_in_the_spec_range() {
        for dsf in [-1024i32, -1, 256, 1023] {
            for col in [[i16::MIN, i16::MIN], [i16::MAX, i16::MAX], [i16::MIN, i16::MAX]] {
                let (mv0, mv1) = SliceDecoder::temporal_direct_mvs(dsf, col);
                for mv in [mv0, mv1] {
                    for c in mv {
                        assert!((-(1 << 15)..(1 << 15)).contains(&i32::from(c)), "dsf {dsf}, col {col:?}: {mv:?}");
                    }
                }
                if dsf == 256 {
                    assert_eq!((mv0, mv1), (col, [0, 0]), "dsf 256 is the identity scale");
                }
            }
        }
        // mvL1 is scaled from the unclamped mvL0 with only the results clipped; deriving it from
        // the clipped mvL0 gave [0, 0] here.
        let (mv0, mv1) = SliceDecoder::temporal_direct_mvs(512, [i16::MAX, 0]);
        assert_eq!((mv0, mv1), ([i16::MAX, 0], [i16::MAX, 0]), "mvL1 keeps mvL0's off-range part");
    }
}

#[cfg(test)]
mod mbaff_neighbour_tests {
    use super::mbaff_neighbour;

    /// Table 6-4, one row per line: xN, yN, currMbFrameFlag, mbIsTopMbFlag, mbAddrX,
    /// mbAddrXFrameFlag (2: any), additional condition, mbAddrN, yM.
    #[rustfmt::skip]
    const TABLE_6_4: [(&str, &str, u8, u8, &str, u8, &str, &str, &str); 28] = [
        ("<0", "<0", 1, 1, "D", 2, "", "D+1", "yN"),
        ("<0", "<0", 1, 0, "A", 1, "", "A", "yN"),
        ("<0", "<0", 1, 0, "A", 0, "", "A+1", "(yN+maxH)>>1"),
        ("<0", "<0", 0, 1, "D", 1, "", "D+1", "2*yN"),
        ("<0", "<0", 0, 1, "D", 0, "", "D", "yN"),
        ("<0", "<0", 0, 0, "D", 2, "", "D+1", "yN"),
        ("<0", "in", 1, 1, "A", 1, "", "A", "yN"),
        ("<0", "in", 1, 1, "A", 0, "yN%2==0", "A", "yN>>1"),
        ("<0", "in", 1, 1, "A", 0, "yN%2!=0", "A+1", "yN>>1"),
        ("<0", "in", 1, 0, "A", 1, "", "A+1", "yN"),
        ("<0", "in", 1, 0, "A", 0, "yN%2==0", "A", "(yN+maxH)>>1"),
        ("<0", "in", 1, 0, "A", 0, "yN%2!=0", "A+1", "(yN+maxH)>>1"),
        ("<0", "in", 0, 1, "A", 1, "yN<maxH/2", "A", "yN<<1"),
        ("<0", "in", 0, 1, "A", 1, "yN>=maxH/2", "A+1", "(yN<<1)-maxH"),
        ("<0", "in", 0, 1, "A", 0, "", "A", "yN"),
        ("<0", "in", 0, 0, "A", 1, "yN<maxH/2", "A", "(yN<<1)+1"),
        ("<0", "in", 0, 0, "A", 1, "yN>=maxH/2", "A+1", "(yN<<1)+1-maxH"),
        ("<0", "in", 0, 0, "A", 0, "", "A+1", "yN"),
        ("in", "<0", 1, 1, "B", 2, "", "B+1", "yN"),
        ("in", "<0", 1, 0, "Curr", 2, "", "Curr-1", "yN"),
        ("in", "<0", 0, 1, "B", 1, "", "B+1", "2*yN"),
        ("in", "<0", 0, 1, "B", 0, "", "B", "yN"),
        ("in", "<0", 0, 0, "B", 2, "", "B+1", "yN"),
        (">", "<0", 1, 1, "C", 2, "", "C+1", "yN"),
        (">", "<0", 1, 0, "na", 2, "", "na", "na"),
        (">", "<0", 0, 1, "C", 1, "", "C+1", "2*yN"),
        (">", "<0", 0, 1, "C", 0, "", "C", "yN"),
        (">", "<0", 0, 0, "C", 2, "", "C+1", "yN"),
    ];

    fn offset(x: &str) -> (isize, isize) {
        match x {
            "A" => (-1, 0),
            "B" => (0, -1),
            "C" => (1, -1),
            "D" => (-1, -1),
            _ => (0, 0),
        }
    }

    fn eval(e: &str, yn: i32, max_h: i32) -> i32 {
        match e {
            "yN" => yn,
            "2*yN" => 2 * yn,
            "yN>>1" => yn >> 1,
            "yN<<1" => yn << 1,
            "(yN+maxH)>>1" => (yn + max_h) >> 1,
            "(yN<<1)-maxH" => (yn << 1) - max_h,
            "(yN<<1)+1" => (yn << 1) + 1,
            "(yN<<1)+1-maxH" => (yn << 1) + 1 - max_h,
            _ => panic!("{e}"),
        }
    }

    fn holds(c: &str, yn: i32, max_h: i32) -> bool {
        match c {
            "" => true,
            "yN%2==0" => yn % 2 == 0,
            "yN%2!=0" => yn % 2 != 0,
            "yN<maxH/2" => yn < max_h / 2,
            "yN>=maxH/2" => yn >= max_h / 2,
            _ => panic!("{c}"),
        }
    }

    /// Every neighbour location, current macroblock kind and availability and kind of the four
    /// neighbouring pairs, for luma (maxH 16) and chroma (maxH 8), against the table.
    #[test]
    fn mbaff_neighbours_follow_table_6_4() {
        let mut used = [false; TABLE_6_4.len()];
        for max_h in [16, 8] {
            let max_w = max_h;
            let xs = [("<0", -1), ("in", 0), ("in", max_w - 1), (">", max_w), (">", max_w + 3)];
            let ys = std::iter::once(("<0", -1)).chain((0..max_h).map(|y| ("in", y)));
            for (yc, yn) in ys {
                for (xc, xn) in xs {
                    if xc != "<0" && yc == "in" {
                        continue; // inside the current macroblock or not available: not looked up
                    }
                    for (frame, top) in [(true, true), (true, false), (false, true), (false, false)] {
                        // A, B, C, D: None (not available), Some(frame macroblock pair)
                        for combo in 0..81u32 {
                            let kind = |i: u32| match combo / 3u32.pow(i) % 3 {
                                0 => None,
                                1 => Some(true),
                                _ => Some(false),
                            };
                            let pair = |dx: isize, dy: isize| match (dx, dy) {
                                (-1, 0) => kind(0),
                                (0, -1) => kind(1),
                                (1, -1) => kind(2),
                                (-1, -1) => kind(3),
                                (0, 0) => Some(frame),
                                _ => panic!("pair ({dx}, {dy})"),
                            };
                            let rows: Vec<usize> = (0..TABLE_6_4.len())
                                .filter(|&i| {
                                    let r = TABLE_6_4[i];
                                    r.0 == xc && r.1 == yc && (r.2 == 1) == frame && (r.3 == 1) == top
                                })
                                .collect();
                            let x = TABLE_6_4[rows[0]].4;
                            let want = if x == "na" {
                                used[rows[0]] = true;
                                None
                            } else if let Some(xf) = pair(offset(x).0, offset(x).1) {
                                let row = rows
                                    .iter()
                                    .copied()
                                    .filter(|&i| TABLE_6_4[i].5 == 2 || (TABLE_6_4[i].5 == 1) == xf)
                                    .filter(|&i| holds(TABLE_6_4[i].6, yn, max_h))
                                    .collect::<Vec<_>>();
                                assert_eq!(row.len(), 1, "rows for xN {xn} yN {yn}");
                                used[row[0]] = true;
                                let r = TABLE_6_4[row[0]];
                                let (n, lower) = match r.7.strip_suffix("+1") {
                                    Some(n) => (n, true),
                                    None => (r.7.strip_suffix("-1").unwrap_or(r.7), false),
                                };
                                let (dx, dy) = offset(n);
                                Some((dx, dy, lower, eval(r.8, yn, max_h)))
                            } else {
                                None
                            };
                            let got = mbaff_neighbour(xn, yn, max_h, frame, !top, pair);
                            assert_eq!(got, want, "xN {xn} yN {yn} maxH {max_h} frame {frame} top {top} pairs {combo}");
                        }
                    }
                }
            }
        }
        assert!(used.iter().all(|&u| u), "every row is exercised: {used:?}");
    }
}
