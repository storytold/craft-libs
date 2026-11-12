#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! DPX (SMPTE ST 268 Digital Picture Exchange) and Kodak Cineon (`.cin`) frames, read and written
//! from the published layouts. The crate goes from bytes to code values and header fields and
//! back; colour (log conversion, Y'CbCr matrices) and turning the picture upright stay with the
//! caller, who gets the fields those need.
//!
//! Reading ([`probe`], [`decode`]):
//!
//! * DPX in both byte orders (`SDPX` big-endian, `XPDS` little-endian), Cineon in both.
//! * The first image element: luma (descriptor 6), RGB (50), RGBA (51), ABGR (52, returned as
//!   RGBA) and Y'CbCr (100 CbYCrY 4:2:2, 101 CbYACrYA 4:2:2:4, 102 CbYCr 4:4:4, 103 CbYCrA
//!   4:4:4:4, returned in stored component order); Cineon with one or three channels.
//! * Unsigned samples without run-length encoding at 8, 10, 12 or 16 bits. 10-bit samples sit
//!   three to a 32-bit word with the padding bits at the bottom (packing 1, method A; Cineon
//!   packing 5) or the top (packing 2, method B; Cineon packing 6), the first sample in the high
//!   bits, except that a DPX luma element fills each word from its low end; 12-bit samples sit
//!   one to a 16-bit word the same two ways. Packed 10- and 12-bit samples (packing 0) follow each other
//!   through the line's 32-bit words, the first in the low bits. Lines start on a 32-bit boundary
//!   and may carry end-of-line padding; 8- and 16-bit files whose lines are not padded to a word
//!   are read too.
//! * [`Frame::codes`] holds the code values as stored, line by line. [`widen`] maps a code to 16
//!   bits and [`narrow`] back, exactly.
//! * [`Info`] carries what a pipeline needs: orientation, transfer and colorimetric codes, the
//!   reference low and high codes, pixel aspect ratio, creator, project, copyright, creation
//!   time, frame position, frame rate and time code.
//!
//! Writing ([`encode`], [`encode_cineon`]): one element, big-endian, unencoded; DPX luma, RGB or
//! RGBA at 8 and 16 bits (packing 0) or 10 and 12 bits (packing 1, method A); Cineon luma or RGB
//! at 10 bits.
//!
//! Not read (an [`Error::Unsupported`]): run-length encoded elements, signed, 1-bit and
//! floating-point samples, Cineon with line or channel interleave.
//!
//! Input is treated as hostile: every header read is bounds-checked, the picture size is capped
//! by [`Limits`] and the image data must be present in full before anything is allocated.

#[cfg(test)]
mod tests;

/// Generic plus industry header of a DPX file: where written image data starts.
const HEADER_LEN: usize = 2048;
const GENERIC_LEN: u32 = 1664;
const INDUSTRY_LEN: u32 = 384;
/// "Undefined" for 32-bit fields (also the bit pattern of an undefined float).
const UNDEF32: u32 = u32::MAX;
const UNDEF8: u8 = u8::MAX;
/// Offset of the first image element and the size of one.
const ELEMENT0: usize = 780;
const ELEMENT_LEN: usize = 72;
const MAX_ELEMENTS: u16 = 8;

// Header field offsets (generic file header, image information, orientation, film and TV headers).
const CREATION_TIME: (usize, usize) = (136, 24);
const CREATOR: (usize, usize) = (160, 100);
const PROJECT: (usize, usize) = (260, 200);
const COPYRIGHT: (usize, usize) = (460, 200);
const ORIENTATION: usize = 768;
const PIXEL_ASPECT: usize = 1628;
const FRAME_POSITION: usize = 1712;
const FRAME_RATE: usize = 1724;
const TIME_CODE: usize = 1920;

const CINEON_MAGIC: [u8; 4] = [0x80, 0x2A, 0x5F, 0xD7];
/// Cineon's generic plus film header: where written image data starts.
const CINEON_HEADER_LEN: usize = 2048;
const CINEON_GENERIC_LEN: u32 = 1024;
/// Offset of the first of the eight 28-byte channel descriptors.
const CINEON_CHANNEL0: usize = 196;

/// Why a file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes start with neither a DPX nor a Cineon magic number.
    #[error("not a DPX or Cineon file")]
    NotDpx,
    /// The header or the image data is damaged or cut short.
    #[error("DPX: {0}")]
    Malformed(String),
    /// A valid file this reader does not cover.
    #[error("unsupported DPX: {0}")]
    Unsupported(String),
    /// The file declares more than [`Limits`] allow.
    #[error("DPX exceeds a limit: {0}")]
    LimitExceeded(&'static str),
    /// The picture handed to the writer cannot be stored.
    #[error("DPX: cannot write: {0}")]
    Encode(String),
}

/// Result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Caps on what a header may ask the reader to allocate, checked before anything is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most pixels (width × height) a picture may have. Default 2^28 (16384 × 16384).
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pixels: 1 << 28 }
    }
}

/// Which of the two formats a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dpx,
    Cineon,
}

