//! Clean-room MPEG-4 Part 2 (ISO/IEC 14496-2, MPEG-4 Visual / Xvid / DivX) video decoder.
//!
//! Implemented from the published ISO/IEC 14496-2 and ITU-T H.263 specifications:
//! - Visual Object Sequence, Visual Object (`video_signal_type` colour tags), Video Object Layer
//!   (VOL), Group of VOP, VOP, video packet resync markers, and H.263 short video headers.
//! - Simple Profile and Advanced Simple Profile tools: I-, P-, B-, S(GMC)- and N-VOPs, 1MV and 4MV
//!   motion vectors, half-sample and quarter-sample (`quarter_sample`) motion compensation,
//!   adaptive intra DC and AC prediction with alternate horizontal/vertical scans, H.263 and MPEG
//!   quantisation matrices (with mismatch control), display-order B-VOP reordering, and packed
//!   bitstream unpacking.

mod headers;
mod mc;
mod picture;
mod vlc;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use craft_media::bitstream::{BitError, BitReader};

pub use headers::{ColorDescription, VopType};
use headers::{
    GROUP_OF_VOP_START, USER_DATA_START, VISUAL_OBJECT_SEQUENCE_END, VISUAL_OBJECT_SEQUENCE_START, VISUAL_OBJECT_START, VOP_START, VolConfig,
    is_packed_userdata, parse_visual_object, parse_vol, parse_vop_header, scan_start_codes, try_parse_short_video_header,
};
use mc::RefFrame;
use picture::decode_vop;

/// Maximum pictures held in memory at once for budget checks: two anchors, the frame being
/// decoded, up to [`MAX_OUTPUT_PICTURES_PER_PACKET`] retained frames, and one recovery frame.
const BUDGET_PICTURES: u64 = 4 + MAX_OUTPUT_PICTURES_PER_PACKET as u64;

/// Maximum pictures a single `decode` call may produce before refusing the packet.
const MAX_OUTPUT_PICTURES_PER_PACKET: usize = 8;

/// Errors returned by the MPEG-4 Part 2 decoder.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The bitstream is corrupt, truncated, or violates the specification.
    #[error("invalid MPEG-4 Part 2 video: {0}")]
    Invalid(String),
    /// The bitstream uses a valid MPEG-4 Visual tool that is not supported.
    #[error("unsupported MPEG-4 Part 2 video: {0}")]
    Unsupported(String),
}

impl From<BitError> for DecodeError {
    fn from(e: BitError) -> Self {
        DecodeError::Invalid(e.to_string())
    }
}

/// Stream-level configuration parsed from `VisualObjectSequence`, `VisualObject`, and
/// `VideoObjectLayer` headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceInfo {
    pub width: u32,
    pub height: u32,
    pub profile_and_level: Option<u8>,
    pub sar: (u8, u8),
    pub color: Option<ColorDescription>,
    pub low_delay: bool,
    pub interlaced: bool,
    pub quarter_sample: bool,
    pub time_increment_resolution: u32,
    pub fixed_time_increment: Option<u32>,
}

/// One decoded 8-bit 4:2:0 MPEG-4 Part 2 picture in display order, cropped to the coded size.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub pts: i64,
    pub vop_type: VopType,
    pub interlaced: bool,
    pub top_field_first: bool,
    pub info: Arc<SequenceInfo>,
}

impl Picture {
    /// Width of the `u` and `v` chroma planes (`(width + 1) / 2`).
    pub fn chroma_width(&self) -> u32 {
        self.width.div_ceil(2)
    }

    /// Height of the `u` and `v` chroma planes (`(height + 1) / 2`).
    pub fn chroma_height(&self) -> u32 {
        self.height.div_ceil(2)
    }
}

struct Anchor {
    frame: RefFrame,
    shown: bool,
}

/// Stateful MPEG-4 Part 2 video decoder.
pub struct Decoder {
    vol: Option<VolConfig>,
    info: Option<Arc<SequenceInfo>>,
    profile_and_level: Option<u8>,
    color: Option<ColorDescription>,
    packed_bitstream: bool,
    packed_b_pending: bool,
    past_ref: Option<Anchor>,
    future_ref: Option<Anchor>,
    anchor_seconds: i64,
    prev_anchor_seconds: i64,
    memory_budget: u64,
    /// Pictures displaced from the reorder store by a new layer, returned by the next call.
    held: Vec<Picture>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Create a new decoder with the default memory budget ([`crate::budget::DECODE_MEMORY_BUDGET`]).
    pub fn new() -> Self {
        Self {
            vol: None,
            info: None,
            profile_and_level: None,
            color: None,
            packed_bitstream: false,
            packed_b_pending: false,
            past_ref: None,
            future_ref: None,
            anchor_seconds: 0,
            prev_anchor_seconds: 0,
            memory_budget: crate::budget::DECODE_MEMORY_BUDGET,
            held: Vec::new(),
        }
    }

