//! MPEG-2 video encoder (ITU-T H.262 | ISO/IEC 13818-2), feature `mpeg2-encode`.
//!
//! MPEG-2 encoding in this crate is built in steps (storytold/craft-libs#1133). So far: I, P and B
//! pictures, progressive frame pictures, 4:2:0 (Main Profile) and 4:2:2 (4:2:2 Profile), closed or
//! open GOPs, constant quantiser or single-pass average bitrate. A VBV guarantee, interlaced
//! coding and DVD / broadcast presets come later.
//!
//! ```no_run
//! use craft_video::mpeg2::encode::{Encoder, EncoderConfig, RateControl, YuvFrame};
//!
//! let mut cfg = EncoderConfig::new(720, 576, 25, 1);
//! cfg.rate = RateControl::Abr { kbps: 6000 };
//! let mut enc = Encoder::new(cfg)?;
//! let (y, c) = (vec![128u8; 720 * 576], vec![128u8; 360 * 288]);
//! let mut es = Vec::new();
//! for i in 0..50 {
//!     let frame = YuvFrame { y: &y, cb: &c, cr: &c, y_stride: 720, c_stride: 360 };
//!     for p in enc.encode(&frame, i)? {
//!         es.extend_from_slice(&p.data);
//!     }
//! }
//! for p in enc.flush() {
//!     es.extend_from_slice(&p.data);
//! }
//! es.extend_from_slice(enc.end_of_sequence());
//! # Ok::<(), craft_video::mpeg2::encode::EncodeError>(())
//! ```

mod bitwriter;
mod motion;
mod picture;
mod rate;
mod tables;
mod transform;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use crate::mpeg2::ChromaFormat;
use crate::mpeg2::headers::{DEFAULT_INTRA, PictureType, SCAN};
use crate::mpeg2::mc::Frame;

use bitwriter::BitWriter;
use picture::{PicCtx, RowOut, RowPlanes, encode_row, qcode_for, qscale};
use rate::RateCtl;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("unsupported MPEG-2 encoder configuration: {0}")]
    Config(String),
    #[error("bad input frame: {0}")]
    BadFrame(String),
    /// With B pictures, presentation times must increase by a constant step (the decoding times
    /// are the presentation times one step earlier).
    #[error("bad presentation time: {0}")]
    BadTimestamp(String),
}

/// aspect_ratio_information (H.262 Table 6-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AspectRatio {
    /// Square samples.
    Square,
    /// 4:3 display.
    Dar4x3,
    /// 16:9 display.
    Dar16x9,
    /// 2.21:1 display.
    Dar221x100,
}

