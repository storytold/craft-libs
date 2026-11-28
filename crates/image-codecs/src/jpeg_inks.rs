//! A JPEG's headers and four-component samples read the way `zune-jpeg` and libjpeg-style readers
//! give them, for apps that keep a CMYK JPEG's inks: the component count and ICC profile from the
//! headers alone, and the ink amounts of a CMYK or YCCK JPEG.
//!
//! [`crate::decode`] reads the same files differently, on purpose: it keeps the samples of a
//! four-component JPEG without Adobe's marker as they are, rounds its YCCK conversion, and hands
//! arithmetic-coded, lossless and 12-bit files to its whole-coefficient decoder. The readers here
//! follow the older convention instead, so a document that kept a CMYK JPEG's inks that way reads
//! the same: every four-component file is taken to hold Adobe's inverted samples, YCCK is turned
//! into CMYK as libjpeg does (truncating), and only the 8-bit Huffman-coded frames `zune-jpeg`
//! decodes are read.

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::Format;
use crate::error::CodecError;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

/// A decoder of `bytes` whose headers have been read, with the default options (16384 pixels a
/// side at most); `None` when they aren't a JPEG or its headers can't be read.
fn headers(bytes: &[u8]) -> Option<JpegDecoder<ZCursor<&[u8]>>> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut d = JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
    d.decode_headers().ok()?;
    Some(d)
}

/// The number of colour components the frame header of JPEG `bytes` declares, whatever it is;
/// `None` when they aren't a JPEG whose headers can be read.
pub fn jpeg_components(bytes: &[u8]) -> Option<usize> {
    Some(usize::from(headers(bytes)?.info()?.components))
}

/// The width and height the frame header of JPEG `bytes` declares, for any colour type; `None`
/// when they aren't a JPEG whose headers can be read (which includes sides past 16384 pixels).
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = headers(bytes)?.dimensions()?;
    Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))
}

/// The ICC profile of JPEG `bytes`, put together from all its numbered `ICC_PROFILE` APP2
/// segments; `None` without one, when it is empty, and when the headers can't be read.
pub fn jpeg_icc_profile(bytes: &[u8]) -> Option<Vec<u8>> {
    headers(bytes)?.icc_profile().filter(|icc| !icc.is_empty())
}

/// The ink amounts of a four-component (CMYK or YCCK) JPEG: C, M, Y and K bytes a pixel, 0 for
/// no ink, as a [`ChannelLayout::Cmyk`] image. The samples are taken to be Adobe's inverted ones
/// whether or not the file has Adobe's marker; YCCK is first turned into CMYK as libjpeg does
/// (the complement of the YCbCr colour's RGB, each value truncated).
///
/// Errors: bytes that aren't a JPEG with readable headers ([`CodecError::InvalidImage`]); one of
/// other than four components, or a frame `zune-jpeg` doesn't decode, such as an arithmetic-coded
/// or 12-bit one ([`CodecError::Unsupported`]); a size past `limits`; scan data too short for the
/// frame (under a byte for every 2048 pixels, refused before anything is allocated); and data that
/// doesn't decode,
/// including frames `zune-jpeg` panics on, which are caught.
pub fn decode_jpeg_inks(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let bad = |m: &str| CodecError::InvalidImage(format!("JPEG inks: {m}"));
    let mut d = headers(bytes).ok_or_else(|| bad("not a JPEG with readable headers"))?;
    let space = d.input_colorspace().ok_or_else(|| bad("no colour space"))?;
    let components = d.info().map_or(0, |i| i.components);
    if components != 4 || !matches!(space, ColorSpace::CMYK | ColorSpace::YCCK) {
        return Err(CodecError::unsupported(Format::Jpeg, format!("inks of a {components}-component {space:?} image")));
    }
    let (w, h) = d.dimensions().ok_or_else(|| bad("no size"))?;
    let (w, h) = (u32::try_from(w).map_err(|_| bad("width"))?, u32::try_from(h).map_err(|_| bad("height"))?);
    limits.check(w, h, ChannelLayout::Cmyk, SampleType::U8)?;
    if crate::codecs::jpeg::scan_too_short(bytes, w, h) {
        return Err(bad("the scan data is too short for the frame it declares"));
    }
    d.set_options(DecoderOptions::default().jpeg_set_out_colorspace(space));
    // zune-jpeg 0.5 asserts on some subsampled four-component frames whose first component isn't
    // the largest (a scratch row too short for its upsampler): that is an error here, not a panic.
    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.decode()));
    let mut data = decoded
        .map_err(|_| CodecError::unsupported(Format::Jpeg, "inks: zune-jpeg failed on this file"))?
        .map_err(|e| CodecError::unsupported(Format::Jpeg, format!("inks: {e}")))?;
    for p in data.as_chunks_mut::<4>().0 {
        if space == ColorSpace::YCCK {
            let [y, cb, cr] = [p[0], p[1], p[2]].map(f32::from);
            // libjpeg's YCCK → CMYK: the complement of the YCbCr colour's RGB.
            p[0] = (434.456 - y - 1.402 * cr).clamp(0.0, 255.0) as u8;
            p[1] = (119.541 - y + 0.344 * cb + 0.714 * cr).clamp(0.0, 255.0) as u8;
            p[2] = (481.816 - y - 1.772 * cb).clamp(0.0, 255.0) as u8;
        }
        for v in p.iter_mut() {
            *v = 255 - *v;
        }
    }
    Image::from_u8(w, h, ChannelLayout::Cmyk, data)
}

