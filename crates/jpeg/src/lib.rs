#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room decoder for the JPEG processes most Rust JPEG decoders don't handle, written from
//! ITU-T T.81 (ISO/IEC 10918-1).
//!
//! [`decode`] supports these frames:
//!
//! - sequential DCT, Huffman coded, 8 or 12-bit samples (SOF0 and SOF1; SOF0 files that declare
//!   12 bits are read as SOF1);
//! - extended sequential DCT, arithmetic coded, 8 or 12-bit (SOF9);
//! - progressive DCT, Huffman coded, 8 or 12-bit (SOF2);
//! - progressive DCT, arithmetic coded, 8 or 12-bit (SOF10).
//!
//! Every other frame returns [`Error::Unsupported`]: lossless (SOF3, SOF11) and hierarchical
//! (SOF5–SOF7, SOF13–SOF15) files, and DCT precisions other than 8 and 12 bits.
//!
//! [`Bands`] decodes the common case, a sequential Huffman-coded 8-bit frame in one scan, a band
//! of rows at a time, into the samples [`decode`] would give for those rows, so that images too
//! large for one buffer can be decoded straight into the caller's storage.
//!
//! An app that also uses a baseline decoder routes each file with [`frame`], which reports the
//! first frame marker and its precision without decoding. For example, to send this crate the
//! arithmetic-coded files and the Huffman-coded ones that aren't 8-bit:
//!
//! ```
//! fn use_craft_jpeg(bytes: &[u8]) -> bool {
//!     match craft_jpeg::frame(bytes) {
//!         Some(f) if (0xC9..=0xCB).contains(&f.marker) => true,
//!         Some(f) if matches!(f.marker, 0xC0 | 0xC1) => f.precision != 8,
//!         _ => false,
//!     }
//! }
//! # assert!(!use_craft_jpeg(&[0xFF, 0xD8]));
//! ```
//!
//! Every scan is decoded into a whole-image buffer of quantized coefficients (Annex F for the
//! sequential scans, Annex G for the progressive ones; the arithmetic decoder of Annex D with the
//! conditioning of `DAC`), then dequantized, inverse transformed in floating point (A.3.3),
//! upsampled and color converted. 12-bit images come out as 16-bit samples (scaled, not
//! truncated). Restart intervals, 1, 3 and 4 components and any sampling factors are supported.
//! Metadata (ICC profiles, EXIF, XMP, Photoshop resources, resolution) is left to the caller,
//! which also reads the APP14 `Adobe` segment and passes its transform byte to [`decode`].
//!
//! The decoder never panics on input: every read from the stream is bounds-checked, the work
//! is bounded by 64 passes over the frame's blocks (`SCAN_PASSES`), and
//! allocations are checked against the [`Limits`] first.
//! Damaged entropy-coded data stops that scan; what was decoded is kept and the caller is told
//! through [`Decoded::damaged`].
#![warn(missing_docs)]

/// Why a stream could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The stream breaks T.81 in a way the decoder cannot recover from.
    #[error("malformed JPEG: {0}")]
    Malformed(String),
    /// A valid JPEG process or feature this decoder does not implement.
    #[error("{0}")]
    Unsupported(String),
    /// Decoding would exceed the caller's [`Limits`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

fn err(m: impl std::fmt::Display) -> Error {
    Error::Malformed(m.to_string())
}

fn unsupported(m: impl Into<String>) -> Error {
    Error::Unsupported(m.into())
}

/// Annex K Huffman tables used by AVI1 Motion JPEG streams that omit DHT.
const AVI1_DEFAULT_DHT: &[u8] = &[
    0xFF, 0xC4, 0x01, 0xA2, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02,
    0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x01, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05,
    0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22,
    0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A,
    0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A,
    0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8,
    0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0x11, 0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00, 0x01, 0x02, 0x03, 0x11, 0x04,
    0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33,
    0x52, 0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x35, 0x36,
    0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66,
    0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA,
    0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7,
    0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// Insert Annex K Huffman tables into an AVI1 MJPEG frame that omits DHT.
///
/// Frames that are not JPEG, already define DHT, or do not reach SOS are returned unchanged.
/// The caller should use this only for an AVI1/MJPEG stream, since ordinary JPEG streams must
/// provide their own tables.
pub fn with_avi1_default_huffman(jpeg: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    if jpeg.get(..2) != Some(&[0xFF, 0xD8]) {
        return Cow::Borrowed(jpeg);
    }
    let mut at = 2usize;
    while at + 1 < jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        match marker {
            0xC4 => return Cow::Borrowed(jpeg),
            0xDA => {
                let mut out = Vec::with_capacity(jpeg.len().saturating_add(AVI1_DEFAULT_DHT.len()));
                out.extend_from_slice(&jpeg[..at]);
                out.extend_from_slice(AVI1_DEFAULT_DHT);
                out.extend_from_slice(&jpeg[at..]);
                return Cow::Owned(out);
            }
            0xD0..=0xD9 | 0x01 => at += 2,
            0xFF => at += 1,
            _ => {
                let Some(len) = jpeg.get(at + 2..at + 4).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))) else { break };
                if len < 2 {
                    break;
                }
                let Some(next) = at.checked_add(2 + len).filter(|&n| n <= jpeg.len()) else { break };
                at = next;
            }
        }
    }
    Cow::Borrowed(jpeg)
}

