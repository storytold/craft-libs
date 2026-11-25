//! Clean-room H.263 encoder (ITU-T H.263), written from the recommendation.
//!
//! - I- and P-pictures, 4:2:0 8-bit, one motion vector a macroblock at half-sample precision
//!   inside the picture, found with a predictor-seeded diamond search; skipped macroblocks;
//!   intra macroblocks in P-pictures when they predict worse than they code on their own, and
//!   every 128 inter codings (§4.4 asks for one within 132).
//! - The five standard picture sizes with the baseline syntax; any other size that is a multiple
//!   of four, and any pixel aspect ratio, with the custom picture format of PLUSPTYPE
//!   (§5.1.4, §5.1.5).
//! - Optional GOB headers, so a decoder can resynchronise after an error.
//! - Rate control: a constant quantiser, or a 1-pass average bitrate.
//!
//! No annex is used, so a packet is decodable by every H.263 decoder, and by the PLUSPTYPE ones
//! when the size or aspect ratio is not standard.
//!
//! ```no_run
//! use craft_video::h263::{Encoder, EncoderConfig, YuvFrame};
//! let cfg = EncoderConfig::new(176, 144, 25, 1);
//! let mut enc = Encoder::new(cfg).unwrap();
//! let (y, u, v) = (vec![128u8; 176 * 144], vec![128u8; 88 * 72], vec![128u8; 88 * 72]);
//! let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: 176, uv_stride: 88 };
//! let packet = enc.encode(&frame).unwrap();
//! ```

mod picture;

use craft_media::bitstream::BitWriter;

pub use crate::mpeg4::{Packet, RateControl, Reconstruction, YuvFrame};

use crate::mpeg4::encode::RateState;
use crate::mpeg4::encode::picture::Frame;
use picture::PictureCoder;

/// Picture start code, 22 bits (§5.1.1).
const PSC: u32 = 0x20;

/// The standard picture sizes, indexed by source format code minus one (Table 3 of §5.1.3).
const STANDARD_SIZES: [(u32, u32); 5] = [(128, 96), (176, 144), (352, 288), (704, 576), (1408, 1152)];

/// The pixel aspect ratio of the standard formats and the first code of the custom ones (§5.1.5).
const STANDARD_ASPECT: (u8, u8) = (12, 11);

/// Pixel aspect ratios with a code of their own, indexed by code minus one.
const ASPECT_CODES: [(u8, u8); 5] = [(1, 1), (12, 11), (10, 11), (16, 11), (40, 33)];

/// The custom picture format's limits: nine bits of width and height in units of four samples.
const MAX_WIDTH: u32 = 2048;
const MAX_HEIGHT: u32 = 1152;

/// Encoder errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("invalid dimensions {0}x{1} (must be a multiple of 4, at most {MAX_WIDTH}x{MAX_HEIGHT})")]
    InvalidDimensions(u32, u32),
    #[error("invalid frame rate {0}/{1}")]
    InvalidFrameRate(u32, u32),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("input frame planes too small for the configured size")]
    BadFrame,
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub rate: RateControl,
    /// Largest distance between I-pictures (1 = all intra).
    pub keyint: u32,
    /// Pixel aspect ratio (each 1–255). Anything but 12:11 needs the custom picture format.
    pub sar: (u8, u8),
    /// Write a GOB header before every GOB but the first, which lets a decoder resynchronise
    /// after an error at a cost of about 30 bits a GOB.
    pub gob_headers: bool,
}

impl EncoderConfig {
    /// Quantiser 4, an I-picture at least every 250 frames, the 12:11 pixel aspect ratio of the
    /// standard formats, no GOB headers.
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig { width, height, fps_num, fps_den, rate: RateControl::Quant(4), keyint: 250, sar: STANDARD_ASPECT, gob_headers: false }
    }
}

/// Macroblock rows in a GOB (§5.2): one up to 400 lines, two up to 800, four above.
fn gob_rows(height: u32) -> usize {
    match height {
        0..=400 => 1,
        401..=800 => 2,
        _ => 4,
    }
}