/// What the components of a pixel are, in the order [`Frame::codes`] holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One component.
    Luma,
    /// Red, green, blue.
    Rgb,
    /// Red, green, blue, alpha (descriptor 51, and descriptor 52 ABGR reordered).
    Rgba,
    /// 4:2:2: two components a pixel, Cb then Y' on even pixels and Cr then Y' on odd ones.
    CbYCrY422,
    /// 4:2:2:4: three components a pixel, Cb, Y', A on even pixels and Cr, Y', A on odd ones.
    CbYACrYA4224,
    /// 4:4:4: Cb, Y', Cr.
    CbYCr444,
    /// 4:4:4:4: Cb, Y', Cr, A.
    CbYCrA4444,
}

impl Layout {
    /// Components stored for each pixel.
    pub fn components(self) -> usize {
        match self {
            Layout::Luma => 1,
            Layout::CbYCrY422 => 2,
            Layout::Rgb | Layout::CbYACrYA4224 | Layout::CbYCr444 => 3,
            Layout::Rgba | Layout::CbYCrA4444 => 4,
        }
    }

    /// Whether the last component of a pixel is alpha.
    pub fn has_alpha(self) -> bool {
        matches!(self, Layout::Rgba | Layout::CbYACrYA4224 | Layout::CbYCrA4444)
    }

    /// Whether the components are Y'CbCr, which the caller converts with its own matrices.
    pub fn is_ycbcr(self) -> bool {
        matches!(self, Layout::CbYCrY422 | Layout::CbYACrYA4224 | Layout::CbYCr444 | Layout::CbYCrA4444)
    }
}

/// What the header says about the first image element and the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub kind: Kind,
    pub big_endian: bool,
    pub width: u32,
    pub height: u32,
    /// Image elements the file declares; only the first is read. Always 1 for Cineon.
    pub elements: u16,
    /// The element's descriptor as stored (DPX; 52 for ABGR although [`Info::layout`] is RGBA). 0 for Cineon.
    pub descriptor: u8,
    pub layout: Layout,
    /// Bits per component: 8, 10, 12 or 16.
    pub bits: u8,
    /// The image orientation as the DPX code 0–7 (Cineon's codes are translated); other values,
    /// undefined included, are kept as stored and read as 0. See [`Info::exif_orientation`].
    pub orientation: u16,
    /// Transfer characteristic code (DPX), `None` when undefined.
    pub transfer: Option<u8>,
    /// Colorimetric specification code (DPX), `None` when undefined.
    pub colorimetric: Option<u8>,
    /// Reference low data code value as stored (DPX; undefined is `u32::MAX`).
    pub reference_low_code: u32,
    /// Reference high data code value as stored (DPX; undefined is `u32::MAX`).
    pub reference_high_code: u32,
    /// Pixel aspect ratio as horizontal and vertical parts, both non-zero, when stated.
    pub pixel_aspect: Option<(u32, u32)>,
    pub creator: Option<String>,
    pub project: Option<String>,
    pub copyright: Option<String>,
    /// Creation time as written (`yyyy:mm:dd:hh:mm:ss:LTZ` by the standard).
    pub creation_time: Option<String>,
    /// Frame position in the sequence (film header), when the industry header is present.
    pub frame_position: Option<u32>,
    /// Frame rate of the original (film header), when the industry header is present.
    pub frame_rate: Option<f32>,
    /// SMPTE time code as BCD `HHMMSSFF` (television header). See [`format_time_code`].
    pub time_code: Option<u32>,
}

impl Info {
    /// The EXIF orientation (1–8) that turns the stored lines upright. Anything outside the
    /// codes 0–7 is taken as the default (1).
    pub fn exif_orientation(&self) -> u16 {
        match self.orientation {
            1 => 2,
            2 => 4,
            3 => 3,
            4 => 5,
            5 => 6,
            6 => 8,
            7 => 7,
            _ => 1,
        }
    }

    /// For the four orientations that keep lines horizontal: whether pixels run right to left and
    /// whether lines run bottom to top. `None` for the transposed orientations (4–7).
    pub fn flips(&self) -> Option<(bool, bool)> {
        match self.orientation {
            1 => Some((true, false)),
            2 => Some((false, true)),
            3 => Some((true, true)),
            4..=7 => None,
            _ => Some((false, false)),
        }
    }

    /// The largest code value: `2^bits − 1`.
    pub fn max_code(&self) -> u16 {
        max_code(self.bits)
    }
}

/// A decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub info: Info,
    /// Code values as stored: `height` lines of `width × layout.components()` values, each in
    /// `0..=info.max_code()`, first stored line first (see [`Info::orientation`]).
    pub codes: Vec<u16>,
}

impl Frame {
    /// The code values as 16-bit samples ([`widen`]); 16-bit codes are returned unchanged.
    pub fn samples_u16(&self) -> Vec<u16> {
        if self.info.bits == 16 { self.codes.clone() } else { self.codes.iter().map(|&c| widen(c, self.info.bits)).collect() }
    }
}

fn malformed(msg: impl Into<String>) -> Error {
    Error::Malformed(msg.into())
}

fn unsupported(msg: impl Into<String>) -> Error {
    Error::Unsupported(msg.into())
}

