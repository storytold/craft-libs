//! Picture buffers and per-macroblock state.

use std::sync::{Arc, OnceLock};

/// Chroma format and bit depths of a picture (4:2:0 or 4:2:2, 8 to 10 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PicFormat {
    /// 4:2:2 chroma (full-height chroma planes); 4:2:0 otherwise.
    pub c422: bool,
    /// BitDepthY / BitDepthC.
    pub bd_y: u8,
    pub bd_c: u8,
}

impl PicFormat {
    /// 8-bit 4:2:0.
    #[cfg(test)]
    pub const P420_8: PicFormat = PicFormat { c422: false, bd_y: 8, bd_c: 8 };

    /// Samples are stored as `u16` (a bit depth above 8).
    pub fn wide(self) -> bool {
        self.bd_y > 8 || self.bd_c > 8
    }
    /// Largest luma / chroma sample value (Clip1Y / Clip1C bound).
    pub fn max_y(self) -> i32 {
        (1 << self.bd_y.min(16)) - 1
    }
    pub fn max_c(self) -> i32 {
        (1 << self.bd_c.min(16)) - 1
    }
    /// Mid-grey luma / chroma value, 1 << (BitDepth - 1).
    pub fn mid_y(self) -> i32 {
        1 << self.bd_y.clamp(1, 16).saturating_sub(1)
    }
    pub fn mid_c(self) -> i32 {
        1 << self.bd_c.clamp(1, 16).saturating_sub(1)
    }
    /// Chroma lines per macroblock (MbHeightC: 8 for 4:2:0, 16 for 4:2:2).
    pub fn mbhc(self) -> usize {
        if self.c422 { 16 } else { 8 }
    }
}

/// Sample storage type: `u8` for 8-bit pictures, `u16` above. Every operation that writes a
/// sample clips to the plane's bit depth (`max`); the `u8` versions ignore `max`, which is
/// always 255 for them.
pub trait Sample: Copy + Default + PartialEq + Eq + std::fmt::Debug + Send + Sync + 'static {
    /// Unclipped first-pass 6-tap luma interpolation value: `i16` holds it for 8-bit samples
    /// (-2550..=10710), wider samples need `i32`.
    type Tap: Copy + Default + std::fmt::Debug;
    /// Dequantised coefficients beyond +-R_MAX leave the inverse transforms' output unspecified
    /// (2^(7 + bit depth), 8.5.12.1); larger values are clamped to keep the i32 butterflies exact.
    const R_MAX: i32;
    fn to_i32(self) -> i32;
    /// `v` clipped to 0..=`max`.
    fn clip(v: i32, max: i32) -> Self;
    /// `v`, already within the sample range (an average of samples), in the storage type.
    fn from_i32(v: i32) -> Self;
    /// The eight samples p3 .. q3 of each of `N` lines as eight lanes of `N` values (the
    /// deblocking filter's layout), and back.
    #[inline(always)]
    fn to_lanes<const N: usize>(lines: &[[Self; 8]; N], s: &mut [[i16; N]; 8]) {
        for (k, row) in s.iter_mut().enumerate() {
            for i in 0..N {
                row[i] = lines[i][k].to_i32() as i16;
            }
        }
    }
    #[inline(always)]
    fn from_lanes<const N: usize>(s: &[[i16; N]; 8], lines: &mut [[Self; 8]; N]) {
        for (i, line) in lines.iter_mut().enumerate() {
            for k in 0..8 {
                line[k] = Self::from_i32(s[k][i] as i32);
            }
        }
    }
    /// Rounded average of two samples.
    fn avg(a: Self, b: Self) -> Self;
    /// a - 5b + 20c + 20d - 5e + f.
    fn tap6(s: [Self; 6]) -> Self::Tap;
    fn tap_i32(t: Self::Tap) -> i32;
    /// Clip1((t + 16) >> 5).
    fn round5(t: Self::Tap, max: i32) -> Self;
    /// ((w0 s0 + w1 s1 + w2 s2 + w3 s3) + 32) >> 6 with weights summing to 64 (chroma interpolation).
    fn bilin(w: [u16; 4], s: [Self; 4]) -> Self;
    /// An empty buffer with room for `n` samples (output planes).
    fn alloc(n: usize) -> Vec<Self>;
    /// The rows of `f` if they are stored with this type.
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<Self>>]>;
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<Self>>]>) -> Rows;
}

