//! Clean-room MPEG-4 Part 2 (ISO/IEC 14496-2, MPEG-4 Visual) encoder, written from the
//! standard. It writes Simple Profile and Advanced Simple Profile (ASP) streams: the kind of
//! stream Xvid and DivX produce and that MP4 (`mp4v`), Matroska (`V_MPEG4/ISO/SP`,
//! `V_MPEG4/ISO/ASP`) and AVI (`XVID`, `DIVX`, `FMP4`) carry.
//!
//! - I-VOPs, P-VOPs, and B-VOPs, 4:2:0 8-bit, rectangular, progressive, H.263 or MPEG matrix
//!   quantisation (`quant_type` 0 or 1, with default or custom intra/non-intra matrices and
//!   mismatch control).
//! - One or four motion vectors a macroblock (`inter` / `inter4v`) at half-sample or
//!   quarter-sample (`quarter_sample`) precision (`fcode` 1–2), found with a predictor-seeded
//!   diamond search; skipped macroblocks; intra macroblocks in P-VOPs when they predict worse
//!   than they code on their own, and every 128 inter codings (§7.4.5 asks for one within 132).
//! - Intra DC prediction and adaptive intra AC prediction (`ac_pred_flag`) with alternate
//!   horizontal and vertical scans (Tables 7-2 and 7-3); all three coefficient escape modes.
//! - B-VOPs with direct, interpolate, forward, and backward macroblock modes, in display/decode
//!   reordered packet mode ([`Encoder::encode_packets`] / [`Encoder::flush`]) or DivX/Xvid
//!   packed-bitstream mode ([`Encoder::encode`]).
//! - Video packet resync markers (`resync_marker_disable = 0`) for slice-level error resilience.
//! - Rate control: a constant quantiser, or a 1-pass average bitrate.
//!
//! ```no_run
//! use craft_video::mpeg4::{Encoder, EncoderConfig, RateControl, YuvFrame};
//! let mut cfg = EncoderConfig::new(640, 360, 25, 1);
//! cfg.rate = RateControl::Quant(5);
//! let mut enc = Encoder::new(cfg).unwrap();
//! let (y, u, v) = (vec![128u8; 640 * 360], vec![128u8; 320 * 180], vec![128u8; 320 * 180]);
//! let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: 640, uv_stride: 320 };
//! let packet = enc.encode(&frame).unwrap();
//! ```

#![allow(clippy::needless_range_loop)]

mod dct;
mod headers;
mod picture;
mod tables;

#[cfg(test)]
mod tests;

use craft_media::bitstream::BitWriter;
use craft_media::{CodecId, CodecParams};

use headers::{Sequence, Timing, VopCodingType, VopHeader};
use picture::{Coded, Frame, PictureCoder, Plane};
use tables::{DEFAULT_INTRA_MATRIX, DEFAULT_NON_INTRA_MATRIX};

/// Largest width or height (the VOL stores 13 bits; sizes are even).
pub const MAX_SIDE: u32 = 8190;

/// Largest number of consecutive B-VOPs between anchor VOPs.
pub const MAX_B_FRAMES: u8 = 8;

/// Encoder errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid dimensions {0}x{1} (must be even, non-zero and at most {MAX_SIDE})")]
    InvalidDimensions(u32, u32),
    #[error("invalid frame rate {0}/{1} (the reduced numerator must fit in 16 bits)")]
    InvalidFrameRate(u32, u32),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("input frame planes too small for the configured size")]
    BadFrame,
}

/// An 8-bit 4:2:0 input picture: chroma planes are half the width and height.
#[derive(Clone, Copy, Debug)]
pub struct YuvFrame<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub uv_stride: usize,
}