impl AspectRatio {
    fn code(self) -> u32 {
        match self {
            AspectRatio::Square => 1,
            AspectRatio::Dar4x3 => 2,
            AspectRatio::Dar16x9 => 3,
            AspectRatio::Dar221x100 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateControl {
    /// A constant quantiser_scale_code (1..=31; mapped through H.262 Table 7-6 with
    /// `nonlinear_quant`). Adaptive quantisation still varies it per macroblock when enabled.
    Quantiser(u8),
    /// Single-pass average bitrate in kbit/s. The stream is signalled as variable bitrate and
    /// is not checked against the VBV buffer.
    Abr { kbps: u32 },
}

#[derive(Clone, Debug)]
pub struct EncoderConfig {
    /// Picture size: even, at most 1920×1152 (4:2:0) or 1920×1088 (4:2:2).
    pub width: u32,
    pub height: u32,
    /// Frame rate: one of H.262 Table 6-4 (23.976, 24, 25, 29.97, 30, 50, 59.94, 60).
    pub fps_num: u32,
    pub fps_den: u32,
    /// 4:2:0 (Main Profile) or 4:2:2 (4:2:2 Profile).
    pub chroma: ChromaFormat,
    pub aspect: AspectRatio,
    /// Pictures from one I picture to the next (display order). 1 codes every picture as I.
    /// 1..=1023, and at most 1024 with `b_pictures` added.
    pub gop_size: u32,
    /// B pictures between two anchor (I or P) pictures, 0..=4. 0 gives I and P pictures only and a
    /// low-delay stream (no reordering).
    pub b_pictures: u32,
    /// Closed GOPs: the B pictures shown before a GOP's I picture predict from it alone, so the
    /// GOP decodes on its own. Open GOPs let them predict from the previous GOP too.
    pub closed_gop: bool,
    pub rate: RateControl,
    /// q_scale_type: the non-linear quantiser scale (finer steps at low quantisers).
    pub nonlinear_quant: bool,
    /// Vary the quantiser per macroblock with its activity (finer in flat areas).
    pub adaptive_quant: bool,
    /// Intra DC precision in bits: 8, 9 or 10.
    pub intra_dc_bits: u8,
    /// Quantiser matrices (raster order, values 1..=255); `None`: the defaults.
    pub intra_matrix: Option<[u8; 64]>,
    pub non_intra_matrix: Option<[u8; 64]>,
    /// Largest motion vector searched, in luma samples (1..=63).
    pub search_range: u32,
    /// Code slices on the rayon pool (with the `threads` feature).
    pub threads: bool,
}

impl EncoderConfig {
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        Self {
            width,
            height,
            fps_num,
            fps_den,
            chroma: ChromaFormat::Yuv420,
            aspect: AspectRatio::Square,
            gop_size: 12,
            b_pictures: 2,
            closed_gop: true,
            rate: RateControl::Quantiser(4),
            nonlinear_quant: true,
            adaptive_quant: true,
            intra_dc_bits: 9,
            intra_matrix: None,
            non_intra_matrix: None,
            search_range: 31,
            threads: cfg!(feature = "threads"),
        }
    }
}

/// Planar 8-bit input: luma `width × height`, chroma `width/2 × height/2` (4:2:0) or
/// `width/2 × height` (4:2:2), rows `y_stride` / `c_stride` bytes apart.
#[derive(Clone, Copy, Debug)]
pub struct YuvFrame<'a> {
    pub y: &'a [u8],
    pub cb: &'a [u8],
    pub cr: &'a [u8],
    pub y_stride: usize,
    pub c_stride: usize,
}

/// One coded picture, in decoding order.
#[derive(Clone, Debug)]
pub struct Packet {
    /// Elementary stream bytes: the sequence header and GOP header before every I picture, then
    /// the picture.
    pub data: Vec<u8>,
    pub pts: i64,
    /// Decoding timestamp: `pts` without B pictures; with them, the presentation times in
    /// decoding order one picture interval earlier, so `dts <= pts`.
    pub dts: i64,
    /// An I picture (random access point; in an open GOP the B pictures after it in decoding
    /// order also need the previous GOP).
    pub keyframe: bool,
    pub picture_type: PictureType,
    /// Average quantiser_scale of the picture's macroblocks.
    pub quantiser_scale: f32,
}

/// The encoder's reconstruction of a picture (what a decoder outputs), cropped.
#[derive(Clone, Debug)]
pub struct ReconFrame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
}

/// The sequence_end_code.
const SEQUENCE_END: [u8; 4] = [0, 0, 1, 0xB7];

/// A profile and level (H.262 Tables 8-8 to 8-13): limits and profile_and_level_indication.
#[derive(Clone, Copy, Debug)]
struct Level {
    indication: u8,
    max_w: u32,
    max_h: u32,
    max_fps: u32,
    luma_rate: u64,
    bit_rate: u64,
    vbv_bits: u64,
}

const MAIN_LEVELS: [Level; 4] = [
    Level { indication: 0x4A, max_w: 352, max_h: 288, max_fps: 30, luma_rate: 3_041_280, bit_rate: 4_000_000, vbv_bits: 475_136 },
    Level { indication: 0x48, max_w: 720, max_h: 576, max_fps: 30, luma_rate: 10_368_000, bit_rate: 15_000_000, vbv_bits: 1_835_008 },
    Level { indication: 0x46, max_w: 1440, max_h: 1152, max_fps: 60, luma_rate: 47_001_600, bit_rate: 60_000_000, vbv_bits: 7_340_032 },
    Level { indication: 0x44, max_w: 1920, max_h: 1152, max_fps: 60, luma_rate: 62_668_800, bit_rate: 80_000_000, vbv_bits: 9_781_248 },
];

const PROFILE_422_LEVELS: [Level; 2] = [
    Level { indication: 0x85, max_w: 720, max_h: 608, max_fps: 30, luma_rate: 11_059_200, bit_rate: 50_000_000, vbv_bits: 9_437_184 },
    Level { indication: 0x82, max_w: 1920, max_h: 1088, max_fps: 60, luma_rate: 62_668_800, bit_rate: 300_000_000, vbv_bits: 47_185_920 },
];