impl Sample for u8 {
    type Tap = i16;
    const R_MAX: i32 = 1 << 15;
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn clip(v: i32, _max: i32) -> Self {
        v.clamp(0, 255) as u8
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
    // Each line as one little-endian word, byte k = sample k: shifts move whole lines at once.
    #[inline(always)]
    fn to_lanes<const N: usize>(lines: &[[u8; 8]; N], s: &mut [[i16; N]; 8]) {
        let mut w = [0u64; N];
        for (w, l) in w.iter_mut().zip(lines) {
            *w = u64::from_le_bytes(*l);
        }
        for (k, row) in s.iter_mut().enumerate() {
            for i in 0..N {
                row[i] = ((w[i] >> (8 * k)) & 0xff) as i16;
            }
        }
    }
    #[inline(always)]
    fn from_lanes<const N: usize>(s: &[[i16; N]; 8], lines: &mut [[u8; 8]; N]) {
        for (i, l) in lines.iter_mut().enumerate() {
            *l = (0..8).fold(0u64, |acc, k| acc | ((s[k][i] as u8 as u64) << (8 * k))).to_le_bytes();
        }
    }
    #[inline(always)]
    fn avg(a: Self, b: Self) -> Self {
        ((a as u16 + b as u16 + 1) >> 1) as u8
    }
    #[inline(always)]
    fn tap6(s: [Self; 6]) -> i16 {
        let [a, b, c, d, e, f] = s.map(i16::from);
        a - 5 * b + 20 * c + 20 * d - 5 * e + f
    }
    #[inline(always)]
    fn tap_i32(t: i16) -> i32 {
        t as i32
    }
    #[inline(always)]
    fn round5(t: i16, _max: i32) -> Self {
        ((t + 16) >> 5).clamp(0, 255) as u8
    }
    #[inline(always)]
    fn bilin(w: [u16; 4], s: [Self; 4]) -> Self {
        let v = w[0] * s[0] as u16 + w[1] * s[1] as u16 + w[2] * s[2] as u16 + w[3] * s[3] as u16;
        ((v + 32) >> 6) as u8
    }
    fn alloc(n: usize) -> Vec<Self> {
        plane_allocator()(n)
    }
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<Self>>]> {
        match &f.rows {
            Rows::U8(r) => Some(r),
            Rows::U16(_) => None,
        }
    }
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<Self>>]>) -> Rows {
        Rows::U8(rows)
    }
}

impl Sample for u16 {
    type Tap = i32;
    const R_MAX: i32 = 1 << 21;
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn clip(v: i32, max: i32) -> Self {
        v.max(0).min(max) as u16
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
    #[inline(always)]
    fn avg(a: Self, b: Self) -> Self {
        ((a as u32 + b as u32 + 1) >> 1) as u16
    }
    #[inline(always)]
    fn tap6(s: [Self; 6]) -> i32 {
        let [a, b, c, d, e, f] = s.map(i32::from);
        a - 5 * b + 20 * c + 20 * d - 5 * e + f
    }
    #[inline(always)]
    fn tap_i32(t: i32) -> i32 {
        t
    }
    #[inline(always)]
    fn round5(t: i32, max: i32) -> Self {
        ((t + 16) >> 5).max(0).min(max) as u16
    }
    #[inline(always)]
    fn bilin(w: [u16; 4], s: [Self; 4]) -> Self {
        let v = w[0] as u32 * s[0] as u32 + w[1] as u32 * s[1] as u32 + w[2] as u32 * s[2] as u32 + w[3] as u32 * s[3] as u32;
        ((v + 32) >> 6) as u16
    }
    fn alloc(n: usize) -> Vec<Self> {
        Vec::with_capacity(n)
    }
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<Self>>]> {
        match &f.rows {
            Rows::U16(r) => Some(r),
            Rows::U8(_) => None,
        }
    }
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<Self>>]>) -> Rows {
        Rows::U16(rows)
    }
}

/// Planar sample buffers, 4:2:0 or 4:2:2 (MB-aligned dimensions, stride == width).
#[derive(Clone)]
pub struct Planes<S: Sample> {
    pub y: Vec<S>,
    pub cb: Vec<S>,
    pub cr: Vec<S>,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub fmt: PicFormat,
}

