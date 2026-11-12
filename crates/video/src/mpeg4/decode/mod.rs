//! Clean-room MPEG-4 Part 2 (ISO/IEC 14496-2, MPEG-4 Visual) decoder, written from the
//! standard. It decodes the Simple and Advanced Simple profiles: the streams Xvid, DivX 4/5/6
//! and FFmpeg's `mpeg4` encoder write, as MP4 (`mp4v`), Matroska (`V_MPEG4/ISO/ASP`), AVI
//! (`XVID`, `DIVX`, `DX50`, `FMP4`) and raw `.m4v` carry them.
//!
//! - I-, P-, B- and S-VOPs (global motion compensation with up to three warping points).
//! - Quarter-sample motion, four vectors a macroblock, AC/DC prediction.
//! - H.263 and MPEG quantisation, with custom matrices.
//! - Interlaced coding: field DCT, field motion compensation and field direct mode.
//! - Video packets (resync markers); a damaged packet is concealed and decoding resumes at the
//!   next one.
//! - Packed B-VOPs (several VOPs in one container frame, as DivX and Xvid write in AVI) and
//!   uncoded placeholder VOPs.
//!
//! Not supported: data partitioning and reversible VLCs, shapes, static sprites, scalability,
//! reduced resolution and NEWPRED (none of which Xvid or DivX write), and the short video header
//! (H.263 baseline). Streams using them are refused with [`DecodeError::Unsupported`].
//!
//! Where the standard leaves room (reference padding, field chrominance vectors) the decoder does
//! what Xvid and FFmpeg do; see the module README for those cases and for the two where it follows
//! the standard and FFmpeg does not.
//!
//! ```no_run
//! use craft_video::mpeg4::Decoder;
//! # let packets: Vec<(Vec<u8>, i64)> = Vec::new();
//! let mut dec = Decoder::new();
//! for (data, pts) in packets {
//!     for pic in dec.decode(&data, pts).unwrap() {
//!         // pic.y, pic.u, pic.v: 4:2:0 planes, pic.width × pic.height
//!     }
//! }
//! for pic in dec.flush() { /* the last reference picture */ }
//! ```

#![allow(clippy::needless_range_loop)]

mod headers;
mod mc;
mod picture;
mod vlc;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use headers::{Sprite, UserData, VideoSignal, Vol, Vop, VopKind};
use picture::{Frame, MbInfo, Refs, Times};

/// Decoder errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid MPEG-4 video: {0}")]
    Invalid(&'static str),
    #[error("unsupported MPEG-4 video: {0}")]
    Unsupported(&'static str),
    #[error("{0}x{1} pictures do not fit in the decoder's memory budget")]
    TooLarge(u32, u32),
}

type Result<T> = std::result::Result<T, DecodeError>;

/// `vop_coding_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VopType {
    I,
    P,
    B,
    /// Sprite VOP: a P-VOP with global motion compensation.
    S,
}

/// Stream-level properties from the configuration headers.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
    /// `profile_and_level_indication` of the visual object sequence header.
    pub profile_and_level: Option<u8>,
    /// `video_object_type_indication` (1 Simple, 17 Advanced Simple, …).
    pub object_type: u8,
    /// Sample aspect ratio.
    pub sar: (u16, u16),
    /// `vop_time_increment_resolution`: the time stamps' units per second.
    pub time_resolution: u32,
    /// Frames per second (num, den) when the VOL fixes the VOP rate.
    pub frame_rate: Option<(u32, u32)>,
    /// No B-VOPs: pictures come out as soon as they are decoded.
    pub low_delay: bool,
    pub interlaced: bool,
    pub quarter_sample: bool,
    /// Global motion compensation (S-VOPs).
    pub gmc: bool,
    /// MPEG quantisation (`quant_type` 1) rather than H.263.
    pub mpeg_quant: bool,
    pub video_format: Option<u8>,
    pub full_range: bool,
    /// `(colour_primaries, transfer_characteristics, matrix_coefficients)`.
    pub colour: Option<(u8, u8, u8)>,
    /// The writing encoder, when its user data names it ("Xvid build 64", "DivX build 1393").
    pub encoder: Option<String>,
}