/// frame_rate_code for an exact frame rate (Table 6-4).
fn frame_rate_code(num: u32, den: u32) -> Option<u32> {
    if num == 0 || den == 0 {
        return None;
    }
    let rates: [(u64, u64); 8] = [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1)];
    rates.iter().position(|&(n, d)| u64::from(num) * d == n * u64::from(den)).map(|i| i as u32 + 1)
}

/// The motion search's f_code: the smallest whose range [-8f, 8f-1] samples covers `range`.
fn f_code_for(range: u32) -> u8 {
    (1..=4u8).find(|&fc| 8 * (1u32 << (fc - 1)) > range).unwrap_or(4)
}

/// Pick the lowest level of the profile that holds the stream.
fn choose_level(cfg: &EncoderConfig) -> Result<Level, EncodeError> {
    let levels: &[Level] = match cfg.chroma {
        ChromaFormat::Yuv420 => &MAIN_LEVELS,
        ChromaFormat::Yuv422 => &PROFILE_422_LEVELS,
        ChromaFormat::Yuv444 => return Err(EncodeError::Config("4:4:4 has no MPEG-2 profile in use".into())),
    };
    let fps = f64::from(cfg.fps_num) / f64::from(cfg.fps_den);
    let luma_rate = f64::from(cfg.width) * f64::from(cfg.height) * fps;
    let bps = match cfg.rate {
        RateControl::Abr { kbps } => u64::from(kbps) * 1000,
        RateControl::Quantiser(_) => 0,
    };
    levels
        .iter()
        .find(|l| {
            cfg.width <= l.max_w
                && cfg.height <= l.max_h
                && fps <= f64::from(l.max_fps) + 0.01
                && luma_rate <= l.luma_rate as f64
                && bps <= l.bit_rate
        })
        .copied()
        .ok_or_else(|| {
            EncodeError::Config(format!(
                "{}x{} at {:.3} fps{} is beyond every level of the profile",
                cfg.width,
                cfg.height,
                fps,
                if bps > 0 { format!(", {bps} bit/s") } else { String::new() }
            ))
        })
}

fn validate(cfg: &EncoderConfig) -> Result<(), EncodeError> {
    let bad = |s: String| Err(EncodeError::Config(s));
    if cfg.width == 0 || cfg.height == 0 || !cfg.width.is_multiple_of(2) || !cfg.height.is_multiple_of(2) {
        return bad(format!("picture size {}x{} must be even and non-zero", cfg.width, cfg.height));
    }
    if frame_rate_code(cfg.fps_num, cfg.fps_den).is_none() {
        return bad(format!("frame rate {}/{} is not one of H.262 Table 6-4", cfg.fps_num, cfg.fps_den));
    }
    if !(1..=1023).contains(&cfg.gop_size) {
        return bad(format!("GOP size {} (1..=1023)", cfg.gop_size));
    }
    if cfg.b_pictures > 4 {
        return bad(format!("{} B pictures between anchors (0..=4)", cfg.b_pictures));
    }
    // temporal_reference (10 bits) counts a GOP's leading B pictures too
    if cfg.gop_size + cfg.b_pictures > 1024 {
        return bad(format!("GOP size {} with {} B pictures: more than 1024 pictures to number", cfg.gop_size, cfg.b_pictures));
    }
    match cfg.rate {
        RateControl::Quantiser(q) if !(1..=31).contains(&q) => return bad(format!("quantiser_scale_code {q} (1..=31)")),
        RateControl::Abr { kbps: 0 } => return bad("bitrate 0".into()),
        _ => {}
    }
    if !(8..=10).contains(&cfg.intra_dc_bits) {
        return bad(format!("intra DC precision {} bits (8..=10)", cfg.intra_dc_bits));
    }
    if [cfg.intra_matrix, cfg.non_intra_matrix].iter().flatten().any(|m| m.contains(&0)) {
        return bad("quantiser matrix with a zero entry".into());
    }
    if !(1..=63).contains(&cfg.search_range) {
        return bad(format!("search range {} (1..=63)", cfg.search_range));
    }
    Ok(())
}

/// Write a quantiser matrix (raster order) in zigzag order.
fn put_matrix(w: &mut BitWriter, m: &[u8; 64]) {
    for &pos in &SCAN[0] {
        w.put(u32::from(m[usize::from(pos)]), 8);
    }
}

/// A source picture waiting for the anchor after it (a B picture).
struct Held {
    src: Frame,
    pts: i64,
    /// Display index.
    n: u64,
}

/// A coded picture before rate control and timestamps.
struct Coded {
    packet: Packet,
    recon: Frame,
    mvs: Vec<[i32; 2]>,
    avg_q: f64,
}