/// Bounds-checked reads in the file's byte order.
#[derive(Clone, Copy)]
struct Reader<'a> {
    b: &'a [u8],
    big: bool,
}

impl Reader<'_> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.b.get(at).copied()
    }

    fn u16(&self, at: usize) -> Option<u16> {
        let s: [u8; 2] = self.b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if self.big { u16::from_be_bytes(s) } else { u16::from_le_bytes(s) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let s: [u8; 4] = self.b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.big { u32::from_be_bytes(s) } else { u32::from_le_bytes(s) })
    }

    /// A 32-bit field that may be undefined (all ones).
    fn defined_u32(&self, at: usize) -> Option<u32> {
        self.u32(at).filter(|&v| v != UNDEF32)
    }

    fn f32(&self, at: usize) -> Option<f32> {
        self.defined_u32(at).map(f32::from_bits).filter(|v| v.is_finite())
    }

    /// An ASCII header string: up to the first NUL, printable characters only, trimmed. `None`
    /// when empty or undefined (all ones).
    fn string(&self, (at, len): (usize, usize)) -> Option<String> {
        let raw = self.b.get(at..at.checked_add(len)?)?;
        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        let s: String = raw.get(..end)?.iter().filter(|c| (32..=126).contains(*c)).map(|&c| c as char).collect();
        Some(s.trim().to_string()).filter(|s| !s.is_empty())
    }
}

/// The format and byte order from the magic number, `None` when the bytes are neither.
fn magic(bytes: &[u8]) -> Option<(Kind, bool)> {
    match bytes.get(..4)? {
        b"SDPX" => Some((Kind::Dpx, true)),
        b"XPDS" => Some((Kind::Dpx, false)),
        [0x80, 0x2A, 0x5F, 0xD7] => Some((Kind::Cineon, true)),
        [0xD7, 0x5F, 0x2A, 0x80] => Some((Kind::Cineon, false)),
        _ => None,
    }
}

/// Whether `bytes` starts with a DPX or Cineon magic number (either byte order), and which.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    magic(bytes).map(|(kind, _)| kind)
}

/// How samples are stored in a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Packing {
    /// One byte per sample.
    Bytes,
    /// One 16-bit word per sample; the value is `(word >> shift) & mask`.
    Words { shift: u32, mask: u16 },
    /// Three 10-bit samples per 32-bit word above `pad` bits: the first in the high bits, or in
    /// the low bits when `low_first` (a DPX element of one component).
    Ten { pad: u32, low_first: bool },
    /// `bits`-bit samples back to back through 32-bit words, low bits first (packing 0).
    Packed { bits: u32 },
}

impl Packing {
    /// Bytes of a line of `samples` samples, padded to a 32-bit word.
    fn line_bytes(self, samples: usize) -> Option<usize> {
        let raw = match self {
            Packing::Bytes => samples,
            Packing::Words { .. } => samples.checked_mul(2)?,
            Packing::Ten { .. } => samples.div_ceil(3).checked_mul(4)?,
            Packing::Packed { bits } => samples.checked_mul(bits as usize)?.div_ceil(32).checked_mul(4)?,
        };
        raw.checked_next_multiple_of(4)
    }

    /// Bytes of a line that is not padded to a word (some writers' 8- and 16-bit lines).
    fn tight_line_bytes(self, samples: usize) -> Option<usize> {
        match self {
            Packing::Bytes => Some(samples),
            Packing::Words { shift: 0, mask: u16::MAX } => samples.checked_mul(2),
            _ => None,
        }
    }

    /// The code values of one stored line.
    fn unpack(self, line: &[u8], big: bool, out: &mut [u16]) -> Result<()> {
        let short = || malformed("truncated image data");
        let r = Reader { b: line, big };
        match self {
            Packing::Bytes => {
                let src = line.get(..out.len()).ok_or_else(short)?;
                for (v, &c) in out.iter_mut().zip(src) {
                    *v = u16::from(c);
                }
            }
            Packing::Words { shift, mask } => {
                for (i, v) in out.iter_mut().enumerate() {
                    *v = (r.u16(i * 2).ok_or_else(short)? >> shift) & mask;
                }
            }
            Packing::Ten { pad, low_first } => {
                for (i, v) in out.iter_mut().enumerate() {
                    let word = r.u32(i / 3 * 4).ok_or_else(short)?;
                    let slot = (i % 3) as u32;
                    *v = ((word >> (pad + 10 * if low_first { slot } else { 2 - slot })) & 0x3FF) as u16;
                }
            }
            Packing::Packed { bits } => {
                let mask = (1u64 << bits) - 1;
                let (mut acc, mut have, mut word) = (0u64, 0u32, 0usize);
                for v in out.iter_mut() {
                    if have < bits {
                        acc |= u64::from(r.u32(word * 4).ok_or_else(short)?) << have;
                        word += 1;
                        have += 32;
                    }
                    *v = (acc & mask) as u16;
                    acc >>= bits;
                    have -= bits;
                }
            }
        }
        Ok(())
    }
}

