//! JPEG-compressed strips: TIFF compression 7 ("new-style" JPEG).
//!
//! Each strip is a complete baseline JPEG datastream (SOI to EOI) of `ImageWidth` × the strip's
//! rows, so no `JPEGTables` tag is needed. RGB is stored as JPEG's YCbCr, with
//! `PhotometricInterpretation` 6 and a `YCbCrSubSampling` that matches the JPEG frame; gray is
//! one-component JPEG with `PhotometricInterpretation` 1. `RowsPerStrip` is a multiple of 16 (the
//! 4:2:0 MCU height) unless one strip holds the whole image.
//!
//! # Sources
//!
//! - TIFF Revision 6.0 (Final, June 3, 1992), Section 21 "YCbCr Images": `YCbCrSubSampling`
//!   defaults to 2, 2, so it is always written; `ReferenceBlackWhite` defaults to
//!   [0, 255, 0, 255, 0, 255], and the section lists [0, 255, 128, 255, 128, 255] for YCbCr
//!   with no headroom or footroom, which is JPEG's full range.
//!   <https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf>
//! - TIFF Technical Note #2, "Replacement TIFF/JPEG specification": compression
//!   7, one JPEG datastream per strip, optional `JPEGTables`, PhotometricInterpretation 6 when
//!   the JPEG data is YCbCr, and `RowsPerStrip` a multiple of 8 × the vertical sampling factor.

use super::super::{CodecError, EncodeOptions, Image, Plan, Rational, ResolutionUnit, SampleFormat, Tag, TiffEncoder, TiffKind, io};
use super::{TagSet, write_sub_directories, write_tags};
use crate::image::{ChannelLayout, SampleType};
use std::io::Cursor;

use super::F;

/// TIFF `Compression` value for JPEG (Technical Note #2).
const COMPRESSION_JPEG: u16 = 7;
/// TIFF `ReferenceBlackWhite` (532): not among the `tiff` crate's named tags.
const TAG_REFERENCE_BLACK_WHITE: u16 = 532;
/// The JPEG (JFIF) code range: full-range luma, chroma centred on 128.
const JFIF_REFERENCE_BLACK_WHITE: [u32; 6] = [0, 255, 128, 255, 128, 255];
/// Strips hold about this many bytes of samples before compression, as the other compressions'.
const STRIP_BYTES: u64 = 1_000_000;
/// The tallest MCU the strips use (4:2:0): every strip but the last is a multiple of it.
const MCU_ROWS: u64 = 16;
/// `jpeg-encoder` takes `u16` dimensions: the tallest multiple of [`MCU_ROWS`] it accepts.
const MAX_STRIP_ROWS: u64 = u16::MAX as u64 / MCU_ROWS * MCU_ROWS;

/// How the strips are compressed.
#[derive(Clone, Copy)]
pub(in crate::codecs::tiff) struct Strips {
    quality: u8,
    /// 4:2:0 chroma for RGB (4:4:4 otherwise).
    subsampled: bool,
}

impl Strips {
    pub(in crate::codecs::tiff) fn of(opts: &EncodeOptions) -> Self {
        Strips { quality: opts.jpeg_quality.clamp(1, 100), subsampled: opts.jpeg_chroma_subsampling }
    }
}

/// Why an image of `layout` and `sample` with `extras` extra channels can't be written as JPEG
/// strips, or `None` when it can.
fn refusal(layout: ChannelLayout, sample: SampleType, extras: usize, width: u32) -> Option<String> {
    if !matches!(layout, ChannelLayout::Gray | ChannelLayout::Rgb) {
        return Some(format!("TIFF JPEG compression stores gray or RGB images without alpha, not {layout:?}"));
    }
    if sample != SampleType::U8 {
        return Some(format!("TIFF JPEG compression stores 8-bit images only, not {sample:?}"));
    }
    if extras > 0 {
        return Some(format!("TIFF JPEG compression can't store extra channels ({extras} present)"));
    }
    if u16::try_from(width).is_err() {
        return Some(format!("TIFF JPEG compression stores images at most {} pixels wide, not {width}", u16::MAX));
    }
    None
}

/// Refuses an image JPEG strips can't hold, before anything is converted: both the image as the
/// caller passed it and as the plan would write it must be 8-bit gray or RGB.
pub(in crate::codecs::tiff) fn check(src: &Image, plan: Plan) -> Result<(), CodecError> {
    let extras = src.meta.extra_samples.len();
    let width = src.dimensions().0;
    match refusal(src.layout(), src.sample_type(), extras, width).or_else(|| refusal(plan.layout, plan.sample, extras, width)) {
        Some(why) => Err(CodecError::encode(F, why)),
        None => Ok(()),
    }
}

fn invalid(why: impl Into<String>) -> tiff::TiffError {
    io::Error::new(io::ErrorKind::InvalidInput, why.into()).into()
}