impl StreamInfo {
    /// "MPEG-4 Visual (Advanced Simple@L5)".
    pub fn codec_name(&self) -> String {
        match self.profile_and_level.and_then(profile_level_name) {
            Some(p) => format!("MPEG-4 Visual ({p})"),
            None => "MPEG-4 Visual".into(),
        }
    }
}

fn profile_level_name(p: u8) -> Option<String> {
    let (name, level) = match p {
        0x08 => ("Simple", "L0"),
        0x09 => ("Simple", "L0b"),
        0x01..=0x03 => ("Simple", ["L1", "L2", "L3"][usize::from(p - 1)]),
        0x04 => ("Simple", "L4a"),
        0x05 => ("Simple", "L5"),
        0x06 => ("Simple", "L6"),
        0xF0..=0xF5 => ("Advanced Simple", ["L0", "L1", "L2", "L3", "L4", "L5"][usize::from(p - 0xF0)]),
        0xF7 => ("Advanced Simple", "L3b"),
        _ => return None,
    };
    Some(format!("{name}@{level}"))
}

/// A decoded picture, cropped to the coded size. Chrominance is 4:2:0: `(width + 1) / 2` by
/// `(height + 1) / 2`.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub vop_type: VopType,
    /// The time stamp passed with the data the VOP came in.
    pub pts: i64,
    /// The VOP's time stamp from the bitstream, in units of 1/`info.time_resolution` seconds
    /// from the stream's first time base.
    pub time: i64,
    /// The quantiser of the VOP header.
    pub quant: u8,
    /// Interlaced streams: whether the top field comes first.
    pub top_field_first: Option<bool>,
    pub info: Arc<StreamInfo>,
}

impl Picture {
    pub fn chroma_width(&self) -> u32 {
        self.width.div_ceil(2)
    }
    pub fn chroma_height(&self) -> u32 {
        self.height.div_ceil(2)
    }
}

/// A reference picture.
struct Ref {
    frame: Frame,
    mbs: Vec<MbInfo>,
    meta: Meta,
    /// The stream description at decoding time: a later VOL may change it before this picture
    /// is shown.
    info: Arc<StreamInfo>,
    shown: bool,
}

#[derive(Clone, Copy)]
struct Meta {
    vop_type: VopType,
    pts: i64,
    time: i64,
    quant: u8,
    top_field_first: bool,
}

/// One access unit must not hold an unbounded number of pictures.
const MAX_PICTURES_PER_CALL: usize = 16;