/// Decompression-bomb guards, checked against the frame header before any image buffer is
/// allocated.
///
/// ```
/// let mut limits = craft_jpeg::Limits::default();
/// limits.max_alloc = 512 << 20;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Maximum `width * height`.
    pub max_pixels: u64,
    /// Maximum bytes for the decoded output and the decoder's own coefficient and sample
    /// buffers together. Totals above `isize::MAX` are rejected whatever this is set to.
    pub max_alloc: u64,
}

impl Default for Limits {
    fn default() -> Self {
        // 2^30 pixels (e.g. 32768²) and, on 64-bit targets, 8 GiB. On 32-bit targets (wasm32 has
        // a 4 GiB address space shared with the rest of the app) 1 GiB is the most one buffer can
        // safely ask for.
        let max_alloc = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 1 << 30 };
        Limits { max_pixels: 1 << 30, max_alloc }
    }
}

impl Limits {
    /// Check an output buffer of `width × height` pixels of `bytes_per_pixel` bytes.
    fn check(&self, width: usize, height: usize, bytes_per_pixel: u64) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(err(format!("zero-sized image {width}x{height}")));
        }
        let pixels = (width as u64).saturating_mul(height as u64);
        if pixels > self.max_pixels {
            return Err(Error::LimitExceeded(format!("{pixels} pixels exceed max {}", self.max_pixels)));
        }
        let bytes = pixels.saturating_mul(bytes_per_pixel);
        if bytes > self.max_alloc {
            return Err(Error::LimitExceeded(format!("{bytes} bytes exceed max_alloc {}", self.max_alloc)));
        }
        if usize::try_from(bytes).is_err() {
            return Err(Error::LimitExceeded(format!("{bytes} bytes do not fit in memory")));
        }
        Ok(())
    }
}

/// Zigzag scan position `k` → natural (row-major) coefficient index (Figure A.6).
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The first frame header of a stream, as reported by [`frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameKind {
    /// The second byte of the SOFn marker: `0xC0` (SOF0) to `0xCF` (SOF15), never `0xC4`,
    /// `0xC8` or `0xCC`.
    pub marker: u8,
    /// The sample precision in bits (P in B.2.2), as declared; not validated.
    pub precision: u8,
}

/// The first frame marker (SOFn) and its sample precision, found by walking the marker
/// segments that precede it. `None` when the stream doesn't start with SOI or reaches a scan,
/// EOI or the end of the data first. Reads only the headers, so it is cheap enough to route
/// every JPEG an app opens.
pub fn frame(b: &[u8]) -> Option<FrameKind> {
    if b.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    loop {
        if *b.get(i)? != 0xFF {
            return None;
        }
        let m = *b.get(i + 1)?;
        match m {
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0xD9 | 0xDA => return None,
            _ => {
                let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
                if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    return Some(FrameKind { marker: m, precision: *b.get(i + 4)? });
                }
                i = i.checked_add(2 + len)?;
            }
        }
    }
}

/// Decoded samples, interleaved by pixel (`components` values per pixel), rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Samples {
    /// 8-bit frames: `0..=255`.
    U8(Vec<u8>),
    /// 12-bit frames, scaled to the full 16-bit range: `round(v × 65535 / 4095)`.
    U16(Vec<u16>),
}

/// A decoded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Samples per pixel: 1 for gray, 3 for RGB, 4 for CMYK.
    ///
    /// Three-component frames are converted from YCbCr to RGB (JFIF, T.871) unless the Adobe
    /// transform is 0 or the component IDs are `R`, `G`, `B`. Four-component samples are ink
    /// amounts (0 = no ink): the stored values are taken to follow Adobe's convention, inverted
    /// (maximum = no ink), and are inverted back. With Adobe transform 2 (YCCK) the first three
    /// components are converted from YCbCr, which yields the ink amounts of C, M and Y directly,
    /// and K is inverted.
    pub components: u8,
    /// The pixel data.
    pub samples: Samples,
    /// Entropy-coded data was damaged or cut short: the rest of that scan, and any scans past
    /// the decoder's cap, were left out, and those blocks keep what earlier scans decoded.
    pub damaged: bool,
}

mod arith;
mod bands;
mod huffman;
mod idct;
mod parse;
mod scan;

pub use bands::{Band, Bands, Reader};

use idct::render;
use scan::decode_coefficients;

/// Decode a JPEG stream of one of the frame types listed in the [crate docs](crate).
///
/// `adobe_transform` is the transform byte of the stream's APP14 `Adobe` segment, if the caller
/// found one: it selects the color conversion of 3 and 4-component images (see
/// [`Decoded::components`]). The decoder ignores every APPn segment itself.
///
/// Damaged entropy-coded data after at least one decoded MCU returns `Ok` with
/// [`Decoded::damaged`] set; a stream that yields no image data returns an error.
pub fn decode(bytes: &[u8], limits: &Limits, adobe_transform: Option<u8>) -> Result<Decoded, Error> {
    let (frame, tables, damaged) = decode_coefficients(bytes, limits)?;
    let (width, height) = (frame.width as u32, frame.height as u32);
    let (components, samples) = render(frame, &tables, adobe_transform)?;
    Ok(Decoded { width, height, components, samples, damaged })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
extern crate self as craft_jpeg;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