impl<S: Sample> Planes<S> {
    /// Black luma, mid-grey chroma.
    pub fn new(width: usize, height: usize, fmt: PicFormat) -> Self {
        let (cw, ch) = (width / 2, if fmt.c422 { height } else { height / 2 });
        let mid = S::clip(fmt.mid_c(), fmt.max_c());
        Planes { y: vec![S::default(); width * height], cb: vec![mid; cw * ch], cr: vec![mid; cw * ch], width, height, cwidth: cw, fmt }
    }
    /// Mid-grey everywhere.
    pub fn gray(width: usize, height: usize, fmt: PicFormat) -> Self {
        let mut p = Self::new(width, height, fmt);
        p.y.fill(S::clip(fmt.mid_y(), fmt.max_y()));
        p
    }
    pub fn c422(&self) -> bool {
        self.fmt.c422
    }
    /// Chroma lines per macroblock (8 for 4:2:0, 16 for 4:2:2).
    pub fn mbhc(&self) -> usize {
        self.fmt.mbhc()
    }
}

/// Macroblock coding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MbKind {
    #[default]
    None,
    I4x4,
    I8x8,
    I16x16,
    IPcm,
    PSkip,
    BSkip,
    BDirect16x16,
    /// Any other P or B inter macroblock.
    Inter,
}

impl MbKind {
    #[inline]
    pub fn is_intra(self) -> bool {
        matches!(self, MbKind::I4x4 | MbKind::I8x8 | MbKind::I16x16 | MbKind::IPcm)
    }
}

/// Per-macroblock state kept for the whole picture (neighbour derivations, deblocking, co-located data).
#[derive(Clone, Copy)]
pub struct MbState {
    /// Slice number within the picture; `u32::MAX` = not (yet) decoded.
    pub slice_num: u32,
    pub kind: MbKind,
    pub transform_8x8: bool,
    /// coded_block_pattern: bits 0..3 luma, bits 4..5 chroma.
    pub cbp: u8,
    /// QPY (negative down to -QpBdOffsetY above 8 bits).
    pub qp: i8,
    /// QPc for Cb / Cr (used by deblocking).
    pub qpc: [i8; 2],
    pub intra_chroma_mode: u8,
    /// Intra4x4PredMode / Intra8x8PredMode per 4x4 block (raster order).
    pub intra_modes: [u8; 16],
    /// CAVLC: TotalCoeff per luma 4x4 block (raster). CABAC: coded_block_flag.
    pub nnz: [u8; 16],
    /// Same for chroma AC blocks [Cb, Cr][raster 2x2 (4:2:0) or 2x4 (4:2:2)].
    pub nnz_c: [[u8; 8]; 2],
    /// CABAC coded_block_flag of DC blocks: bit0 luma (I16x16), bit1 Cb, bit2 Cr.
    pub cbf_dc: u8,
    /// Luma 4x4 blocks (raster bit) with non-zero coefficients, for deblocking bS = 2.
    pub nz_mask: u16,
    pub ref_idx: [[i8; 4]; 2],
    /// Motion vectors per 4x4 block (raster order).
    pub mv: [[[i16; 2]; 16]; 2],
    /// |mvd| per 4x4 block (raster), saturated, for CABAC context selection.
    pub mvd: [[[u8; 2]; 16]; 2],
    /// Bit per 8x8 block predicted in direct mode.
    pub direct8x8: u8,
}

impl MbState {
    #[inline]
    pub fn kind_is_skip(&self) -> bool {
        matches!(self.kind, MbKind::PSkip | MbKind::BSkip)
    }
}

impl Default for MbState {
    fn default() -> Self {
        MbState {
            slice_num: u32::MAX,
            kind: MbKind::None,
            transform_8x8: false,
            cbp: 0,
            qp: 0,
            qpc: [0; 2],
            intra_chroma_mode: 0,
            intra_modes: [2; 16],
            nnz: [0; 16],
            nnz_c: [[0; 8]; 2],
            cbf_dc: 0,
            nz_mask: 0,
            ref_idx: [[-1; 4]; 2],
            mv: [[[0; 2]; 16]; 2],
            mvd: [[[0; 2]; 16]; 2],
            direct8x8: 0,
        }
    }
}

/// Motion data of one published macroblock row, the co-located information of direct prediction.
pub struct RowMotion {
    /// Per 4x4 block: index mb_x * 16 + raster.
    pub mv: [Box<[[i16; 2]]>; 2],
    /// Per 8x8 block: index mb_x * 4 + b8.
    pub ref_idx: [Box<[i8]>; 2],
    /// Unique id of the referenced picture per 8x8 block (u32::MAX = none).
    pub ref_id: [Box<[u32]>; 2],
    pub intra: Box<[bool]>,
}

