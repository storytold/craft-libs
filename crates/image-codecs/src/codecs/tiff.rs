//! TIFF and BigTIFF: 1/2/4/8/16/32/64-bit integer and 16/32/64-bit float samples, gray / RGB /
//! CMYK / palette with optional alpha (unassociated, or associated and un-premultiplied) and
//! extra samples, WhiteIsZero, strips or tiles, planar configuration 1 or 2, ICC (34675), XMP
//! (700), DPI and a few ASCII text tags.
//!
//! **Structure.** [`super::tiff_ifd`] walks the directories with bounds checks (classic and
//! BigTIFF, either byte order, the whole IFD chain and its SubIFDs, cycles and hostile counts
//! defeated) and lists them as pages. Decoding opens the first full-resolution page, like
//! Photoshop (reduced-resolution copies and masks are skipped); further pages are reported as a
//! [`DecodeWarning::MorePages`], and [`decode_page`] opens any directory.
//!
//! **Pixels.** The `tiff` crate decompresses one strip or tile at a time (none, LZW, Deflate,
//! PackBits, with the horizontal and floating-point predictors). The decoder sees the chosen
//! directory through a byte view whose header points at it ([`Patched`]), so SubIFDs decode
//! like IFD0. Decoding is banded: the output image is allocated once, at its final size, and
//! each band of rows (one strip, or one row of tiles, across every plane) is decoded and
//! converted straight into it, in parallel on native targets. A strip that already has the
//! output's layout is decoded in place; anything else goes through one strip- or tile-sized
//! scratch buffer per worker, so no second full-size buffer exists. Image data cut off by the
//! end of the file leaves the missing chunks empty and adds a [`DecodeWarning::Truncated`].
//! [`TiffBands`] runs the same decode a group of bands at a time and hands each group to its
//! caller, so a document can be filled without the image ever existing whole (#8169).
//!
//! Photoshop's private tags are carried as opaque bytes in [`Metadata`]: 34377 (image
//! resources) and 37724 (`ImageSourceData`, the layers of a layered TIFF). They are read with
//! a bounds-checked walk of the first directory ([`photoshop_tags`]) rather than through the
//! decoder, which would materialize a multi-megabyte layer block as one `Value` per byte, and
//! written back as BYTE / UNDEFINED tags. The encoder writes files in the native byte order
//! (see [`writes_little_endian`]), which the layer data must match.

use std::borrow::Cow;
use std::io::{self, Read, Seek, SeekFrom};

use half::f16;
use tiff::decoder::{ChunkType, Decoder};
use tiff::encoder::colortype::{self, ColorType as TiffColorType};
use tiff::encoder::{Rational, TiffEncoder, TiffKind, TiffKindBig, TiffKindStandard, TiffValue};
use tiff::tags::{PhotometricInterpretation, ResolutionUnit, SampleFormat, Tag, Type};

use super::tiff_exif::{self, ExifDirs};
use super::tiff_ifd::{self as ifd, File, Ifd};
use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits, TiffCompression};

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

fn map_err(e: tiff::TiffError) -> CodecError {
    match e {
        tiff::TiffError::LimitsExceeded => CodecError::LimitExceeded("TIFF decoder limit".into()),
        tiff::TiffError::UnsupportedError(u) => CodecError::unsupported(F, u.to_string()),
        e => err(e),
    }
}

/// TIFF/EP `PhotometricInterpretation` for colour filter array (sensor) data.
const PHOTOMETRIC_CFA: u32 = 32803;
/// DNG `PhotometricInterpretation` for demosaiced but undeveloped sensor data.
const PHOTOMETRIC_LINEAR_RAW: u32 = 34892;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_SUB_IFDS: u16 = 330;
const TAG_DNG_VERSION: u16 = 50706;

