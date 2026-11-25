//! H.263 decoder (ITU-T H.263): baseline pictures, custom picture formats and the Unrestricted
//! Motion Vector (Annex D), Advanced Prediction (Annex F) and Deblocking Filter (Annex J) modes.
//!
//! Implemented from the published ITU-T H.263 (01/2005) text. The code tables, the inverse
//! transform and the half-sample interpolation are those of [`crate::mpeg4`], which codes the same
//! macroblock layer in its short video header.

mod headers;
mod picture;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use craft_media::bitstream::{BitError, BitReader};

pub use headers::PictureType;
use headers::{HeaderState, PictureHeader, at_picture_start, parse_picture_header};
use picture::{Frame, decode_picture};

/// Pictures the decoder holds at once: the reference, the one being decoded and the output.
const BUDGET_PICTURES: u64 = 4;

/// Pictures one `decode` call may return; a packet with more start codes is refused.
const MAX_PICTURES_PER_PACKET: usize = 8;

/// Errors returned by the H.263 decoder.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The bitstream is corrupt, truncated, or violates the specification.
    #[error("invalid H.263 video: {0}")]
    Invalid(String),
    /// The bitstream uses an H.263 tool that is not supported.
    #[error("unsupported H.263 video: {0}")]
    Unsupported(String),
}

impl From<BitError> for DecodeError {
    fn from(e: BitError) -> Self {
        DecodeError::Invalid(e.to_string())
    }
}

impl From<crate::mpeg4::DecodeError> for DecodeError {
    fn from(e: crate::mpeg4::DecodeError) -> Self {
        match e {
            crate::mpeg4::DecodeError::Unsupported(m) => DecodeError::Unsupported(m),
            crate::mpeg4::DecodeError::Invalid(m) => DecodeError::Invalid(m),
        }
    }
}

/// The picture format the decoder is producing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceInfo {
    pub width: u32,
    pub height: u32,
    /// Pixel aspect ratio, width to height (12:11 for the standard formats).
    pub pixel_aspect: (u16, u16),
}

/// A decoded picture: 8-bit 4:2:0 planes at the picture's own size.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub pts: i64,
    pub picture_type: PictureType,
    pub info: Arc<SequenceInfo>,
}

impl Picture {
    /// Width of the chrominance planes.
    pub fn chroma_width(&self) -> u32 {
        self.width.div_ceil(2)
    }

    /// Height of the chrominance planes.
    pub fn chroma_height(&self) -> u32 {
        self.height.div_ceil(2)
    }
}

/// Stateful H.263 video decoder. Pictures come out in decoding order, which is display order.
pub struct Decoder {
    state: HeaderState,
    reference: Option<Frame>,
    info: Option<Arc<SequenceInfo>>,
    memory_budget: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Create a decoder with the default memory budget ([`crate::budget::DECODE_MEMORY_BUDGET`]).
    pub fn new() -> Self {
        Self { state: HeaderState::default(), reference: None, info: None, memory_budget: crate::budget::DECODE_MEMORY_BUDGET }
    }

    /// Set the most memory, in bytes, the decoder's pictures may take.
    pub fn set_memory_budget(&mut self, bytes: u64) {
        self.memory_budget = bytes;
    }

    /// The format of the last picture, once one has been decoded.
    pub fn sequence(&self) -> Option<&SequenceInfo> {
        self.info.as_deref()
    }

    fn check_budget(&self, hdr: &PictureHeader) -> Result<(), DecodeError> {
        let (mb_w, mb_h) = hdr.mb_dims();
        let shape = crate::budget::PictureShape {
            width: mb_w as u64 * 16,
            height: mb_h as u64 * 16,
            shift_x: 1,
            shift_y: 1,
            chroma_planes: 2,
            bytes_per_sample: 1,
        };
        if shape.fits(BUDGET_PICTURES, self.memory_budget) {
            Ok(())
        } else {
            Err(DecodeError::Invalid("picture size exceeds the decoder memory budget".into()))
        }
    }

    /// Decode a packet, which holds one picture or several pictures back to back, tagged with `pts`.
    ///
    /// Every picture of the packet carries `pts`. A packet that fails to decode leaves the
    /// reference picture as it was.
    pub fn decode(&mut self, packet: &[u8], pts: i64) -> Result<Vec<Picture>, DecodeError> {
        let mut out = Vec::new();
        let mut at = find_picture_start(packet, 0).ok_or_else(|| DecodeError::Invalid("no picture start code in the packet".into()))?;
        loop {
            if out.len() >= MAX_PICTURES_PER_PACKET {
                return Err(DecodeError::Invalid("packet holds too many pictures".into()));
            }
            let mut r = BitReader::new(&packet[at..]);
            debug_assert!(at_picture_start(&r));
            let hdr = parse_picture_header(&mut r, &mut self.state)?;
            self.check_budget(&hdr)?;
            let frame = decode_picture(&mut r, &hdr, self.reference.as_ref())?;
            let info = match self.info.as_ref() {
                Some(i) if i.width == hdr.width && i.height == hdr.height && i.pixel_aspect == hdr.pixel_aspect => Arc::clone(i),
                _ => Arc::new(SequenceInfo { width: hdr.width, height: hdr.height, pixel_aspect: hdr.pixel_aspect }),
            };
            out.push(to_picture(&frame, pts, hdr.picture_type, &info));
            self.info = Some(info);
            self.reference = Some(frame);
            r.byte_align();
            let end = at + r.byte_pos();
            match find_picture_start(packet, end) {
                Some(next) => at = next,
                None => return Ok(out),
            }
        }
    }

    /// Nothing is held back: every picture is returned by the call that decoded it.
    pub fn flush(&mut self) -> Vec<Picture> {
        Vec::new()
    }

    /// Forget the reference picture (after a seek); the next picture must be an INTRA picture.
    pub fn reset(&mut self) {
        self.reference = None;
        self.state = HeaderState::default();
    }
}

fn to_picture(frame: &Frame, pts: i64, picture_type: PictureType, info: &Arc<SequenceInfo>) -> Picture {
    let (w, h) = (frame.width as usize, frame.height as usize);
    Picture {
        width: frame.width,
        height: frame.height,
        y: frame.y.crop(w, h),
        u: frame.u.crop(w.div_ceil(2), h.div_ceil(2)),
        v: frame.v.crop(w.div_ceil(2), h.div_ceil(2)),
        pts,
        picture_type,
        info: Arc::clone(info),
    }
}

/// The byte offset of the next byte-aligned picture start code at or after `from`.
fn find_picture_start(data: &[u8], from: usize) -> Option<usize> {
    let tail = data.get(from..)?;
    tail.windows(3).position(|w| w[0] == 0 && w[1] == 0 && w[2] & 0xFC == 0x80).map(|p| from + p)
}