/// How a picture's size is signalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// A source format code of the baseline syntax (1–5).
    Standard(u32),
    /// The custom picture format of PLUSPTYPE.
    Custom,
}

/// The encoder: one picture in, one picture out (no B-pictures, so no reordering or delay).
pub struct Encoder {
    cfg: EncoderConfig,
    format: Format,
    mb_w: usize,
    mb_h: usize,
    reference: Option<Frame>,
    frame: u64,
    since_key: u32,
    force_key: bool,
    refresh: Vec<u8>,
    rate: Option<RateState>,
    /// GFID of the GOB headers: it changes when PTYPE does (§5.2.4).
    gfid: u32,
    last_intra: Option<bool>,
}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Encoder, EncodeError> {
        let (w, h) = (cfg.width, cfg.height);
        if w == 0 || h == 0 || w % 4 != 0 || h % 4 != 0 || w > MAX_WIDTH || h > MAX_HEIGHT {
            return Err(EncodeError::InvalidDimensions(w, h));
        }
        if cfg.fps_num == 0 || cfg.fps_den == 0 {
            return Err(EncodeError::InvalidFrameRate(cfg.fps_num, cfg.fps_den));
        }
        if cfg.keyint == 0 {
            return Err(EncodeError::InvalidConfig("keyint must be at least 1".into()));
        }
        if cfg.sar.0 == 0 || cfg.sar.1 == 0 {
            return Err(EncodeError::InvalidConfig("pixel aspect ratio terms must be non-zero".into()));
        }
        let rate = match cfg.rate {
            RateControl::Quant(q) if (1..=31).contains(&q) => None,
            RateControl::Quant(q) => return Err(EncodeError::InvalidConfig(format!("quantiser {q} is outside 1–31"))),
            RateControl::Bitrate { kbps: 0 } => return Err(EncodeError::InvalidConfig("bitrate must be non-zero".into())),
            RateControl::Bitrate { kbps } => Some(RateState::new(f64::from(kbps) * 1000.0 * f64::from(cfg.fps_den) / f64::from(cfg.fps_num))),
        };
        let standard = STANDARD_SIZES.iter().position(|&s| s == (w, h)).filter(|_| cfg.sar == STANDARD_ASPECT);
        let format = standard.map_or(Format::Custom, |i| Format::Standard(i as u32 + 1));
        let (mb_w, mb_h) = (w.div_ceil(16) as usize, h.div_ceil(16) as usize);
        Ok(Encoder {
            cfg,
            format,
            mb_w,
            mb_h,
            reference: None,
            frame: 0,
            since_key: 0,
            force_key: false,
            refresh: vec![0; mb_w * mb_h],
            rate,
            gfid: 0,
            last_intra: None,
        })
    }

    /// Code the next picture as an I-picture.
    pub fn request_keyframe(&mut self) {
        self.force_key = true;
    }

    /// The decoder's picture for the last packet, cropped to the configured size.
    pub fn reconstruction(&self) -> Option<Reconstruction> {
        crate::mpeg4::encode::crop(self.reference.as_ref()?, self.cfg.width, self.cfg.height)
    }

    /// Encode one picture.
    pub fn encode(&mut self, frame: &YuvFrame) -> Result<Packet, EncodeError> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let src = crate::mpeg4::encode::load_frame(frame, w, h, self.mb_w, self.mb_h).ok_or(EncodeError::BadFrame)?;
        let intra = self.reference.is_none() || self.force_key || self.since_key >= self.cfg.keyint;
        let qp = match (&self.rate, self.cfg.rate) {
            (Some(r), _) => r.qp(intra),
            (None, RateControl::Quant(q)) => q,
            (None, RateControl::Bitrate { .. }) => 8,
        };
        let mut refresh = std::mem::take(&mut self.refresh);
        if intra {
            refresh.fill(0);
        }
        let rows = gob_rows(self.cfg.height);
        let coded = PictureCoder {
            src: &src,
            reference: if intra { None } else { self.reference.as_ref() },
            width: w,
            height: h,
            mb_w: self.mb_w,
            mb_h: self.mb_h,
            qp: i32::from(qp),
            gob_rows: self.cfg.gob_headers.then_some(rows),
            refresh: &mut refresh,
        }
        .code();
        self.refresh = refresh;

        if self.last_intra.is_some_and(|last| last != intra) {
            self.gfid = (self.gfid + 1) & 3;
        }
        self.last_intra = Some(intra);

        let mut bits = BitWriter::new();
        self.picture_header(&mut bits, intra, qp);
        for (i, row) in coded.mbs.chunks(self.mb_w).enumerate() {
            if self.cfg.gob_headers && i > 0 && i % rows == 0 {
                bits.align_zero(); // GSTUF
                bits.write_bits(1, 17); // GBSC
                bits.write_bits((i / rows) as u32, 5);
                bits.write_bits(self.gfid, 2);
                bits.write_bits(u32::from(qp), 5);
            }
            for mb in row {
                picture::write_mb(&mut bits, mb, intra);
            }
        }
        bits.align_zero(); // PSTUF of the next picture
        let data = bits.finish();

        if let Some(r) = &mut self.rate {
            r.update(intra, data.len() * 8, qp);
        }
        self.reference = Some(coded.recon);
        let pts = self.frame as i64;
        self.frame += 1;
        self.since_key = if intra { 1 } else { self.since_key + 1 };
        self.force_key = false;
        Ok(Packet { data, keyframe: intra, pts, quant: qp })
    }

    /// The picture header (§5.1): the baseline PTYPE for a standard size, PLUSPTYPE otherwise.
    fn picture_header(&self, w: &mut BitWriter, intra: bool, qp: u8) {
        w.write_bits(PSC, 22);
        w.write_bits((self.frame & 0xFF) as u32, 8); // TR
        w.write_bits(0b10, 2); // PTYPE bits 1 and 2
        w.write_bits(0, 3); // split screen, document camera, freeze picture release
        match self.format {
            Format::Standard(code) => {
                w.write_bits(code, 3);
                w.write_bit(!intra);
                w.write_bits(0, 4); // UMV, SAC, AP, PB-frames: none
            }
            Format::Custom => {
                w.write_bits(0b111, 3); // PLUSPTYPE follows
                // UFEP 001 sends OPPTYPE, with the picture format, in every I-picture; a P-picture
                // keeps it (UFEP 000).
                w.write_bits(u32::from(intra), 3);
                if intra {
                    w.write_bits(0b110, 3); // custom source format
                    w.write_bits(0, 11); // no optional mode
                    w.write_bit(true); // bit 15
                    w.write_bits(0, 3);
                }
                // MPPTYPE: picture type, no RPR or RRU, RTYPE 0, reserved, marker.
                w.write_bits(u32::from(!intra), 3);
                w.write_bits(0, 3);
                w.write_bits(0, 2);
                w.write_bit(true);
                w.write_bit(false); // CPM
                if intra {
                    self.custom_format(w);
                }
            }
        }
        w.write_bits(u32::from(qp), 5);
        if matches!(self.format, Format::Standard(_)) {
            w.write_bit(false); // CPM
        }
        w.write_bit(false); // PEI
    }

    /// CPFMT and EPAR: the pixel aspect ratio and the picture size (§5.1.5, §5.1.6).
    fn custom_format(&self, w: &mut BitWriter) {
        let code = ASPECT_CODES.iter().position(|&a| a == self.cfg.sar).map_or(15, |i| i as u32 + 1);
        w.write_bits(code, 4);
        w.write_bits(self.cfg.width / 4 - 1, 9);
        w.write_bit(true);
        w.write_bits(self.cfg.height / 4, 9);
        if code == 15 {
            w.write_bits(u32::from(self.cfg.sar.0), 8);
            w.write_bits(u32::from(self.cfg.sar.1), 8);
        }
    }
}
