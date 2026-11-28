//! DV video decoding: the 25 Mb/s frames of IEC 61834-2 (consumer DV and DVCAM) and SMPTE ST 314 (DVCPRO), and
//! SMPTE ST 314's 50 Mb/s (DVCPRO50, 4:2:2), as a camcorder records them and a capture stores them in an AVI, a
//! QuickTime file or a raw `.dv` file (one frame after another).
//!
//! - [`sniff`]: whether bytes start like a DV frame.
//! - [`header`]: a frame's system (525/60 or 625/50), colour sampling, picture shape and field order.
//! - [`decode`]: a frame's picture, 720 × 480 or 720 × 576 8-bit Y'CbCr in its own sampling: 4:1:1 (525/60, and
//!   DVCPRO's 625/50), 4:2:0 (IEC 61834's 625/50) or 4:2:2 (DVCPRO50). It converts into
//!   [`crate::frame::VideoFrame`].
//!
//! Written from ITU-R BT.1618-1 and SMPTE ST 314 (see `README.md` in this folder for what comes from where). Every
//! length and number read from a frame is checked: a damaged frame gives a picture with the damaged blocks grey, or
//! an error, never a panic. The sound in the frame's DIF blocks, DVCPRO HD (an error) and an encoder are not here yet.

mod video;
mod vlc;

pub use video::{Layout, Picture};

use crate::frame::FieldOrder;

/// Bytes of a DIF block.
pub const DIF_BLOCK: usize = 80;
/// Bytes of a DIF sequence (150 DIF blocks).
pub const SEQUENCE: usize = 150 * DIF_BLOCK;

/// Errors from reading a DV frame.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a DV frame, or are cut short.
    #[error("invalid DV frame: {0}")]
    Invalid(String),
    /// The frame signals a kind of DV this decoder does not read (DVCPRO HD, for one).
    #[error("unsupported DV: {0}")]
    Unsupported(String),
}

/// Result with this module's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// The television system of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    /// 525 lines, 29.97 frames a second: 480 picture lines, 10 DIF sequences.
    Lines525,
    /// 625 lines, 25 frames a second: 576 picture lines, 12 DIF sequences.
    Lines625,
}

impl System {
    /// DIF sequences in a frame (one channel).
    pub fn sequences(self) -> usize {
        match self {
            System::Lines525 => 10,
            System::Lines625 => 12,
        }
    }

    /// Bytes in a frame of one channel.
    pub fn frame_bytes(self) -> usize {
        self.sequences() * SEQUENCE
    }

    /// Picture lines.
    pub fn height(self) -> usize {
        match self {
            System::Lines525 => 480,
            System::Lines625 => 576,
        }
    }

    /// Frames a second as a fraction.
    pub fn frame_rate(self) -> (u32, u32) {
        match self {
            System::Lines525 => (30_000, 1_001),
            System::Lines625 => (25, 1),
        }
    }
}

/// How the colour difference of a frame is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// A sample every 4 pixels on every line: 525/60, and SMPTE ST 314's 625/50 (DVCPRO).
    C411,
    /// A sample every 2 pixels on every other line: IEC 61834's 625/50 (consumer DV and DVCAM).
    C420,
    /// A sample every 2 pixels on every line: the 50 Mb/s structure (DVCPRO50), its frame in two channels.
    C422,
}

/// What a frame's header and auxiliary data say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// 525/60 or 625/50.
    pub system: System,
    /// The colour sampling.
    pub sampling: Sampling,
    /// The picture is 16:9 (squeezed into the 720 pixels), not 4:3.
    pub wide: bool,
    /// Progressive when the VAUX source control pack's interlace flag says noninterlaced; else two interlaced fields,
    /// in the order its first/second field flag gives: field 1, the lower line of each pair, first when the flag is
    /// 1 (bottom field first), field 2 first when it is 0 (top field first). Bottom field first without the pack.
    pub field_order: FieldOrder,
}

impl Header {
    /// Channels of DIF sequences in a frame: 2 for 50 Mb/s (4:2:2), else 1.
    pub fn channels(&self) -> usize {
        if self.sampling == Sampling::C422 { 2 } else { 1 }
    }

    /// Bytes in a frame: 120 000 / 144 000, or twice that at 50 Mb/s.
    pub fn frame_bytes(&self) -> usize {
        self.system.frame_bytes() * self.channels()
    }