/// Quantiser ratio of B to P pictures with a constant quantiser (TM5 uses 1.4 for its rate
/// allocation; a constant quantiser keeps B pictures a little closer to the anchors).
const B_QUANT_FACTOR: f64 = 1.25;

/// A stateful encoder. Feed pictures in display order; packets come out in decoding order.
pub struct Encoder {
    cfg: EncoderConfig,
    level: Level,
    mb_w: usize,
    mb_h: usize,
    shx: u32,
    shy: u32,
    f_code: u8,
    intra_matrix: [u8; 64],
    inter_matrix: [u8; 64],
    /// Sequence header and sequence extension, sent before every GOP.
    seq_header: Vec<u8>,
    /// The reconstructions of the last two anchor pictures and their display indices: `latest`
    /// is the backward reference of the B pictures held, and the forward reference of the next
    /// P picture.
    older: Option<(Frame, u64)>,
    latest: Option<(Frame, u64)>,
    /// Forward vectors of the latest anchor when it is a P picture (search candidates).
    anchor_mvs: Vec<[i32; 2]>,
    /// B pictures waiting for their backward reference.
    held: Vec<Held>,
    /// Display index of the next picture.
    count: u64,
    /// Display indices of the current GOP's I picture and of its first picture in display order
    /// (a leading B picture in front of the I picture, or the I picture).
    gop_start: u64,
    gop_first: u64,
    /// The current GOP is closed: its leading B pictures predict backward only.
    gop_closed: bool,
    /// Presentation times not yet given to a coded picture as its decoding time, in input order.
    display_pts: VecDeque<i64>,
    last_pts: Option<i64>,
    pts_step: Option<i64>,
    /// Packets waiting for the presentation time step (the decoding time offset).
    out: Vec<Packet>,
    rc: Option<RateCtl>,
    recon_capture: bool,
    recon: Vec<(u64, ReconFrame)>,
}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Self, EncodeError> {
        validate(&cfg)?;
        let level = choose_level(&cfg)?;
        let (shx, shy) = if cfg.chroma == ChromaFormat::Yuv422 { (1, 0) } else { (1, 1) };
        let nonlinear = cfg.nonlinear_quant;
        let q_range = (f64::from(qscale(nonlinear, 1)), f64::from(qscale(nonlinear, 31)));
        let rc = match cfg.rate {
            RateControl::Abr { kbps } => Some(RateCtl::new(f64::from(kbps) * 1000.0, f64::from(cfg.fps_num) / f64::from(cfg.fps_den), q_range)),
            RateControl::Quantiser(_) => None,
        };
        let mut enc = Encoder {
            level,
            mb_w: (cfg.width as usize).div_ceil(16),
            mb_h: (cfg.height as usize).div_ceil(16),
            shx,
            shy,
            f_code: f_code_for(cfg.search_range),
            intra_matrix: cfg.intra_matrix.unwrap_or(DEFAULT_INTRA),
            inter_matrix: cfg.non_intra_matrix.unwrap_or([16; 64]),
            seq_header: Vec::new(),
            older: None,
            latest: None,
            anchor_mvs: Vec::new(),
            held: Vec::new(),
            count: 0,
            gop_start: 0,
            gop_first: 0,
            gop_closed: true,
            display_pts: VecDeque::new(),
            last_pts: None,
            pts_step: None,
            out: Vec::new(),
            rc,
            recon_capture: false,
            recon: Vec::new(),
            cfg,
        };
        enc.seq_header = enc.sequence_header_bytes();
        Ok(enc)
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// profile_and_level_indication of the stream (e.g. 0x48 Main Profile @ Main Level).
    pub fn profile_and_level(&self) -> u8 {
        self.level.indication
    }

    /// The sequence header and sequence extension (for containers that store them apart).
    pub fn sequence_header(&self) -> &[u8] {
        &self.seq_header
    }

    /// The sequence_end_code to append after the last packet of an elementary stream.
    pub fn end_of_sequence(&self) -> &'static [u8] {
        &SEQUENCE_END
    }

    /// Keep each picture's reconstruction for [`Encoder::take_recon`].
    pub fn set_recon_capture(&mut self, on: bool) {
        self.recon_capture = on;
    }

    /// The reconstructions captured so far, in display order.
    pub fn take_recon(&mut self) -> Vec<ReconFrame> {
        let mut r = std::mem::take(&mut self.recon);
        r.sort_by_key(|(n, _)| *n);
        r.into_iter().map(|(_, f)| f).collect()
    }

    /// Code the pictures held for reordering (the last of them becomes a P picture) and return
    /// every packet not yet returned. The next picture starts a new GOP.
    pub fn flush(&mut self) -> Vec<Packet> {
        let mut out = Vec::new();
        if let Some(last) = self.held.pop() {
            self.code_anchor(last, PictureType::P, &mut out);
        }
        self.older = None;
        self.latest = None;
        self.anchor_mvs.clear();
        self.release(out, true)
    }

    fn sequence_header_bytes(&self) -> Vec<u8> {
        let c = &self.cfg;
        let mut w = BitWriter::new();
        let bit_rate = self.level.bit_rate.div_ceil(400);
        let vbv = self.level.vbv_bits / 16384;
        w.start_code(0xB3);
        w.put(c.width & 0xFFF, 12);
        w.put(c.height & 0xFFF, 12);
        w.put(c.aspect.code(), 4);
        w.put(frame_rate_code(c.fps_num, c.fps_den).unwrap_or(3), 4);
        w.put((bit_rate & 0x3FFFF) as u32, 18);
        w.bit(true); // marker_bit
        w.put((vbv & 0x3FF) as u32, 10);
        w.bit(false); // constrained_parameters_flag
        for m in [&c.intra_matrix, &c.non_intra_matrix] {
            w.bit(m.is_some());
            if let Some(m) = m {
                put_matrix(&mut w, m);
            }
        }
        w.start_code(0xB5);
        w.put(1, 4); // sequence extension
        w.put(u32::from(self.level.indication), 8);
        w.bit(true); // progressive_sequence
        w.put(if c.chroma == ChromaFormat::Yuv422 { 2 } else { 1 }, 2);
        w.put(c.width >> 12, 2);
        w.put(c.height >> 12, 2);
        w.put((bit_rate >> 18) as u32 & 0xFFF, 12);
        w.bit(true); // marker_bit
        w.put((vbv >> 10) as u32 & 0xFF, 8);
        w.bit(c.b_pictures == 0); // low_delay: no B pictures, no reordering
        w.put(0, 2); // frame_rate_extension_n
        w.put(0, 5); // frame_rate_extension_d
        w.finish()
    }

    /// group_of_pictures_header with the time code of display picture `n`.
    fn gop_header(&self, w: &mut BitWriter, n: u64, closed: bool) {
        let fps = u64::from(self.cfg.fps_num.div_ceil(self.cfg.fps_den)).max(1);
        let secs = n / fps;
        w.start_code(0xB8);
        w.bit(false); // drop_frame_flag
        w.put(((secs / 3600) % 24) as u32, 5);
        w.put(((secs / 60) % 60) as u32, 6);
        w.bit(true); // marker_bit
        w.put((secs % 60) as u32, 6);
        w.put((n % fps) as u32, 6);
        w.bit(closed); // closed_gop
        w.bit(false); // broken_link
    }

    fn picture_header(&self, w: &mut BitWriter, ptype: PictureType, temporal_reference: u64) {
        w.start_code(0x00);
        w.put((temporal_reference & 0x3FF) as u32, 10); // display order within the GOP
        w.put(
            match ptype {
                PictureType::P => 2,
                PictureType::B => 3,
                _ => 1,
            },
            3,
        );
        w.put(0xFFFF, 16); // vbv_delay: variable bitrate
        for dir in [PictureType::P, PictureType::B] {
            if ptype == dir || ptype == PictureType::B {
                // full_pel_{forward,backward}_vector and {forward,backward}_f_code: MPEG-2
                // carries them in the extension
                w.bit(false);
                w.put(7, 3);
            }
        }
        w.bit(false); // extra_bit_picture
        w.start_code(0xB5);
        w.put(8, 4); // picture coding extension
        let fc = u32::from(self.f_code);
        let f_codes = match ptype {
            PictureType::P => [fc, fc, 15, 15],
            PictureType::B => [fc; 4],
            _ => [15; 4],
        };
        for v in f_codes {
            w.put(v, 4);
        }
        w.put(u32::from(self.cfg.intra_dc_bits - 8), 2);
        w.put(3, 2); // frame picture
        w.bit(false); // top_field_first
        w.bit(true); // frame_pred_frame_dct
        w.bit(false); // concealment_motion_vectors
        w.bit(self.cfg.nonlinear_quant); // q_scale_type
        w.bit(true); // intra_vlc_format
        w.bit(false); // alternate_scan
        w.bit(false); // repeat_first_field
        w.bit(self.cfg.chroma == ChromaFormat::Yuv420); // chroma_420_type = progressive_frame
        w.bit(true); // progressive_frame
        w.bit(false); // composite_display_flag
    }

    /// Copy the input into a macroblock-aligned frame, repeating the last column and row.
    fn load(&self, f: &YuvFrame) -> Result<Frame, EncodeError> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let (cw, ch) = (w >> self.shx, h >> self.shy);
        let (pw, ph) = (self.mb_w * 16, self.mb_h * 16);
        let check = |name: &str, plane: &[u8], stride: usize, pw: usize, ph: usize| {
            let need = stride.checked_mul(ph - 1).and_then(|n| n.checked_add(pw));
            if stride < pw || need.is_none_or(|n| plane.len() < n) {
                return Err(EncodeError::BadFrame(format!("{name}: {} bytes with stride {stride} for {pw}x{ph}", plane.len())));
            }
            Ok(())
        };
        check("luma", f.y, f.y_stride, w, h)?;
        check("Cb", f.cb, f.c_stride, cw, ch)?;
        check("Cr", f.cr, f.c_stride, cw, ch)?;
        let mut out = Frame::new(pw, ph, pw >> self.shx, ph >> self.shy);
        for (k, (src, stride, sw, sh)) in [(f.y, f.y_stride, w, h), (f.cb, f.c_stride, cw, ch), (f.cr, f.c_stride, cw, ch)].into_iter().enumerate() {
            let dw = if k == 0 { pw } else { out.cw };
            let dh = if k == 0 { ph } else { out.ch };
            let dst = &mut out.planes[k];
            for r in 0..dh {
                let s = &src[r.min(sh - 1) * stride..][..sw];
                let d = &mut dst[r * dw..(r + 1) * dw];
                d[..sw].copy_from_slice(s);
                let last = s[sw - 1];
                d[sw..].fill(last);
            }
        }
        Ok(out)
    }

    /// quantiser_scale_code per macroblock: `base` scaled by each macroblock's activity
    /// (H.262 Test Model 5 §5.2.3) with adaptive quantisation.
    fn mb_quant(&self, src: &Frame, base: f64) -> Vec<u8> {
        let n = self.mb_w * self.mb_h;
        let nl = self.cfg.nonlinear_quant;
        if !self.cfg.adaptive_quant {
            return vec![qcode_for(nl, base); n];
        }
        let acts: Vec<f64> = (0..n).map(|a| activity(src, a % self.mb_w, a / self.mb_w)).collect();
        let avg = acts.iter().sum::<f64>() / n.max(1) as f64;
        acts.iter().map(|&a| qcode_for(nl, base * (2.0 * a + avg) / (a + 2.0 * avg))).collect()
    }

    /// With B pictures, presentation times must step by a constant amount. Checked before any
    /// state changes.
    fn check_pts(&self, pts: i64) -> Result<Option<i64>, EncodeError> {
        if self.cfg.b_pictures == 0 {
            return Ok(None);
        }
        let Some(last) = self.last_pts else { return Ok(None) };
        let step = pts.checked_sub(last).filter(|&d| d > 0).ok_or_else(|| EncodeError::BadTimestamp(format!("{pts} after {last}")))?;
        match self.pts_step {
            Some(s) if s != step => Err(EncodeError::BadTimestamp(format!("{pts} after {last}: step {step}, earlier {s}"))),
            _ => Ok(Some(step)),
        }
    }

    /// Encode one picture (display order). Returns the packets that became ready, in decoding
    /// order: with B pictures, a B picture is held until the anchor after it is coded.
    pub fn encode(&mut self, frame: &YuvFrame, pts: i64) -> Result<Vec<Packet>, EncodeError> {
        let src = self.load(frame)?;
        if let Some(step) = self.check_pts(pts)? {
            self.pts_step = Some(step);
        }
        self.last_pts = Some(pts);
        let n = self.count;
        self.count += 1;
        self.display_pts.push_back(pts);
        let held = Held { src, pts, n };
        let mut out = Vec::new();
        let m = u64::from(self.cfg.b_pictures) + 1;
        if self.latest.is_none() || n - self.gop_start >= u64::from(self.cfg.gop_size) {
            self.code_anchor(held, PictureType::I, &mut out);
        } else if (n - self.gop_start).is_multiple_of(m) {
            self.code_anchor(held, PictureType::P, &mut out);
        } else {
            self.held.push(held);
        }
        Ok(self.release(out, false))
    }

    /// Code an anchor picture, then the B pictures held in front of it.
    fn code_anchor(&mut self, h: Held, ptype: PictureType, out: &mut Vec<Packet>) {
        let intra = ptype == PictureType::I;
        if intra {
            self.gop_start = h.n;
            self.gop_first = self.held.first().map_or(h.n, |b| b.n);
            self.gop_closed = self.cfg.closed_gop || self.held.is_empty();
            let n = u64::from(self.cfg.gop_size);
            let m = u64::from(self.cfg.b_pictures) + 1;
            let np = (n - 1) / m;
            if let Some(rc) = self.rc.as_mut() {
                rc.start_gop(np as u32, (n - 1 - np) as u32);
            }
        }
        let fwd = if intra { None } else { self.latest.as_ref().map(|r| &r.0) };
        let cand: &[[i32; 2]] = if intra { &[] } else { &self.anchor_mvs };
        let c = self.code_picture(&h, ptype, (fwd, None), [cand, &[]]);
        let (recon, mvs) = self.finish(c, h.n, out);
        self.older = self.latest.take();
        self.latest = Some((recon, h.n));
        self.anchor_mvs = if intra { Vec::new() } else { mvs };
        for b in std::mem::take(&mut self.held) {
            self.code_b(b, out);
        }
    }

    /// Code a B picture between the two latest anchors. The leading B pictures of a closed GOP
    /// (shown before its I picture) predict backward only.
    fn code_b(&mut self, b: Held, out: &mut Vec<Packet>) {
        let Some((bwd, bn)) = self.latest.as_ref() else { return };
        let leading = b.n < self.gop_start;
        let fwd = self.older.as_ref().filter(|_| !(leading && self.gop_closed));
        // search candidates: the latest anchor's vectors (which span the two anchors) scaled to
        // this picture's distance from each
        let span = self.older.as_ref().map_or(1, |o| bn.saturating_sub(o.1).max(1)) as i64;
        let scaled = |d: i64| -> Vec<[i32; 2]> {
            self.anchor_mvs.iter().map(|v| [(i64::from(v[0]) * d / span) as i32, (i64::from(v[1]) * d / span) as i32]).collect()
        };
        let cf = fwd.map_or(Vec::new(), |f| scaled(b.n as i64 - f.1 as i64));
        let cb = scaled(b.n as i64 - *bn as i64);
        let c = self.code_picture(&b, PictureType::B, (fwd.map(|f| &f.0), Some(bwd)), [&cf, &cb]);
        self.finish(c, b.n, out);
    }

    /// Code one picture with the given references and search candidates.
    fn code_picture(&self, h: &Held, ptype: PictureType, (fwd, bwd): (Option<&Frame>, Option<&Frame>), cand: [&[[i32; 2]]; 2]) -> Coded {
        let base = match (&self.rc, self.cfg.rate) {
            (Some(rc), _) => rc.qscale(ptype),
            (None, RateControl::Quantiser(q)) => {
                let qs = f64::from(qscale(self.cfg.nonlinear_quant, q));
                if ptype == PictureType::B { qs * B_QUANT_FACTOR } else { qs }
            }
            (None, _) => 8.0,
        };
        let src = &h.src;
        let mb_qcode = self.mb_quant(src, base);
        let mut recon = Frame::new(src.w, src.h, src.cw, src.ch);
        let ctx = PicCtx {
            ptype,
            src,
            fwd,
            bwd,
            cand,
            mb_w: self.mb_w,
            shx: self.shx,
            shy: self.shy,
            intra_matrix: &self.intra_matrix,
            inter_matrix: &self.inter_matrix,
            nonlinear: self.cfg.nonlinear_quant,
            dc_prec: self.cfg.intra_dc_bits - 8,
            f_code: self.f_code,
            search: self.cfg.search_range as i32,
            mb_qcode: &mb_qcode,
        };
        let rows = encode_rows(&ctx, &mut recon, self.cfg.threads);
        let intra = ptype == PictureType::I;
        let mut w = BitWriter::new();
        if intra {
            self.gop_header(&mut w, self.gop_first, self.gop_closed);
        }
        self.picture_header(&mut w, ptype, h.n - self.gop_first);
        let mut data = if intra { self.seq_header.clone() } else { Vec::new() };
        data.extend_from_slice(&w.finish());
        let mut qsum = 0u64;
        let mut mvs = Vec::with_capacity(self.mb_w * self.mb_h);
        for r in rows {
            data.extend_from_slice(&r.bytes);
            qsum += r.stats.qsum;
            mvs.extend_from_slice(&r.mvs);
        }
        let avg_q = qsum as f64 / (self.mb_w * self.mb_h).max(1) as f64;
        let packet = Packet { data, pts: h.pts, dts: h.pts, keyframe: intra, picture_type: ptype, quantiser_scale: avg_q as f32 };
        Coded { packet, recon, mvs, avg_q }
    }

    /// Account for a coded picture (display index `n`): rate control, its decoding time, the
    /// reconstruction capture. Returns the reconstruction and the picture's vectors.
    fn finish(&mut self, c: Coded, n: u64, out: &mut Vec<Packet>) -> (Frame, Vec<[i32; 2]>) {
        let Coded { mut packet, recon, mvs, avg_q } = c;
        if let Some(rc) = self.rc.as_mut() {
            rc.update(packet.picture_type, packet.data.len() as u64 * 8, avg_q);
        }
        // the k-th coded picture decodes at the k-th presentation time (less the offset that
        // `release` subtracts); one time is queued per input picture
        packet.dts = self.display_pts.pop_front().unwrap_or(packet.pts);
        if self.recon_capture {
            self.recon.push((n, self.crop(&recon)));
        }
        out.push(packet);
        (recon, mvs)
    }

    /// Hand out packets once their decoding times are known: with B pictures they are the
    /// presentation times one step earlier, so packets wait for the second picture's time.
    fn release(&mut self, out: Vec<Packet>, flushing: bool) -> Vec<Packet> {
        self.out.extend(out);
        if self.cfg.b_pictures == 0 {
            return std::mem::take(&mut self.out);
        }
        if self.pts_step.is_none() && !flushing {
            return Vec::new();
        }
        let off = self.pts_step.unwrap_or(0);
        let mut res = std::mem::take(&mut self.out);
        for p in &mut res {
            p.dts -= off;
        }
        res
    }

    fn crop(&self, f: &Frame) -> ReconFrame {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        let (cw, ch) = (w >> self.shx, h >> self.shy);
        let crop = |p: &[u8], stride: usize, w: usize, h: usize| (0..h).flat_map(|r| p[r * stride..r * stride + w].iter().copied()).collect();
        ReconFrame {
            width: w as u32,
            height: h as u32,
            y: crop(&f.planes[0], f.w, w, h),
            cb: crop(&f.planes[1], f.cw, cw, ch),
            cr: crop(&f.planes[2], f.cw, cw, ch),
        }
    }
}

