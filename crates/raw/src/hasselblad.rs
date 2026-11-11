//! Hasselblad 3FR and FFF raws ([`crate::RawFormat::CfaTiff`]).
//!
//! Sources: TIFF 6.0 (container), the DNG Specification (the tags these files borrow) and black-box analysis, done
//! for LightCraft, of a CC0 H4D-40 FFF from raw.pixls.us (931, "Hasselblad - H4D-40 - 16bit (4:3).fff"); every
//! number below was measured on it. PhotoCraft's reader measured the same layout independently on an X2D II 100C
//! (red at the data origin, against the camera's embedded JPEG).
//!
//! - Both are TIFFs without a DNG version tag that describe their raw image with DNG's tags: a full-resolution
//!   CFA sub-IFD with `BlackLevel`, `WhiteLevel`, `DefaultCropOrigin` / `DefaultCropSize`, and `AsShotNeutral`.
//!   [`crate::dng::decode_as`] reads all of that; this module adds what that reader does not know about.
//! - The raw image is uncompressed (X1D, X1D II, X2D, CFV 100C) or Hasselblad's lossless JPEG with predictor
//!   selection value 8 ([`crate::ljpeg::decode_hasselblad`]); the H4D-40 FFF's 7410 x 5586 strip is the latter.
//! - No `CFAPattern`. The layout is RGGB from the sensor origin: averaged over the 6 x 6 sensor blocks behind each
//!   pixel of the file's 1217 x 913 RGB IFD0 image (the default crop is exactly six times its size), the sites at
//!   (0, 0) correlate 0.976 with its red and 0.726 with its blue, the sites at (1, 1) 0.896 with blue and 0.847
//!   with red, and the two green sites' diagonal differences are half those of the other pair.
//! - An FFF is a 3FR converted on the desktop. It has no colour matrix of its own: the camera's matrix stays in
//!   the original 3FR header, which the FFF carries whole as a little-endian TIFF in the private IFD0 tag
//!   `0xC51B` (4096 bytes in the sample: an IFD0 with Make, Model, `ColorMatrix1` and `AsShotNeutral`, a
//!   `SubIFDs` pair for the raw and preview images, whose data offsets point into the old 3FR and are not read).
//!   [`decode`] takes the matrices from there when the outer file has none. The white balance stays the outer
//!   file's, which is the one the converter last saved.
//! - No JPEG preview. IFD0 is an uncompressed 8-bit RGB image (1217 x 913 in the H4D-40 FFF, IFD1 pointing at the
//!   same strip; 218 x 218 in the CFV 3FR, under the file's own `Orientation`); the FFF also has a 16-bit RGB
//!   copy in a `SubIFDs` child, with a `TransferFunction` whose meaning is not known, so it is not used.
//!   [`rgb_preview`] hands the largest 8-bit one on as a small baseline TIFF.
use crate::dng;
use crate::{ColorData, Limits, Mode, RawFormat, RawImage, Result};
use craft_tiff::image::{Layout, chunk_bytes};
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, ParseOptions, Tiff, TiffWriter, Value, tags as t};

/// Private IFD0 tag holding the original 3FR header (a complete TIFF) inside an FFF.
pub(crate) const ORIGINAL_HEADER: u16 = 0xc51b;

/// Upper bound on the embedded header's size: the sample's is 4 KiB; a larger block is not a header.
const MAX_HEADER: usize = 1 << 20;

/// Decode a 3FR / FFF (or another TIFF raw in the DNG layout, see [`dng::is_plain_cfa_tiff`]).
pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let mut img = dng::decode_as(bytes, mode, RawFormat::CfaTiff, limits)?;
    if !has_matrix(&img.color)
        && let Some(header) = original_header_color(bytes)
    {
        adopt_matrices(&mut img.color, &header);
    }
    Ok(img)
}

/// The colour tags of the original 3FR header an FFF embeds in [`ORIGINAL_HEADER`], when it has a colour matrix.
fn original_header_color(bytes: &[u8]) -> Option<ColorData> {
    let outer = Tiff::parse(bytes).ok()?;
    let block = outer.ifds.first()?.bytes(ORIGINAL_HEADER)?;
    if block.len() > MAX_HEADER {
        return None;
    }
    let opts = ParseOptions { max_ifds: 16, max_depth: 2, ..Default::default() };
    let header = Tiff::parse_with(block, &opts).ok()?;
    let ifd0 = header.ifds.first()?;
    let raw = dng::raw_ifd(&header).unwrap_or(ifd0);
    let color = dng::color_data(ifd0, raw, header.order);
    has_matrix(&color).then_some(color)
}

/// Whether the colour tags hold a camera matrix.
fn has_matrix(color: &ColorData) -> bool {
    color.color_matrix.iter().any(Option::is_some)
}