    /// Set the maximum bytes the decoder's picture store may allocate.
    pub fn set_memory_budget(&mut self, bytes: u64) {
        self.memory_budget = bytes;
    }

    /// The active sequence configuration, once a `VideoObjectLayer` (or short video header) has been parsed.
    pub fn sequence(&self) -> Option<&SequenceInfo> {
        self.info.as_deref()
    }

    /// Apply out-of-band configuration headers (`esds` `DecoderSpecificInfo` or Matroska `CodecPrivate`).
    /// Configuration data must not contain any `VideoObjectPlane` (`0xB6`) picture start codes.
    pub fn configure(&mut self, extradata: &[u8]) -> Result<(), DecodeError> {
        if extradata.is_empty() {
            return Ok(());
        }
        let units = scan_start_codes(extradata);
        if units.is_empty() {
            return Err(DecodeError::Invalid("configuration contains no start codes".into()));
        }
        for (code, payload) in units {
            match code {
                VISUAL_OBJECT_SEQUENCE_START => {
                    if let Some(&pl) = payload.first() {
                        self.profile_and_level = Some(pl);
                    }
                }
                VISUAL_OBJECT_START => {
                    if let Some(c) = parse_visual_object(payload)? {
                        self.color = Some(c);
                    }
                }
                USER_DATA_START => {
                    if is_packed_userdata(payload) {
                        self.packed_bitstream = true;
                    }
                }
                0x00..=0x1F | GROUP_OF_VOP_START | VISUAL_OBJECT_SEQUENCE_END => {}
                0x20..=0x2F => {
                    let vol = parse_vol(payload)?;
                    self.activate_vol(vol)?;
                }
                other => {
                    return Err(DecodeError::Invalid(format!("configuration contains start code {other:#04x}, not a sequence or layer header")));
                }
            }
        }
        Ok(())
    }

    /// Check that the picture store for `vol` fits the current memory budget.
    fn check_budget(&self, vol: &VolConfig) -> Result<(), DecodeError> {
        let shape = crate::budget::PictureShape {
            width: vol.mb_width() as u64 * 16,
            height: vol.mb_height() as u64 * 16,
            shift_x: 1,
            shift_y: 1,
            chroma_planes: 2,
            bytes_per_sample: 1,
        };
        if !shape.fits(BUDGET_PICTURES, self.memory_budget) {
            return Err(DecodeError::Invalid("VOL dimensions exceed decoder memory budget".into()));
        }
        Ok(())
    }

    fn activate_vol(&mut self, vol: VolConfig) -> Result<(), DecodeError> {
        self.check_budget(&vol)?;
        let changed = self.vol.as_ref().is_none_or(|old| old.width != vol.width || old.height != vol.height);
        if changed {
            if let (Some(fut), Some(old_vol)) = (self.future_ref.as_ref(), self.vol.as_ref())
                && !fut.shown
            {
                // The displaced picture is copied at the old layer's dimensions before the old
                // references are dropped: ensure the old layer also fits the current budget.
                self.check_budget(old_vol)?;
            }
            // A delayed picture still waiting for display keeps its own size and metadata.
            if let (Some(fut), Some(old_info)) = (self.future_ref.as_mut(), self.info.as_ref())
                && !fut.shown
            {
                fut.shown = true;
                self.held.push(Self::to_picture(&fut.frame, old_info));
            }
            self.past_ref = None;
            self.future_ref = None;
            self.packed_b_pending = false;
        }
        let info = SequenceInfo {
            width: vol.width,
            height: vol.height,
            profile_and_level: self.profile_and_level,
            sar: vol.sar,
            color: self.color,
            low_delay: vol.low_delay,
            interlaced: vol.interlaced,
            quarter_sample: vol.quarter_sample,
            time_increment_resolution: vol.time_increment_resolution,
            fixed_time_increment: vol.fixed_time_increment,
        };
        self.vol = Some(vol);
        self.info = Some(Arc::new(info));
        Ok(())
    }

