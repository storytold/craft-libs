//! VOP decoding: video packets, macroblocks and blocks (§6.2.6 to §6.2.8), intra prediction
//! (§7.4.3), inverse quantisation (§7.4.2), motion vectors (§7.6.3, §7.6.5, §7.6.9) and
//! reconstruction.

use craft_media::bitstream::BitReader;

use super::super::dct::idct;
use super::super::tables::ZIGZAG;
use super::headers::{self, Vol, Vop, VopKind};
use super::mc::{self, Dst, Gmc, Plane, div_round};
use super::vlc::{self, MCBPC_STUFFING, TcoefVlc, eof};
use super::{DecodeError, Result};

/// The alternate-vertical scan (§7.4.1): interlaced VOPs with `alternate_vertical_scan_flag`, and
/// intra blocks whose AC coefficients are predicted from the left.
pub(super) const ALT_VERTICAL: [usize; 64] = [
    0, 8, 16, 24, 1, 9, 2, 10, 17, 25, 32, 40, 48, 56, 57, 49, 41, 33, 26, 18, 3, 11, 4, 12, 19, 27, 34, 42, 50, 58, 35, 43, 51, 59, 20, 28, 5, 13,
    6, 14, 21, 29, 36, 44, 52, 60, 37, 45, 53, 61, 22, 30, 7, 15, 23, 31, 38, 46, 54, 62, 39, 47, 55, 63,
];

/// The alternate-horizontal scan: the transpose of [`ALT_VERTICAL`].
pub(super) const ALT_HORIZONTAL: [usize; 64] = {
    let mut t = [0usize; 64];
    let mut i = 0;
    while i < 64 {
        let p = ALT_VERTICAL[i];
        t[i] = (p % 8) * 8 + p / 8;
        i += 1;
    }
    t
};

/// The forward and backward direct-mode vector components (§7.6.9.5.2) of co-located component
/// `v` with delta `d` and temporal distances `trb` and `trd`. Hostile time stamps make the
/// products exceed `i32` (`v` reaches ±2048 and the distances ±2^20), so they are taken in `i64`;
/// the results saturate. Where nothing overflows the values are those of `i32` arithmetic, as
/// both divisions truncate toward zero.
pub(super) fn direct_scale(v: i32, d: i32, trb: i32, trd: i32) -> [i32; 2] {
    let (v, d, trb) = (i64::from(v), i64::from(d), i64::from(trb));
    let trd = if trd == 0 { 1 } else { i64::from(trd) };
    let f = v * trb / trd + d;
    let b = if d == 0 { v * (trb - trd) / trd } else { f - v };
    let narrow = |x: i64| x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    [narrow(f), narrow(b)]
}

/// A decoded picture: luminance and both chrominance planes, sized to whole macroblocks.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub planes: [Plane; 3],
}

impl Frame {
    pub(super) fn new(width: u32, height: u32) -> Frame {
        let (mb_w, mb_h) = (width.div_ceil(16) as usize, height.div_ceil(16) as usize);
        Frame { planes: [Plane::new(mb_w * 16, mb_h * 16), Plane::new(mb_w * 8, mb_h * 8), Plane::new(mb_w * 8, mb_h * 8)] }
    }

    /// Whether this buffer has the size of a `width`×`height` VOP.
    pub(super) fn fits(&self, width: u32, height: u32) -> bool {
        let (mb_w, mb_h) = (width.div_ceil(16) as usize, height.div_ceil(16) as usize);
        let [y, u, v] = &self.planes;
        (y.stride, y.rows) == (mb_w * 16, mb_h * 16) && [u, v].iter().all(|p| (p.stride, p.rows) == (mb_w * 8, mb_h * 8))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MbKind {
    Intra,
    Inter,
    /// `not_coded` in a P-VOP: B-VOPs skip the co-located macroblock too.
    Skipped,
    /// Predicted by global motion compensation (`mcsel`, or skipped in an S-VOP).
    Gmc,
}

/// Two field vectors (field units) and the reference field of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FieldMv {
    pub mv: [[i32; 2]; 2],
    pub sel: [usize; 2],
}

/// What a B-VOP needs of the co-located macroblock of its future reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MbInfo {
    pub kind: MbKind,
    /// The vector of each 8×8 luminance block (frame units).
    pub mv: [[i32; 2]; 4],
    pub field: Option<FieldMv>,
}

impl MbInfo {
    const INTRA: MbInfo = MbInfo { kind: MbKind::Intra, mv: [[0; 2]; 4], field: None };
    const SKIPPED: MbInfo = MbInfo { kind: MbKind::Skipped, mv: [[0; 2]; 4], field: None };
}

/// Temporal distances for direct mode (§7.6.9.5).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Times {
    /// Past reference to B-VOP, and past to future reference.
    pub trb: i32,
    pub trd: i32,
    /// The same in field periods, for interlaced direct mode.
    pub trb_field: i32,
    pub trd_field: i32,
}

/// The reference pictures of a VOP.
pub(super) struct Refs<'a> {
    /// P- and S-VOPs: the reference. B-VOPs: the past reference.
    pub past: Option<&'a Frame>,
    /// B-VOPs: the future reference.
    pub future: Option<&'a Frame>,
    /// B-VOPs: the macroblocks of the future reference.
    pub colocated: &'a [MbInfo],
    pub times: Times,
    /// Picture copied where a VOP is damaged.
    pub conceal: Option<&'a Frame>,
}