/// A parsed header: the public description plus where and how the samples are stored.
struct Parsed {
    info: Info,
    packing: Packing,
    data_offset: usize,
    eol_padding: usize,
    /// Descriptor 52: the stored component order is A, B, G, R.
    abgr: bool,
}

fn parse(bytes: &[u8]) -> Result<Parsed> {
    let (kind, big) = magic(bytes).ok_or(Error::NotDpx)?;
    let r = Reader { b: bytes, big };
    match kind {
        Kind::Dpx => parse_dpx(r),
        Kind::Cineon => parse_cineon(r),
    }
}

fn parse_dpx(r: Reader) -> Result<Parsed> {
    let missing = |what: &str| malformed(format!("the header ends before {what}"));
    let image_offset = r.u32(4).ok_or_else(|| missing("the image data offset"))?;
    let orientation = r.u16(ORIENTATION).ok_or_else(|| missing("the orientation"))?;
    let elements = r.u16(ORIENTATION + 2).ok_or_else(|| missing("the element count"))?;
    if !(1..=MAX_ELEMENTS).contains(&elements) {
        return Err(malformed(format!("invalid number of image elements {elements}")));
    }
    let width = r.u32(772).ok_or_else(|| missing("the line length"))?;
    let height = r.u32(776).ok_or_else(|| missing("the line count"))?;
    let e = ELEMENT0;
    let field = |at: usize| r.u32(e + at).ok_or_else(|| missing("image element 1"));
    let byte = |at: usize| r.u8(e + at).ok_or_else(|| missing("image element 1"));
    let half = |at: usize| r.u16(e + at).ok_or_else(|| missing("image element 1"));
    let signed = field(0)? == 1;
    let (descriptor, transfer, colorimetric, bits) = (byte(20)?, byte(21)?, byte(22)?, byte(23)?);
    let (packing, encoding) = (half(24)?, half(26)?);
    let data_offset = match field(28)? {
        0 | UNDEF32 => image_offset,
        v => v,
    };
    let eol_padding = match field(32)? {
        UNDEF32 => 0,
        v => v,
    };
    let layout = match descriptor {
        6 => Layout::Luma,
        50 => Layout::Rgb,
        51 | 52 => Layout::Rgba,
        100 => Layout::CbYCrY422,
        101 => Layout::CbYACrYA4224,
        102 => Layout::CbYCr444,
        103 => Layout::CbYCrA4444,
        d => return Err(unsupported(format!("descriptor {d} (luma, RGB, RGBA, ABGR and Y'CbCr are read)"))),
    };
    if signed {
        return Err(unsupported("data sign 1 (signed samples)"));
    }
    if encoding != 0 {
        return Err(unsupported(format!("encoding {encoding} (run-length encoded data)")));
    }
    let packing = match (bits, packing) {
        (8, _) => Packing::Bytes,
        (16, _) => Packing::Words { shift: 0, mask: u16::MAX },
        // A one-component element fills each word from its low end; RGB keeps red on top.
        (10, 1) => Packing::Ten { pad: 2, low_first: layout == Layout::Luma },
        (10, 2) => Packing::Ten { pad: 0, low_first: layout == Layout::Luma },
        (12, 1) => Packing::Words { shift: 4, mask: 0xFFF },
        (12, 2) => Packing::Words { shift: 0, mask: 0xFFF },
        (10 | 12, 0) => Packing::Packed { bits: u32::from(bits) },
        (10 | 12, p) => return Err(unsupported(format!("packing {p} at {bits} bits"))),
        (b, _) => return Err(unsupported(format!("bit size {b} (8, 10, 12 and 16 are read)"))),
    };
    // The industry header is there when the generic header says so and the file holds it.
    let industry = r.u32(28).is_some_and(|l| l != 0 && l != UNDEF32) && r.b.len() >= HEADER_LEN;
    let pixel_aspect = match (r.defined_u32(PIXEL_ASPECT), r.defined_u32(PIXEL_ASPECT + 4)) {
        (Some(h), Some(v)) if h != 0 && v != 0 => Some((h, v)),
        _ => None,
    };
    let info = Info {
        kind: Kind::Dpx,
        big_endian: r.big,
        width,
        height,
        elements,
        descriptor,
        layout,
        bits,
        orientation,
        transfer: Some(transfer).filter(|&t| t != UNDEF8),
        colorimetric: Some(colorimetric).filter(|&c| c != UNDEF8),
        reference_low_code: field(4)?,
        reference_high_code: field(12)?,
        pixel_aspect,
        creator: r.string(CREATOR),
        project: r.string(PROJECT),
        copyright: r.string(COPYRIGHT),
        creation_time: r.string(CREATION_TIME),
        frame_position: if industry { r.defined_u32(FRAME_POSITION) } else { None },
        frame_rate: if industry { r.f32(FRAME_RATE) } else { None },
        time_code: if industry { r.defined_u32(TIME_CODE) } else { None },
    };
    Ok(Parsed {
        info,
        packing,
        data_offset: usize::try_from(data_offset).map_err(|_| malformed("image data offset out of range"))?,
        eol_padding: usize::try_from(eol_padding).map_err(|_| malformed("line padding out of range"))?,
        abgr: descriptor == 52,
    })
}

