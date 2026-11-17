//! The resolution a raster file records, read from its headers without decoding any pixels.
//!
//! A layout app placing a picture wants its physical size before (or without) decoding it: a
//! 600-pixel photo saved at 600 ppi places one inch wide. [`file_resolution`] answers from the
//! container alone:
//!
//! - JPEG: the resolution [`crate::decode`] reports, from the same marker scan (stopping at the
//!   start of scan): Photoshop ResolutionInfo, then XMP, then EXIF, then the JFIF density.
//! - PNG: the `pHYs` chunk when its unit is the metre (read as the decoder reads it, so a stored
//!   11811 is 300 ppi), else the `eXIf` resolution; the chunk scan stops at `IDAT` or `IEND`.
//! - WebP: the IFD0 resolution of the `EXIF` chunk.
//! - BMP: the pixels-per-metre fields of a `BITMAPINFOHEADER` or later header.
//! - Classic TIFF: the IFD0 resolution tags.
//!
//! EXIF and TIFF resolution tags are read by [`crate::resolution::exif_resolution`]. Every scan
//! is bounded by the input length and a chunk count, and anything malformed, absent,
//! non-positive or past [`MAX_PPI`] gives `None`, so the caller keeps its own default.

use crate::Limits;
use crate::codecs::{jpeg, png, webp};
use crate::resolution::exif_resolution;

/// Resolutions above this many pixels per inch are treated as corrupt.
pub const MAX_PPI: f32 = 100_000.0;

/// PNG chunks looked at before giving up.
const MAX_CHUNKS: usize = 4096;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Pixels per inch (horizontal, vertical) that `bytes` records in its headers, or `None` when
/// the format is not one listed in the [module documentation](self) or carries no usable value.
pub fn file_resolution(bytes: &[u8]) -> Option<(f32, f32)> {
    let ppi = if bytes.starts_with(b"\xFF\xD8\xFF") {
        jpeg::scan_metadata(bytes).dpi
    } else if bytes.starts_with(PNG_SIGNATURE) {
        png_phys(bytes).or_else(|| png_exif(bytes))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        webp_exif(bytes)
    } else if bytes.starts_with(b"BM") {
        bmp(bytes)
    } else if is_classic_tiff(bytes) {
        exif_resolution(bytes)
    } else {
        None
    };
    ppi.filter(|&p| usable(p))
}

/// Whether both axes are finite and between 1 and [`MAX_PPI`].
pub fn usable((x, y): (f32, f32)) -> bool {
    let ok = |v: f32| v.is_finite() && (1.0..=MAX_PPI).contains(&v);
    ok(x) && ok(y)
}

fn is_classic_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn le_i32(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn png_phys(bytes: &[u8]) -> Option<(f32, f32)> {
    let mut pos = PNG_SIGNATURE.len();
    for _ in 0..MAX_CHUNKS {
        let len = usize::try_from(be_u32(bytes, pos)?).ok()?;
        let data = pos.checked_add(8)?;
        match bytes.get(pos.checked_add(4)?..data)? {
            // A `pHYs` chunk holds exactly nine bytes; the whole chunk, CRC included, must be there.
            b"pHYs" if len == 9 => return bytes.get(data..data.checked_add(13)?).and_then(|chunk| phys(chunk.get(..9)?)),
            b"pHYs" => return None,
            b"IDAT" | b"IEND" => return None,
            _ => pos = data.checked_add(len)?.checked_add(4)?,
        }
    }
    None
}

/// `pHYs`: pixels per unit in x and y, then the unit (1 = metre; 0 = aspect ratio only).
fn phys(data: &[u8]) -> Option<(f32, f32)> {
    let (x, y) = (be_u32(data, 0)?, be_u32(data, 4)?);
    // As the decoder does: a zero count, or a unit other than the metre, records no resolution,
    // so the `eXIf` resolution can still answer.
    if *data.get(8)? != 1 || x == 0 || y == 0 {
        return None;
    }
    Some((png::dpi_of_ppm(x), png::dpi_of_ppm(y)))
}

/// The resolution of a PNG's `eXIf` chunk, read by the decoder's own header probe.
fn png_exif(bytes: &[u8]) -> Option<(f32, f32)> {
    let (_, _, exif) = png::probe(bytes, &Limits::default()).ok()?;
    exif_resolution(&exif?)
}

/// The resolution of a WebP's `EXIF` chunk, read by the decoder's own container probe.
fn webp_exif(bytes: &[u8]) -> Option<(f32, f32)> {
    let (_, _, exif) = webp::probe(bytes, &Limits::default()).ok()?;
    exif_resolution(&exif?)
}

/// BMP: the info header size at 14 (40 or more for a header with resolution fields), then the
/// signed pixels-per-metre values at 38 and 42.
fn bmp(bytes: &[u8]) -> Option<(f32, f32)> {
    if le_u32(bytes, 14)? < 40 {
        return None;
    }
    let (x, y) = (le_i32(bytes, 38)?, le_i32(bytes, 42)?);
    if x <= 0 || y <= 0 {
        return None;
    }
    Some((png::dpi_of_ppm(x.unsigned_abs()), png::dpi_of_ppm(y.unsigned_abs())))
}

#[cfg(test)]
#[path = "resolution_probe_tests.rs"]
mod tests;