/// The motion of one macroblock from one reference.
#[derive(Clone, Copy, Debug)]
enum Motion {
    Frame([i32; 2]),
    Four([[i32; 2]; 4]),
    Field([[i32; 2]; 2], [usize; 2]),
}

/// One macroblock's samples while it is reconstructed.
#[derive(Clone)]
struct MbPix {
    y: [u8; 256],
    c: [[u8; 64]; 2],
}

impl MbPix {
    fn new() -> MbPix {
        MbPix { y: [0; 256], c: [[0; 64]; 2] }
    }

    /// The average of two predictions, rounded up (§7.6.9.6).
    fn average(&mut self, o: &MbPix) {
        let avg = |a: &mut u8, b: &u8| *a = ((u16::from(*a) + u16::from(*b) + 1) >> 1) as u8;
        self.y.iter_mut().zip(&o.y).for_each(|(a, b)| avg(a, b));
        for k in 0..2 {
            self.c[k].iter_mut().zip(&o.c[k]).for_each(|(a, b)| avg(a, b));
        }
    }

    /// Add (or, intra, store) the residual of block `b`.
    fn add(&mut self, b: usize, field_dct: bool, res: &[i32; 64], intra: bool) {
        for (j, res) in res.as_chunks::<8>().0.iter().enumerate() {
            let (buf, at): (&mut [u8], usize) = match b {
                0..=3 => {
                    let y = if field_dct { (b >> 1) + 2 * j } else { (b >> 1) * 8 + j };
                    (&mut self.y, y * 16 + (b & 1) * 8)
                }
                _ => (&mut self.c[(b - 4).min(1)], j * 8),
            };
            let Some(row) = buf.get_mut(at..at + 8) else { continue };
            if intra {
                row.iter_mut().zip(res).for_each(|(p, &r)| *p = r.clamp(0, 255) as u8);
            } else {
                row.iter_mut().zip(res).for_each(|(p, &r)| *p = (i32::from(*p) + r).clamp(0, 255) as u8);
            }
        }
    }
}

/// What intra prediction keeps of a block (§7.4.3).
#[derive(Clone, Copy, Default)]
struct PredBlk {
    /// Index + 1 of the intra macroblock holding the block, 0 when not intra.
    mb: u32,
    /// Reconstructed DC, `F[0][0]`.
    dc: i32,
    /// `QF[0][1..8]` and `QF[1..8][0]`.
    row: [i32; 7],
    col: [i32; 7],
    qp: i32,
}

/// `intra_dc_vlc_thr`: below this quantiser, DC uses its own VLC.
const DC_VLC_THRESHOLD: [i32; 8] = [32, 13, 15, 17, 19, 21, 23, 0];

/// Table 7-1.
fn dc_scaler(qp: i32, luma: bool) -> i32 {
    match (luma, qp) {
        (_, ..=4) => 8,
        (true, 5..=8) => 2 * qp,
        (true, 9..=24) => qp + 8,
        (true, _) => 2 * qp - 16,
        (false, 5..=24) => (qp + 13) / 2,
        (false, _) => qp - 6,
    }
}

/// B-VOP `mb_type`.
const B_DIRECT: u32 = 0;
const B_INTERPOLATE: u32 = 1;
const B_BACKWARD: u32 = 2;
const B_FORWARD: u32 = 3;

/// Decode one VOP into `out`. Returns the macroblock information (for later B-VOPs) and the first
/// error; a damaged part of the picture is copied from `refs.conceal`.
pub(super) fn decode_vop(data: &[u8], vol: &Vol, vop: &Vop, refs: &Refs, out: &mut Frame) -> (Vec<MbInfo>, Option<DecodeError>) {
    let mb_w = vol.width.div_ceil(16) as usize;
    let mb_h = vol.height.div_ceil(16) as usize;
    let gmc = (vop.kind == VopKind::S).then(|| Gmc::new(vol.width, vol.height, vol.warp_points, vol.warp_accuracy, &vop.warp, vop.rounding));
    let mut d = Dec {
        vol,
        vop,
        refs,
        gmc,
        mb_w,
        mb_h,
        qp: i32::from(vop.quant),
        packet_start: 0,
        mbs: vec![MbInfo::INTRA; mb_w * mb_h],
        bmv: vec![[0; 2]; mb_w * mb_h * 4],
        pred: [vec![PredBlk::default(); mb_w * mb_h * 4], vec![PredBlk::default(); mb_w * mb_h], vec![PredBlk::default(); mb_w * mb_h]],
        last_mv: [[[0; 2]; 2]; 2],
    };
    let err = d.run(data, out).err();
    (d.mbs, err)
}

struct Dec<'a> {
    vol: &'a Vol,
    vop: &'a Vop,
    refs: &'a Refs<'a>,
    gmc: Option<Gmc>,
    mb_w: usize,
    mb_h: usize,
    qp: i32,
    /// Index of the first macroblock of the current video packet.
    packet_start: usize,
    mbs: Vec<MbInfo>,
    /// P- and S-VOPs: the vector of every 8×8 luminance block, for prediction.
    bmv: Vec<[i32; 2]>,
    /// Intra prediction state: luminance blocks, then each chrominance plane.
    pred: [Vec<PredBlk>; 3],
    /// B-VOP vector predictors: `[direction][field]`, frame units.
    last_mv: [[[i32; 2]; 2]; 2],
}

fn rd(r: &mut BitReader, n: u32) -> Result<u32> {
    r.read_bits(n).map_err(eof)
}