// Cineon is the film-scan format DPX grew from (same header family). The generic header is 1024
// bytes: the file information fields, then at offset 192 the image orientation (1 byte), the
// channel count (1 byte) and up to eight 28-byte channel descriptors (each carries its bits per
// pixel and the line and image dimensions), and at 680 the interleave, packing, sign and
// end-of-line padding. 10-bit data is three samples to a 32-bit word, lines on a word boundary:
// the same layout as DPX's filled 10-bit methods, so the unpacking and packing are shared.
fn parse_cineon(r: Reader) -> Result<Parsed> {
    let missing = |what: &str| malformed(format!("the Cineon header ends before {what}"));
    let image_offset = r.u32(4).ok_or_else(|| missing("the image data offset"))?;
    let orientation = r.u8(192).ok_or_else(|| missing("the orientation"))?;
    let channels = r.u8(193).ok_or_else(|| missing("the channel count"))?;
    let bits = r.u8(198).ok_or_else(|| missing("the bits per pixel"))?;
    let width = r.u32(200).ok_or_else(|| missing("the pixels per line"))?;
    let height = r.u32(204).ok_or_else(|| missing("the lines per image"))?;
    let layout = match channels {
        1 => Layout::Luma,
        3 => Layout::Rgb,
        n => return Err(unsupported(format!("Cineon with {n} channels (1 and 3 are read)"))),
    };
    // Channel 0 describes the picture; every other channel must match it.
    for c in 1..usize::from(channels) {
        let base = CINEON_CHANNEL0 + 28 * c;
        if r.u8(base + 2) != Some(bits) || r.u32(base + 4) != Some(width) || r.u32(base + 8) != Some(height) {
            return Err(unsupported("Cineon channels of different sizes or depths"));
        }
    }
    let interleave = r.u8(680).ok_or_else(|| missing("the data format"))?;
    let packing = r.u8(681).ok_or_else(|| missing("the data format"))?;
    let signed = r.u8(682).ok_or_else(|| missing("the data format"))?;
    let eol_padding = r.u32(684).ok_or_else(|| missing("the data format"))?;
    let eol_padding = if eol_padding == UNDEF32 { 0 } else { eol_padding };
    if interleave != 0 && channels != 1 {
        return Err(unsupported("Cineon with line or channel interleave"));
    }
    if signed == 1 {
        return Err(unsupported("Cineon with signed samples"));
    }
    let packing = match (bits, packing) {
        (8, _) => Packing::Bytes,
        (16, _) => Packing::Words { shift: 0, mask: u16::MAX },
        (10, 6) => Packing::Ten { pad: 0, low_first: false },
        // 5 is the usual "32-bit left justified"; 0 and undefined come from writers that leave the field unset.
        (10, 0 | 5 | UNDEF8) => Packing::Ten { pad: 2, low_first: false },
        (10, p) => return Err(unsupported(format!("Cineon with 10-bit packing {p} (32-bit words are read)"))),
        (b, _) => return Err(unsupported(format!("Cineon with {b}-bit samples (8, 10 and 16 are read)"))),
    };
    // Cineon numbers "left to right, bottom to top" 1 and "right to left, top to bottom" 2: the
    // reverse of DPX. 0 and 3–7 mean the same in both.
    let orientation = match orientation {
        1 => 2,
        2 => 1,
        o => u16::from(o),
    };
    let info = Info {
        kind: Kind::Cineon,
        big_endian: r.big,
        width,
        height,
        elements: 1,
        descriptor: 0,
        layout,
        bits,
        orientation,
        transfer: None,
        colorimetric: None,
        reference_low_code: UNDEF32,
        reference_high_code: UNDEF32,
        pixel_aspect: None,
        creator: None,
        project: None,
        copyright: None,
        creation_time: None,
        frame_position: None,
        frame_rate: None,
        time_code: None,
    };
    Ok(Parsed {
        info,
        packing,
        data_offset: usize::try_from(image_offset).map_err(|_| malformed("image data offset out of range"))?,
        eol_padding: usize::try_from(eol_padding).map_err(|_| malformed("line padding out of range"))?,
        abgr: false,
    })
}

/// Where the lines of a validated file are.
struct Lines<'a> {
    data: &'a [u8],
    /// Samples in a line.
    per_line: usize,
    /// Bytes from one line's start to the next.
    stride: usize,
}