/// Rows per strip for a `row_bytes`-wide image `height` rows tall: about [`STRIP_BYTES`], a
/// multiple of [`MCU_ROWS`], or the whole height when that is less.
fn rows_per_strip(row_bytes: u64, height: u32) -> u32 {
    let rows = STRIP_BYTES.div_ceil(row_bytes.max(1)).next_multiple_of(MCU_ROWS).min(MAX_STRIP_ROWS);
    // Both bounds fit a u32: `MAX_STRIP_ROWS` < 2^16.
    u32::try_from(rows).map_or(height, |r| r.min(height))
}

/// Writes one image directory with JPEG-compressed strips.
pub(in crate::codecs::tiff) fn write<K: TiffKind>(
    enc: &mut TiffEncoder<&mut Cursor<Vec<u8>>, K>,
    img: &Image,
    tags: &TagSet<'_>,
    strips: Strips,
) -> tiff::TiffResult<()> {
    let (w, h) = img.dimensions();
    if let Some(why) = refusal(img.layout(), img.sample_type(), img.meta.extra_samples.len(), w) {
        return Err(invalid(why));
    }
    let rgb = img.layout() == ChannelLayout::Rgb;
    let (color, channels) = if rgb { (jpeg_encoder::ColorType::Rgb, 3usize) } else { (jpeg_encoder::ColorType::Luma, 1) };
    let width = u16::try_from(w).map_err(|_| invalid("image too wide for JPEG"))?;
    let row_bytes = usize::from(width).checked_mul(channels).ok_or_else(|| invalid("image row size overflows"))?;
    let rows = rows_per_strip(row_bytes as u64, h);
    let strip_bytes = row_bytes.checked_mul(rows as usize).ok_or_else(|| invalid("strip size overflows"))?;
    let len = row_bytes.checked_mul(h as usize).ok_or_else(|| invalid("image size overflows"))?;
    let data = img.data().get(..len).ok_or_else(|| invalid("image buffer is shorter than its dimensions"))?;
    let sampling = if rgb && strips.subsampled { jpeg_encoder::SamplingFactor::R_4_2_0 } else { jpeg_encoder::SamplingFactor::R_4_4_4 };

    let subs = write_sub_directories(enc, tags)?;
    let mut d = enc.image_directory()?;
    let mut offsets = Vec::new();
    let mut counts = Vec::new();
    let mut jpeg = Vec::new();
    for strip in data.chunks(strip_bytes.max(1)) {
        let lines = u16::try_from(strip.len() / row_bytes).map_err(|_| invalid("strip too tall for JPEG"))?;
        jpeg.clear();
        let mut je = jpeg_encoder::Encoder::new(&mut jpeg, strips.quality);
        je.set_sampling_factor(sampling);
        je.encode(strip, width, lines, color).map_err(|e| invalid(format!("JPEG strip: {e}")))?;
        let at = d.write_data(jpeg.as_slice())?;
        offsets.push(K::convert_offset(at)?);
        counts.push(K::convert_offset(jpeg.len() as u64)?);
    }

    d.write_tag(Tag::ImageWidth, w)?;
    d.write_tag(Tag::ImageLength, h)?;
    d.write_tag(Tag::BitsPerSample, &[8u16; 3][..channels])?;
    d.write_tag(Tag::Compression, COMPRESSION_JPEG)?;
    d.write_tag(Tag::PhotometricInterpretation, if rgb { 6u16 } else { 1 })?;
    d.write_tag(Tag::SamplesPerPixel, channels as u16)?;
    d.write_tag(Tag::RowsPerStrip, rows)?;
    d.write_tag(Tag::StripOffsets, K::convert_slice(&offsets))?;
    d.write_tag(Tag::StripByteCounts, K::convert_slice(&counts))?;
    d.write_tag(Tag::SampleFormat, &[SampleFormat::Uint.to_u16(); 3][..channels])?;
    // Baseline TIFF requires a resolution: none unless `write_tags` writes the image's own.
    d.write_tag(Tag::XResolution, Rational { n: 1, d: 1 })?;
    d.write_tag(Tag::YResolution, Rational { n: 1, d: 1 })?;
    d.write_tag(Tag::ResolutionUnit, ResolutionUnit::None.to_u16())?;
    if rgb {
        let factor = if strips.subsampled { 2u16 } else { 1 };
        d.write_tag(Tag::ChromaSubsampling, &[factor, factor][..])?;
        let reference = JFIF_REFERENCE_BLACK_WHITE.map(|n| Rational { n, d: 1 });
        d.write_tag(Tag::Unknown(TAG_REFERENCE_BLACK_WHITE), &reference[..])?;
    }
    write_tags(&mut d, tags, subs)?;
    d.finish()
}

#[cfg(test)]
mod tests {
    use super::{MCU_ROWS, rows_per_strip};

    /// Every strip but the last is a whole number of 4:2:0 MCUs (Technical Note #2), however
    /// the width divides the strip budget; a short image is one strip.
    #[test]
    fn strips_are_whole_mcus_or_the_whole_image() {
        for row_bytes in [1u64, 3, 7, 999, 3000, 65_535 * 3, 1 << 30] {
            let rows = rows_per_strip(row_bytes, 100_000);
            assert!(rows >= 16 && u64::from(rows) % MCU_ROWS == 0, "{row_bytes}: {rows}");
        }
        assert_eq!(rows_per_strip(30, 5), 5);
    }
}