/// The bytes after an `ICC_PROFILE` APP2 segment's length: its name, then the segment's number and
/// the count of segments.
const ICC_SEGMENT: &[u8; 12] = b"ICC_PROFILE\0";

/// JPEG `bytes` carrying `icc` in place of the ICC profile they carry, if any: every
/// `ICC_PROFILE` APP2 segment before the scan taken out, `icc` written as numbered APP2 segments
/// after the leading application segments, and the other segments and the scan as they are. `None`
/// for bytes whose segments before the scan can't be read, an empty `icc`, and one longer than 255
/// segments hold.
pub fn jpeg_with_icc(bytes: &[u8], icc: &[u8]) -> Option<Vec<u8>> {
    // A segment's length counts itself (2 bytes) and the name, number and count (14).
    const CHUNK: usize = 0xFFFF - 2 - ICC_SEGMENT.len() - 2;
    let chunks: Vec<&[u8]> = icc.chunks(CHUNK).collect();
    let count = u8::try_from(chunks.len()).ok().filter(|n| *n > 0)?;
    let mut segments = Vec::with_capacity(icc.len() + 18 * chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let len = u16::try_from(chunk.len() + 2 + ICC_SEGMENT.len() + 2).ok()?;
        segments.extend_from_slice(&[0xFF, 0xE2]);
        segments.extend_from_slice(&len.to_be_bytes());
        segments.extend_from_slice(ICC_SEGMENT);
        segments.extend_from_slice(&[u8::try_from(i + 1).ok()?, count]);
        segments.extend_from_slice(chunk);
    }
    with_icc_segments(bytes, &segments)
}

/// JPEG `bytes` without the ICC profile they carry: every `ICC_PROFILE` APP2 segment before the
/// scan taken out, and the other segments and the scan as they are. `None` for bytes whose
/// segments before the scan can't be read.
pub fn jpeg_without_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    with_icc_segments(bytes, &[])
}

/// JPEG `bytes` with every `ICC_PROFILE` APP2 segment before the scan taken out and `segments`
/// written after the leading application segments; `None` when the bytes don't start a JPEG or
/// their segments before the scan can't be read.
fn with_icc_segments(bytes: &[u8], segments: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + segments.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut icc_segments = Some(segments);
    let mut at = 2;
    // The segments before the scan (SOS), each with its length; a marker without one ends them.
    // Fill bytes (0xFF) before a marker are left out.
    loop {
        while bytes.get(at..at + 2) == Some(&[0xFF, 0xFF]) {
            at += 1;
        }
        let (Some(&0xFF), Some(&marker)) = (bytes.get(at), bytes.get(at + 1)) else { return None };
        let app = (0xE0..=0xEF).contains(&marker);
        if !app && let Some(s) = icc_segments.take() {
            out.extend_from_slice(s);
        }
        if matches!(marker, 0xDA | 0xD8 | 0xD9 | 0x01 | 0xD0..=0xD7) {
            break;
        }
        let len = usize::from(u16::from_be_bytes(bytes.get(at + 2..at + 4)?.try_into().ok()?));
        let segment = bytes.get(at..(at + 2).checked_add(len)?).filter(|_| len >= 2)?;
        if !(marker == 0xE2 && segment.get(4..16) == Some(ICC_SEGMENT.as_slice())) {
            out.extend_from_slice(segment);
        }
        at += segment.len();
    }
    out.extend_from_slice(bytes.get(at..)?);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{jpeg_icc_profile, jpeg_with_icc, jpeg_without_icc};

    fn minimal_jpeg() -> Vec<u8> {
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]); // APP0
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]); // SOF0
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 63, 0]); // SOS
        jpeg.extend_from_slice(&[0x00, 0xFF, 0xD9]); // scan + EOI
        jpeg
    }

    #[test]
    fn replaces_and_strips_jpeg_icc_segments() {
        let base = minimal_jpeg();
        let icc_a = vec![0x11; 70_000];
        let with_a = jpeg_with_icc(&base, &icc_a).unwrap();
        assert_eq!(jpeg_icc_profile(&with_a), Some(icc_a));

        let icc_b = b"short-profile";
        let with_b = jpeg_with_icc(&with_a, icc_b).unwrap();
        assert_eq!(jpeg_icc_profile(&with_b), Some(icc_b.to_vec()));

        let stripped = jpeg_without_icc(&with_b).unwrap();
        assert_eq!(jpeg_icc_profile(&stripped), None);
        assert_eq!(stripped, base);
        assert_eq!(jpeg_with_icc(&base, &[]), None);
    }
}