/// Checks the picture size against `limits` and that the file holds every line.
fn locate<'a>(bytes: &'a [u8], p: &Parsed, limits: &Limits) -> Result<Lines<'a>> {
    let (width, height) = (p.info.width, p.info.height);
    if width == 0 || height == 0 {
        return Err(malformed("empty picture"));
    }
    if u64::from(width) * u64::from(height) > limits.max_pixels {
        return Err(Error::LimitExceeded("pixel count"));
    }
    let too_large = || Error::LimitExceeded("image size");
    let per_line = usize::try_from(width).ok().and_then(|w| w.checked_mul(p.info.layout.components())).ok_or_else(too_large)?;
    let height = usize::try_from(height).map_err(|_| too_large())?;
    let data = bytes.get(p.data_offset..).ok_or_else(|| malformed("the image data offset is past the end of the file"))?;
    let mut line_len = p.packing.line_bytes(per_line).ok_or_else(too_large)?;
    let mut stride = line_len.checked_add(p.eol_padding).ok_or_else(too_large)?;
    // Lines run `stride` apart; the last one needs no padding after it.
    let span = |stride: usize, line_len: usize| stride.checked_mul(height - 1)?.checked_add(line_len);
    // Some writers store 8- and 16-bit lines without padding them to a word; take that layout
    // when the file is too short for padded lines but holds the unpadded ones.
    if let Some(tight) = p.packing.tight_line_bytes(per_line)
        && p.eol_padding == 0
        && line_len != tight
        && span(stride, line_len).is_none_or(|need| data.len() < need)
    {
        (line_len, stride) = (tight, tight);
    }
    if span(stride, line_len).ok_or_else(too_large)? > data.len() {
        return Err(malformed("truncated image data"));
    }
    Ok(Lines { data, per_line, stride })
}

/// Reads the header and checks that the frame can be decoded within `limits` (supported layout,
/// size, image data present) without decoding it.
pub fn probe(bytes: &[u8], limits: &Limits) -> Result<Info> {
    let p = parse(bytes)?;
    locate(bytes, &p, limits)?;
    Ok(p.info)
}

/// Decodes the first image element to code values.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Frame> {
    let p = parse(bytes)?;
    let lines = locate(bytes, &p, limits)?;
    // `locate` proved the file holds at least a byte per sample, so this is bounded by its size.
    let total = lines.per_line.checked_mul(p.info.height as usize).ok_or(Error::LimitExceeded("image size"))?;
    let mut codes = vec![0u16; total];
    for (y, row) in codes.chunks_exact_mut(lines.per_line).enumerate() {
        let line = y.checked_mul(lines.stride).and_then(|start| lines.data.get(start..)).ok_or_else(|| malformed("truncated image data"))?;
        p.packing.unpack(line, p.info.big_endian, row)?;
        if p.abgr {
            for px in row.as_chunks_mut::<4>().0 {
                px.reverse();
            }
        }
    }
    Ok(Frame { info: p.info, codes })
}

/// The largest code value at `bits` bits (`bits` above 16 is taken as 16, 0 as 1).
fn max_code(bits: u8) -> u16 {
    u16::MAX >> (16 - u32::from(bits.clamp(1, 16)))
}

/// A `bits`-bit code value as a 16-bit sample: `round(c × 65535 / (2^bits − 1))`. A code written
/// back with [`narrow`] at the same depth comes back exactly.
pub fn widen(code: u16, bits: u8) -> u16 {
    let max = u64::from(max_code(bits));
    ((u64::from(code) * 2 * 65535 + max) / (2 * max)).min(65535) as u16
}

/// The inverse of [`widen`]: `round(v × (2^bits − 1) / 65535)`.
pub fn narrow(sample: u16, bits: u8) -> u16 {
    let max = u64::from(max_code(bits));
    ((u64::from(sample) * 2 * max + 65535) / (2 * 65535)) as u16
}

/// A SMPTE time code (BCD `HHMMSSFF`) as `HH:MM:SS:FF`, or `0x…` when it is not valid BCD.
pub fn format_time_code(v: u32) -> String {
    let d: [u32; 8] = std::array::from_fn(|i| (v >> ((7 - i) * 4)) & 0xF);
    if d.iter().all(|&d| d <= 9) { format!("{}{}:{}{}:{}{}:{}{}", d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]) } else { format!("{v:#010x}") }
}

/// The inverse of [`format_time_code`]; `None` for anything else.
pub fn parse_time_code(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok();
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 4 || parts.iter().any(|p| p.len() != 2 || !p.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    parts.iter().flat_map(|p| p.bytes()).try_fold(0u32, |acc, c| Some((acc << 4) | u32::from(c - b'0')))
}

/// A picture to write: `height` lines of `width × layout.components()` code values, top line first.
#[derive(Debug, Clone, Copy)]
pub struct Picture<'a> {
    pub width: u32,
    pub height: u32,
    /// [`Layout::Luma`], [`Layout::Rgb`] or [`Layout::Rgba`] (Cineon: luma or RGB).
    pub layout: Layout,
    /// Bits per component: 8, 10, 12 or 16 (Cineon: 10). Codes above the depth's largest are masked.
    pub bits: u8,
    pub codes: &'a [u16],
}

/// Header fields a DPX writer fills in; everything left `None` is written as undefined or empty.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metadata<'a> {
    pub creator: Option<&'a str>,
    pub project: Option<&'a str>,
    pub copyright: Option<&'a str>,
    pub creation_time: Option<&'a str>,
    /// Transfer characteristic code. No claim about the encoding is made when `None`.
    pub transfer: Option<u8>,
    /// Colorimetric specification code.
    pub colorimetric: Option<u8>,
    pub frame_position: Option<u32>,
    pub frame_rate: Option<f32>,
    /// SMPTE time code as BCD `HHMMSSFF` (see [`parse_time_code`]).
    pub time_code: Option<u32>,
}