/// Colour description written to the visual object header (ITU-T H.273 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorConfig {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// Every VOP at this quantiser (1–31, lower is better).
    Quant(u8),
    /// An average of `kbps` kilobits a second, the quantiser chosen a frame at a time (2–31).
    Bitrate { kbps: u32 },
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub rate: RateControl,
    /// Largest distance between I-VOPs (1 = all intra).
    pub keyint: u32,
    /// Pixel aspect ratio (each 1–255).
    pub sar: (u8, u8),
    /// Colour description; `None` writes no `video_signal_type`.
    pub color: Option<ColorConfig>,
    /// Repeat the configuration headers ([`Encoder::extradata`]) before every I-VOP, as a raw
    /// `.m4v` elementary stream or an AVI file has them. Off for MP4 and Matroska, which carry
    /// them in the track header.
    pub headers_in_band: bool,
    /// Enable adaptive intra AC prediction (`ac_pred_flag`) with alternate horizontal and
    /// vertical coefficient scans (Tables 7-2 and 7-3).
    pub ac_prediction: bool,
    /// Enable 4 motion vectors per macroblock (`inter4v`, Table B-7 `mb_type = 2`) in P-VOPs.
    pub four_mv: bool,
    /// Use MPEG quantisation matrices (`quant_type = 1`, §7.4.2.1) instead of H.263 quantisation.
    pub mpeg_quant: bool,
    /// Custom 8×8 intra quantisation matrix in raster order when `mpeg_quant` is enabled; `None`
    /// uses the ISO/IEC 14496-2 default intra matrix (Table 7-4).
    pub intra_matrix: Option<[u8; 64]>,
    /// Custom 8×8 non-intra quantisation matrix in raster order when `mpeg_quant` is enabled;
    /// `None` uses the ISO/IEC 14496-2 default non-intra matrix (Table 7-5).
    pub non_intra_matrix: Option<[u8; 64]>,
    /// Enable quarter-sample (`quarter_sample`, §7.6.2.3) luminance motion estimation and
    /// compensation (Advanced Simple Profile).
    pub quarter_sample: bool,
    /// Number of consecutive B-VOPs between anchor (I/P) VOPs (`0..=MAX_B_FRAMES`, Advanced Simple
    /// Profile when non-zero).
    pub b_frames: u8,
    /// When `b_frames > 0`, pack each future anchor P-VOP and its B-VOPs into one packet and emit
    /// non-coded P-VOP placeholders (`vop_coded = 0`) for the remaining slots (DivX/Xvid packed
    /// bitstream).
    pub packed_b_frames: bool,
    /// Insert a video packet resync marker (`resync_marker_disable = 0`, §6.2.5.2) every `n`
    /// macroblocks (`None` disables resync markers).
    pub slice_mbs: Option<usize>,
}

impl EncoderConfig {
    /// Quantiser 4, an I-VOP at least every 250 frames, square pixels, headers in band, Simple
    /// Profile defaults.
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig {
            width,
            height,
            fps_num,
            fps_den,
            rate: RateControl::Quant(4),
            keyint: 250,
            sar: (1, 1),
            color: None,
            headers_in_band: true,
            ac_prediction: false,
            four_mv: false,
            mpeg_quant: false,
            intra_matrix: None,
            non_intra_matrix: None,
            quarter_sample: false,
            b_frames: 0,
            packed_b_frames: false,
            slice_mbs: None,
        }
    }
}

/// One coded VOP (or a packed anchor+B packet when `packed_b_frames` is enabled).
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    /// An I-VOP (with the configuration headers in front when they are in band).
    pub keyframe: bool,
    /// The presentation timestamp in units of `fps_den / fps_num` seconds.
    pub pts: i64,
    /// The quantiser the VOP was coded with.
    pub quant: u8,
}

