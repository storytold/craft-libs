//! FFV1 lossless video codec (RFC 9043): version 3, range coder, intra-only, YCbCr and RGB (JPEG 2000
//! RCT), 8 to 16 bits per sample, any number of slices.
//!
//! - [`Decoder`]: built from the *configuration record* a container stores (Matroska `CodecPrivate`,
//!   AVI `strf` extension, MP4 `glbl`) and the frame size; [`Decoder::decode`] turns one packet into a
//!   [`Frame`]. Slice CRCs are checked.
//! - [`Encoder`]: [`Encoder::config_record`] gives the bytes for the container, [`Encoder::encode`]
//!   one packet per frame; every frame is a keyframe.
//!
//! Not supported (reported as [`Error::Unsupported`]): the Golomb-Rice coder (`coder_type` 0),
//! versions 0 and 1 (no configuration record, parameters in every keyframe), inter frames
//! (`intra` 0 with `keyframe` 0), and slice rasters whose boundaries are not multiples of the
//! chroma subsampling. See `src/ffv1/README.md`.
//!
//! Implemented from RFC 9043. Frames are planar `u16`: for YCbCr, `[Y, Cb, Cr]` (plus an optional
//! transparency plane); for RGB, `[R, G, B]` (plus transparency), all at full size.

mod rc;

use crate::budget::DECODE_MEMORY_BUDGET;
use rc::{Decoder as Rac, Encoder as RacEnc, Transitions};

const CONTEXT_SIZE: usize = 32;
const MAX_CONTEXT_INPUTS: usize = 5;
const MAX_CONTEXTS: u64 = 32768;