/// A fixed-size big-endian header under construction.
struct Writer {
    h: Vec<u8>,
}

impl Writer {
    fn bytes(&mut self, at: usize, v: &[u8]) {
        if let Some(s) = at.checked_add(v.len()).and_then(|end| self.h.get_mut(at..end)) {
            s.copy_from_slice(v);
        }
    }

    fn u8(&mut self, at: usize, v: u8) {
        self.bytes(at, &[v]);
    }

    fn u16(&mut self, at: usize, v: u16) {
        self.bytes(at, &v.to_be_bytes());
    }

    fn u32(&mut self, at: usize, v: u32) {
        self.bytes(at, &v.to_be_bytes());
    }

    /// Marks a run of numeric fields undefined (all ones).
    fn undefined(&mut self, from: usize, to: usize) {
        if let Some(s) = self.h.get_mut(from..to) {
            s.fill(0xFF);
        }
    }

    /// An ASCII string field, cut to leave room for a terminating NUL.
    fn string(&mut self, (at, len): (usize, usize), v: &str) {
        let ascii: Vec<u8> = v.chars().map(|c| if (' '..='~').contains(&c) { c as u8 } else { b'?' }).take(len.saturating_sub(1)).collect();
        self.bytes(at, &ascii);
    }
}

/// Appends one line's code values, padded to a 32-bit word. `low_first`: 10-bit samples fill each
/// word from its low end (a DPX element of one component).
fn pack_line(out: &mut Vec<u8>, codes: &[u16], bits: u8, low_first: bool) {
    let start = out.len();
    match bits {
        8 => out.extend(codes.iter().map(|&c| c as u8)),
        10 => {
            for chunk in codes.chunks(3) {
                let word = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |w, (i, &c)| w | (u32::from(c & 0x3FF) << (2 + 10 * if low_first { i as u32 } else { 2 - i as u32 })));
                out.extend_from_slice(&word.to_be_bytes());
            }
        }
        12 => out.extend(codes.iter().flat_map(|&c| ((c & 0xFFF) << 4).to_be_bytes())),
        _ => out.extend(codes.iter().flat_map(|&c| c.to_be_bytes())),
    }
    let written = out.len() - start;
    out.resize(start + written.div_ceil(4) * 4, 0);
}

/// Samples in a line and bytes of image data for `pic`, after checking its sample count.
fn layout_of(pic: &Picture, header_len: usize, what: &str) -> Result<(usize, usize, u32)> {
    let enc = |msg: &str| Error::Encode(msg.into());
    if pic.width == 0 || pic.height == 0 {
        return Err(enc("empty picture"));
    }
    let (w, h) = (pic.width as usize, pic.height as usize);
    let per_line = w.checked_mul(pic.layout.components()).ok_or_else(|| enc("image too large"))?;
    if per_line.checked_mul(h) != Some(pic.codes.len()) {
        return Err(enc("the picture has the wrong number of samples"));
    }
    let line = match pic.bits {
        8 => Packing::Bytes,
        10 => Packing::Ten { pad: 2, low_first: false },
        _ => Packing::Words { shift: 0, mask: u16::MAX },
    }
    .line_bytes(per_line);
    let data_len = line.and_then(|l| l.checked_mul(h)).ok_or_else(|| enc("image too large"))?;
    let total = header_len
        .checked_add(data_len)
        .and_then(|t| u32::try_from(t).ok())
        .ok_or_else(|| Error::Encode(format!("image too large for a {what} file (4 GiB)")))?;
    Ok((per_line, data_len, total))
}