/// Macroblock activity: one plus the smallest variance of its four luma blocks (TM5).
fn activity(f: &Frame, mbx: usize, mby: usize) -> f64 {
    let mut min = f64::MAX;
    for b in 0..4 {
        let (x0, y0) = (mbx * 16 + (b & 1) * 8, mby * 16 + (b >> 1) * 8);
        let (mut s, mut s2) = (0u64, 0u64);
        for r in 0..8 {
            for &v in &f.planes[0][(y0 + r) * f.w + x0..][..8] {
                s += u64::from(v);
                s2 += u64::from(v) * u64::from(v);
            }
        }
        let mean = s as f64 / 64.0;
        min = min.min(s2 as f64 / 64.0 - mean * mean);
    }
    1.0 + min.max(0.0)
}

/// Code every macroblock row of the picture into `recon`, in parallel when allowed.
fn encode_rows(ctx: &PicCtx, recon: &mut Frame, threads: bool) -> Vec<RowOut> {
    let (w, cw) = (recon.w, recon.cw);
    let clines = 16 >> ctx.shy;
    let [y, cb, cr] = &mut recon.planes;
    let run = |r: usize, y: &mut [u8], cb: &mut [u8], cr: &mut [u8]| encode_row(ctx, r, &mut RowPlanes { y, cb, cr });
    #[cfg(feature = "threads")]
    if threads {
        use rayon::prelude::*;
        return y
            .par_chunks_mut(16 * w)
            .zip(cb.par_chunks_mut(clines * cw))
            .zip(cr.par_chunks_mut(clines * cw))
            .enumerate()
            .map(|(r, ((y, cb), cr))| run(r, y, cb, cr))
            .collect();
    }
    let _ = threads;
    y.chunks_mut(16 * w)
        .zip(cb.chunks_mut(clines * cw))
        .zip(cr.chunks_mut(clines * cw))
        .enumerate()
        .map(|(r, ((y, cb), cr))| run(r, y, cb, cr))
        .collect()
}