/// Upper bound on a preview's pixel bytes (8-bit RGB): 64 MP, far above the 1.1 MP of the largest one seen.
const MAX_PREVIEW_BYTES: usize = (64 << 20) * 3;

/// The file's largest uncompressed 8-bit RGB image (chunky, in strips), as a little-endian baseline TIFF holding
/// just those pixels, which any TIFF reader decodes. No `Orientation` tag is written: the viewer
/// applies the raw file's, which is the one that stands over this image in IFD0.
pub(crate) fn rgb_preview(tiff: &Tiff, bytes: &[u8]) -> Option<Vec<u8>> {
    let info = tiff
        .all_ifds()
        .into_iter()
        .filter_map(|i| i.image().ok())
        .filter(|i| {
            i.photometric == t::photometric::RGB
                && i.compression == t::compression::NONE
                && i.samples_per_pixel == 3
                && i.bits_per_sample.iter().all(|&b| b == 8)
                && i.planar == 1
                && i.sample_format == 1
                && matches!(i.layout, Layout::Strips { .. })
                && i.width > 0
                && i.height > 0
                // uncompressed: a preview larger than the whole file is not in it, and is passed over before any buffer
                // is reserved (strips sharing one offset would otherwise copy a few bytes up to MAX_PREVIEW_BYTES); a
                // smaller complete one elsewhere in the file is still taken
                && (u64::from(i.width) * u64::from(i.height)).saturating_mul(3) <= MAX_PREVIEW_BYTES.min(bytes.len()) as u64
        })
        .max_by_key(|i| u64::from(i.width) * u64::from(i.height))?;
    let size = info.width as usize * info.height as usize * 3;
    let mut pixels = Vec::with_capacity(size);
    for chunk in info.chunks(bytes.len() as u64) {
        pixels.extend_from_slice(chunk_bytes(bytes, &chunk)?);
        if pixels.len() >= size {
            break;
        }
    }
    // a short last strip is a truncated file: no preview rather than a picture with a black band
    if pixels.len() < size {
        return None;
    }
    pixels.truncate(size);
    let mut ifd = IfdBuilder::new();
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![info.width]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![info.height]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
    ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::RGB]));
    ifd.set(t::COMPRESSION, Value::Short(vec![t::compression::NONE]));
    ifd.set(t::PLANAR_CONFIGURATION, Value::Short(vec![1]));
    ifd.set_image(ImageData::Strips { rows_per_strip: info.height, strips: vec![pixels] });
    TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).ok()
}