/// Writes a DPX 2.0 file: one element, big-endian, unencoded, default orientation.
pub fn encode(pic: &Picture, meta: &Metadata) -> Result<Vec<u8>> {
    let descriptor = match pic.layout {
        Layout::Luma => 6u8,
        Layout::Rgb => 50,
        Layout::Rgba => 51,
        l => return Err(Error::Encode(format!("{l:?} is not written (luma, RGB and RGBA are)"))),
    };
    let bits = pic.bits;
    if !matches!(bits, 8 | 10 | 12 | 16) {
        return Err(Error::Encode(format!("{bits} bits is not written (8, 10, 12 and 16 are)")));
    }
    let (per_line, data_len, total) = layout_of(pic, HEADER_LEN, "DPX")?;

    let mut wr = Writer { h: vec![0; HEADER_LEN] };
    // Generic file header.
    wr.bytes(0, b"SDPX");
    wr.u32(4, HEADER_LEN as u32);
    wr.string((8, 8), "V2.0");
    wr.u32(16, total);
    wr.u32(20, 1);
    wr.u32(24, GENERIC_LEN);
    wr.u32(28, INDUSTRY_LEN);
    wr.u32(32, 0);
    wr.string(CREATION_TIME, meta.creation_time.unwrap_or(""));
    wr.string(CREATOR, meta.creator.unwrap_or(""));
    wr.string(PROJECT, meta.project.unwrap_or(""));
    wr.string(COPYRIGHT, meta.copyright.unwrap_or(""));
    wr.u32(660, UNDEF32);
    // Image information: default orientation, one element.
    wr.u16(ORIENTATION, 0);
    wr.u16(ORIENTATION + 2, 1);
    wr.u32(772, pic.width);
    wr.u32(776, pic.height);
    let e = ELEMENT0;
    wr.u32(e, 0);
    wr.u32(e + 4, 0);
    wr.u32(e + 8, UNDEF32);
    wr.u32(e + 12, u32::from(max_code(bits)));
    wr.u32(e + 16, UNDEF32);
    wr.u8(e + 20, descriptor);
    wr.u8(e + 21, meta.transfer.unwrap_or(UNDEF8));
    wr.u8(e + 22, meta.colorimetric.unwrap_or(UNDEF8));
    wr.u8(e + 23, bits);
    wr.u16(e + 24, if matches!(bits, 10 | 12) { 1 } else { 0 });
    wr.u16(e + 26, 0);
    wr.u32(e + 28, HEADER_LEN as u32);
    wr.u32(e + 32, 0);
    wr.u32(e + 36, 0);
    // The seven unused elements, the image source header's numbers and the industry header's
    // numbers are undefined; their strings stay empty.
    wr.undefined(e + ELEMENT_LEN, e + ELEMENT_LEN * usize::from(MAX_ELEMENTS));
    wr.undefined(1408, 1432);
    wr.undefined(1620, 1644);
    wr.undefined(FRAME_POSITION, 1732);
    wr.undefined(TIME_CODE, 1972);
    if let Some(v) = meta.frame_position {
        wr.u32(FRAME_POSITION, v);
    }
    if let Some(v) = meta.frame_rate.filter(|v| v.is_finite()) {
        wr.u32(FRAME_RATE, v.to_bits());
    }
    if let Some(v) = meta.time_code {
        wr.u32(TIME_CODE, v);
    }

    let mut out = wr.h;
    out.try_reserve_exact(data_len).map_err(|_| Error::Encode("out of memory".into()))?;
    for row in pic.codes.chunks_exact(per_line) {
        pack_line(&mut out, row, bits, pic.layout == Layout::Luma);
    }
    Ok(out)
}

/// Writes a Cineon 4.5 frame: the generic header, an undefined film header, then 10-bit code
/// values three to a big-endian word, left justified (packing 5), lines on a word boundary.
pub fn encode_cineon(pic: &Picture) -> Result<Vec<u8>> {
    // Channel designators: 0 for black and white, 1–3 for red, green and blue.
    let designators: &[u8] = match pic.layout {
        Layout::Luma => &[0],
        Layout::Rgb => &[1, 2, 3],
        l => return Err(Error::Encode(format!("Cineon cannot store {l:?}"))),
    };
    if pic.bits != 10 {
        return Err(Error::Encode(format!("Cineon is written at 10 bits, not {}", pic.bits)));
    }
    let (per_line, data_len, total) = layout_of(pic, CINEON_HEADER_LEN, "Cineon")?;

    let mut wr = Writer { h: vec![0; CINEON_HEADER_LEN] };
    // File information.
    wr.bytes(0, &CINEON_MAGIC);
    wr.u32(4, CINEON_HEADER_LEN as u32);
    wr.u32(8, CINEON_GENERIC_LEN);
    wr.u32(12, CINEON_HEADER_LEN as u32 - CINEON_GENERIC_LEN);
    wr.u32(16, 0);
    wr.u32(20, total);
    wr.string((24, 8), "V4.5");
    // Image information: default orientation, then each channel's descriptor; the unused ones,
    // the white point and the primaries are undefined.
    wr.u8(192, 0);
    wr.u8(193, designators.len() as u8);
    wr.undefined(CINEON_CHANNEL0, 420);
    for (i, &designator) in designators.iter().enumerate() {
        let c = CINEON_CHANNEL0 + 28 * i;
        wr.u8(c, 0);
        wr.u8(c + 1, designator);
        wr.u8(c + 2, 10);
        wr.u8(c + 3, 0);
        wr.u32(c + 4, pic.width);
        wr.u32(c + 8, pic.height);
        // Code values 0–1023 stand for printing densities 0–2.048, the Cineon convention. All
        // four are 32-bit floats here (DPX stores the data values as integers).
        wr.u32(c + 12, 0f32.to_bits());
        wr.u32(c + 16, 0f32.to_bits());
        wr.u32(c + 20, 1023f32.to_bits());
        wr.u32(c + 24, 2.048f32.to_bits());
    }
    // Pixel-interleaved, unsigned, positive, no line or channel padding.
    wr.u8(681, 5);
    // Image origination: pixel pitches and gamma unknown.
    wr.undefined(972, 984);
    // Film header: manufacturer, type, perforation offset, prefix, count, frame position and rate
    // unknown; its strings stay empty.
    wr.undefined(1024, 1036);
    wr.undefined(1068, 1076);

    let mut out = wr.h;
    out.try_reserve_exact(data_len).map_err(|_| Error::Encode("out of memory".into()))?;
    for row in pic.codes.chunks_exact(per_line) {
        pack_line(&mut out, row, 10, false);
    }
    Ok(out)
}