impl RowMotion {
    /// Intra macroblocks with no motion.
    fn intra(mb_w: usize) -> Self {
        let per_list = |n: usize| -> [Box<[u32]>; 2] { [vec![u32::MAX; n].into(), vec![u32::MAX; n].into()] };
        RowMotion {
            mv: [vec![[0; 2]; mb_w * 16].into(), vec![[0; 2]; mb_w * 16].into()],
            ref_idx: [vec![-1; mb_w * 4].into(), vec![-1; mb_w * 4].into()],
            ref_id: per_list(mb_w * 4),
            intra: vec![true; mb_w].into(),
        }
    }
}

/// One published macroblock row of a decoded frame: final (deblocked) samples plus its motion data.
pub struct FrameRow<S: Sample> {
    /// 16 luma lines.
    pub y: Box<[S]>,
    /// 8 (4:2:0) or 16 (4:2:2) lines per chroma component.
    pub cb: Box<[S]>,
    pub cr: Box<[S]>,
    pub motion: RowMotion,
}

/// The rows of a frame, in the frame's sample type.
pub enum Rows {
    U8(Box<[OnceLock<FrameRow<u8>>]>),
    U16(Box<[OnceLock<FrameRow<u16>>]>),
}

/// A decoded (or in-progress) frame as stored in the DPB and referenced by later pictures.
///
/// Rows are published in order by the thread decoding the picture; readers block (per row) until the
/// data they need is available, which lets several pictures decode concurrently.
pub struct Frame {
    pub id: u32,
    pub poc: i32,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub cheight: usize,
    pub mb_w: usize,
    pub fmt: PicFormat,
    rows: Rows,
}

impl Frame {
    pub fn new(id: u32, poc: i32, mb_w: usize, mb_h: usize, fmt: PicFormat) -> Self {
        let rows = if fmt.wide() {
            u16::wrap_rows((0..mb_h).map(|_| OnceLock::new()).collect())
        } else {
            u8::wrap_rows((0..mb_h).map(|_| OnceLock::new()).collect())
        };
        Frame { id, poc, width: mb_w * 16, height: mb_h * 16, cwidth: mb_w * 8, cheight: mb_h * fmt.mbhc(), mb_w, fmt, rows }
    }

    pub fn mb_h(&self) -> usize {
        match &self.rows {
            Rows::U8(r) => r.len(),
            Rows::U16(r) => r.len(),
        }
    }

    pub fn c422(&self) -> bool {
        self.fmt.c422
    }

    /// Sample bytes this picture holds (luma and both chroma planes), as the decoder's memory
    /// budget counts a picture.
    pub fn bytes(&self) -> u64 {
        let samples = (self.width as u64)
            .saturating_mul(self.height as u64)
            .saturating_add((self.cwidth as u64).saturating_mul(self.cheight as u64).saturating_mul(2));
        samples.saturating_mul(if self.fmt.wide() { 2 } else { 1 })
    }

    /// Publish MB row `r` (ignored if already published, or if `S` is not this frame's sample type).
    pub fn publish<S: Sample>(&self, r: usize, row: FrameRow<S>) {
        if let Some(slot) = S::rows(self).and_then(|rows| rows.get(r)) {
            let _ = slot.set(row);
        }
    }

    pub fn is_published(&self, r: usize) -> bool {
        match &self.rows {
            Rows::U8(rows) => rows.get(r).is_some_and(|x| x.get().is_some()),
            Rows::U16(rows) => rows.get(r).is_some_and(|x| x.get().is_some()),
        }
    }

    /// MB row `r`, waiting until it has been published. None when `r` is out of range or `S`
    /// is not this frame's sample type (a reference of another bit depth).
    #[inline]
    pub fn row<S: Sample>(&self, r: usize) -> Option<&FrameRow<S>> {
        let slot = S::rows(self)?.get(r)?;
        #[cfg(feature = "threads")]
        {
            Some(slot.wait())
        }
        #[cfg(not(feature = "threads"))]
        {
            // Rows are decoded in order here, so a row is published before it is read; one that
            // is not reads as gray with no motion instead of panicking.
            Some(slot.get_or_init(|| self.gray_row()))
        }
    }

    /// Motion data of MB row `r`, waiting until it has been published.
    pub fn motion(&self, r: usize) -> Option<&RowMotion> {
        match &self.rows {
            Rows::U8(_) => self.row::<u8>(r).map(|x| &x.motion),
            Rows::U16(_) => self.row::<u16>(r).map(|x| &x.motion),
        }
    }

