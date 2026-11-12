//! Clean-room MPEG-4 Part 2 (ISO/IEC 14496-2, MPEG-4 Visual) encoder, written from the
//! standard. It writes the Simple profile: the kind of stream Xvid and DivX produce and that
//! MP4 (`mp4v`), Matroska (`V_MPEG4/ISO/SP`) and AVI (`XVID`, `DIVX`, `FMP4`) carry.
//!
//! - I-VOPs and P-VOPs, 4:2:0 8-bit, rectangular, progressive, H.263 quantisation.
//! - One motion vector a macroblock at half-sample precision (`fcode` 1–2), found with a
//!   predictor-seeded diamond search; skipped macroblocks; intra macroblocks in P-VOPs when they
//!   predict worse than they code on their own, and every 128 inter codings (§7.4.5 asks for one
//!   within 132).
//! - Intra DC prediction, no AC prediction; all three coefficient escape modes.
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

use headers::{Sequence, Timing, VopHeader};
use picture::{Frame, PictureCoder, Plane};

/// Largest width or height (the VOL stores 13 bits; sizes are even).
pub const MAX_SIDE: u32 = 8190;

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
}

impl EncoderConfig {
    /// Quantiser 4, an I-VOP at least every 250 frames, square pixels, headers in band.
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig { width, height, fps_num, fps_den, rate: RateControl::Quant(4), keyint: 250, sar: (1, 1), color: None, headers_in_band: true }
    }
}

/// One coded VOP.
#[derive(Clone, Debug)]
pub struct Packet {
    pub data: Vec<u8>,
    /// An I-VOP (with the configuration headers in front when they are in band).
    pub keyframe: bool,
    /// The frame number, in units of `fps_den / fps_num` seconds.
    pub pts: i64,
    /// The quantiser the VOP was coded with.
    pub quant: u8,
}

/// The picture a decoder reconstructs from the last packet (to within the IDCT accuracy a
/// conforming decoder is held to).
#[derive(Clone, Debug)]
pub struct Reconstruction {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

/// `profile_and_level_indication` (Simple profile) for a picture of `mbs` macroblocks: the
/// smallest level whose picture size holds it. Bitrate and buffer limits are not checked, and
/// pictures over 3600 macroblocks are larger than any Simple profile level allows.
fn simple_level(mbs: usize) -> u8 {
    match mbs {
        0..=396 => 0x03,
        397..=1200 => 0x04,
        1201..=1620 => 0x05,
        _ => 0x06,
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
            // No history: about 0.15 bits a sample at quantiser 8 for detailed content.
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

/// The encoder: one picture in, one VOP out (no B-VOPs, so no reordering or delay).
pub struct Encoder {
    cfg: EncoderConfig,
    timing: Timing,
    mb_w: usize,
    mb_h: usize,
    headers: Vec<u8>,
    reference: Option<Frame>,
    frame: u64,
    since_key: u32,
    force_key: bool,
    last_seconds: u64,
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
        let headers = headers::sequence_headers(&Sequence {
            width: w,
            height: h,
            timing,
            profile_level: simple_level(mb_w * mb_h),
            sar: cfg.sar,
            color: cfg.color,
        });
        Ok(Encoder {
            cfg,
            timing,
            mb_w,
            mb_h,
            headers,
            reference: None,
            frame: 0,
            since_key: 0,
            force_key: false,
            last_seconds: 0,
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

    /// The decoder's picture for the last packet, cropped to the configured size.
    pub fn reconstruction(&self) -> Option<Reconstruction> {
        let r = self.reference.as_ref()?;
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let crop = |p: &Plane, w: usize, h: usize| (0..h).flat_map(|y| p.data[y * p.w..][..w].iter().copied()).collect();
        Some(Reconstruction {
            width: self.cfg.width,
            height: self.cfg.height,
            y: crop(&r.y, w, h),
            u: crop(&r.u, w / 2, h / 2),
            v: crop(&r.v, w / 2, h / 2),
        })
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

    /// Encode one picture.
    pub fn encode(&mut self, frame: &YuvFrame) -> Result<Packet, Error> {
        let src = self.load(frame)?;
        let intra = self.reference.is_none() || self.force_key || self.since_key >= self.cfg.keyint;
        let qp = match (&self.rate, self.cfg.rate) {
            (Some(r), _) => r.qp(intra),
            (None, RateControl::Quant(q)) => q,
            (None, RateControl::Bitrate { .. }) => 8,
        };
        if !intra {
            self.rounding = !self.rounding;
        }
        let mut refresh = std::mem::take(&mut self.refresh);
        if intra {
            refresh.fill(0);
        }
        let coded = PictureCoder {
            src: &src,
            reference: if intra { None } else { self.reference.as_ref() },
            width: self.cfg.width as usize,
            height: self.cfg.height as usize,
            mb_w: self.mb_w,
            mb_h: self.mb_h,
            qp: i32::from(qp),
            rounding: self.rounding,
            refresh: &mut refresh,
        }
        .code();
        self.refresh = refresh;

        let ticks = self.frame * u64::from(self.timing.increment);
        let res = u64::from(self.timing.resolution);
        let seconds = ticks / res;
        let mut w = BitWriter::new();
        if intra && self.cfg.headers_in_band {
            w.write_bytes(&self.headers);
        }
        let header = VopHeader {
            intra,
            seconds: (seconds - self.last_seconds).min(u64::from(u32::MAX)) as u32,
            time_increment: (ticks % res) as u32,
            rounding: self.rounding,
            quant: qp,
            fcode: coded.fcode,
        };
        headers::vop_header(&mut w, &header, self.timing);
        for mb in &coded.mbs {
            picture::write_mb(&mut w, mb, intra, coded.fcode, false);
        }
        headers::stuffing(&mut w);
        let data = w.finish();

        if let Some(r) = &mut self.rate {
            r.update(intra, data.len() * 8, qp);
        }
        self.reference = Some(coded.recon);
        self.last_seconds = seconds;
        let pts = self.frame as i64;
        self.frame += 1;
        self.since_key = if intra { 1 } else { self.since_key + 1 };
        self.force_key = false;
        Ok(Packet { data, keyframe: intra, pts, quant: qp })
    }
}