fn bit(r: &mut BitReader) -> Result<bool> {
    r.read_bit().map_err(eof)
}

/// At a macroblock boundary: the bit position just past a resync marker of `len` bits that
/// follows the byte-alignment stuffing here, if there is one.
fn at_resync(r: &BitReader, len: u32) -> Option<usize> {
    let pos = r.position();
    let n = 8 - (pos % 8) as u32;
    if r.peek(n) != (1 << (n - 1)) - 1 {
        return None;
    }
    let mut m = r.clone();
    m.seek_bits(pos + n as usize);
    (m.bits_left() >= len as usize && m.peek(len) == 1).then_some(pos + n as usize + len as usize)
}

/// The bit position just past the next byte-aligned resync marker at or after `from`.
fn find_resync(data: &[u8], from: usize, len: u32) -> Option<usize> {
    let mut r = BitReader::new(data);
    let mut pos = from.div_ceil(8) * 8;
    while pos + len as usize <= data.len() * 8 {
        r.seek_bits(pos);
        if r.peek(len) == 1 {
            return Some(pos + len as usize);
        }
        pos += 8;
    }
    None
}

/// One motion vector component (§7.6.3.1): `motion_code`, `motion_residual`, the predictor,
/// and wrapping into the range of `fcode`.
fn read_mv(r: &mut BitReader, fcode: u8, pred: i32) -> Result<i32> {
    let mag = vlc::mvd().read(r, "invalid motion vector code")? as i32;
    let code = if mag != 0 && bit(r)? { -mag } else { mag };
    let r_size = u32::from(fcode.clamp(1, 7)) - 1;
    let f = 1i32 << r_size;
    let diff = if f == 1 || code == 0 {
        code
    } else {
        let d = (code.abs() - 1) * f + rd(r, r_size)? as i32 + 1;
        if code < 0 { -d } else { d }
    };
    let (low, high, range) = (-32 * f, 32 * f - 1, 64 * f);
    let mut v = pred.clamp(low, high) + diff;
    if v < low {
        v += range;
    } else if v > high {
        v -= range;
    }
    Ok(v)
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// `dct_dc_differential` of an intra block.
fn read_dc(r: &mut BitReader, luma: bool) -> Result<i32> {
    let size = vlc::dc_size(luma).read(r, "invalid DC size")?;
    if size == 0 {
        return Ok(0);
    }
    let v = rd(r, size)? as i32;
    let diff = if v >> (size - 1) == 0 { v - ((1 << size) - 1) } else { v };
    if size > 8 {
        vlc::marker(r)?;
    }
    Ok(diff)
}

/// TCOEF events of a block into `qf` (raster order) along `scan`, from coefficient `start`.
fn read_coeffs(r: &mut BitReader, table: &TcoefVlc, scan: &[usize; 64], start: usize, qf: &mut [i32; 64]) -> Result<()> {
    let mut i = start;
    loop {
        let (ev, level) = table.read(r)?;
        i += ev.run as usize;
        let Some(&pos) = scan.get(i) else {
            return Err(DecodeError::Invalid("more than 64 coefficients in a block"));
        };
        qf[pos] = level;
        i += 1;
        if ev.last {
            return Ok(());
        }
    }
}

impl Dec<'_> {
    fn run(&mut self, data: &[u8], out: &mut Frame) -> Result<()> {
        let mut r = BitReader::new(data);
        r.seek_bits(self.vop.data_start);
        let total = self.mb_w * self.mb_h;
        let marker_len = headers::resync_marker_len(self.vop);
        let mut idx = 0;
        let mut first_err = None;
        while idx < total {
            let start = r.position();
            let step = if self.vol.resync_markers
                && idx > 0
                && let Some(pos) = at_resync(&r, marker_len)
            {
                r.seek_bits(pos);
                self.packet(&mut r, idx, out)
            } else {
                self.macroblock(&mut r, idx, out).map(|()| idx + 1)
            };
            match step {
                Ok(next) => idx = next,
                Err(e) => {
                    first_err.get_or_insert(e);
                    match self.resume(data, start, idx, marker_len, out) {
                        Some((next, pos)) => {
                            r.seek_bits(pos);
                            idx = next;
                        }
                        None => {
                            self.conceal(idx, total, out);
                            break;
                        }
                    }
                }
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// After an error: the next video packet past `from` that starts after macroblock `idx`,
    /// with the macroblocks up to it concealed. Returns its first macroblock and the bit position
    /// of its data.
    fn resume(&mut self, data: &[u8], from: usize, idx: usize, len: u32, out: &mut Frame) -> Option<(usize, usize)> {
        if !self.vol.resync_markers {
            return None;
        }
        let mut pos = from;
        while let Some(p) = find_resync(data, pos, len) {
            let mut r = BitReader::new(data);
            r.seek_bits(p);
            if let Ok(h) = headers::parse_packet_header(&mut r, self.vol, self.vop, self.mb_w * self.mb_h)
                && h.mb_index >= idx
            {
                self.conceal(idx, h.mb_index, out);
                self.start_packet(h);
                return Some((h.mb_index, r.position()));
            }
            pos = p;
        }
        None
    }

    fn start_packet(&mut self, h: headers::PacketHeader) {
        self.qp = i32::from(h.quant);
        self.packet_start = h.mb_index;
        self.last_mv = [[[0; 2]; 2]; 2];
    }

    /// A video packet header; returns the packet's first macroblock.
    fn packet(&mut self, r: &mut BitReader, idx: usize, out: &mut Frame) -> Result<usize> {
        let h = headers::parse_packet_header(r, self.vol, self.vop, self.mb_w * self.mb_h)?;
        if h.mb_index < idx {
            return Err(DecodeError::Invalid("video packet goes back"));
        }
        self.conceal(idx, h.mb_index, out);
        self.start_packet(h);
        Ok(h.mb_index)
    }

    /// Copy macroblocks `from..to` from the concealment picture, or make them grey without one.
    /// Either way they are written: the picture buffer may hold an older VOP.
    fn conceal(&mut self, from: usize, to: usize, out: &mut Frame) {
        for idx in from..to.min(self.mbs.len()) {
            let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
            for (p, d) in out.planes.iter_mut().enumerate() {
                let n = if p == 0 { 16 } else { 8 };
                let src = self.refs.conceal.map(|f| &f.planes[p]);
                for j in 0..n {
                    let at = (my * n + j) * d.stride + mx * n;
                    let Some(dst) = d.data.get_mut(at..at + n) else { continue };
                    match src.and_then(|s| s.data.get(at..at + n)) {
                        Some(src) => dst.copy_from_slice(src),
                        None => dst.fill(128),
                    }
                }
            }
            self.mbs[idx] = MbInfo { kind: MbKind::Inter, ..MbInfo::INTRA };
            self.set_bmv(idx, [[0; 2]; 4]);
        }
    }

    fn macroblock(&mut self, r: &mut BitReader, idx: usize, out: &mut Frame) -> Result<()> {
        match self.vop.kind {
            VopKind::I => self.mb_i(r, idx, out),
            VopKind::P | VopKind::S => self.mb_p(r, idx, out),
            VopKind::B => self.mb_b(r, idx, out),
        }
    }

    fn dquant(&mut self, r: &mut BitReader) -> Result<()> {
        const D: [i32; 4] = [-1, -2, 1, 2];
        self.qp = (self.qp + D[rd(r, 2)? as usize]).clamp(1, 31);
        Ok(())
    }

    fn mb_i(&mut self, r: &mut BitReader, idx: usize, out: &mut Frame) -> Result<()> {
        let mcbpc = loop {
            let v = vlc::mcbpc_intra().read(r, "invalid mcbpc")?;
            if v != MCBPC_STUFFING {
                break v;
            }
        };
        self.intra_mb(r, idx, mcbpc >> 2 == 1, mcbpc & 3, out)
    }

    /// An intra macroblock after its `mcbpc`.
    fn intra_mb(&mut self, r: &mut BitReader, idx: usize, with_q: bool, cbpc: u32, out: &mut Frame) -> Result<()> {
        let ac_pred = bit(r)?;
        let cbpy = vlc::cbpy().read(r, "invalid cbpy")?;
        if with_q {
            self.dquant(r)?;
        }
        let field_dct = self.vol.interlaced && bit(r)?;
        let cbp = cbpy << 2 | cbpc;
        let use_dc_vlc = self.qp < DC_VLC_THRESHOLD[usize::from(self.vop.dc_thr & 7)];
        let mut pix = MbPix::new();
        for b in 0..6 {
            let res = self.intra_block(r, idx, b, ac_pred, cbp & (32 >> b) != 0, use_dc_vlc)?;
            pix.add(b, field_dct, &res, true);
        }
        self.store(out, idx, &pix);
        self.mbs[idx] = MbInfo::INTRA;
        self.set_bmv(idx, [[0; 2]; 4]);
        Ok(())
    }

    /// Prediction grid, position and grid width of block `b` of macroblock `idx`.
    fn block_pos(&self, idx: usize, b: usize) -> (usize, usize, usize, usize) {
        let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
        match b {
            0..=3 => (0, 2 * mx + (b & 1), 2 * my + (b >> 1), 2 * self.mb_w),
            _ => (b - 3, mx, my, self.mb_w),
        }
    }

    /// The intra prediction state of a neighbouring block, if it is usable.
    fn neighbour(&self, comp: usize, x: usize, y: usize, dx: usize, dy: usize, w: usize) -> Option<PredBlk> {
        let (x, y) = (x.checked_sub(dx)?, y.checked_sub(dy)?);
        let p = *self.pred[comp].get(y * w + x)?;
        (p.mb != 0 && p.mb as usize > self.packet_start).then_some(p)
    }

    /// Decode, predict and dequantise intra block `b`; returns its IDCT.
    fn intra_block(&mut self, r: &mut BitReader, idx: usize, b: usize, ac_pred: bool, coded: bool, use_dc_vlc: bool) -> Result<[i32; 64]> {
        let (comp, x, y, w) = self.block_pos(idx, b);
        let luma = comp == 0;
        let qp = self.qp;
        let scaler = dc_scaler(qp, luma);
        let a = self.neighbour(comp, x, y, 1, 0, w);
        let bb = self.neighbour(comp, x, y, 1, 1, w);
        let c = self.neighbour(comp, x, y, 0, 1, w);
        let dc = |p: Option<PredBlk>| p.map_or(1024, |p| p.dc);
        // Predict from above when the horizontal gradient is the smaller.
        let vertical = (dc(a) - dc(bb)).abs() < (dc(bb) - dc(c)).abs();
        let scan = if self.vop.alternate_scan || (ac_pred && !vertical) {
            &ALT_VERTICAL
        } else if ac_pred {
            &ALT_HORIZONTAL
        } else {
            &ZIGZAG
        };
        let mut qf = [0i32; 64];
        let mut start = 0;
        if use_dc_vlc {
            qf[0] = read_dc(r, luma)?;
            start = 1;
        }
        if coded {
            read_coeffs(r, vlc::tcoef_intra(), scan, start, &mut qf)?;
        }
        let pred_dc = if vertical { dc(c) } else { dc(a) };
        qf[0] = (qf[0] + div_round(i64::from(pred_dc), i64::from(scaler)) as i32).clamp(-2048, 2047);
        if ac_pred {
            let scale = |v: i32, from: i32| div_round(i64::from(v) * i64::from(from), i64::from(qp)) as i32;
            match (vertical, c, a) {
                (true, Some(c), _) => (1..8).for_each(|u| qf[u] = (qf[u] + scale(c.row[u - 1], c.qp)).clamp(-2048, 2047)),
                (false, _, Some(a)) => (1..8).for_each(|v| qf[v * 8] = (qf[v * 8] + scale(a.col[v - 1], a.qp)).clamp(-2048, 2047)),
                _ => {}
            }
        }
        let mut f = self.dequant(&qf, true, qp, scaler);
        self.pred[comp][y * w + x] =
            PredBlk { mb: idx as u32 + 1, dc: f[0], row: std::array::from_fn(|k| qf[k + 1]), col: std::array::from_fn(|k| qf[(k + 1) * 8]), qp };
        idct(&mut f);
        Ok(f)
    }

    /// Inverse quantisation (§7.4.2) and saturation; the MPEG method's mismatch control.
    fn dequant(&self, qf: &[i32; 64], intra: bool, qp: i32, dc_scaler: i32) -> [i32; 64] {
        let mut f = [0i32; 64];
        let first = usize::from(intra);
        if intra {
            f[0] = (qf[0] * dc_scaler).clamp(-2048, 2047);
        }
        if !self.vol.mpeg_quant {
            for k in first..64 {
                let q = qf[k];
                if q != 0 {
                    let a = (2 * q.abs() + 1) * qp - i32::from(qp % 2 == 0);
                    f[k] = (if q < 0 { -a } else { a }).clamp(-2048, 2047);
                }
            }
            return f;
        }
        let w = if intra { &self.vol.intra_matrix } else { &self.vol.inter_matrix };
        let mut sum = 0i32;
        for k in 0..64 {
            if k >= first && qf[k] != 0 {
                let q = qf[k];
                let k2 = if intra { 2 * q } else { 2 * q + q.signum() };
                f[k] = ((k2 * i32::from(w[k]) * qp) / 16).clamp(-2048, 2047);
            }
            sum += f[k];
        }
        if sum & 1 == 0 {
            f[63] ^= 1;
        }
        f
    }

    /// An inter block's coefficients, dequantised and transformed.
    fn inter_block(&self, r: &mut BitReader) -> Result<[i32; 64]> {
        let scan = if self.vop.alternate_scan { &ALT_VERTICAL } else { &ZIGZAG };
        let mut qf = [0i32; 64];
        read_coeffs(r, vlc::tcoef_inter(), scan, 0, &mut qf)?;
        let mut f = self.dequant(&qf, false, self.qp, 8);
        idct(&mut f);
        Ok(f)
    }

    fn residual(&self, r: &mut BitReader, cbp: u32, field_dct: bool, pix: &mut MbPix) -> Result<()> {
        for b in 0..6 {
            if cbp & (32 >> b) != 0 {
                let res = self.inter_block(r)?;
                pix.add(b, field_dct, &res, false);
            }
        }
        Ok(())
    }

    fn store(&self, out: &mut Frame, idx: usize, pix: &MbPix) {
        let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
        let put = |p: &mut Plane, src: &[u8], n: usize| {
            for j in 0..n {
                let at = (my * n + j) * p.stride + mx * n;
                if let Some(d) = p.data.get_mut(at..at + n) {
                    d.copy_from_slice(&src[j * n..j * n + n]);
                }
            }
        };
        let [y, u, v] = &mut out.planes;
        put(y, &pix.y, 16);
        put(u, &pix.c[0], 8);
        put(v, &pix.c[1], 8);
    }

    fn set_bmv(&mut self, idx: usize, mv: [[i32; 2]; 4]) {
        let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
        for (k, v) in mv.iter().enumerate() {
            let at = (2 * my + (k >> 1)) * 2 * self.mb_w + 2 * mx + (k & 1);
            if let Some(slot) = self.bmv.get_mut(at) {
                *slot = *v;
            }
        }
    }

    /// The vector of the 8×8 block at `(x, y)` if it is a usable predictor.
    fn candidate(&self, x: isize, y: isize) -> Option<[i32; 2]> {
        let gw = 2 * self.mb_w as isize;
        if x < 0 || y < 0 || x >= gw {
            return None;
        }
        let mb = (y as usize / 2) * self.mb_w + x as usize / 2;
        (mb >= self.packet_start).then(|| self.bmv.get(y as usize * gw as usize + x as usize).copied()).flatten()
    }

    /// The vector predictor of block `k` (0 for a 16×16 vector) of macroblock `idx` (§7.6.5).
    fn mv_pred(&self, idx: usize, k: usize) -> [i32; 2] {
        let (mx, my) = ((idx % self.mb_w) as isize, (idx / self.mb_w) as isize);
        let (bx, by) = (2 * mx + (k & 1) as isize, 2 * my + (k >> 1) as isize);
        let cands = match k {
            0 => [(bx - 1, by), (bx, by - 1), (bx + 2, by - 1)],
            1 => [(bx - 1, by), (bx, by - 1), (bx + 1, by - 1)],
            2 => [(bx - 1, by), (bx, by - 1), (bx + 1, by - 1)],
            _ => [(bx - 1, by), (bx - 1, by - 1), (bx, by - 1)],
        };
        let c = cands.map(|(x, y)| self.candidate(x, y));
        let valid: Vec<[i32; 2]> = c.iter().flatten().copied().collect();
        match valid.len() {
            0 => [0, 0],
            1 => valid[0],
            _ => {
                let v = c.map(|m| m.unwrap_or([0, 0]));
                [median(v[0][0], v[1][0], v[2][0]), median(v[0][1], v[1][1], v[2][1])]
            }
        }
    }

    fn mb_p(&mut self, r: &mut BitReader, idx: usize, out: &mut Frame) -> Result<()> {
        let mcbpc = loop {
            if bit(r)? {
                return self.skip_p(idx, out);
            }
            let v = vlc::mcbpc_inter().read(r, "invalid mcbpc")?;
            if v != MCBPC_STUFFING {
                break v;
            }
        };
        let (mb_type, cbpc) = (mcbpc >> 2, mcbpc & 3);
        if mb_type >= 3 {
            return self.intra_mb(r, idx, mb_type == 4, cbpc, out);
        }
        let mcsel = self.vop.kind == VopKind::S && mb_type != 2 && bit(r)?;
        let cbpy = 15 - vlc::cbpy().read(r, "invalid cbpy")?;
        if mb_type == 1 {
            self.dquant(r)?;
        }
        let cbp = cbpy << 2 | cbpc;
        let mut field_dct = false;
        let mut field_sel = None;
        if self.vol.interlaced {
            field_dct = cbp != 0 && bit(r)?;
            if mb_type != 2 && !mcsel && bit(r)? {
                field_sel = Some([usize::from(bit(r)?), usize::from(bit(r)?)]);
            }
        }
        let mut pix = MbPix::new();
        if mcsel {
            self.gmc_mb(idx, &mut pix);
        } else {
            let motion = self.p_vectors(r, idx, mb_type == 2, field_sel)?;
            if let Some(past) = self.refs.past {
                self.predict(past, idx, motion, self.vop.rounding, &mut pix);
            }
        }
        self.residual(r, cbp, field_dct, &mut pix)?;
        self.store(out, idx, &pix);
        Ok(())
    }

    /// Read the vectors of an inter macroblock of a P-VOP and record them.
    fn p_vectors(&mut self, r: &mut BitReader, idx: usize, four: bool, field_sel: Option<[usize; 2]>) -> Result<Motion> {
        let fcode = self.vop.fcode_f;
        if four {
            let mut mv = [[0; 2]; 4];
            for k in 0..4 {
                let p = self.mv_pred(idx, k);
                mv[k] = [read_mv(r, fcode, p[0])?, read_mv(r, fcode, p[1])?];
                self.set_bmv(idx, std::array::from_fn(|j| if j < k { mv[j] } else { mv[k] }));
            }
            self.mbs[idx] = MbInfo { kind: MbKind::Inter, mv, field: None };
            return Ok(Motion::Four(mv));
        }
        let p = self.mv_pred(idx, 0);
        if let Some(sel) = field_sel {
            let mut mv = [[0; 2]; 2];
            for f in &mut mv {
                *f = [read_mv(r, fcode, p[0])?, read_mv(r, fcode, p[1] / 2)?];
            }
            let sx = mv[0][0] + mv[1][0];
            let avg = [(sx >> 1) | (sx & 1), mv[0][1] + mv[1][1]];
            self.set_bmv(idx, [avg; 4]);
            self.mbs[idx] = MbInfo { kind: MbKind::Inter, mv: [avg; 4], field: Some(FieldMv { mv, sel }) };
            return Ok(Motion::Field(mv, sel));
        }
        let mv = [read_mv(r, fcode, p[0])?, read_mv(r, fcode, p[1])?];
        self.set_bmv(idx, [mv; 4]);
        self.mbs[idx] = MbInfo { kind: MbKind::Inter, mv: [mv; 4], field: None };
        Ok(Motion::Frame(mv))
    }

    /// A `not_coded` macroblock of a P- or S-VOP.
    fn skip_p(&mut self, idx: usize, out: &mut Frame) -> Result<()> {
        let mut pix = MbPix::new();
        if self.gmc.is_some() {
            self.gmc_mb(idx, &mut pix);
        } else {
            if let Some(past) = self.refs.past {
                self.predict(past, idx, Motion::Frame([0, 0]), false, &mut pix);
            }
            self.mbs[idx] = MbInfo::SKIPPED;
            self.set_bmv(idx, [[0; 2]; 4]);
        }
        self.store(out, idx, &pix);
        Ok(())
    }

    /// Global motion compensation of a macroblock, and its vector for prediction.
    fn gmc_mb(&mut self, idx: usize, pix: &mut MbPix) {
        let (mx, my) = (idx % self.mb_w, idx / self.mb_w);
        let Some(g) = &self.gmc else { return };
        let mv = g.average_mv(mx, my, self.vol.quarter_sample, self.vop.fcode_f);
        if let Some(past) = self.refs.past {
            g.predict(past.planes[0].view(None), mx, my, false, &mut Dst { out: &mut pix.y, stride: 16, w: 16, h: 16 });
            for (k, c) in pix.c.iter_mut().enumerate() {
                g.predict(past.planes[k + 1].view(None), mx, my, true, &mut Dst { out: c, stride: 8, w: 8, h: 8 });
            }
        }
        self.mbs[idx] = MbInfo { kind: MbKind::Gmc, mv: [mv; 4], field: None };
        self.set_bmv(idx, [mv; 4]);
    }

    /// Motion-compensated prediction of macroblock `idx` from `src`.
    fn predict(&self, src: &Frame, idx: usize, motion: Motion, rounding: bool, pix: &mut MbPix) {
        let (mx, my) = ((idx % self.mb_w) as i32, (idx / self.mb_w) as i32);
        let q = self.vol.quarter_sample;
        let [ly, lu, lv] = &src.planes;
        let chroma = |pix: &mut MbPix, field: Option<usize>, cmv: [i32; 2], f: usize| {
            for (k, p) in [lu, lv].iter().enumerate() {
                let (out, stride, h, y) = match field {
                    None => (&mut pix.c[k][..], 8, 8, my * 8),
                    Some(_) => (&mut pix.c[k][f * 8..], 16, 4, my * 4),
                };
                mc::half(p.view(field), mx * 8, y, cmv, rounding, &mut Dst { out, stride, w: 8, h });
            }
        };
        match motion {
            Motion::Frame(mv) => {
                mc::luma(ly.view(None), mx * 16, my * 16, mv, q, rounding, &mut Dst { out: &mut pix.y, stride: 16, w: 16, h: 16 });
                chroma(pix, None, mc::chroma_mv_1(mv, q), 0);
            }
            Motion::Four(mvs) => {
                for (k, mv) in mvs.iter().enumerate() {
                    let (x, y) = (mx * 16 + (k as i32 & 1) * 8, my * 16 + (k as i32 >> 1) * 8);
                    let out = &mut pix.y[(k >> 1) * 128 + (k & 1) * 8..];
                    mc::luma(ly.view(None), x, y, *mv, q, rounding, &mut Dst { out, stride: 16, w: 8, h: 8 });
                }
                chroma(pix, None, mc::chroma_mv_4(&mvs, q), 0);
            }
            Motion::Field(mvs, sel) => {
                for f in 0..2 {
                    let view = ly.view(Some(sel[f]));
                    if q {
                        for half in 0..2 {
                            let out = &mut pix.y[f * 16 + half * 8..];
                            mc::quarter(view, mx * 16 + half as i32 * 8, my * 8, mvs[f], rounding, &mut Dst { out, stride: 32, w: 8, h: 8 });
                        }
                    } else {
                        mc::half(view, mx * 16, my * 8, mvs[f], rounding, &mut Dst { out: &mut pix.y[f * 16..], stride: 32, w: 16, h: 8 });
                    }
                    chroma(pix, Some(sel[f]), field_chroma_mv(mvs[f], q), f);
                }
            }
        }
    }

    fn mb_b(&mut self, r: &mut BitReader, idx: usize, out: &mut Frame) -> Result<()> {
        let (Some(past), Some(future)) = (self.refs.past, self.refs.future) else {
            return Err(DecodeError::Invalid("B-VOP without two references"));
        };
        if idx.is_multiple_of(self.mb_w) {
            // The vector predictors restart at each row (§7.6.9.3).
            self.last_mv = [[[0; 2]; 2]; 2];
        }
        let col = self.refs.colocated.get(idx).copied().unwrap_or(MbInfo::INTRA);
        let mut pix = MbPix::new();
        if col.kind == MbKind::Skipped {
            self.predict(past, idx, Motion::Frame([0, 0]), false, &mut pix);
            self.store(out, idx, &pix);
            return Ok(());
        }
        // modb '1': direct mode with no vector difference and no residual.
        let modb_skip = bit(r)?;
        let (mb_type, cbp) = if modb_skip {
            (B_DIRECT, 0)
        } else {
            let no_cbp = bit(r)?;
            let t = vlc::mb_type_b().read(r, "invalid B mb_type")?;
            (t, if no_cbp { 0 } else { rd(r, 6)? })
        };
        if mb_type != B_DIRECT && cbp != 0 && bit(r)? {
            self.qp = (self.qp + if bit(r)? { 2 } else { -2 }).clamp(1, 31);
        }
        let mut field_dct = false;
        let mut sel = [[0usize; 2]; 2];
        let mut field = false;
        if self.vol.interlaced {
            field_dct = cbp != 0 && bit(r)?;
            if mb_type != B_DIRECT && bit(r)? {
                field = true;
                for dir in 0..2 {
                    if uses(mb_type, dir) {
                        sel[dir] = [usize::from(bit(r)?), usize::from(bit(r)?)];
                    }
                }
            }
        }
        let motion = if mb_type == B_DIRECT {
            let delta = if modb_skip { [0, 0] } else { [read_mv(r, 1, 0)?, read_mv(r, 1, 0)?] };
            self.direct(col, delta)
        } else {
            let mut m = [None, None];
            for (dir, slot) in m.iter_mut().enumerate() {
                if uses(mb_type, dir) {
                    *slot = Some(self.b_vector(r, dir, field, sel[dir])?);
                }
            }
            m
        };
        match motion {
            [Some(f), Some(b)] => {
                self.predict(past, idx, f, false, &mut pix);
                let mut other = MbPix::new();
                self.predict(future, idx, b, false, &mut other);
                pix.average(&other);
            }
            [Some(f), None] => self.predict(past, idx, f, false, &mut pix),
            [None, Some(b)] => self.predict(future, idx, b, false, &mut pix),
            [None, None] => {}
        }
        self.residual(r, cbp, field_dct, &mut pix)?;
        self.store(out, idx, &pix);
        Ok(())
    }

    /// One direction's vector(s) of a B macroblock, predicted from the last of that direction.
    fn b_vector(&mut self, r: &mut BitReader, dir: usize, field: bool, sel: [usize; 2]) -> Result<Motion> {
        let fcode = if dir == 0 { self.vop.fcode_f } else { self.vop.fcode_b };
        if field {
            let mut mv = [[0; 2]; 2];
            for (f, m) in mv.iter_mut().enumerate() {
                let p = self.last_mv[dir][f];
                *m = [read_mv(r, fcode, p[0])?, read_mv(r, fcode, p[1] / 2)?];
                self.last_mv[dir][f] = [m[0], m[1] * 2];
            }
            return Ok(Motion::Field(mv, sel));
        }
        let p = self.last_mv[dir][0];
        let mv = [read_mv(r, fcode, p[0])?, read_mv(r, fcode, p[1])?];
        self.last_mv[dir] = [mv; 2];
        Ok(Motion::Frame(mv))
    }

    /// Direct-mode vectors from the co-located macroblock (§7.6.9.5).
    fn direct(&self, col: MbInfo, delta: [i32; 2]) -> [Option<Motion>; 2] {
        let t = self.refs.times;
        let vec = |mv: [i32; 2], trb: i32, trd: i32| -> ([i32; 2], [i32; 2]) {
            let x = direct_scale(mv[0], delta[0], trb, trd);
            let y = direct_scale(mv[1], delta[1], trb, trd);
            ([x[0], y[0]], [x[1], y[1]])
        };
        if let (Some(fm), MbKind::Inter) = (col.field, col.kind) {
            let mut fwd = [[0; 2]; 2];
            let mut bwd = [[0; 2]; 2];
            for i in 0..2 {
                let s = fm.sel[i] as i32;
                let k = i as i32;
                let (tpp, tpb) =
                    if self.vop.top_field_first { (t.trd_field - s + k, t.trb_field - s + k) } else { (t.trd_field + s - k, t.trb_field + s - k) };
                (fwd[i], bwd[i]) = vec(fm.mv[i], tpb, tpp);
            }
            return [Some(Motion::Field(fwd, fm.sel)), Some(Motion::Field(bwd, [0, 1]))];
        }
        let mvs = if col.kind == MbKind::Intra { [[0; 2]; 4] } else { col.mv };
        let mut fwd = [[0; 2]; 4];
        let mut bwd = [[0; 2]; 4];
        for k in 0..4 {
            (fwd[k], bwd[k]) = vec(mvs[k], t.trb, t.trd);
        }
        [Some(Motion::Four(fwd)), Some(Motion::Four(bwd))]
    }
}

/// Whether B `mb_type` predicts from the past (`dir` 0) or the future (1).
fn uses(mb_type: u32, dir: usize) -> bool {
    match mb_type {
        B_INTERPOLATE => true,
        B_FORWARD => dir == 0,
        B_BACKWARD => dir == 1,
        _ => false,
    }
}

/// Chrominance vector of a field vector.
fn field_chroma_mv(mv: [i32; 2], quarter_sample: bool) -> [i32; 2] {
    let round = |v: i32| (v >> 1) | (v & 1);
    if quarter_sample { [round(mv[0] / 2), round(mv[1] >> 1)] } else { [round(mv[0]), round(mv[1])] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_are_permutations() {
        for scan in [&ALT_VERTICAL, &ALT_HORIZONTAL, &ZIGZAG] {
            let mut seen = [false; 64];
            for &p in scan.iter() {
                seen[p] = true;
            }
            assert!(seen.iter().all(|&s| s));
        }
        assert_eq!(&ALT_HORIZONTAL[..16], &[0, 1, 2, 3, 8, 9, 16, 17, 10, 11, 4, 5, 6, 7, 15, 14]);
    }

    #[test]
    fn dc_scaler_table() {
        assert_eq!((1..=31).map(|q| dc_scaler(q, true)).collect::<Vec<_>>()[..10], [8, 8, 8, 8, 10, 12, 14, 16, 17, 18]);
        assert_eq!(dc_scaler(31, true), 46);
        assert_eq!(dc_scaler(5, false), 9);
        assert_eq!(dc_scaler(24, false), 18);
        assert_eq!(dc_scaler(31, false), 25);
    }

    #[test]
    fn vector_wraps_into_the_fcode_range() {
        use craft_media::bitstream::BitWriter;
        // motion_code +1 with fcode 1 from a predictor of 31 wraps to -32.
        let mut w = BitWriter::new();
        w.write_bits(0b01, 2);
        w.write_bit(false);
        w.write_bits(0, 16);
        let data = w.finish();
        let mut r = BitReader::new(&data);
        assert_eq!(read_mv(&mut r, 1, 31).ok(), Some(-32));
        assert_eq!(median(3, -1, 7), 3);
        assert_eq!(median(5, 5, 1), 5);
    }
}