    fn to_picture(frame: &RefFrame, info: &Arc<SequenceInfo>) -> Picture {
        let w = frame.width as usize;
        let h = frame.height as usize;
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        Picture {
            width: frame.width,
            height: frame.height,
            y: frame.y.crop(w, h),
            u: frame.u.crop(cw, ch),
            v: frame.v.crop(cw, ch),
            pts: frame.pts,
            vop_type: frame.vop_type,
            interlaced: info.interlaced,
            top_field_first: frame.top_field_first,
            info: Arc::clone(info),
        }
    }

    /// Decode one packet (or elementary stream chunk) tagged with `pts`.
    /// Returns any pictures ready for display, in display order.
    pub fn decode(&mut self, packet: &[u8], pts: i64) -> Result<Vec<Picture>, DecodeError> {
        if packet.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = std::mem::take(&mut self.held);
        match self.decode_inner(packet, pts, &mut out) {
            Ok(()) => {
                if out.len() > MAX_OUTPUT_PICTURES_PER_PACKET {
                    self.held = out.split_off(MAX_OUTPUT_PICTURES_PER_PACKET);
                }
                Ok(out)
            }
            Err(e) => {
                out.append(&mut self.held);
                self.held = out;
                Err(e)
            }
        }
    }

    fn decode_inner(&mut self, packet: &[u8], pts: i64, out: &mut Vec<Picture>) -> Result<(), DecodeError> {
        let units = scan_start_codes(packet);

        if units.is_empty() {
            let mut r = BitReader::new(packet);
            if let Some((vol, mut vop)) = try_parse_short_video_header(&mut r)? {
                let pending = self.future_ref.as_ref().is_some_and(|a| !a.shown);
                if out.len() + usize::from(pending) + 1 > MAX_OUTPUT_PICTURES_PER_PACKET + 1 {
                    return Err(DecodeError::Invalid("packet could return too many pictures".into()));
                }
                self.activate_vol(vol)?;
                out.append(&mut self.held);
                let vol_cfg = self.vol.as_ref().ok_or_else(|| DecodeError::Invalid("missing VOL".into()))?.clone();
                let info = self.info.as_ref().ok_or_else(|| DecodeError::Invalid("missing sequence info".into()))?.clone();
                let ticks = i64::from(vop.time_increment);
                let frame = decode_vop(
                    &mut r,
                    &vol_cfg,
                    &mut vop,
                    (self.past_ref.as_ref().map(|a| &a.frame), self.future_ref.as_ref().map(|a| &a.frame)),
                    (ticks, pts),
                )?;
                let pic = Self::to_picture(&frame, &info);
                self.past_ref = self.future_ref.take();
                self.future_ref = Some(Anchor { frame, shown: true });
                out.push(pic);
                return Ok(());
            }
            return Err(DecodeError::Invalid("packet contains no start codes or short video header".into()));
        }

        let vop_count = units.iter().filter(|&&(c, _)| c == VOP_START).count();
        if vop_count > MAX_OUTPUT_PICTURES_PER_PACKET {
            return Err(DecodeError::Invalid("packet contains too many VOPs".into()));
        }
        // Every picture is returned at most once, so a packet produces at most the pictures already
        // retained in `out`, the delayed anchor waiting at entry, and one per VOP. Allow up to one
        // extra picture when draining a full retained batch through a single-VOP recovery packet,
        // and refuse multi-VOP packets that would grow the retained store beyond the budget.
        let pending = self.future_ref.as_ref().is_some_and(|a| !a.shown);
        let worst_case = out.len() + usize::from(pending) + vop_count;
        let max_total = MAX_OUTPUT_PICTURES_PER_PACKET + usize::from(!out.is_empty() && vop_count <= 1);
        if worst_case > max_total {
            return Err(DecodeError::Invalid("packet could return too many pictures".into()));
        }
        let multi_vop_packet = vop_count > 1;

        for (code, payload) in units {
            match code {
                VISUAL_OBJECT_SEQUENCE_START => {
                    if let Some(&pl) = payload.first() {
                        self.profile_and_level = Some(pl);
                    }
                }
                VISUAL_OBJECT_START => {
                    if let Some(c) = parse_visual_object(payload)? {
                        self.color = Some(c);
                        if let Some(info) = &self.info {
                            let mut updated = (**info).clone();
                            updated.color = Some(c);
                            self.info = Some(Arc::new(updated));
                        }
                    }
                }
                USER_DATA_START => {
                    if is_packed_userdata(payload) {
                        self.packed_bitstream = true;
                    }
                }
                0x20..=0x2F => {
                    let vol = parse_vol(payload)?;
                    self.activate_vol(vol)?;
                    out.append(&mut self.held);
                }
                GROUP_OF_VOP_START | 0x00..=0x1F => {}
                VISUAL_OBJECT_SEQUENCE_END => {
                    out.extend(self.flush());
                }
                VOP_START => {
                    let vol_cfg = self.vol.as_ref().ok_or_else(|| DecodeError::Invalid("VOP before VideoObjectLayer header".into()))?.clone();
                    self.check_budget(&vol_cfg)?;
                    let info = self.info.as_ref().ok_or_else(|| DecodeError::Invalid("missing sequence info".into()))?.clone();

                    let mut r = BitReader::new(payload);
                    let mut vop = parse_vop_header(&mut r, &vol_cfg)?;
                    let res = i64::from(vol_cfg.time_increment_resolution);

                    if !vop.vop_coded || vop.coding_type == VopType::N {
                        // In a packed bitstream, an uncoded N-VOP after a multi-VOP [P, B] packet is
                        // the placeholder that outputs the held P-VOP.
                        if self.packed_b_pending {
                            self.packed_b_pending = false;
                            if let Some(fut) = self.future_ref.as_mut()
                                && !fut.shown
                            {
                                fut.shown = true;
                                out.push(Self::to_picture(&fut.frame, &info));
                                continue;
                            }
                        }
                        if let Some(fut) = self.future_ref.as_mut()
                            && !fut.shown
                        {
                            fut.shown = true;
                            out.push(Self::to_picture(&fut.frame, &info));
                        }
                        let ticks = self.anchor_seconds * res + i64::from(vop.time_increment);
                        let frame = decode_vop(
                            &mut r,
                            &vol_cfg,
                            &mut vop,
                            (self.past_ref.as_ref().map(|a| &a.frame), self.future_ref.as_ref().map(|a| &a.frame)),
                            (ticks, pts),
                        )?;
                        out.push(Self::to_picture(&frame, &info));
                        continue;
                    }

                    match vop.coding_type {
                        VopType::I | VopType::P | VopType::S => {
                            self.prev_anchor_seconds = self.anchor_seconds;
                            self.anchor_seconds += i64::from(vop.modulo_time_base);
                            let ticks = self.anchor_seconds * res + i64::from(vop.time_increment);
                            let frame = decode_vop(
                                &mut r,
                                &vol_cfg,
                                &mut vop,
                                (self.past_ref.as_ref().map(|a| &a.frame), self.future_ref.as_ref().map(|a| &a.frame)),
                                (ticks, pts),
                            )?;
                            if let Some(mut prev) = self.future_ref.take() {
                                if !prev.shown {
                                    prev.shown = true;
                                    out.push(Self::to_picture(&prev.frame, &info));
                                }
                                self.past_ref = Some(prev);
                            }
                            let show_now = vol_cfg.low_delay && !multi_vop_packet && !self.packed_bitstream;
                            if show_now {
                                out.push(Self::to_picture(&frame, &info));
                            }
                            self.future_ref = Some(Anchor { frame, shown: show_now });
                        }
                        VopType::B => {
                            let b_sec = self.prev_anchor_seconds + i64::from(vop.modulo_time_base);
                            let ticks = b_sec * res + i64::from(vop.time_increment);
                            let frame = decode_vop(
                                &mut r,
                                &vol_cfg,
                                &mut vop,
                                (self.past_ref.as_ref().map(|a| &a.frame), self.future_ref.as_ref().map(|a| &a.frame)),
                                (ticks, pts),
                            )?;
                            if multi_vop_packet {
                                self.packed_b_pending = true;
                            }
                            out.push(Self::to_picture(&frame, &info));
                        }
                        VopType::N => {}
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Flush any delayed anchor picture held for B-VOP reordering at the end of the stream.
    pub fn flush(&mut self) -> Vec<Picture> {
        self.packed_b_pending = false;
        let mut out = std::mem::take(&mut self.held);
        if let (Some(fut), Some(info)) = (self.future_ref.as_mut(), self.info.as_ref())
            && !fut.shown
        {
            fut.shown = true;
            out.push(Self::to_picture(&fut.frame, info));
        }
        out
    }

    /// Reset reference frames and reordering state (e.g. after seeking), preserving the active
    /// `VideoObjectLayer` configuration.
    pub fn reset(&mut self) {
        self.past_ref = None;
        self.future_ref = None;
        self.packed_b_pending = false;
        self.anchor_seconds = 0;
        self.prev_anchor_seconds = 0;
    }
}