/// Recognizes TIFF-structured camera raw files (TIFF/EP, DNG, CR2 and the
/// TIFF-based NEF/ARW/PEF… layouts): a CR2 signature, a DNGVersion tag, or
/// a CFA / LinearRaw image in IFD0, its SubIFDs or the next few IFDs. Their
/// first IFD is often a reduced preview, so this runs before decoding.
/// Reads only bounds-checked header bytes; anything malformed is "not raw".
fn camera_raw(b: &[u8]) -> bool {
    let le = match b.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return false, // BigTIFF and others: no raw layouts to recognize
    };
    let u16_at = |o: usize| {
        let s = b.get(o..o.checked_add(2)?)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let u32_at = |o: usize| {
        let s: [u8; 4] = b.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    // CR2: "CR" and major version 2 right after the TIFF header.
    if b.get(8..11) == Some(b"CR\x02") {
        return true;
    }
    // Entry value (count 1) as SHORT or LONG; anything else is ignored.
    let value = |e: usize| match u16_at(e.saturating_add(2))? {
        3 => u16_at(e.saturating_add(8)).map(u32::from),
        4 | 13 => u32_at(e.saturating_add(8)),
        _ => None,
    };
    let mut queue: Vec<u32> = u32_at(4).into_iter().collect();
    let mut visited = 0;
    while let Some(ifd) = queue.pop() {
        // A handful of IFDs is enough for every real layout; also stops loops.
        visited += 1;
        if visited > 16 {
            break;
        }
        let ifd = ifd as usize;
        let Some(n) = u16_at(ifd) else { continue };
        for i in 0..usize::from(n) {
            let e = ifd.saturating_add(2 + 12 * i);
            let (Some(tag), Some(count)) = (u16_at(e), u32_at(e.saturating_add(4))) else {
                break;
            };
            match tag {
                TAG_DNG_VERSION => return true,
                TAG_PHOTOMETRIC if matches!(value(e), Some(PHOTOMETRIC_CFA | PHOTOMETRIC_LINEAR_RAW)) => {
                    return true;
                }
                TAG_SUB_IFDS if count == 1 => queue.extend(value(e)),
                TAG_SUB_IFDS => {
                    // More than one offset: they are stored at the entry's offset.
                    let at = u32_at(e.saturating_add(8)).unwrap_or(0) as usize;
                    for k in 0..count.min(8) as usize {
                        queue.extend(u32_at(at.saturating_add(4 * k)));
                    }
                }
                _ => {}
            }
        }
        if let Some(next) = u32_at(ifd.saturating_add(2 + 12 * usize::from(n))).filter(|&o| o != 0) {
            queue.push(next);
        }
    }
    false
}

macro_rules! custom_colortype {
    ($name:ident, $inner:ty, $photo:expr, $bits:expr, $fmt:expr, int) => {
        struct $name;
        impl TiffColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $fmt;
            fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
                let n = Self::SAMPLE_FORMAT.len();
                let n = n.min(row.len());
                result.extend_from_slice(&row[..n]);
                result.extend(row.iter().zip(&row[n..]).map(|(p, c)| c.wrapping_sub(*p)));
            }
        }
    };
    ($name:ident, $inner:ty, $photo:expr, $bits:expr, $fmt:expr, float) => {
        struct $name;
        impl TiffColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $fmt;
            fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
                result.extend_from_slice(row);
            }
        }
    };
}

use PhotometricInterpretation as PI;

mod bands;
mod decode;
mod encode;
mod tags;
mod ycbcr;

pub use decode::TiffBands;
pub(crate) use decode::{decode, decode_page, has_unretained_samples, orientation};
pub(crate) use encode::encode;
#[cfg(test)]
use encode::{Extras, try_with_capacity};
use tags::F;
pub use tags::{PhotoshopTags, photoshop_tags, writes_little_endian};
custom_colortype!(GrayA8, u8, PI::BlackIsZero, &[8, 8], &[SampleFormat::Uint; 2], int);
custom_colortype!(GrayA16, u16, PI::BlackIsZero, &[16, 16], &[SampleFormat::Uint; 2], int);
custom_colortype!(GrayA32F, f32, PI::BlackIsZero, &[32, 32], &[SampleFormat::IEEEFP; 2], float);
custom_colortype!(CmykA16, u16, PI::CMYK, &[16; 5], &[SampleFormat::Uint; 5], int);
custom_colortype!(CmykA32F, f32, PI::CMYK, &[32; 5], &[SampleFormat::IEEEFP; 5], float);
// CIELab as Photoshop writes it: a* and b* signed, the sample format left unsigned.
custom_colortype!(Lab8, u8, PI::CIELab, &[8; 3], &[SampleFormat::Uint; 3], int);
custom_colortype!(Lab16, u16, PI::CIELab, &[16; 3], &[SampleFormat::Uint; 3], int);
custom_colortype!(LabA8, u8, PI::CIELab, &[8; 4], &[SampleFormat::Uint; 4], int);
custom_colortype!(LabA16, u16, PI::CIELab, &[16; 4], &[SampleFormat::Uint; 4], int);

#[cfg(test)]
mod bits_len {
    use super::Extras;
    use tiff::encoder::colortype::ColorType as TiffColorType;

    #[test]
    fn const_generic_sample_tables_have_one_entry_per_sample() {
        // A repeat expression in an associated const (`&[BITS; N]`) has, on some compilers,
        // promoted to a slice shorter than `N`. The encoder sizes every strip from that length.
        assert_eq!(<Extras<u8, 2, 8, 4> as TiffColorType>::BITS_PER_SAMPLE.len(), 4, "{:?}", <Extras<u8, 2, 8, 4> as TiffColorType>::BITS_PER_SAMPLE);
        assert_eq!(<Extras<u8, 2, 8, 5> as TiffColorType>::BITS_PER_SAMPLE.len(), 5);
        assert_eq!(<Extras<u8, 2, 8, 19> as TiffColorType>::BITS_PER_SAMPLE.len(), 19);
        assert_eq!(<Extras<u16, 5, 16, 6> as TiffColorType>::SAMPLE_FORMAT.len(), 6);
    }
}

#[cfg(test)]
mod extra_sample_buffers {
    use super::try_with_capacity;

    /// The extra-sample writer's buffers are allocated fallibly: a size no allocation can serve
    /// is an out-of-memory error the save reports, not an abort.
    #[test]
    fn an_impossible_buffer_is_an_error() {
        let err = try_with_capacity::<u8>(usize::MAX).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::OutOfMemory);
        assert!(try_with_capacity::<u16>(usize::MAX / 2).is_err());
        assert!(try_with_capacity::<f32>(64).is_ok_and(|v| v.is_empty() && v.capacity() >= 64));
    }
}