    /// A gray intra row with no motion, what "non-existing" frames publish.
    #[cfg(not(feature = "threads"))]
    fn gray_row<S: Sample>(&self) -> FrameRow<S> {
        let (y, c) = (S::clip(self.fmt.mid_y(), self.fmt.max_y()), S::clip(self.fmt.mid_c(), self.fmt.max_c()));
        let hc = self.fmt.mbhc();
        FrameRow {
            y: vec![y; self.width * 16].into(),
            cb: vec![c; self.cwidth * hc].into(),
            cr: vec![c; self.cwidth * hc].into(),
            motion: RowMotion::intra(self.mb_w),
        }
    }

    /// Wait until the whole frame is available.
    pub fn wait_complete(&self) {
        for r in 0..self.mb_h() {
            self.motion(r);
        }
    }

    /// Build a published row from a contiguous plane set (rows of MB row `r`) and motion data.
    pub fn make_row<S: Sample>(planes: &Planes<S>, r: usize, mbs: &[MbState], ref_ids: &dyn Fn(&MbState, usize, i8) -> u32) -> FrameRow<S> {
        let w = planes.width;
        let cw = planes.cwidth;
        let mb_w = w / 16;
        let hc = planes.mbhc();
        let part = |p: &[S], n: usize| -> Box<[S]> { p.get(r * n..(r + 1) * n).unwrap_or_default().into() };
        let (y, cb, cr) = (part(&planes.y, 16 * w), part(&planes.cb, hc * cw), part(&planes.cr, hc * cw));
        let mut motion = RowMotion::intra(mb_w);
        let row_mbs = mbs.get(r * mb_w..(r + 1) * mb_w).unwrap_or_default();
        for (x, st) in row_mbs.iter().enumerate() {
            if st.slice_num == u32::MAX {
                continue;
            }
            motion.intra[x] = st.kind.is_intra();
            for l in 0..2 {
                motion.mv[l][x * 16..x * 16 + 16].copy_from_slice(&st.mv[l]);
                for b in 0..4 {
                    let ri = st.ref_idx[l][b];
                    motion.ref_idx[l][x * 4 + b] = ri;
                    if ri >= 0 {
                        motion.ref_id[l][x * 4 + b] = ref_ids(st, l, ri);
                    }
                }
            }
        }
        FrameRow { y, cb, cr, motion }
    }

    /// A frame with every row published from `planes` and no motion (intra), e.g. "non-existing"
    /// frames inferred for frame_num gaps.
    pub fn from_planes<S: Sample>(id: u32, poc: i32, planes: &Planes<S>) -> Self {
        let (mb_w, mb_h) = (planes.width / 16, planes.height / 16);
        let f = Frame::new(id, poc, mb_w, mb_h, planes.fmt);
        let blank = vec![MbState::default(); mb_w * mb_h];
        for r in 0..mb_h {
            f.publish(r, Frame::make_row(planes, r, &blank, &|_, _, _| u32::MAX));
        }
        f
    }

    /// Copy the w x h luma window at (x0, y0) (clamped to the picture) into `out` (stride `os`).
    #[inline]
    pub fn luma_window<S: Sample>(&self, (x0, y0): (i32, i32), w: usize, h: usize, out: &mut [S], os: usize) {
        let (width, height) = (self.width, self.height);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, height as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> 4) else { return };
            let line = &row.y[(yy & 15) * width..(yy & 15) * width + width];
            copy_line::<S, 24>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Copy the w x h window of chroma component `c` at (x0, y0) (clamped) into `out`.
    #[inline]
    pub fn chroma_window<S: Sample>(&self, c: usize, (x0, y0): (i32, i32), w: usize, h: usize, out: &mut [S], os: usize) {
        let (cw, ch) = (self.cwidth, self.cheight);
        let (shift, mask) = if self.c422() { (4, 15) } else { (3, 7) };
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, ch as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> shift) else { return };
            let plane = if c == 0 { &row.cb } else { &row.cr };
            let line = &plane[(yy & mask) * cw..(yy & mask) * cw + cw];
            copy_line::<S, 16>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Crop and copy the frame into planar buffers (waits for completion). None when `S` is not
    /// the frame's sample type.
    pub fn copy_cropped<S: Sample>(&self, crop: (usize, usize, usize, usize)) -> Option<(Vec<S>, Vec<S>, Vec<S>)> {
        let (cx, cy, cw, ch) = crop;
        S::rows(self)?;
        let mut y = S::alloc(cw * ch);
        for r in cy..cy + ch {
            let row = self.row::<S>(r >> 4)?;
            y.extend_from_slice(row.y.get((r & 15) * self.width + cx..(r & 15) * self.width + cx + cw)?);
        }
        let c422 = self.c422();
        let (ccx, ccw) = (cx / 2, cw.div_ceil(2));
        let (ccy, cch) = if c422 { (cy, ch) } else { (cy / 2, ch.div_ceil(2)) };
        let (shift, mask) = if c422 { (4, 15) } else { (3, 7) };
        let mut u = S::alloc(ccw * cch);
        let mut v = S::alloc(ccw * cch);
        for r in ccy..ccy + cch {
            let row = self.row::<S>(r >> shift)?;
            let o = (r & mask) * self.cwidth + ccx;
            u.extend_from_slice(row.cb.get(o..o + ccw)?);
            v.extend_from_slice(row.cr.get(o..o + ccw)?);
        }
        Some((y, u, v))
    }
}