/// A stateful decoder. Feed the stream in decode order in chunks that hold whole VOPs (container
/// frames, or the whole elementary stream); pictures come out in display order.
pub struct Decoder {
    vol: Option<Vol>,
    info: Option<Arc<StreamInfo>>,
    signal: VideoSignal,
    profile: Option<u8>,
    user: UserData,
    /// The older and the most recent reference picture.
    prev: Option<Ref>,
    last: Option<Ref>,
    /// A picture buffer no longer referenced, reused for the next VOP instead of allocating one.
    spare: Option<Frame>,
    /// Bytes of one picture buffer of the current VOL, 0 without one: the references, the spare
    /// buffer and the picture being decoded take one each.
    frame_bytes: u64,
    /// Why the last VOL was refused, until a decodable one arrives: the error of the VOPs that
    /// follow it, so `last_error` keeps naming the cause.
    refused: Option<DecodeError>,
    time_base: i64,
    last_time_base: i64,
    last_non_b_time: i64,
    pp_time: i64,
    /// Frame period estimate for field direct mode.
    t_frame: i64,
    out: Vec<Picture>,
    errors: usize,
    last_error: Option<DecodeError>,
    memory_budget: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Positions of the start codes (`00 00 01 xx`) in `data`: (offset, code).
fn start_codes(data: &[u8]) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            out.push((i, data[i + 3]));
            i += 4;
        } else if data[i + 2] > 1 {
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

/// The configuration headers of a stream (or its extradata), without decoding pictures.
pub fn probe(data: &[u8]) -> Option<StreamInfo> {
    let mut d = Decoder::new();
    d.decode_headers(data).ok()?;
    d.info.as_deref().cloned()
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            vol: None,
            info: None,
            signal: VideoSignal::default(),
            profile: None,
            user: UserData::default(),
            prev: None,
            last: None,
            spare: None,
            frame_bytes: 0,
            refused: None,
            time_base: 0,
            last_time_base: 0,
            last_non_b_time: 0,
            pp_time: 0,
            t_frame: 0,
            out: Vec::new(),
            errors: 0,
            last_error: None,
            memory_budget: crate::budget::DECODE_MEMORY_BUDGET,
        }
    }

    /// Bytes the decoded pictures may take (default [`crate::budget::DECODE_MEMORY_BUDGET`]).
    /// A VOL whose pictures (two references, the picture being decoded and an output copy) do not
    /// fit is refused, and the room left limits the pictures one [`decode`](Self::decode) call
    /// returns. It applies from the next VOL header.
    pub fn set_memory_budget(&mut self, bytes: u64) {
        self.memory_budget = bytes;
    }

    /// VOPs that were damaged (and concealed) or could not be decoded so far.
    pub fn errors(&self) -> usize {
        self.errors
    }

    /// The most recent of those errors.
    pub fn last_error(&self) -> Option<&DecodeError> {
        self.last_error.as_ref()
    }

    /// The active video object layer.
    pub fn info(&self) -> Option<&StreamInfo> {
        self.info.as_deref()
    }

    /// Read configuration headers only (MP4 `esds` `DecoderSpecificInfo`, Matroska
    /// `CodecPrivate`, AVI stream header extradata). VOPs in `data` are ignored.
    pub fn decode_headers(&mut self, data: &[u8]) -> Result<()> {
        let codes = start_codes(data);
        let mut err = None;
        for (k, &(pos, code)) in codes.iter().enumerate() {
            let end = codes.get(k + 1).map_or(data.len(), |c| c.0);
            if code != 0xB6
                && let Err(e) = self.header(code, &data[(pos + 4).min(end)..end])
            {
                err = Some(e);
            }
        }
        match (err, &self.vol) {
            (Some(e), _) => Err(e),
            (None, None) => Err(DecodeError::Invalid("no video object layer header")),
            (None, Some(_)) => Ok(()),
        }
    }

    /// Handle a non-VOP start code.
    fn header(&mut self, code: u8, payload: &[u8]) -> Result<()> {
        match code {
            0xB0 => self.profile = payload.first().copied(),
            0xB2 => headers::parse_user_data(payload, &mut self.user),
            0xB3 => {
                let t = headers::parse_gov(payload)? as i64;
                self.time_base = t;
            }
            0xB5 => self.signal = headers::parse_visual_object(payload)?,
            0x20..=0x2F => {
                let set = Vol::parse(payload).and_then(|vol| self.set_vol(vol));
                if let Err(e @ (DecodeError::Unsupported(_) | DecodeError::TooLarge(..))) = set {
                    // The stream changed to a layer this decoder cannot decode: its VOPs must not
                    // be decoded with the old one. (A damaged layer header, `Invalid`, keeps the
                    // old layer: that is usually a bit error in a repeated header.)
                    self.show_last();
                    self.drop_references();
                    self.vol = None;
                    self.info = None;
                    self.frame_bytes = 0;
                    self.refused = Some(e.clone());
                    return Err(e);
                }
                set?;
            }
            _ => {}
        }
        if let (Some(info), Some(vol)) = (&self.info, &self.vol) {
            let fresh = self.stream_info(vol);
            if **info != fresh {
                self.info = Some(Arc::new(fresh));
            }
        }
        Ok(())
    }

    fn set_vol(&mut self, vol: Vol) -> Result<()> {
        if vol.data_partitioned {
            return Err(DecodeError::Unsupported("data partitioning"));
        }
        let shape = crate::budget::PictureShape {
            width: u64::from(vol.width.div_ceil(16) * 16),
            height: u64::from(vol.height.div_ceil(16) * 16),
            shift_x: 1,
            shift_y: 1,
            chroma_planes: 2,
            bytes_per_sample: 1,
        };
        if !shape.fits(4, self.memory_budget) {
            return Err(DecodeError::TooLarge(vol.width, vol.height));
        }
        let resized = self.vol.as_ref().is_some_and(|v| (v.width, v.height) != (vol.width, vol.height));
        let to_low_delay = self.vol.as_ref().is_some_and(|v| !v.low_delay) && vol.low_delay;
        if resized || to_low_delay {
            // The picture held back for reordering is still owed to the caller, and from here on
            // no later VOP releases it: the references are dropped, or a low-delay VOP outputs
            // only itself.
            self.show_last();
        }
        if resized {
            self.drop_references();
        }
        self.frame_bytes = shape.bytes().unwrap_or(u64::MAX);
        self.refused = None;
        self.info = Some(Arc::new(self.stream_info(&vol)));
        self.vol = Some(vol);
        Ok(())
    }

    fn stream_info(&self, vol: &Vol) -> StreamInfo {
        let encoder = match (self.user.xvid_build, self.user.divx_build) {
            (Some(b), _) => Some(format!("Xvid build {b}")),
            (None, Some(b)) => Some(format!("DivX build {b}")),
            _ => None,
        };
        StreamInfo {
            width: vol.width,
            height: vol.height,
            profile_and_level: self.profile,
            object_type: vol.object_type,
            sar: vol.sar,
            time_resolution: vol.time_resolution,
            frame_rate: vol.fixed_increment.filter(|&i| i > 0).map(|i| (vol.time_resolution, i)),
            low_delay: vol.low_delay,
            interlaced: vol.interlaced,
            quarter_sample: vol.quarter_sample,
            gmc: vol.sprite == Sprite::Gmc,
            mpeg_quant: vol.mpeg_quant,
            video_format: self.signal.video_format,
            full_range: self.signal.full_range,
            colour: self.signal.colour,
            encoder,
        }
    }

    /// Decode a chunk of the stream holding whole VOPs (a container frame, which may pack a P-
    /// and a B-VOP), tagged with `pts`. Returns the pictures that became displayable, in display
    /// order. A damaged VOP is still output, concealed, and counted in [`errors`](Self::errors).
    ///
    /// One call returns at most 16 pictures, fewer when the memory budget does not hold that
    /// many beside the reference pictures. Decoding of a chunk that would produce more stops at
    /// the limit: the pictures so far are returned, and the rest of the chunk is skipped and
    /// counted as an error.
    pub fn decode(&mut self, data: &[u8], pts: i64) -> Result<Vec<Picture>> {
        let codes = start_codes(data);
        let mut err = None;
        for (k, &(pos, code)) in codes.iter().enumerate() {
            if !self.room_for_a_picture() {
                let e = DecodeError::Invalid("too many pictures in one chunk");
                self.errors += 1;
                self.last_error = Some(e.clone());
                err = Some(e);
                break;
            }
            let end = codes.get(k + 1).map_or(data.len(), |c| c.0);
            let payload = &data[(pos + 4).min(end)..end];
            let r = if code == 0xB6 { self.vop(payload, pts) } else { self.header(code, payload) };
            if let Err(e) = r {
                self.errors += 1;
                self.last_error = Some(e.clone());
                err = Some(e);
            }
        }
        let out = std::mem::take(&mut self.out);
        match err {
            Some(e) if out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    /// Whether one more picture may be queued in this call: at most 16, and the queued pictures
    /// (counted at their own size, which a VOL change in the chunk may have made differ from the
    /// current one) must fit in the memory budget beside the current VOL's picture buffers and
    /// one more picture. Each start code adds at most one picture, so checking before each one
    /// keeps the queue within both.
    fn room_for_a_picture(&self) -> bool {
        let queued = self.out.iter().fold(0u64, |n, p| n.saturating_add((p.y.len() + p.u.len() + p.v.len()) as u64));
        self.out.len() < MAX_PICTURES_PER_CALL && queued.saturating_add(self.frame_bytes.saturating_mul(4)) <= self.memory_budget
    }

    /// Output the picture held back for reordering (end of stream).
    pub fn flush(&mut self) -> Vec<Picture> {
        self.show_last();
        std::mem::take(&mut self.out)
    }

    /// Forget the reference pictures and the time base (before decoding from a new random-access
    /// point), so that decoding from it gives the same pictures and times every time. The
    /// configuration headers are kept.
    pub fn reset(&mut self) {
        self.drop_references();
        self.out.clear();
        self.time_base = 0;
        self.last_time_base = 0;
        self.last_non_b_time = 0;
        self.pp_time = 0;
        self.t_frame = 0;
    }

    fn drop_references(&mut self) {
        self.prev = None;
        self.last = None;
        self.spare = None;
    }

    /// Output the most recent reference unless it was.
    fn show_last(&mut self) {
        if let Some(r) = self.last.as_mut()
            && !r.shown
        {
            r.shown = true;
            self.out.push(to_picture(r.info.clone(), &r.frame, r.meta));
        }
    }

    fn picture(&self, f: &Frame, m: Meta) -> Picture {
        to_picture(self.info.clone().unwrap_or_else(|| Arc::new(empty_info())), f, m)
    }

    /// Update the time base and return the VOP's time (§6.3.5).
    fn vop_time(&mut self, vop: &Vop, resolution: i64) -> i64 {
        let inc = i64::from(vop.increment);
        let modulo = i64::from(vop.modulo);
        if vop.kind == VopKind::B {
            return (self.last_time_base + modulo).saturating_mul(resolution).saturating_add(inc);
        }
        self.last_time_base = self.time_base;
        self.time_base = self.time_base.saturating_add(modulo);
        let t = self.time_base.saturating_mul(resolution).saturating_add(inc);
        self.pp_time = t - self.last_non_b_time;
        self.last_non_b_time = t;
        t
    }

    /// Temporal distances of a B-VOP at `time`.
    fn times(&mut self, time: i64, vol: &Vol) -> Times {
        let past = self.last_non_b_time - self.pp_time;
        let pb = time - past;
        if self.t_frame == 0 {
            self.t_frame = vol.fixed_increment.map_or(pb, i64::from).max(0);
        }
        let tf = self.t_frame.max(1);
        let frames = |t: i64| mc::div_round(t, tf);
        let clamp = |v: i64| v.clamp(-(1 << 20), 1 << 20) as i32;
        Times {
            trb: clamp(pb),
            trd: clamp(self.pp_time),
            trb_field: clamp(2 * (frames(time) - frames(past))),
            trd_field: clamp(2 * (frames(self.last_non_b_time) - frames(past))),
        }
    }

    fn vop(&mut self, payload: &[u8], pts: i64) -> Result<()> {
        let Some(vol) = self.vol.clone() else {
            return Err(self.refused.clone().unwrap_or(DecodeError::Invalid("VOP before a video object layer header")));
        };
        let vop = Vop::parse(payload, &vol)?;
        if !vop.coded {
            // A placeholder (packed B-VOPs in AVI, dropped frames): no picture, and not a
            // reference for direct mode's distances; only its seconds carry on.
            if vop.kind != VopKind::B {
                self.time_base = self.time_base.saturating_add(i64::from(vop.modulo));
            }
            return Ok(());
        }
        let time = self.vop_time(&vop, i64::from(vol.time_resolution));
        let meta = Meta {
            vop_type: match vop.kind {
                VopKind::I => VopType::I,
                VopKind::P => VopType::P,
                VopKind::B => VopType::B,
                VopKind::S => VopType::S,
            },
            pts,
            time,
            quant: vop.quant,
            top_field_first: vop.top_field_first,
        };
        if vop.kind == VopKind::B {
            return self.b_vop(payload, &vol, &vop, time, meta);
        }
        if vop.kind != VopKind::I && self.last.is_none() {
            return Err(DecodeError::Invalid("P-VOP without a reference picture"));
        }
        let mut frame = take_frame(&mut self.spare, &vol);
        let reference = self.last.as_ref().map(|r| &r.frame);
        let refs = Refs { past: reference, future: None, colocated: &[], times: Times::default(), conceal: reference };
        let (mbs, err) = picture::decode_vop(payload, &vol, &vop, &refs, &mut frame);
        if vol.low_delay {
            let p = self.picture(&frame, meta);
            self.out.push(p);
        } else {
            self.show_last();
        }
        if let Some(old) = std::mem::replace(&mut self.prev, self.last.take()) {
            self.spare = Some(old.frame);
        }
        let info = self.info.clone().unwrap_or_else(|| Arc::new(empty_info()));
        self.last = Some(Ref { frame, mbs, meta, info, shown: vol.low_delay });
        err.map_or(Ok(()), |e| self.damaged(e))
    }

    fn b_vop(&mut self, payload: &[u8], vol: &Vol, vop: &Vop, time: i64, meta: Meta) -> Result<()> {
        if self.prev.is_none() || self.last.is_none() {
            // A B-VOP of an open GOP at the start of decoding: its past reference is missing. It
            // is dropped before `times`, whose frame period estimate it must not set.
            return Ok(());
        }
        let times = self.times(time, vol);
        let (Some(past), Some(future)) = (self.prev.as_ref(), self.last.as_ref()) else {
            return Ok(());
        };
        let mut frame = take_frame(&mut self.spare, vol);
        let refs = Refs { past: Some(&past.frame), future: Some(&future.frame), colocated: &future.mbs, times, conceal: Some(&past.frame) };
        let (_, err) = picture::decode_vop(payload, vol, vop, &refs, &mut frame);
        let p = self.picture(&frame, meta);
        self.out.push(p);
        self.spare = Some(frame);
        err.map_or(Ok(()), |e| self.damaged(e))
    }

    /// Count a damaged VOP whose picture was still output.
    fn damaged(&mut self, e: DecodeError) -> Result<()> {
        self.errors += 1;
        self.last_error = Some(e);
        Ok(())
    }
}

/// A picture buffer for a VOP of `vol`: the spare one when its size matches. Every macroblock of
/// a VOP is written (decoded or concealed), so the old contents never show.
fn take_frame(spare: &mut Option<Frame>, vol: &Vol) -> Frame {
    match spare.take() {
        Some(f) if f.fits(vol.width, vol.height) => f,
        _ => Frame::new(vol.width, vol.height),
    }
}

fn empty_info() -> StreamInfo {
    StreamInfo {
        width: 0,
        height: 0,
        profile_and_level: None,
        object_type: 0,
        sar: (1, 1),
        time_resolution: 1,
        frame_rate: None,
        low_delay: true,
        interlaced: false,
        quarter_sample: false,
        gmc: false,
        mpeg_quant: false,
        video_format: None,
        full_range: false,
        colour: None,
        encoder: None,
    }
}

fn to_picture(info: Arc<StreamInfo>, f: &Frame, m: Meta) -> Picture {
    let (w, h) = (info.width as usize, info.height as usize);
    let crop = |p: &picture::Frame, k: usize, w: usize, h: usize| -> Vec<u8> {
        let pl = &p.planes[k];
        let mut out = Vec::with_capacity(w * h);
        for line in pl.data.chunks(pl.stride.max(1)).take(h) {
            out.extend_from_slice(line.get(..w).unwrap_or(line));
        }
        out.resize(w * h, 128);
        out
    };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    Picture {
        width: info.width,
        height: info.height,
        y: crop(f, 0, w, h),
        u: crop(f, 1, cw, ch),
        v: crop(f, 2, cw, ch),
        vop_type: m.vop_type,
        pts: m.pts,
        time: m.time,
        quant: m.quant,
        top_field_first: info.interlaced.then_some(m.top_field_first),
        info,
    }
}