    /// The sample aspect ratio that makes the whole 720 × 480 / 576 picture 4:3 or 16:9 (as the display select
    /// mode says): 8:9 / 32:27 for 525/60, 16:15 / 64:45 for 625/50.
    pub fn sample_aspect(&self) -> (u32, u32) {
        match (self.system, self.wide) {
            (System::Lines525, false) => (8, 9),
            (System::Lines525, true) => (32, 27),
            (System::Lines625, false) => (16, 15),
            (System::Lines625, true) => (64, 45),
        }
    }
}

/// Whether `bytes` start like a DV frame: a header DIF block, then the two subcode blocks and the first VAUX block.
pub fn sniff(bytes: &[u8]) -> bool {
    let section = |block: usize| bytes.get(block * DIF_BLOCK).map(|&id| id >> 5);
    let header = matches!(bytes.get(..3), Some([a, b, 0]) if a >> 5 == 0 && b >> 4 == 0);
    header && section(1) == Some(1) && section(2) == Some(1) && section(3) == Some(2)
}

/// Reads the header of the DV frame `frame` (its first DIF sequence is enough; [`Header::frame_bytes`] says how long
/// the whole frame is).
///
/// # Errors
/// When the bytes do not start with a header DIF block, or are shorter than a DIF sequence; and
/// [`Error::Unsupported`] when the VAUX source pack signals a kind of DV other than the 25 and 50 Mb/s structures.
pub fn header(frame: &[u8]) -> Result<Header> {
    let Some(&[id, _, _, dsf, apt, ..]) = frame.get(..8) else {
        return Err(Error::Invalid("the frame is too short".to_owned()));
    };
    if id >> 5 != 0 {
        return Err(Error::Invalid("the frame does not start with a header DIF block".to_owned()));
    }
    // DSF: 12 DIF sequences (625/50) or 10 (525/60).
    let system = if dsf & 0x80 != 0 { System::Lines625 } else { System::Lines525 };
    if frame.len() < SEQUENCE {
        return Err(Error::Invalid(format!("the frame has {} bytes, less than a DIF sequence", frame.len())));
    }
    // The 25 Mb/s structure: APT 000 is IEC 61834 (625/50 in 4:2:0); 001 is SMPTE ST 314 (4:1:1 in both systems).
    let structure_25 = match (system, apt & 0x07) {
        (System::Lines625, 0) => Sampling::C420,
        _ => Sampling::C411,
    };
    // The VAUX source pack's signal type (BT.1618-1 Table 13): 00000 is the 25 Mb/s structure, 00100 the 50 Mb/s
    // structure of two channels, and the rest is reserved there (SMPTE ST 370 gives some of it to DVCPRO HD, whose
    // frames this decoder does not read). A frame without the pack is taken for 25 Mb/s.
    let sampling = match vaux_pack(frame, system, 0x60).map(|pack| pack[3] & 0x1F) {
        None | Some(0b00000) => structure_25,
        Some(0b00100) => Sampling::C422,
        Some(signal) => return Err(Error::Unsupported(format!("signal type {signal:05b}, not the 25 or 50 Mb/s structure"))),
    };
    // The VAUX source control pack (Table 14): the display select mode, 010 is 16:9; the interlace flag, 0 is
    // noninterlaced; the first/second field flag, 1 when field 1 comes first (the lower line of each pair, Fig. 15).
    let control = vaux_pack(frame, system, 0x61);
    let wide = control.is_some_and(|pack| pack[2] & 0x07 == 0b010);
    let field_order = match control.map(|pack| pack[3]) {
        Some(flags) if flags & 0x10 == 0 => FieldOrder::Progressive,
        Some(flags) if flags & 0x40 == 0 => FieldOrder::TopFieldFirst,
        _ => FieldOrder::BottomFieldFirst,
    };
    Ok(Header { system, sampling, wide, field_order })
}

/// The first VAUX pack whose header is `id` (fifteen 5-byte packs in each VAUX DIF block, BT.1618-1 Fig. 7).
fn vaux_pack(frame: &[u8], system: System, id: u8) -> Option<[u8; 5]> {
    for sequence in 0..system.sequences() {
        for block in 3..6 {
            let start = sequence * SEQUENCE + block * DIF_BLOCK + 3;
            for pack in 0..15 {
                let at = start + 5 * pack;
                if let Some(bytes) = frame.get(at..at + 5)
                    && bytes.first() == Some(&id)
                    && let Ok(found) = <[u8; 5]>::try_from(bytes)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Decodes the picture of the DV frame `frame`.
///
/// # Errors
/// As [`header`], and when the frame is shorter than its header says.
pub fn decode(frame: &[u8]) -> Result<Picture> {
    let header = header(frame)?;
    video::decode(frame, &header)
}
