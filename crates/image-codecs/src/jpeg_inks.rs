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