static PLANE_ALLOCATOR: std::sync::OnceLock<fn(usize) -> Vec<u8>> = std::sync::OnceLock::new();

/// Where 8-bit output planes come from: an empty buffer with room for the given number of samples.
/// The host sets it once to recycle the planes of pictures it is done with; the first call wins.
pub fn set_plane_allocator(alloc: fn(usize) -> Vec<u8>) {
    let _ = PLANE_ALLOCATOR.set(alloc);
}

fn plane_allocator() -> fn(usize) -> Vec<u8> {
    PLANE_ALLOCATOR.get().copied().unwrap_or(Vec::with_capacity)
}

/// Copy `w` samples of `line` starting at x0 into `out`; when possible a fixed-size block of `N`
/// samples is copied instead (cheaper than a variable-length copy; `out` must then have room for `N`).
#[inline(always)]
fn copy_line<S: Sample, const N: usize>(line: &[S], x0: i32, out: &mut [S], w: usize) {
    if w <= N
        && x0 >= 0
        && let Some(src) = line.get(x0 as usize..).and_then(|l| l.first_chunk::<N>())
        && let Some(dst) = out.first_chunk_mut::<N>()
    {
        *dst = *src;
    } else {
        copy_clamped(line, x0, &mut out[..w]);
    }
}

/// Copy `out.len()` samples of `line` starting at x0, replicating edge samples outside the line.
/// Kept out of line so the fixed-size path of `copy_line` stays an inline move rather than being
/// merged with this variable-length copy into a libc memcpy call.
#[cold]
#[inline(never)]
fn copy_clamped<S: Sample>(line: &[S], x0: i32, out: &mut [S]) {
    let w = out.len();
    if x0 >= 0 && x0 as usize + w <= line.len() {
        out.copy_from_slice(&line[x0 as usize..x0 as usize + w]);
    } else {
        let max = line.len() as i32 - 1;
        for (c, o) in out.iter_mut().enumerate() {
            *o = line[(x0 + c as i32).clamp(0, max) as usize];
        }
    }
}

pub type FrameRef = Arc<Frame>;

/// An entry of RefPicList0/1 for the current slice.
#[derive(Clone)]
pub struct RefPic {
    pub frame: FrameRef,
    pub long_term: bool,
}

impl RefPic {
    pub fn poc(&self) -> i32 {
        self.frame.poc
    }
    pub fn id(&self) -> u32 {
        self.frame.id
    }
}

#[cfg(all(test, not(feature = "threads")))]
mod row_tests {
    use super::*;

    /// Without threads a row read before it was published is gray, not a panic.
    #[test]
    fn an_unpublished_row_reads_as_gray_without_threads() {
        let f = Frame::new(0, 0, 2, 2, PicFormat::P420_8);
        let row = f.row::<u8>(1).unwrap();
        assert!(row.y.iter().chain(row.cb.iter()).chain(row.cr.iter()).all(|&s| s == 128));
        let m = &row.motion;
        assert_eq!((row.y.len(), row.cb.len(), m.mv[0].len(), m.ref_idx[1].len(), m.intra.len()), (32 * 16, 16 * 8, 32, 8, 2));
        assert!(m.intra.iter().all(|&i| i) && m.ref_id[0].iter().all(|&id| id == u32::MAX));
    }
}