/// Codec errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The data ends early or its sizes do not add up.
    #[error("truncated FFV1 data")]
    Truncated,
    /// The data is not valid FFV1.
    #[error("invalid FFV1 data: {0}")]
    Invalid(&'static str),
    /// Valid FFV1 that this module does not decode.
    #[error("unsupported FFV1 feature: {0}")]
    Unsupported(&'static str),
    /// A slice or the configuration record failed its CRC.
    #[error("FFV1 CRC mismatch in {0}")]
    Crc(&'static str),
    /// The encoder was given a frame or settings it cannot encode.
    #[error("invalid encoder input: {0}")]
    Input(String),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The colour space coded (RFC 9043 table 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colorspace {
    /// YCbCr, coded plane by plane without a pixel transform.
    YCbCr,
    /// RGB through the JPEG 2000 reversible colour transform, coded line by line.
    Rgb,
}

/// One frame of planar samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Colour space of the planes.
    pub colorspace: Colorspace,
    /// Bits per sample, 8 to 16.
    pub bit_depth: u8,
    /// log2 of the horizontal and vertical chroma subsampling (YCbCr only; 0, 0 is 4:4:4).
    pub chroma_shift: (u8, u8),
    /// The planes, in the order the module doc gives.
    pub planes: Vec<Vec<u16>>,
}

impl Frame {
    /// Whether the planes are Y, Cb, Cr (or R, G, B) rather than Y (gray) alone, or with transparency.
    fn plane_count_ok(&self) -> bool {
        matches!(self.planes.len(), 1..=4)
    }

    fn dims(&self, plane: usize, chroma_planes: bool) -> (usize, usize) {
        plane_dims(self.width as usize, self.height as usize, self.colorspace, self.chroma_shift, plane, chroma_planes)
    }
}

/// `(width, height)` of plane `p` for a full frame (or a slice) of `w` x `h`.
fn plane_dims(w: usize, h: usize, cs: Colorspace, shift: (u8, u8), p: usize, chroma_planes: bool) -> (usize, usize) {
    if cs == Colorspace::YCbCr && chroma_planes && (p == 1 || p == 2) { (w.div_ceil(1 << shift.0), h.div_ceil(1 << shift.1)) } else { (w, h) }
}

/// One Quantization Table Set (section 4.1).
#[derive(Clone)]
struct QuantSet {
    tables: Vec<[i32; 256]>,
    context_count: usize,
}

/// The parsed "Parameters" of a stream.
#[derive(Clone)]
struct Params {
    coder_type: u32,
    transitions: Transitions,
    colorspace: Colorspace,
    bits: u32,
    chroma_planes: bool,
    shift: (u8, u8),
    extra_plane: bool,
    num_h: u32,
    num_v: u32,
    sets: Vec<QuantSet>,
    initial_states: Vec<Option<Vec<[u8; CONTEXT_SIZE]>>>,
    ec: bool,
    intra: bool,
}

impl Params {
    fn plane_count(&self) -> usize {
        1 + if self.chroma_planes { 2 } else { 0 } + usize::from(self.extra_plane)
    }

    /// Quantization Table Set index slot of plane `p` (section 3.6).
    fn qset_slot(&self, p: usize) -> usize {
        match p {
            0 => 0,
            1 | 2 if self.chroma_planes => 1,
            _ => 2,
        }
    }
}

fn crc32(data: &[u8]) -> u32 {
    // The IEEE polynomial 0x04C11DB7, initial value 0, no reflection, no final inversion.
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { crc << 1 ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

// ---- Parameters ----

fn read_quant_set(rac: &mut Rac, t: &Transitions) -> Result<QuantSet> {
    let mut scale: u64 = 1;
    let mut tables = Vec::with_capacity(MAX_CONTEXT_INPUTS);
    for _ in 0..MAX_CONTEXT_INPUTS {
        let mut table = [0i32; 256];
        // Each quantization table starts from fresh states.
        let mut state = [128u8; CONTEXT_SIZE];
        let mut v: i64 = 0;
        let mut k = 0usize;
        while k < 128 {
            let len = rac.symbol(&mut state, false, t) as usize + 1;
            if k + len > 128 {
                return Err(Error::Invalid("quantization table run passes its half"));
            }
            for _ in 0..len {
                table[k] = i32::try_from(scale as i64 * v).map_err(|_| Error::Invalid("quantization table overflows"))?;
                k += 1;
            }
            v += 1;
        }
        for k in 1..128 {
            table[256 - k] = -table[k];
        }
        table[128] = -table[127];
        tables.push(table);
        scale = scale.checked_mul(2 * v as u64 - 1).filter(|&s| s <= 2 * MAX_CONTEXTS).ok_or(Error::Invalid("too many contexts"))?;
    }
    let context_count = scale.div_ceil(2);
    if context_count > MAX_CONTEXTS {
        return Err(Error::Invalid("too many contexts"));
    }
    Ok(QuantSet { tables, context_count: context_count as usize })
}

fn parse_params(rac: &mut Rac) -> Result<Params> {
    let mut state = [128u8; CONTEXT_SIZE];
    // Until the custom table is known, the default table decodes the fields before it.
    let t0 = Transitions::default_table();
    let version = rac.symbol(&mut state, false, &t0);
    if version != 3 {
        return Err(Error::Unsupported("only FFV1 version 3 has a configuration record; versions 0 and 1 are not supported"));
    }
    let _micro = rac.symbol(&mut state, false, &t0);
    let coder_type = rac.symbol(&mut state, false, &t0) as u32;
    let mut delta = [0i32; 256];
    if coder_type > 1 {
        for d in delta.iter_mut().skip(1) {
            *d = rac.symbol(&mut state, true, &t0);
        }
    }
    let transitions = Transitions::new(&delta);
    if coder_type == 0 {
        return Err(Error::Unsupported("the Golomb-Rice coder (coder_type 0)"));
    }
    if coder_type > 2 {
        return Err(Error::Invalid("unknown coder_type"));
    }
    let colorspace = match rac.symbol(&mut state, false, &t0) {
        0 => Colorspace::YCbCr,
        1 => Colorspace::Rgb,
        _ => return Err(Error::Unsupported("colorspace_type")),
    };
    let bits = rac.symbol(&mut state, false, &t0) as u32;
    let bits = if bits == 0 { 8 } else { bits };
    let chroma_planes = rac.bit(&mut state[0], &t0);
    let (h, v) = (rac.symbol(&mut state, false, &t0), rac.symbol(&mut state, false, &t0));
    let extra_plane = rac.bit(&mut state[0], &t0);
    let num_h = rac.symbol(&mut state, false, &t0) as u32 + 1;
    let num_v = rac.symbol(&mut state, false, &t0) as u32 + 1;
    let set_count = rac.symbol(&mut state, false, &t0);
    if !(8..=16).contains(&bits) || h > 3 || v > 3 || h < 0 || v < 0 || num_h > 1024 || num_v > 1024 || !(1..=8).contains(&set_count) {
        return Err(Error::Invalid("parameter out of range"));
    }
    if colorspace == Colorspace::Rgb && (!chroma_planes || h != 0 || v != 0) {
        return Err(Error::Unsupported("RGB with chroma_planes 0 or subsampling"));
    }
    let mut sets = Vec::new();
    for _ in 0..set_count {
        // The Parameters are range coded with the default table; the custom one is for the slices.
        sets.push(read_quant_set(rac, &t0)?);
    }
    let mut initial_states = Vec::new();
    for set in &sets {
        if rac.bit(&mut state[0], &t0) {
            let mut all = Vec::with_capacity(set.context_count);
            let mut prev = [128u8; CONTEXT_SIZE];
            for _ in 0..set.context_count {
                let mut cur = [0u8; CONTEXT_SIZE];
                for k in 0..CONTEXT_SIZE {
                    let d = rac.symbol(&mut state, true, &t0);
                    cur[k] = (i32::from(prev[k]) + d) as u8;
                }
                prev = cur;
                all.push(cur);
            }
            initial_states.push(Some(all));
        } else {
            initial_states.push(None);
        }
    }
    let ec = rac.symbol(&mut state, false, &t0) != 0;
    let intra = rac.symbol(&mut state, false, &t0) != 0;
    Ok(Params {
        coder_type,
        transitions,
        colorspace,
        bits,
        chroma_planes,
        shift: (h as u8, v as u8),
        extra_plane,
        num_h,
        num_v,
        sets,
        initial_states,
        ec,
        intra,
    })
}

/// The default quantisation tables of this encoder: 11 levels for the two gradients next to the
/// sample and 5 for the other three, which makes 15125 contexts (FFmpeg refuses more than 32768).
fn default_quant_set() -> QuantSet {
    // |difference| thresholds: the first |d| of each level above 0 (levels 0..=5 and 0..=2).
    let first_of_level: [&[usize]; MAX_CONTEXT_INPUTS] = [&[1, 2, 4, 8, 16], &[1, 2, 4, 8, 16], &[1, 3], &[1, 3], &[1, 3]];
    let mut scale: i32 = 1;
    let mut tables = Vec::new();
    let mut counts = [0usize; MAX_CONTEXT_INPUTS];
    for (j, firsts) in first_of_level.iter().enumerate() {
        let mut table = [0i32; 256];
        for (k, slot) in table.iter_mut().take(128).enumerate() {
            let level = firsts.iter().filter(|&&f| k >= f).count() as i32;
            *slot = scale * level;
        }
        for k in 1..128 {
            table[256 - k] = -table[k];
        }
        table[128] = -table[127];
        counts[j] = firsts.len() + 1;
        scale *= 2 * counts[j] as i32 - 1;
        tables.push(table);
    }
    QuantSet { tables, context_count: (scale as usize).div_ceil(2) }
}

/// Run lengths (`len - 1` per step) that describe the first half of a table, as written to the stream.
fn quant_runs(table: &[i32; 256]) -> Vec<i32> {
    let mut runs = Vec::new();
    let mut k = 0usize;
    while k < 128 {
        let start = k;
        while k < 128 && table[k] == table[start] {
            k += 1;
        }
        runs.push((k - start) as i32 - 1);
    }
    runs
}

fn write_params(enc: &mut RacEnc, p: &Params) {
    let t = Transitions::default_table();
    let mut state = [128u8; CONTEXT_SIZE];
    enc.symbol(&mut state, 3, false, &t); // version
    enc.symbol(&mut state, 4, false, &t); // micro_version: first stable variant
    enc.symbol(&mut state, p.coder_type as i32, false, &t);
    enc.symbol(&mut state, if p.colorspace == Colorspace::YCbCr { 0 } else { 1 }, false, &t);
    enc.symbol(&mut state, p.bits as i32, false, &t);
    enc.bit(p.chroma_planes, &mut state[0], &t);
    enc.symbol(&mut state, i32::from(p.shift.0), false, &t);
    enc.symbol(&mut state, i32::from(p.shift.1), false, &t);
    enc.bit(p.extra_plane, &mut state[0], &t);
    enc.symbol(&mut state, p.num_h as i32 - 1, false, &t);
    enc.symbol(&mut state, p.num_v as i32 - 1, false, &t);
    enc.symbol(&mut state, p.sets.len() as i32, false, &t);
    for set in &p.sets {
        for table in &set.tables {
            let mut qstate = [128u8; CONTEXT_SIZE];
            for run in quant_runs(table) {
                enc.symbol(&mut qstate, run, false, &t);
            }
        }
    }
    for _ in &p.sets {
        enc.bit(false, &mut state[0], &t); // states_coded
    }
    enc.symbol(&mut state, i32::from(p.ec), false, &t);
    enc.symbol(&mut state, i32::from(p.intra), false, &t);
}

// ---- slices ----

/// A slice's place in the frame, in pixels.
#[derive(Clone, Copy)]
struct SliceRect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

fn slice_rect(p: &Params, width: usize, height: usize, sx: usize, sy: usize, sw: usize, sh: usize) -> Result<SliceRect> {
    let (nh, nv) = (p.num_h as usize, p.num_v as usize);
    if sx + sw > nh || sy + sh > nv || sw == 0 || sh == 0 {
        return Err(Error::Invalid("slice outside the slice raster"));
    }
    let x0 = sx * width / nh;
    let x1 = (sx + sw) * width / nh;
    let y0 = sy * height / nv;
    let y1 = (sy + sh) * height / nv;
    if x1 <= x0 || y1 <= y0 {
        return Err(Error::Invalid("empty slice"));
    }
    Ok(SliceRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// The samples of one plane of a slice with the border of RFC 9043 section 3.1 around them: two
/// columns to the left, one to the right, two rows above.
struct Border {
    buf: Vec<i32>,
    stride: usize,
    w: usize,
}

impl Border {
    fn new(w: usize, h: usize) -> Border {
        let stride = w + 3;
        Border { buf: vec![0; stride * (h + 2)], stride, w }
    }

    fn at(&self, x: usize, y: usize) -> usize {
        (y + 2) * self.stride + x + 2
    }

    /// Set up the left border of row `y` (the column left of it repeats the row above's first sample).
    fn start_row(&mut self, y: usize) {
        let row = (y + 2) * self.stride;
        self.buf[row] = 0;
        self.buf[row + 1] = if y > 0 { self.buf[(y + 1) * self.stride + 2] } else { 0 };
    }

    /// The right border of row `y` repeats its last sample.
    fn end_row(&mut self, y: usize) {
        let row = (y + 2) * self.stride;
        self.buf[row + self.w + 2] = self.buf[row + self.w + 1];
    }

    /// `(context, predictor)` for sample `(x, y)`; `context` is signed (section 3.5).
    fn context(&self, x: usize, y: usize, tables: &[[i32; 256]], sixteen_bit_ycbcr: bool) -> (i32, i32) {
        let i = self.at(x, y);
        let (b, s) = (&self.buf, self.stride);
        let (l, ll, t, tl, tr, tt) = (b[i - 1], b[i - 2], b[i - s], b[i - s - 1], b[i - s + 1], b[i - 2 * s]);
        let ctx = tables[0][((l - tl) & 255) as usize]
            + tables[1][((tl - t) & 255) as usize]
            + tables[2][((t - tr) & 255) as usize]
            + tables[3][((ll - l) & 255) as usize]
            + tables[4][((tt - t) & 255) as usize];
        let (pl, pt, ptl) = if sixteen_bit_ycbcr {
            let s16 = |v: i32| if v >= 32768 { v - 65536 } else { v };
            (s16(l), s16(t), s16(tl))
        } else {
            (l, t, tl)
        };
        (ctx, median(pl, pt, pl + pt - ptl))
    }
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// The reversible colour transform bits: a plane's samples have `bits + 1` bits.
fn rct_swapped(p: &Params) -> bool {
    p.colorspace == Colorspace::Rgb && (9..=15).contains(&p.bits) && !p.extra_plane
}

fn coded_bits(p: &Params) -> u32 {
    if p.colorspace == Colorspace::Rgb { p.bits + 1 } else { p.bits }
}

/// One plane's samples inside a slice, and which class of states codes them.
struct PlaneCoder {
    border: Border,
    slot: usize,
}

/// The adaptive states of one plane class: Y, Cb and Cr together, then transparency (section 3.6).
struct ClassStates {
    set: usize,
    states: Vec<[u8; CONTEXT_SIZE]>,
}

fn make_coders(p: &Params, rect: SliceRect, set_idx: &[usize]) -> Result<(Vec<PlaneCoder>, Vec<ClassStates>)> {
    let mut planes = Vec::new();
    for pl in 0..p.plane_count() {
        let (w, h) = plane_dims(rect.w, rect.h, p.colorspace, p.shift, pl, p.chroma_planes);
        planes.push(PlaneCoder { border: Border::new(w, h), slot: p.qset_slot(pl) });
    }
    let mut classes = Vec::new();
    for slot in 0..=planes.iter().map(|c| c.slot).max().unwrap_or(0) {
        let set = *set_idx.get(slot).ok_or(Error::Invalid("missing quant table set index"))?;
        let q = p.sets.get(set).ok_or(Error::Invalid("quant table set index out of range"))?;
        let states = match p.initial_states.get(set) {
            Some(Some(init)) => init.clone(),
            _ => vec![[128u8; CONTEXT_SIZE]; q.context_count],
        };
        classes.push(ClassStates { set, states });
    }
    Ok((planes, classes))
}

/// The number of Quantization Table Set indexes in a slice header (section 4.6.5, version 3).
fn set_index_count(p: &Params) -> usize {
    2 + usize::from(p.extra_plane)
}

// Output u16 samples, an encoder's temporary i32 planes, and the full bordered i32
// slice working copy. Explicit padding matters for 1-pixel-wide/tall hostile frames.
fn plane_store_bytes(w: u64, h: u64) -> Option<u64> {
    let samples = w.checked_mul(h)?.checked_mul(6)?;
    let border = w.checked_add(3)?.checked_mul(h.checked_add(2)?)?.checked_mul(4)?;
    samples.checked_add(border)
}

fn check_frame_size(p: &Params, w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 {
        return Err(Error::Invalid("empty frame"));
    }
    let mut bytes = 0u64;
    for plane in 0..p.plane_count() {
        let (pw, ph) = plane_dims(w as usize, h as usize, p.colorspace, p.shift, plane, p.chroma_planes);
        let plane_bytes = plane_store_bytes(pw as u64, ph as u64).ok_or(Error::Unsupported("frame size above the memory budget"))?;
        bytes = bytes.checked_add(plane_bytes).ok_or(Error::Unsupported("frame size above the memory budget"))?;
    }
    // Retained initial contexts, up to three active context classes, and the packet's slice spans.
    let initial: u64 = p.initial_states.iter().filter_map(Option::as_ref).map(|v| v.len() as u64 * CONTEXT_SIZE as u64).sum();
    let contexts = 3 * MAX_CONTEXTS * CONTEXT_SIZE as u64;
    let spans = u64::from(p.num_h) * u64::from(p.num_v) * (2 * std::mem::size_of::<usize>()) as u64;
    if bytes.checked_add(initial + contexts + spans).is_none_or(|total| total > DECODE_MEMORY_BUDGET) {
        return Err(Error::Unsupported("frame size above the memory budget"));
    }
    Ok(())
}

/// A decoder for one stream.
pub struct Decoder {
    params: Params,
    width: u32,
    height: u32,
}

impl Decoder {
    /// Build a decoder from the container's configuration record and the frame size.
    pub fn new(config_record: &[u8], width: u32, height: u32) -> Result<Decoder> {
        if config_record.len() < 5 {
            return Err(Error::Truncated);
        }
        let (body, _crc) = config_record.split_at(config_record.len() - 4);
        if crc32(config_record) != 0 {
            return Err(Error::Crc("the configuration record"));
        }
        let mut rac = Rac::new(body);
        let params = parse_params(&mut rac)?;
        if rac.consumed() > body.len() + 2 {
            return Err(Error::Truncated);
        }
        check_frame_size(&params, width, height)?;
        Ok(Decoder { params, width, height })
    }

    /// Decode one frame (a packet from the container).
    pub fn decode(&self, packet: &[u8]) -> Result<Frame> {
        let p = &self.params;
        let (w, h) = (self.width as usize, self.height as usize);
        let planes = p.plane_count();
        let mut frame = Frame {
            width: self.width,
            height: self.height,
            colorspace: p.colorspace,
            bit_depth: p.bits as u8,
            chroma_shift: if p.chroma_planes { p.shift } else { (0, 0) },
            planes: Vec::with_capacity(planes),
        };
        for pl in 0..planes {
            let (pw, ph) = plane_dims(w, h, p.colorspace, p.shift, pl, p.chroma_planes);
            let mut v = Vec::new();
            v.try_reserve_exact(pw * ph).map_err(|_| Error::Unsupported("frame size above the memory budget"))?;
            v.resize(pw * ph, 0u16);
            frame.planes.push(v);
        }
        // Slices are found from the end of the packet through their footers (section 4.9).
        let trailer = if p.ec { 8 } else { 3 };
        let slice_count = p.num_h as usize * p.num_v as usize;
        let mut end = packet.len();
        let mut spans = Vec::with_capacity(slice_count);
        for _ in 0..slice_count {
            if end < trailer {
                return Err(Error::Truncated);
            }
            let f = end - trailer;
            let size = usize::from(packet[f]) << 16 | usize::from(packet[f + 1]) << 8 | usize::from(packet[f + 2]);
            let start = f.checked_sub(size).ok_or(Error::Truncated)?;
            spans.push((start, end));
            end = start;
        }
        spans.reverse();
        for (n, &(start, stop)) in spans.iter().enumerate() {
            let slice = &packet[start..stop];
            if p.ec && crc32(slice) != 0 {
                return Err(Error::Crc("a slice"));
            }
            self.decode_slice(slice, n == 0, &mut frame)?;
        }
        Ok(frame)
    }

    fn decode_slice(&self, slice: &[u8], first: bool, frame: &mut Frame) -> Result<()> {
        let p = &self.params;
        let t = &p.transitions;
        let data = &slice[..slice.len() - if p.ec { 8 } else { 3 }];
        let mut rac = Rac::new(data);
        // The keyframe flag is the first symbol of the first slice.
        // The whole slice, header included, is coded with the stream's state transition table.
        let t0 = t;
        if first {
            let mut ks = 128u8;
            if !rac.bit(&mut ks, t0) {
                return Err(Error::Unsupported("inter frames (keyframe 0)"));
            }
        }
        let mut hs = [128u8; CONTEXT_SIZE];
        let sx = rac.symbol(&mut hs, false, t0) as usize;
        let sy = rac.symbol(&mut hs, false, t0) as usize;
        let sw = rac.symbol(&mut hs, false, t0) as usize + 1;
        let sh = rac.symbol(&mut hs, false, t0) as usize + 1;
        let set_idx: Vec<usize> = (0..set_index_count(p)).map(|_| rac.symbol(&mut hs, false, t0).max(0) as usize).collect();
        let _picture_structure = rac.symbol(&mut hs, false, t0);
        let _sar = (rac.symbol(&mut hs, false, t0), rac.symbol(&mut hs, false, t0));
        let rect = slice_rect(p, self.width as usize, self.height as usize, sx, sy, sw, sh)?;
        let (sx_mask, sy_mask) = ((1usize << p.shift.0) - 1, (1usize << p.shift.1) - 1);
        if p.colorspace == Colorspace::YCbCr && p.chroma_planes && (rect.x & sx_mask != 0 || rect.y & sy_mask != 0) {
            return Err(Error::Unsupported("slice boundaries that are not multiples of the chroma subsampling"));
        }
        let (mut coders, mut classes) = make_coders(p, rect, &set_idx)?;
        let bits = coded_bits(p);
        let mask = (1i32 << bits) - 1;
        let sixteen = p.colorspace == Colorspace::YCbCr && p.bits == 16;
        let read = |rac: &mut Rac, c: &mut PlaneCoder, cls: &mut ClassStates, x: usize, y: usize| -> i32 {
            let q = &p.sets[cls.set];
            let (ctx, pred) = c.border.context(x, y, &q.tables, sixteen);
            let (idx, neg) = if ctx < 0 { ((-ctx) as usize, true) } else { (ctx as usize, false) };
            let mut d = rac.symbol(&mut cls.states[idx.min(q.context_count - 1)], true, t);
            if neg {
                d = -d;
            }
            let v = (pred + d) & mask;
            let at = c.border.at(x, y);
            c.border.buf[at] = v;
            v
        };
        match p.colorspace {
            Colorspace::YCbCr => {
                for (pl, c) in coders.iter_mut().enumerate() {
                    let (pw, ph) = plane_dims(rect.w, rect.h, p.colorspace, p.shift, pl, p.chroma_planes);
                    for y in 0..ph {
                        c.border.start_row(y);
                        for x in 0..pw {
                            read(&mut rac, c, &mut classes[c.slot], x, y);
                        }
                        c.border.end_row(y);
                    }
                }
            }
            Colorspace::Rgb => {
                for y in 0..rect.h {
                    for c in coders.iter_mut() {
                        c.border.start_row(y);
                        for x in 0..rect.w {
                            read(&mut rac, c, &mut classes[c.slot], x, y);
                        }
                        c.border.end_row(y);
                    }
                }
            }
        }
        self.store(&coders, rect, frame);
        Ok(())
    }

    /// Copy a decoded slice into the frame, undoing the colour transform for RGB.
    fn store(&self, coders: &[PlaneCoder], rect: SliceRect, frame: &mut Frame) {
        let p = &self.params;
        if p.colorspace == Colorspace::YCbCr {
            for (pl, c) in coders.iter().enumerate() {
                let (pw, ph) = plane_dims(rect.w, rect.h, p.colorspace, p.shift, pl, p.chroma_planes);
                let (fw, _) = plane_dims(self.width as usize, self.height as usize, p.colorspace, p.shift, pl, p.chroma_planes);
                let (ox, oy) = if p.chroma_planes && (pl == 1 || pl == 2) { (rect.x >> p.shift.0, rect.y >> p.shift.1) } else { (rect.x, rect.y) };
                for y in 0..ph {
                    for x in 0..pw {
                        frame.planes[pl][(oy + y) * fw + ox + x] = c.border.buf[c.border.at(x, y)] as u16;
                    }
                }
            }
            return;
        }
        let swapped = rct_swapped(p);
        let off = 1i32 << p.bits;
        let fw = self.width as usize;
        for y in 0..rect.h {
            for x in 0..rect.w {
                let get = |i: usize| coders[i].border.buf[coders[i].border.at(x, y)];
                let (yy, cb, cr) = (get(0), get(1) - off, get(2) - off);
                let (r, g, b);
                if swapped {
                    let bb = yy - ((cb + cr) >> 2);
                    r = cr + bb;
                    g = cb + bb;
                    b = bb;
                } else {
                    let gg = yy - ((cb + cr) >> 2);
                    r = cr + gg;
                    g = gg;
                    b = cb + gg;
                }
                let at = (rect.y + y) * fw + rect.x + x;
                frame.planes[0][at] = r as u16;
                frame.planes[1][at] = g as u16;
                frame.planes[2][at] = b as u16;
                if coders.len() > 3 {
                    frame.planes[3][at] = get(3) as u16;
                }
            }
        }
    }
}

/// Encoder settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Slices across the frame.
    pub slices_h: u32,
    /// Slices down the frame.
    pub slices_v: u32,
    /// Write a CRC in every slice (error detection).
    pub slice_crc: bool,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        EncoderConfig { slices_h: 1, slices_v: 1, slice_crc: true }
    }
}

/// An encoder for frames of one fixed format.
pub struct Encoder {
    params: Params,
    width: u32,
    height: u32,
}

impl Encoder {
    /// An encoder for frames shaped like `template` (its samples are not used).
    pub fn new(template: &Frame, cfg: EncoderConfig) -> Result<Encoder> {
        if !template.plane_count_ok() || template.width == 0 || template.height == 0 {
            return Err(Error::Input("a frame has 1 to 4 planes and a nonzero size".into()));
        }
        if !(8..=16).contains(&template.bit_depth) {
            return Err(Error::Input("bit depth must be 8 to 16".into()));
        }
        if cfg.slices_h == 0
            || cfg.slices_v == 0
            || cfg.slices_h > 1024
            || cfg.slices_v > 1024
            || cfg.slices_h > template.width
            || cfg.slices_v > template.height
        {
            return Err(Error::Input("invalid slice raster".into()));
        }
        let n = template.planes.len();
        let (chroma_planes, extra_plane) = match (template.colorspace, n) {
            (Colorspace::YCbCr, 1) => (false, false),
            (Colorspace::YCbCr, 2) => (false, true),
            (Colorspace::YCbCr, 3) => (true, false),
            (Colorspace::YCbCr, _) => (true, true),
            (Colorspace::Rgb, 3) => (true, false),
            (Colorspace::Rgb, 4) => (true, true),
            (Colorspace::Rgb, _) => return Err(Error::Input("RGB frames have 3 or 4 planes".into())),
        };
        let shift = if template.colorspace == Colorspace::YCbCr && chroma_planes { template.chroma_shift } else { (0, 0) };
        if shift.0 > 3 || shift.1 > 3 {
            return Err(Error::Input("chroma subsampling above 8:1".into()));
        }
        if template.colorspace == Colorspace::Rgb && template.bit_depth > 15 {
            return Err(Error::Input("RGB above 15 bits needs 17-bit coded samples".into()));
        }
        let params = Params {
            coder_type: 1,
            transitions: Transitions::default_table(),
            colorspace: template.colorspace,
            bits: u32::from(template.bit_depth),
            chroma_planes,
            shift,
            extra_plane,
            num_h: cfg.slices_h,
            num_v: cfg.slices_v,
            sets: vec![default_quant_set()],
            initial_states: vec![None],
            ec: cfg.slice_crc,
            intra: true,
        };
        check_frame_size(&params, template.width, template.height)?;
        // Every slice boundary must be a multiple of the chroma subsampling, so the decoder accepts it.
        for i in 0..=cfg.slices_h as usize {
            if params.chroma_planes && (i * template.width as usize / cfg.slices_h as usize) & ((1 << shift.0) - 1) != 0 && i != cfg.slices_h as usize
            {
                return Err(Error::Input("slice boundaries must be multiples of the chroma subsampling".into()));
            }
        }
        for i in 0..cfg.slices_v as usize {
            if params.chroma_planes && (i * template.height as usize / cfg.slices_v as usize) & ((1 << shift.1) - 1) != 0 {
                return Err(Error::Input("slice boundaries must be multiples of the chroma subsampling".into()));
            }
        }
        Ok(Encoder { params, width: template.width, height: template.height })
    }

    /// The configuration record for the container (`CodecPrivate` and the like).
    pub fn config_record(&self) -> Vec<u8> {
        let mut enc = RacEnc::new();
        write_params(&mut enc, &self.params);
        let mut out = enc.finish();
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    /// Encode one frame as a keyframe packet.
    pub fn encode(&self, frame: &Frame) -> Result<Vec<u8>> {
        let p = &self.params;
        if frame.width != self.width || frame.height != self.height || frame.colorspace != p.colorspace || u32::from(frame.bit_depth) != p.bits {
            return Err(Error::Input("the frame does not match the encoder's format".into()));
        }
        if frame.planes.len() != p.plane_count() {
            return Err(Error::Input("the frame has the wrong number of planes".into()));
        }
        let shift = if p.chroma_planes { p.shift } else { (0, 0) };
        if p.colorspace == Colorspace::YCbCr && p.chroma_planes && frame.chroma_shift != shift {
            return Err(Error::Input("the chroma subsampling does not match".into()));
        }
        for (pl, plane) in frame.planes.iter().enumerate() {
            let (w, h) = frame.dims(pl, p.chroma_planes);
            if plane.len() != w * h {
                return Err(Error::Input(format!("plane {pl} has {} samples, expected {}", plane.len(), w * h)));
            }
            if plane.iter().any(|&v| u32::from(v) >> p.bits != 0) {
                return Err(Error::Input(format!("plane {pl} has a sample above {} bits", p.bits)));
            }
        }
        let mut out = Vec::new();
        for sy in 0..p.num_v as usize {
            for sx in 0..p.num_h as usize {
                let rect = slice_rect(p, self.width as usize, self.height as usize, sx, sy, 1, 1)?;
                let mut slice = self.encode_slice(frame, rect, sx, sy, out.is_empty())?;
                let size = slice.len();
                slice.extend_from_slice(&(size as u32).to_be_bytes()[1..]);
                if p.ec {
                    slice.push(0); // error_status
                    let crc = crc32(&slice);
                    slice.extend_from_slice(&crc.to_be_bytes());
                }
                out.extend_from_slice(&slice);
            }
        }
        Ok(out)
    }

    fn encode_slice(&self, frame: &Frame, rect: SliceRect, sx: usize, sy: usize, first: bool) -> Result<Vec<u8>> {
        let p = &self.params;
        let t = &p.transitions;
        let t0 = t;
        let mut rac = RacEnc::new();
        if first {
            let mut ks = 128u8;
            rac.bit(true, &mut ks, t0);
        }
        let mut hs = [128u8; CONTEXT_SIZE];
        for v in [sx as i32, sy as i32, 0, 0] {
            rac.symbol(&mut hs, v, false, t0);
        }
        let set_idx = vec![0usize; set_index_count(p)];
        for &s in &set_idx {
            rac.symbol(&mut hs, s as i32, false, t0);
        }
        for v in [0, 0, 0] {
            rac.symbol(&mut hs, v, false, t0); // picture_structure, sar_num, sar_den
        }
        let (mut coders, mut classes) = make_coders(p, rect, &set_idx)?;
        let bits = coded_bits(p);
        let half = 1i32 << (bits - 1);
        let mask = (1i32 << bits) - 1;
        let sixteen = p.colorspace == Colorspace::YCbCr && p.bits == 16;
        // The samples to code, one bordered plane's worth per coder.
        let values = self.slice_values(frame, rect);
        let write = |rac: &mut RacEnc, c: &mut PlaneCoder, cls: &mut ClassStates, x: usize, y: usize, sample: i32| {
            let q = &p.sets[cls.set];
            let (ctx, pred) = c.border.context(x, y, &q.tables, sixteen);
            let mut d = ((sample - pred + half) & mask) - half;
            let (idx, neg) = if ctx < 0 { ((-ctx) as usize, true) } else { (ctx as usize, false) };
            if neg {
                d = -d;
            }
            rac.symbol(&mut cls.states[idx.min(q.context_count - 1)], d, true, t);
            let at = c.border.at(x, y);
            c.border.buf[at] = sample;
        };
        match p.colorspace {
            Colorspace::YCbCr => {
                for (pl, c) in coders.iter_mut().enumerate() {
                    let (pw, ph) = plane_dims(rect.w, rect.h, p.colorspace, p.shift, pl, p.chroma_planes);
                    for y in 0..ph {
                        c.border.start_row(y);
                        for x in 0..pw {
                            write(&mut rac, c, &mut classes[c.slot], x, y, values[pl][y * pw + x]);
                        }
                        c.border.end_row(y);
                    }
                }
            }
            Colorspace::Rgb => {
                for y in 0..rect.h {
                    for (pl, c) in coders.iter_mut().enumerate() {
                        c.border.start_row(y);
                        for x in 0..rect.w {
                            write(&mut rac, c, &mut classes[c.slot], x, y, values[pl][y * rect.w + x]);
                        }
                        c.border.end_row(y);
                    }
                }
            }
        }
        Ok(rac.finish())
    }

    /// The slice's samples per coded plane (after the colour transform for RGB).
    fn slice_values(&self, frame: &Frame, rect: SliceRect) -> Vec<Vec<i32>> {
        let p = &self.params;
        let fw = self.width as usize;
        if p.colorspace == Colorspace::YCbCr {
            return (0..p.plane_count())
                .map(|pl| {
                    let (pw, ph) = plane_dims(rect.w, rect.h, p.colorspace, p.shift, pl, p.chroma_planes);
                    let (full_w, _) = plane_dims(fw, self.height as usize, p.colorspace, p.shift, pl, p.chroma_planes);
                    let (ox, oy) =
                        if p.chroma_planes && (pl == 1 || pl == 2) { (rect.x >> p.shift.0, rect.y >> p.shift.1) } else { (rect.x, rect.y) };
                    let mut v = Vec::with_capacity(pw * ph);
                    for y in 0..ph {
                        for x in 0..pw {
                            v.push(i32::from(frame.planes[pl][(oy + y) * full_w + ox + x]));
                        }
                    }
                    v
                })
                .collect();
        }
        let swapped = rct_swapped(p);
        let off = 1i32 << p.bits;
        // Cloning an empty Vec drops its capacity, causing geometric growth beyond the budget.
        let mut planes: Vec<Vec<i32>> = (0..p.plane_count()).map(|_| Vec::with_capacity(rect.w * rect.h)).collect();
        for y in 0..rect.h {
            for x in 0..rect.w {
                let at = (rect.y + y) * fw + rect.x + x;
                let (r, g, b) = (i32::from(frame.planes[0][at]), i32::from(frame.planes[1][at]), i32::from(frame.planes[2][at]));
                let (yy, cb, cr) = if swapped {
                    let (cb, cr) = (g - b, r - b);
                    (b + ((cb + cr) >> 2), cb, cr)
                } else {
                    let (cb, cr) = (b - g, r - g);
                    (g + ((cb + cr) >> 2), cb, cr)
                };
                planes[0].push(yy);
                planes[1].push(cb + off);
                planes[2].push(cr + off);
                if p.extra_plane {
                    planes[3].push(i32::from(frame.planes[3][at]));
                }
            }
        }
        planes
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn thin_frame_headers_are_refused_before_pixel_allocation() {
        let small = Frame { width: 1, height: 1, colorspace: Colorspace::YCbCr, bit_depth: 8, chroma_shift: (0, 0), planes: vec![vec![]] };
        let config = Encoder::new(&small, EncoderConfig::default()).unwrap().config_record();
        let axis = (DECODE_MEMORY_BUDGET / 10) as u32;
        // The old 10*w*h bound accepted both; the bordered i32 planes exceed it.
        for (width, height) in [(1, axis), (axis, 1)] {
            assert!(Decoder::new(&config, width, height).is_err());
            let huge = Frame { width, height, ..small.clone() };
            assert!(Encoder::new(&huge, EncoderConfig::default()).is_err());
        }
        assert!(Decoder::new(&config, 1, 100).is_ok());
        assert!(Decoder::new(&config, 100, 1).is_ok());
    }

    #[test]
    fn transparency_plane_is_included_in_the_memory_budget() {
        let frame = Frame {
            width: 1,
            height: (DECODE_MEMORY_BUDGET / 75) as u32,
            colorspace: Colorspace::Rgb,
            bit_depth: 8,
            chroma_shift: (0, 0),
            planes: vec![vec![]; 3],
        };
        assert!(Encoder::new(&frame, EncoderConfig::default()).is_ok());
        let with_alpha = Frame { planes: vec![vec![]; 4], ..frame };
        assert!(Encoder::new(&with_alpha, EncoderConfig::default()).is_err());
    }

    #[test]
    fn rgb_slice_planes_keep_the_budgeted_capacity() {
        // Non-power-of-two lengths expose geometric growth of cloned empty Vecs.
        for plane_count in [3, 4] {
            let frame = Frame {
                width: 3,
                height: 5,
                colorspace: Colorspace::Rgb,
                bit_depth: 8,
                chroma_shift: (0, 0),
                planes: vec![vec![0; 15]; plane_count],
            };
            let encoder = Encoder::new(&frame, EncoderConfig::default()).unwrap();
            let rect = slice_rect(&encoder.params, 3, 5, 0, 0, 1, 1).unwrap();
            let planes = encoder.slice_values(&frame, rect);
            assert_eq!(planes.len(), plane_count);
            for plane in planes {
                assert_eq!(plane.len(), 15);
                assert_eq!(plane.capacity(), 15);
            }
        }
    }

    #[test]
    fn plane_budget_counts_borders_and_detects_overflow() {
        assert_eq!(plane_store_bytes(1, 100), Some(600 + 4 * 102 * 4));
        assert_eq!(plane_store_bytes(100, 1), Some(600 + 103 * 3 * 4));
        assert!(plane_store_bytes(u64::MAX, 1).is_none());
        assert!(plane_store_bytes(1, u64::MAX).is_none());
    }
}
