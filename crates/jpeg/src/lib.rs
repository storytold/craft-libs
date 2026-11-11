#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room decoder for the JPEG processes most Rust JPEG decoders don't handle, written from
//! ITU-T T.81 (ISO/IEC 10918-1).
//!
//! [`decode`] supports these frames:
//!
//! - sequential DCT, Huffman coded, 8 or 12-bit samples (SOF0 and SOF1; SOF0 files that declare
//!   12 bits are read as SOF1);
//! - extended sequential DCT, arithmetic coded, 8 or 12-bit (SOF9);
//! - progressive DCT, arithmetic coded, 8 or 12-bit (SOF10).
//!
//! Every other frame returns [`Error::Unsupported`]: progressive Huffman (SOF2), lossless (SOF3,
//! SOF11) and hierarchical (SOF5–SOF7, SOF13–SOF15) files, and DCT precisions other than 8 and
//! 12 bits.
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