/// Take the camera matrices (and the illuminants they belong to) of `from`; the rest of `color` stays.
fn adopt_matrices(color: &mut ColorData, from: &ColorData) {
    color.illuminant = from.illuminant;
    color.color_matrix = from.color_matrix;
    color.forward_matrix = from.forward_matrix;
    color.camera_calibration = from.camera_calibration;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RawData, RawError, decode as decode_file, probe, probe_info};
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value, tags as t};

    const MATRIX: [(i32, i32); 9] =
        [(686, 1000), (-196, 1000), (-32, 1000), (-437, 1000), (1228, 1000), (235, 1000), (-165, 1000), (317, 1000), (675, 1000)];

    fn rgb_ifd() -> IfdBuilder {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![4]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![3]));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
        ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
        ifd.set(t::PHOTOMETRIC, Value::Short(vec![2]));
        ifd.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd.set_image(ImageData::Strips { rows_per_strip: 3, strips: vec![(0..36).collect()] });
        ifd
    }

    /// The camera's 3FR header as an FFF embeds it: a little-endian TIFF whose IFD0 has the colour matrix.
    fn original_header(matrix: bool) -> Vec<u8> {
        let mut ifd0 = rgb_ifd();
        ifd0.set(t::MAKE, Value::Ascii("Hasselblad".into()));
        ifd0.set(t::MODEL, Value::Ascii("Hasselblad H4D".into()));
        if matrix {
            ifd0.set(t::COLOR_MATRIX_1, Value::SRational(MATRIX.to_vec()));
        }
        ifd0.set(t::AS_SHOT_NEUTRAL, Value::Rational(vec![(43, 100), (1, 1), (79, 100)]));
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
    }

    /// An FFF in miniature: big-endian, IFD0 an RGB image with `AsShotNeutral` and the embedded header `header`,
    /// a 16-bit uncompressed CFA sub-IFD of 8 x 6 samples, no colour matrix of its own unless `own_matrix`.
    fn fff(header: Option<Vec<u8>>, own_matrix: bool) -> Vec<u8> {
        fff_with(header, own_matrix, vec![])
    }

    /// [`fff`] with more `SubIFDs` children of IFD0 (other preview images).
    fn fff_with(header: Option<Vec<u8>>, own_matrix: bool, extra: Vec<IfdBuilder>) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![6]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        raw.set(t::BLACK_LEVEL, Value::Rational(vec![(256, 1)]));
        raw.set(t::WHITE_LEVEL, Value::Short(vec![65535]));
        raw.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![(0..96u8).collect()] });
        let mut ifd0 = rgb_ifd();
        ifd0.set(t::MAKE, Value::Ascii("Hasselblad".into()));
        ifd0.set(t::MODEL, Value::Ascii("Hasselblad H4D-40".into()));
        ifd0.set(t::AS_SHOT_NEUTRAL, Value::Rational(vec![(2, 5), (1, 1), (4, 5)]));
        if own_matrix {
            ifd0.set(t::COLOR_MATRIX_1, Value::SRational(vec![(1, 1), (0, 1), (0, 1), (0, 1), (1, 1), (0, 1), (0, 1), (0, 1), (1, 1)]));
        }
        if let Some(h) = header {
            ifd0.set(ORIGINAL_HEADER, Value::Undefined(h));
        }
        ifd0.add_sub_ifd(raw);
        for e in extra {
            ifd0.add_sub_ifd(e);
        }
        TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    fn matrix_row0(c: &ColorData) -> Option<[f64; 3]> {
        c.color_matrix[0].map(|m| m[0])
    }

    /// The FFF's own tags give the samples and white balance; the colour matrix comes from the 3FR header it
    /// embeds, so a developer has the camera's matrix and needs no fallback colour.
    #[test]
    fn fff_takes_the_colour_matrix_from_the_embedded_3fr_header() {
        let bytes = fff(Some(original_header(true)), false);
        assert_eq!(probe(&bytes), Some(RawFormat::CfaTiff));
        let r = decode_file(&bytes).unwrap();
        assert_eq!(r.data, RawData::U16((0..48u16).map(|i| u16::from_be_bytes([2 * i as u8, 2 * i as u8 + 1])).collect()));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "RGGB");
        assert_eq!(matrix_row0(&r.color), Some([0.686, -0.196, -0.032]));
        assert_eq!(r.color.as_shot_neutral, Some([0.4, 1.0, 0.8]), "the outer file's white balance");
        // the import probe says the same
        assert_eq!(matrix_row0(&probe_info(&bytes).unwrap().color), Some([0.686, -0.196, -0.032]));
    }

    /// A matrix of the file's own wins; without one anywhere (no header, or a header without a matrix) the
    /// file still decodes, with the fallback colour.
    #[test]
    fn own_matrix_wins_and_a_missing_one_is_not_an_error() {
        let own = decode_file(&fff(Some(original_header(true)), true)).unwrap();
        assert_eq!(matrix_row0(&own.color), Some([1.0, 0.0, 0.0]));
        for header in [None, Some(original_header(false)), Some(b"not a tiff".to_vec())] {
            let r = decode_file(&fff(header, false)).unwrap();
            assert!(!has_matrix(&r.color));
        }
    }

    /// A truncated or damaged file never panics: an error, or a decode.
    #[test]
    fn damaged_files_do_not_panic() {
        let bytes = fff(Some(original_header(true)), false);
        for n in (0..bytes.len()).step_by(7) {
            let _ = decode_file(&bytes[..n]);
            let _ = probe_info(&bytes[..n]);
            let _ = crate::embedded_preview(&bytes[..n]);
        }
        let mut header = original_header(true);
        for i in (8..header.len()).step_by(3) {
            header[i] ^= 0x5a;
            match decode_file(&fff(Some(header.clone()), false)) {
                Ok(_) | Err(RawError::Corrupt(_) | RawError::Tiff(_) | RawError::Unsupported(_) | RawError::Limit(_) | RawError::NotRaw) => {}
            }
        }
    }

    /// `(width, height, pixels, has an Orientation tag)` of a wrapped preview.
    fn wrapped(tiff: &[u8]) -> (u32, u32, Vec<u8>, bool) {
        let t = Tiff::parse(tiff).unwrap();
        let ifd = &t.ifds[0];
        let info = ifd.image().unwrap();
        let chunks = info.chunks(tiff.len() as u64);
        let pixels = chunks.iter().flat_map(|c| chunk_bytes(tiff, c).unwrap().to_vec()).collect();
        (info.width, info.height, pixels, ifd.contains(t::ORIENTATION))
    }

    /// No JPEG in the file: the 8-bit RGB IFD0 comes out as a baseline TIFF of the same pixels, without an
    /// orientation of its own (the raw file's applies).
    #[test]
    fn rgb_ifd0_is_the_embedded_preview() {
        let bytes = fff(Some(original_header(true)), false);
        let preview = crate::embedded_preview(&bytes).expect("a preview");
        assert_eq!(wrapped(&preview), (4, 3, (0..36).collect(), false));
        assert_eq!(crate::embedded_preview_covering(&bytes, 2000), Some(preview));
    }

    /// The largest 8-bit RGB image wins; a bigger 16-bit one (the FFF's copy with a `TransferFunction`) is not
    /// used, and nor is a planar or compressed one.
    #[test]
    fn only_8_bit_chunky_uncompressed_rgb_is_taken() {
        let image = |w: u32, h: u32, bits: u16, planar: u16, compression: u16| {
            let mut ifd = IfdBuilder::new();
            ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
            ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
            ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
            ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits; 3]));
            ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
            ifd.set(t::PHOTOMETRIC, Value::Short(vec![2]));
            ifd.set(t::COMPRESSION, Value::Short(vec![compression]));
            ifd.set(t::PLANAR_CONFIGURATION, Value::Short(vec![planar]));
            let n = (w * h * 3 * u32::from(bits / 8)) as usize;
            ifd.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![vec![7; n]] });
            ifd
        };
        let bytes = fff_with(None, false, vec![image(16, 12, 16, 1, 1), image(12, 9, 8, 2, 1), image(12, 9, 8, 1, 8)]);
        assert_eq!(wrapped(&crate::embedded_preview(&bytes).unwrap()).0, 4, "IFD0's 4 x 3");
        let bytes = fff_with(None, false, vec![image(6, 6, 8, 1, 1)]);
        assert_eq!(wrapped(&crate::embedded_preview(&bytes).unwrap()), (6, 6, vec![7; 108], false));
    }

    /// A preview strip cut short by a truncated file gives no preview, not a partial picture.
    #[test]
    fn short_preview_strip_gives_none() {
        let mut ifd0 = rgb_ifd();
        ifd0.set_image(ImageData::Strips { rows_per_strip: 3, strips: vec![vec![1; 30]] });
        let tiff = TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap();
        assert_eq!(rgb_preview(&Tiff::parse(&tiff).unwrap(), &tiff), None);
    }

    /// storytold/lightcraft#2561: a preview declaring more pixel bytes than the whole file gives none, refused before its buffer is
    /// reserved. Its one-row strips all point at the same 24 bytes, so the copy alone would fill it: a few strips
    /// (fewer pixel bytes than the file) still make a preview, a tall grid of them (more than the file) does not.
    #[test]
    fn preview_larger_than_the_file_gives_none() {
        let aliased = |rows: u32| {
            let mut ifd0 = rgb_ifd();
            ifd0.image = None;
            ifd0.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
            ifd0.set(t::IMAGE_LENGTH, Value::Long(vec![rows]));
            ifd0.set(t::ROWS_PER_STRIP, Value::Long(vec![1]));
            ifd0.set(t::STRIP_OFFSETS, Value::Long(vec![0; rows as usize]));
            ifd0.set(t::STRIP_BYTE_COUNTS, Value::Long(vec![24; rows as usize]));
            TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
        };
        let small = aliased(4);
        assert!(small.len() > 8 * 4 * 3);
        let preview = rgb_preview(&Tiff::parse(&small).unwrap(), &small).expect("a preview within the file's size");
        assert_eq!(wrapped(&preview).0, 8);
        let tall = aliased(2000);
        assert!(tall.len() < 8 * 2000 * 3);
        assert_eq!(rgb_preview(&Tiff::parse(&tall).unwrap(), &tall), None);
    }

    /// An oversized RGB IFD is passed over, not the end of the search: beside a 2000-row aliased one (more pixel bytes
    /// than the file), the file's complete 4 × 3 RGB image is still the preview.
    #[test]
    fn an_oversized_rgb_ifd_does_not_hide_a_smaller_complete_one() {
        let mut huge = rgb_ifd();
        huge.image = None;
        huge.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
        huge.set(t::IMAGE_LENGTH, Value::Long(vec![2000]));
        huge.set(t::ROWS_PER_STRIP, Value::Long(vec![1]));
        huge.set(t::STRIP_OFFSETS, Value::Long(vec![0; 2000]));
        huge.set(t::STRIP_BYTE_COUNTS, Value::Long(vec![24; 2000]));
        let mut ifd0 = rgb_ifd();
        ifd0.add_sub_ifd(huge);
        let bytes = TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap();
        assert!(bytes.len() < 8 * 2000 * 3);
        let preview = rgb_preview(&Tiff::parse(&bytes).unwrap(), &bytes).expect("the complete 4 x 3 preview");
        assert_eq!(wrapped(&preview), (4, 3, (0..36).collect(), false));
    }
}