/// The picture a decoder reconstructs from a coded VOP (to within the IDCT accuracy a conforming
/// decoder is held to).
#[derive(Clone, Debug)]
pub struct Reconstruction {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

/// `profile_and_level_indication` (Simple Profile `0x03..=0x06`, or Advanced Simple Profile
/// `0xF3..=0xF5` per ISO/IEC 14496-2 Annex G Table G-1) for a picture of `mbs` macroblocks.
fn profile_level(mbs: usize, asp: bool) -> u8 {
    if asp {
        match mbs {
            0..=396 => 0xF3,   // ASP@L3 (CIF)
            397..=792 => 0xF4, // ASP@L4 (352×576)
            _ => 0xF5,         // ASP@L5 (720×576 and larger)
        }
    } else {
        match mbs {
            0..=396 => 0x03,
            397..=1200 => 0x04,
            1201..=1620 => 0x05,
            _ => 0x06,
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

struct RateState {
    /// Bits a frame at the target rate.
    per_frame: f64,
    /// Bits sent minus bits budgeted so far.
    fullness: f64,
    /// Bits × quantiser of the last I and P VOP (the model: bits fall as 1/quantiser).
    complexity: [Option<f64>; 2],
    last_qp: [u8; 2],
}

impl RateState {
    fn qp(&self, intra: bool) -> u8 {
        let k = usize::from(!intra);
        let share = if intra { 4.0 } else { 1.0 };
        let target = (share * self.per_frame - self.fullness / 25.0).max(0.25 * share * self.per_frame);
        let Some(c) = self.complexity[k].or_else(|| self.complexity[0].map(|c| c / 4.0)) else {
            return 8;
        };
        let q = (c / target).round().clamp(2.0, 31.0) as u8;
        let last = self.last_qp[k];
        if self.complexity[k].is_some() { q.clamp(last.saturating_sub(4).max(2), (last + 4).min(31)) } else { q }
    }

    fn update(&mut self, intra: bool, bits: usize, qp: u8) {
        let k = usize::from(!intra);
        let c = bits as f64 * f64::from(qp);
        self.complexity[k] = Some(self.complexity[k].map_or(c, |old| 0.5 * old + 0.5 * c));
        self.last_qp[k] = qp;
        self.fullness += bits as f64 - self.per_frame;
    }
}

fn crop_recon(r: &Frame, width: u32, height: u32) -> Reconstruction {
    let (w, h) = (width as usize, height as usize);
    let crop = |p: &Plane, w: usize, h: usize| (0..h).flat_map(|y| p.data[y * p.w..][..w].iter().copied()).collect();
    Reconstruction { width, height, y: crop(&r.y, w, h), u: crop(&r.u, w / 2, h / 2), v: crop(&r.v, w / 2, h / 2) }
}

/// The encoder.
pub struct Encoder {
    cfg: EncoderConfig,
    timing: Timing,
    mb_w: usize,
    mb_h: usize,
    headers: Vec<u8>,
    intra_matrix: [u8; 64],
    non_intra_matrix: [u8; 64],
    reference: Option<Frame>,
    /// Display index of the current past anchor (`reference`).
    ref_display_idx: u64,
    /// Whole seconds (`ticks / resolution`) of the last anchor VOP coded.
    last_anchor_seconds: u64,
    /// Whole seconds (`ticks / resolution`) of the current past anchor VOP (`reference`).
    past_anchor_seconds: u64,
    /// Input pictures buffered waiting for the next anchor VOP when `b_frames > 0`:
    /// `(display_index, frame, force_key)`.
    pending_b: Vec<(u64, Frame, bool)>,
    /// Reconstructions in display order produced by the most recent encoding call.
    last_recons: Vec<Reconstruction>,
    frame: u64,
    since_key: u32,
    force_key: bool,
    rounding: bool,
    refresh: Vec<u8>,
    rate: Option<RateState>,
}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Encoder, Error> {
        let (w, h) = (cfg.width, cfg.height);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(Error::InvalidDimensions(w, h));
        }
        if cfg.fps_num == 0 || cfg.fps_den == 0 {
            return Err(Error::InvalidFrameRate(cfg.fps_num, cfg.fps_den));
        }
        let g = gcd(cfg.fps_num, cfg.fps_den);
        let timing = Timing { resolution: cfg.fps_num / g, increment: cfg.fps_den / g };
        if timing.resolution > 0xFFFF {
            return Err(Error::InvalidFrameRate(cfg.fps_num, cfg.fps_den));
        }
        if cfg.keyint == 0 {
            return Err(Error::InvalidConfig("keyint must be at least 1".into()));
        }
        if cfg.sar.0 == 0 || cfg.sar.1 == 0 {
            return Err(Error::InvalidConfig("sample aspect ratio terms must be non-zero".into()));
        }
        if cfg.b_frames > MAX_B_FRAMES {
            return Err(Error::InvalidConfig(format!("b_frames {} exceeds maximum {MAX_B_FRAMES}", cfg.b_frames)));
        }
        if cfg.slice_mbs == Some(0) {
            return Err(Error::InvalidConfig("slice_mbs must be at least 1".into()));
        }
        if let Some(m) = &cfg.intra_matrix {
            if m[0] != 8 {
                return Err(Error::InvalidConfig("intra quantisation matrix DC entry must be 8".into()));
            }
            if m.contains(&0) {
                return Err(Error::InvalidConfig("intra quantisation matrix entries must be non-zero".into()));
            }
        }
        if let Some(m) = &cfg.non_intra_matrix
            && m.contains(&0)
        {
            return Err(Error::InvalidConfig("non-intra quantisation matrix entries must be non-zero".into()));
        }
        let rate = match cfg.rate {
            RateControl::Quant(q) if (1..=31).contains(&q) => None,
            RateControl::Quant(q) => return Err(Error::InvalidConfig(format!("quantiser {q} is outside 1–31"))),
            RateControl::Bitrate { kbps: 0 } => return Err(Error::InvalidConfig("bitrate must be non-zero".into())),
            RateControl::Bitrate { kbps } => Some(RateState {
                per_frame: f64::from(kbps) * 1000.0 * f64::from(timing.increment) / f64::from(timing.resolution),
                fullness: 0.0,
                complexity: [None; 2],
                last_qp: [8; 2],
            }),
        };
        let (mb_w, mb_h) = (w.div_ceil(16) as usize, h.div_ceil(16) as usize);
        let asp = cfg.b_frames > 0 || cfg.quarter_sample || cfg.mpeg_quant;
        let intra_matrix = cfg.intra_matrix.unwrap_or(DEFAULT_INTRA_MATRIX);
        let non_intra_matrix = cfg.non_intra_matrix.unwrap_or(DEFAULT_NON_INTRA_MATRIX);
        let headers = headers::sequence_headers(&Sequence {
            width: w,
            height: h,
            timing,
            profile_level: profile_level(mb_w * mb_h, asp),
            asp,
            sar: cfg.sar,
            color: cfg.color,
            low_delay: cfg.b_frames == 0,
            mpeg_quant: cfg.mpeg_quant,
            intra_matrix: cfg.intra_matrix.as_ref(),
            non_intra_matrix: cfg.non_intra_matrix.as_ref(),
            quarter_sample: cfg.quarter_sample,
            resync_markers: cfg.slice_mbs.is_some(),
            packed_userdata: cfg.b_frames > 0 && cfg.packed_b_frames,
        });
        Ok(Encoder {
            cfg,
            timing,
            mb_w,
            mb_h,
            headers,
            intra_matrix,
            non_intra_matrix,
            reference: None,
            ref_display_idx: 0,
            last_anchor_seconds: 0,
            past_anchor_seconds: 0,
            pending_b: Vec::new(),
            last_recons: Vec::new(),
            frame: 0,
            since_key: 0,
            force_key: false,
            rounding: false,
            refresh: vec![0; mb_w * mb_h],
            rate,
        })
    }

    /// The configuration headers (`VisualObjectSequence` to `VideoObjectLayer`): the MP4 `esds`
    /// `DecoderSpecificInfo` and the Matroska `CodecPrivate`.
    pub fn extradata(&self) -> &[u8] {
        &self.headers
    }

    /// Track description for muxers (`CodecId::Mpeg4Video`, `mp4v` tag, picture size, extradata).
    pub fn codec_params(&self) -> CodecParams {
        CodecParams::new(CodecId::Mpeg4Video).with_tag(*b"mp4v").with_size(self.cfg.width, self.cfg.height).with_extradata(self.headers.clone())
    }

    /// Code the next picture as an I-VOP.
    pub fn request_keyframe(&mut self) {
        self.force_key = true;
    }

    /// The decoder's picture for the most recent packet, cropped to the configured size.
    pub fn reconstruction(&self) -> Option<Reconstruction> {
        if let Some(last) = self.last_recons.last() {
            return Some(last.clone());
        }
        let r = self.reference.as_ref()?;
        Some(crop_recon(r, self.cfg.width, self.cfg.height))
    }

    /// Reconstructions produced by the most recent [`Encoder::encode_packets`] or [`Encoder::flush`]
    /// call, in display order.
    pub fn reconstructions(&self) -> &[Reconstruction] {
        &self.last_recons
    }

    /// Copy the input into macroblock-aligned planes, repeating the last column and row.
    fn load(&self, f: &YuvFrame) -> Result<Frame, Error> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let fits = |p: &[u8], stride: usize, w: usize, h: usize| {
            stride >= w && (h - 1).checked_mul(stride).and_then(|n| n.checked_add(w)).is_some_and(|n| n <= p.len())
        };
        if !fits(f.y, f.y_stride, w, h) || !fits(f.u, f.uv_stride, w / 2, h / 2) || !fits(f.v, f.uv_stride, w / 2, h / 2) {
            return Err(Error::BadFrame);
        }
        let mut out = Frame::new(self.mb_w, self.mb_h);
        let fill = |dst: &mut Plane, src: &[u8], stride: usize, w: usize, h: usize| {
            for y in 0..dst.h {
                let row = &src[y.min(h - 1) * stride..][..w];
                let d = &mut dst.data[y * dst.w..][..dst.w];
                d[..w].copy_from_slice(row);
                let edge = row[w - 1];
                d[w..].fill(edge);
            }
        };
        fill(&mut out.y, f.y, f.y_stride, w, h);
        fill(&mut out.u, f.u, f.uv_stride, w / 2, h / 2);
        fill(&mut out.v, f.v, f.uv_stride, w / 2, h / 2);
        Ok(out)
    }

    fn choose_qp(&self, intra: bool) -> u8 {
        match (&self.rate, self.cfg.rate) {
            (Some(r), _) => r.qp(intra),
            (None, RateControl::Quant(q)) => q,
            (None, RateControl::Bitrate { .. }) => 8,
        }
    }

    fn b_qp(anchor_qp: u8) -> u8 {
        (anchor_qp + 1 + anchor_qp / 4).clamp(2, 31)
    }

    fn code_anchor_picture(&mut self, src: &Frame, intra: bool, qp: u8) -> Coded {
        if !intra {
            self.rounding = !self.rounding;
        } else {
            self.rounding = false;
        }
        let mut refresh = std::mem::take(&mut self.refresh);
        if intra {
            refresh.fill(0);
        }
        let coded = PictureCoder {
            src,
            reference: if intra { None } else { self.reference.as_ref() },
            bwd_reference: None,
            width: self.cfg.width as usize,
            height: self.cfg.height as usize,
            mb_w: self.mb_w,
            mb_h: self.mb_h,
            qp: i32::from(qp),
            rounding: self.rounding,
            ac_prediction: self.cfg.ac_prediction,
            four_mv: self.cfg.four_mv,
            mpeg_quant: self.cfg.mpeg_quant,
            intra_matrix: &self.intra_matrix,
            non_intra_matrix: &self.non_intra_matrix,
            quarter_sample: self.cfg.quarter_sample,
            slice_mbs: self.cfg.slice_mbs,
            refresh: &mut refresh,
        }
        .code();
        self.refresh = refresh;
        coded
    }

    fn code_b_picture(&mut self, src: &Frame, bwd: &Frame, trb: i64, trd: i64, qp: u8) -> Coded {
        let mut refresh = std::mem::take(&mut self.refresh);
        let coded = PictureCoder {
            src,
            reference: self.reference.as_ref(),
            bwd_reference: Some((bwd, trb, trd)),
            width: self.cfg.width as usize,
            height: self.cfg.height as usize,
            mb_w: self.mb_w,
            mb_h: self.mb_h,
            qp: i32::from(qp),
            rounding: false,
            ac_prediction: self.cfg.ac_prediction,
            four_mv: false,
            mpeg_quant: self.cfg.mpeg_quant,
            intra_matrix: &self.intra_matrix,
            non_intra_matrix: &self.non_intra_matrix,
            quarter_sample: self.cfg.quarter_sample,
            slice_mbs: self.cfg.slice_mbs,
            refresh: &mut refresh,
        }
        .code();
        self.refresh = refresh;
        coded
    }

    fn serialize_vop(&self, w: &mut BitWriter, coded: &Coded, coding_type: VopCodingType, display_idx: u64, base_seconds: u64, qp: u8) -> u64 {
        let ticks = display_idx * u64::from(self.timing.increment);
        let res = u64::from(self.timing.resolution);
        let seconds = ticks / res;
        let intra = coding_type == VopCodingType::I;
        if intra && self.cfg.headers_in_band {
            w.write_bytes(&self.headers);
        }
        let header = VopHeader {
            coding_type,
            seconds: seconds.saturating_sub(base_seconds).min(u64::from(u32::MAX)) as u32,
            time_increment: (ticks % res) as u32,
            rounding: self.rounding,
            quant: qp,
            fcode_forward: coded.fcode_forward,
            fcode_backward: coded.fcode_backward,
        };
        headers::vop_header(w, &header, self.timing);
        if coding_type != VopCodingType::N {
            picture::write_vop_mbs(w, &coded.mbs, coding_type, coded.fcode_forward, coded.fcode_backward, qp, false);
        }
        headers::stuffing(w);
        seconds
    }

    fn emit_anchor(&mut self, display_idx: u64, src: &Frame, force_intra: bool) -> (Packet, Reconstruction) {
        let intra = self.reference.is_none() || force_intra || self.since_key >= self.cfg.keyint;
        let qp = self.choose_qp(intra);
        let coded = self.code_anchor_picture(src, intra, qp);
        let coding_type = if intra { VopCodingType::I } else { VopCodingType::P };
        let mut w = BitWriter::new();
        let seconds = self.serialize_vop(&mut w, &coded, coding_type, display_idx, self.last_anchor_seconds, qp);
        let data = w.finish();
        if let Some(r) = &mut self.rate {
            r.update(intra, data.len() * 8, qp);
        }
        let recon = crop_recon(&coded.recon, self.cfg.width, self.cfg.height);
        self.past_anchor_seconds = self.last_anchor_seconds;
        self.last_anchor_seconds = seconds;
        self.reference = Some(coded.recon);
        self.ref_display_idx = display_idx;
        self.since_key = if intra { 1 } else { self.since_key + 1 };
        (Packet { data, keyframe: intra, pts: display_idx as i64, quant: qp }, recon)
    }

    /// Flush buffered B-frames with `anchor` `(display_idx, src)` as the backward reference P-VOP.
    fn emit_b_group(&mut self, anchor_idx: u64, anchor_src: &Frame) -> Vec<Packet> {
        let pending = std::mem::take(&mut self.pending_b);
        let p_qp = self.choose_qp(false);
        let p_coded = self.code_anchor_picture(anchor_src, false, p_qp);
        let past_anchor_sec = self.last_anchor_seconds;
        let past_display_idx = self.ref_display_idx;

        let mut p_writer = BitWriter::new();
        let p_seconds = self.serialize_vop(&mut p_writer, &p_coded, VopCodingType::P, anchor_idx, past_anchor_sec, p_qp);

        let b_qp = Self::b_qp(p_qp);
        let inc = i64::from(self.timing.increment);
        let trd = ((anchor_idx - past_display_idx) as i64) * inc;
        let mut b_codeds = Vec::with_capacity(pending.len());
        let mut b_recons = Vec::with_capacity(pending.len());
        for &(b_idx, ref b_src, _) in &pending {
            let trb = ((b_idx - past_display_idx) as i64) * inc;
            let b_coded = self.code_b_picture(b_src, &p_coded.recon, trb, trd, b_qp);
            b_recons.push(crop_recon(&b_coded.recon, self.cfg.width, self.cfg.height));
            b_codeds.push((b_idx, b_coded));
        }
        let p_recon = crop_recon(&p_coded.recon, self.cfg.width, self.cfg.height);

        let mut packets = Vec::with_capacity(1 + b_codeds.len());
        if self.cfg.packed_b_frames && !b_codeds.is_empty() {
            // Packed bitstream: pack P-VOP + B-VOPs into the first packet, then emit one N-VOP
            // placeholder per packed B-VOP (a total of <= 7 bytes).
            for &(b_idx, ref b_coded) in &b_codeds {
                self.serialize_vop(&mut p_writer, b_coded, VopCodingType::B, b_idx, past_anchor_sec, b_qp);
            }
            let first_data = p_writer.finish();
            if let Some(r) = &mut self.rate {
                r.update(false, first_data.len() * 8, p_qp);
            }
            packets.push(Packet { data: first_data, keyframe: false, pts: pending[0].0 as i64, quant: p_qp });
            let mut nw = BitWriter::new();
            let ticks = anchor_idx * u64::from(self.timing.increment);
            let res = u64::from(self.timing.resolution);
            let seconds = ticks / res;
            let header = VopHeader {
                coding_type: VopCodingType::N,
                seconds: seconds.saturating_sub(p_seconds).min(u64::from(u32::MAX)) as u32,
                time_increment: (ticks % res) as u32,
                rounding: false,
                quant: p_qp,
                fcode_forward: 1,
                fcode_backward: 1,
            };
            headers::vop_header(&mut nw, &header, self.timing);
            headers::stuffing(&mut nw);
            let nvop_bytes = nw.finish();
            for (k, _) in pending.iter().enumerate() {
                let pts = if k + 1 < pending.len() { pending[k + 1].0 as i64 } else { anchor_idx as i64 };
                packets.push(Packet { data: nvop_bytes.clone(), keyframe: false, pts, quant: p_qp });
            }
        } else {
            let p_data = p_writer.finish();
            if let Some(r) = &mut self.rate {
                r.update(false, p_data.len() * 8, p_qp);
            }
            packets.push(Packet { data: p_data, keyframe: false, pts: anchor_idx as i64, quant: p_qp });
            for (b_idx, b_coded) in b_codeds {
                let mut bw = BitWriter::new();
                self.serialize_vop(&mut bw, &b_coded, VopCodingType::B, b_idx, past_anchor_sec, b_qp);
                let b_data = bw.finish();
                if let Some(r) = &mut self.rate {
                    r.update(false, b_data.len() * 8, b_qp);
                }
                packets.push(Packet { data: b_data, keyframe: false, pts: b_idx as i64, quant: b_qp });
            }
        }

        self.last_recons.extend(b_recons);
        self.last_recons.push(p_recon);
        self.past_anchor_seconds = past_anchor_sec;
        self.last_anchor_seconds = p_seconds;
        self.reference = Some(p_coded.recon);
        self.ref_display_idx = anchor_idx;
        self.since_key += (pending.len() as u32) + 1;
        packets
    }

    /// Encode one input picture and return any packets ready for output in decode order.
    ///
    /// When `b_frames == 0`, this always returns a single packet. When `b_frames > 0`, B-frames
    /// are buffered until their future anchor arrives (or [`Encoder::flush`] is called).
    pub fn encode_packets(&mut self, frame: &YuvFrame) -> Result<Vec<Packet>, Error> {
        let src = self.load(frame)?;
        self.last_recons.clear();
        let display_idx = self.frame;
        self.frame += 1;
        let force_key = std::mem::replace(&mut self.force_key, false);

        if self.cfg.b_frames == 0 || self.reference.is_none() {
            let (pkt, recon) = self.emit_anchor(display_idx, &src, force_key);
            self.last_recons.push(recon);
            return Ok(vec![pkt]);
        }

        let need_key = force_key || (self.since_key + (self.pending_b.len() as u32) + 1 >= self.cfg.keyint);
        if need_key {
            let mut out = Vec::new();
            if let Some((last_idx, last_src, _)) = self.pending_b.pop() {
                out.extend(self.emit_b_group(last_idx, &last_src));
            }
            let (pkt, recon) = self.emit_anchor(display_idx, &src, true);
            self.last_recons.push(recon);
            out.push(pkt);
            return Ok(out);
        }

        if self.pending_b.len() < usize::from(self.cfg.b_frames) {
            self.pending_b.push((display_idx, src, false));
            return Ok(Vec::new());
        }

        Ok(self.emit_b_group(display_idx, &src))
    }

    /// Flush any buffered B-frames at the end of a stream, coding the final buffered picture as a
    /// P-VOP anchor.
    pub fn flush(&mut self) -> Vec<Packet> {
        self.last_recons.clear();
        let Some((last_idx, last_src, _)) = self.pending_b.pop() else {
            return Vec::new();
        };
        self.emit_b_group(last_idx, &last_src)
    }

    /// Encode one picture when `b_frames == 0`.
    pub fn encode(&mut self, frame: &YuvFrame) -> Result<Packet, Error> {
        if self.cfg.b_frames > 0 {
            return Err(Error::InvalidConfig("use encode_packets() and flush() when b_frames > 0".into()));
        }
        let mut pkts = self.encode_packets(frame)?;
        pkts.pop().ok_or_else(|| Error::InvalidConfig("no packet produced".into()))
    }
}
