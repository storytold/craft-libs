//! Nikon NEF / NRW — uncompressed and Huffman-compressed (lossless, lossy) variants.
//!
//! Sources: TIFF 6.0 (the raw image is a standard CFA SubIFD), Laurent Clévy's NEF structure notes (prose: IFD
//! layout, SubIFDs, maker note header) and the ExifTool Nikon tag-name documentation (`0x000c` WB_RBLevels,
//! `0x003d` BlackLevel, `0x0096` NEFLinearizationTable). The Huffman-compressed data (compression 34713) is decoded
//! by [`crate::nefc`], which documents its clean-room sources; "lossy after split" files are decoded when their strip follows the
//! rule documented there; files it can't decode are reported as [`RawError::Unsupported`] and their embedded previews still work. So are High Efficiency NEFs
//! (HE / HE★, Z 8, Z 9, Z 6III, Z f): they keep compression 34713 but carry a wavelet codestream that starts with
//! the JPEG XS markers SOC + CAP (ISO/IEC 21122-1, `FF10 FF50`) and have no `0x0096` table (storytold/lightcraft#193).
//! Strips labelled 34713 without a `0x0096`
//! linearization table are packed uncompressed data in several measured layouts ([`PackedLayout`]) or, when the strip
//! is exactly three bytes per pixel, Nikon small raw ([`small_raw`]); the D1X's genuinely Huffman-coded strip stays
//! refused.

use crate::nefc;
use crate::shared::white_from_data;
use crate::tiffraw::{Packing, check_image, read_image};
use crate::{BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result};
use craft_tiff::image::ImageInfo;
use craft_tiff::tags::{self as t, photometric};
use craft_tiff::{ByteOrder, Ifd, Tiff, makernote};

const WB_RB_LEVELS: u16 = 0x000c;
const BLACK_LEVEL: u16 = 0x003d;
const LINEARIZATION_TABLE: u16 = 0x0096;
const CROP_AREA: u16 = 0x0045;
/// Maker note NEFCompression (ExifTool's Nikon tag documentation): `0x0093`, or in newer bodies the `u16` at
/// byte 10 of `0x0051`. 6 is packed 12-bit, 10 packed 14-bit, 13 and 14 High Efficiency and High Efficiency★.
const NEF_COMPRESSION: u16 = 0x0093;
const NEF_COMPRESSION_RECORD: u16 = 0x0051;

fn nef_compression(mn: Option<&makernote::MakerNote>) -> Option<u64> {
    let m = mn?;
    m.ifd.u64(NEF_COMPRESSION).or_else(|| {
        let v: [u8; 2] = m.ifd.bytes(NEF_COMPRESSION_RECORD)?.get(10..12)?.try_into().ok()?;
        Some(u64::from(m.order.u16(v)))
    })
}
/// JPEG XS start of codestream (SOC, `FF10`) followed by its mandatory capabilities marker (CAP, `FF50`).
const JPEG_XS_START: [u8; 4] = [0xff, 0x10, 0xff, 0x50];
/// Why a High Efficiency NEF shows its embedded preview: the UI shows the part before " (".
pub(crate) const HIGH_EFFICIENCY: &str =
    "Nikon High Efficiency NEF (HE / HE★, a JPEG XS-based codec with no public specification; shoot RAW Lossless compressed to edit the raw data)";
/// Maker note `0x0014`, the "NRW" data block of Coolpix raws: `"NRW "` + a 4-character version, then fields. In
/// version `0104` the `u32` at byte `0x20` is the black level in sample units (200 in the four 12-bit files here
/// that have 200 as their darkest samples, 0 in the one whose samples reach 0).
const NRW_DATA: u16 = 0x0014;
const NRW_BLACK_AT: usize = 0x20;

/// The NRW data block, when the maker note has one.
fn nrw_data(mn: Option<&makernote::MakerNote>) -> Option<(&[u8], ByteOrder)> {
    let m = mn?;
    m.ifd.bytes(NRW_DATA).filter(|b| b.starts_with(b"NRW ")).map(|b| (b, m.order))
}

/// Black level from the NRW data block, bounded to a quarter of the sample range so an unknown version's field
/// can't turn into a large offset.
fn nrw_black(mn: Option<&makernote::MakerNote>, bits: u32) -> Option<f32> {
    let (b, order) = nrw_data(mn)?;
    let v = order.u32(b.get(NRW_BLACK_AT..NRW_BLACK_AT + 4)?.try_into().ok()?);
    (u64::from(v) < (1u64 << bits.min(16)) / 4).then_some(v as f32)
}

/// Nikon CropArea is `[left, top, width, height]` in sensor pixels (maker-note tag documentation).
/// Keep the active area/CFA origin unchanged: the default crop is applied after demosaicing.
fn default_crop(mn: Option<&makernote::MakerNote>, w: usize, h: usize) -> Rect {
    let fallback = Rect::new(0, 0, w, h);
    let Some(values) = mn.and_then(|m| m.ifd.u64s(CROP_AREA)) else { return fallback };
    let [x, y, width, height] = values.as_slice() else { return fallback };
    if *width == 0 || *height == 0 || x.checked_add(*width).is_none_or(|v| v > w as u64) || y.checked_add(*height).is_none_or(|v| v > h as u64) {
        return fallback;
    }
    Rect::new(*x as usize, *y as usize, *width as usize, *height as usize)
}

/// A body that records no CropArea, measured on its own files: the image area inside the sensor data (where the
/// camera's full-size JPEG sits when the two are aligned) and, when the data's own top would be wrong, the level
/// where its channels clip.
struct MeasuredBody {
    model: &'static str,
    /// Sensor data size and sample depth of the measured files.
    size: (usize, usize),
    bits: u32,
    /// Image area in sensor pixels: x, y, width, height.
    crop: (usize, usize, usize, usize),
    white: Option<f32>,
}

/// * **NIKON D4** (148 uncompressed 14-bit NEFs from one body, 2023–2026, ISO 100–10000): the data is 4992 × 3292,
///   with 2 columns of optical black left of the image and 42 right of it, then 8 empty ones; the camera's
///   4928 × 3280 JPEG sits at (8, 6). Without this area the optical black shows as purple stripes on both sides.
///   Green saturates at 15546–15672 while red and blue run on to 15687–16383, so the data's own top (~16300) leaves
///   clipped highlights magenta; one level of 15520 for every channel turns them white and keeps the white balance.
/// * **NIKON D7100** (storytold/lightcraft#1248; 14-bit NEFs from the public zjqc/photographing-art repository, with their
///   Lightroom Classic exports): the data holds picture, not masked lines, beyond the camera's frame. In full frame
///   it is 6036 × 4020 and the camera's JPEG is 6000 × 4000; in the 1.3x crop mode it is 4812 × 3212 and the JPEG
///   4800 × 3200. Fitting renders of the full data to Lightroom's exports (feature matches, homography) puts
///   Lightroom's frame, the camera JPEG's size, at (18, 10) and (6, 6): within 0.5 px mean on DSC_1316, DSC_2763
///   and DSC_1486. Without it, renders are larger than Lightroom's and its crops land up to 6 px off (18 px in
///   full frame).
/// * **NIKON D90** (550 compressed 12-bit NEFs from one body, 10 measured): the data is 4352 × 2868 and its last 42
///   columns (from 4310) are filler at a flat ~254 whatever the exposure, above the dark image samples (black 0), so
///   the masked-column test keeps them and they develop as a pink band on the right. The camera's 4288 × 2848 JPEG
///   sits at (10, 10). Green saturates at 3840–3851 while red and blue run on to 4095; one level of 3830 keeps
///   clipped highlights white.
/// * **NIKON D7000** and **NIKON D5100** (and the Coolpix A, the same readout): the data is 4992 × 3280, the picture its
///   first 4948 columns, then 36 masked ones at one level (~600, with a black level of 0) and 8 of 0. The masked
///   columns sit above the dark-column trim's threshold, so without this area they show as a coloured bar on the right
///   edge (the top in portrait; storytold/lightcraft#976). The camera's 4928 × 3264 JPEG sits at (8, 8): in the D5100's 3 CC0
///   samples, and in the D7000's one clean one, with the D5100 and Coolpix A as siblings (see below).
/// * **The other bodies** below keep masked or dark lines inside the whole data (a survey of every Nikon raw on raw.pixls.us):
///   each crop is its camera JPEG's frame as `cargo xtask raw-frame` measures it on raw.pixls.us CC0 samples (the
///   D200, D3200 and Nikon 1 V1 also on other publicly posted sample files), with at least 3 clean
///   files agreeing: of the model (bit depths pooled: both read out the same area), or, with at least
///   one of its own, together with models of the same readout (data and JPEG size) that agree on the same frame
///   (*siblings*). The D4's 12-bit mode reads out the 14-bit area measured above; the Df, the same readout, agrees on
///   (8, 6) in 6 files. Bodies whose crop holds only picture beyond the camera frame have no entry (no change).
const MEASURED_BODIES: &[MeasuredBody] = &[
    MeasuredBody { model: "NIKON D4", size: (4992, 3292), bits: 14, crop: (8, 6, 4928, 3280), white: Some(15520.0) },
    // the 12-bit samples are the 14-bit ones shifted right by two, and so is the level: 15520 >> 2
    MeasuredBody { model: "NIKON D4", size: (4992, 3292), bits: 12, crop: (8, 6, 4928, 3280), white: Some(3880.0) },
    MeasuredBody { model: "NIKON D7100", size: (6036, 4020), bits: 14, crop: (18, 10, 6000, 4000), white: None },
    MeasuredBody { model: "NIKON D7100", size: (4812, 3212), bits: 14, crop: (6, 6, 4800, 3200), white: None },
    MeasuredBody { model: "NIKON D90", size: (4352, 2868), bits: 12, crop: (10, 10, 4288, 2848), white: Some(3830.0) },
    MeasuredBody { model: "NIKON D7000", size: (4992, 3280), bits: 14, crop: (8, 8, 4928, 3264), white: None },
    MeasuredBody { model: "NIKON D7000", size: (4992, 3280), bits: 12, crop: (8, 8, 4928, 3264), white: None },
    MeasuredBody { model: "NIKON D5100", size: (4992, 3280), bits: 14, crop: (8, 8, 4928, 3264), white: None },
    // siblings: D7000 (4), D5100 (3); masked: right 36
    MeasuredBody { model: "COOLPIX A", size: (4992, 3280), bits: 14, crop: (8, 8, 4928, 3264), white: None },
    // 6 files each; masked: left 2, right 2
    MeasuredBody { model: "NIKON D3", size: (4288, 2844), bits: 12, crop: (15, 6, 4256, 2832), white: None },
    MeasuredBody { model: "NIKON D3", size: (4288, 2844), bits: 14, crop: (15, 6, 4256, 2832), white: None },
    MeasuredBody { model: "NIKON D3S", size: (4288, 2844), bits: 12, crop: (15, 6, 4256, 2832), white: None },
    MeasuredBody { model: "NIKON D3S", size: (4288, 2844), bits: 14, crop: (15, 6, 4256, 2832), white: None },
    MeasuredBody { model: "NIKON D700", size: (4288, 2844), bits: 12, crop: (15, 6, 4256, 2832), white: None },
    MeasuredBody { model: "NIKON D700", size: (4288, 2844), bits: 14, crop: (15, 6, 4256, 2832), white: None },
    // 6 and 4 files; masked: right up to 32
    MeasuredBody { model: "NIKON D300", size: (4352, 2868), bits: 12, crop: (15, 10, 4288, 2848), white: None },
    MeasuredBody { model: "NIKON D300", size: (4352, 2868), bits: 14, crop: (15, 10, 4288, 2848), white: None },
    MeasuredBody { model: "NIKON D300S", size: (4352, 2868), bits: 12, crop: (15, 10, 4288, 2848), white: None },
    MeasuredBody { model: "NIKON D300S", size: (4352, 2868), bits: 14, crop: (15, 10, 4288, 2848), white: None },
    // D2X (1) and D2Xs (2) as siblings; dark: right 1
    MeasuredBody { model: "NIKON D2Xs", size: (4320, 2868), bits: 12, crop: (16, 10, 4288, 2848), white: None },
    // D3000, D40X, D60, D80: one file each, siblings; masked: right 4, bottom 3
    MeasuredBody { model: "NIKON D3000", size: (3904, 2616), bits: 12, crop: (14, 9, 3872, 2592), white: None },
    MeasuredBody { model: "NIKON D40X", size: (3904, 2616), bits: 12, crop: (14, 9, 3872, 2592), white: None },
    MeasuredBody { model: "NIKON D60", size: (3904, 2616), bits: 12, crop: (14, 9, 3872, 2592), white: None },
    MeasuredBody { model: "NIKON D80", size: (3904, 2616), bits: 12, crop: (14, 9, 3872, 2592), white: None },
    // 3 files; masked: left 1, right 3 (the D80's data size, framed elsewhere)
    MeasuredBody { model: "NIKON D200", size: (3904, 2616), bits: 12, crop: (16, 12, 3872, 2592), white: None },
    // 22 files; masked: right 46
    MeasuredBody { model: "NIKON D3200", size: (6080, 4012), bits: 12, crop: (8, 6, 6016, 4000), white: None },
    // 8 clean of 19, the rest ~0.3 px between pixels on the same frame; dark: bottom 2
    MeasuredBody { model: "NIKON 1 V1", size: (3904, 2606), bits: 12, crop: (12, 6, 3872, 2592), white: None },
    // Best guesses, short of the evidence rule; every frame below keeps the masked lines out by several pixels.
    // D3100: 3 of 4 clean files at (24, 6), the 4th at (22, 7); masked: left 6, right 22
    MeasuredBody { model: "NIKON D3100", size: (4672, 3084), bits: 12, crop: (24, 6, 4608, 3072), white: None },
    // D5000: 4 of 5 clean files at (11, 10), the 5th at (11, 12); the D90's sensor, whose own files also align at
    // x = 11; masked: right 42 (the same filler as the D90's)
    MeasuredBody { model: "NIKON D5000", size: (4352, 2868), bits: 12, crop: (11, 10, 4288, 2848), white: None },
    // D40: 2 of 2 clean files at (16, 9); masked: right 1. The D50 / D70 / D70s share its data size but not its frame
    MeasuredBody { model: "NIKON D40", size: (3040, 2014), bits: 12, crop: (16, 9, 3008, 2000), white: None },
    // D50, D70, D70s (one readout): one clean D50 and one clean D70 file at (15, 7); the D70s's 2 files align
    // between pixels next to it, at (15.4, 6.9) and (15.6, 6.4); masked: right 1
    MeasuredBody { model: "NIKON D50", size: (3040, 2014), bits: 12, crop: (15, 7, 3008, 2000), white: None },
    MeasuredBody { model: "NIKON D70", size: (3040, 2014), bits: 12, crop: (15, 7, 3008, 2000), white: None },
    MeasuredBody { model: "NIKON D70s", size: (3040, 2014), bits: 12, crop: (15, 7, 3008, 2000), white: None },
    // 3 files; dark: bottom 4
    MeasuredBody { model: "NIKON D4S", size: (4936, 3288), bits: 12, crop: (4, 4, 4928, 3280), white: None },
    MeasuredBody { model: "NIKON D4S", size: (4936, 3288), bits: 14, crop: (4, 4, 4928, 3280), white: None },
    // 4 files; dark: bottom up to 4
    MeasuredBody { model: "NIKON D5600", size: (6016, 4016), bits: 12, crop: (8, 8, 6000, 4000), white: None },
    MeasuredBody { model: "NIKON D5600", size: (6016, 4016), bits: 14, crop: (8, 8, 6000, 4000), white: None },
    // 4 files each; masked: right 8
    MeasuredBody { model: "NIKON D600", size: (6080, 4028), bits: 12, crop: (8, 6, 6016, 4016), white: None },
    MeasuredBody { model: "NIKON D600", size: (6080, 4028), bits: 14, crop: (8, 6, 6016, 4016), white: None },
    MeasuredBody { model: "NIKON D610", size: (6080, 4028), bits: 12, crop: (8, 6, 6016, 4016), white: None },
    MeasuredBody { model: "NIKON D610", size: (6080, 4028), bits: 14, crop: (8, 6, 6016, 4016), white: None },
    // D800: 6 files; D800E: 2, with the D800 as sibling; masked: right 14 (storytold/lightcraft#914, the top in portrait)
    MeasuredBody { model: "NIKON D800", size: (7424, 4924), bits: 12, crop: (8, 6, 7360, 4912), white: None },
    MeasuredBody { model: "NIKON D800", size: (7424, 4924), bits: 14, crop: (8, 6, 7360, 4912), white: None },
    MeasuredBody { model: "NIKON D800E", size: (7424, 4924), bits: 14, crop: (8, 6, 7360, 4912), white: None },
    // 3 files; dark: bottom 1
    MeasuredBody { model: "NIKON D810", size: (7380, 4928), bits: 12, crop: (12, 8, 7360, 4912), white: None },
    MeasuredBody { model: "NIKON D810", size: (7380, 4928), bits: 14, crop: (12, 8, 7360, 4912), white: None },
    // 6 files; masked: left 2, right 42 (the D4's readout)
    MeasuredBody { model: "NIKON Df", size: (4992, 3292), bits: 12, crop: (8, 6, 4928, 3280), white: None },
    MeasuredBody { model: "NIKON Df", size: (4992, 3292), bits: 14, crop: (8, 6, 4928, 3280), white: None },
];

fn measured_body(model: &str, w: usize, h: usize, bits: u32) -> Option<&'static MeasuredBody> {
    MEASURED_BODIES.iter().find(|b| b.model == model.trim() && b.size == (w, h) && b.bits == bits)
}

/// Pixel height over width from the raw IFD's resolution tags: the D1X and D1H write 600 x 300 per inch for a
/// 4028 x 1324 frame, rows twice as tall as the columns are wide. Only ratios from 1.5 to 4 count (a frame
/// squashed 3:1 or 2:1), the rest is rounding or a stray tag.
pub(crate) fn pixel_aspect_of(raw: &Ifd) -> f32 {
    match (raw.f64(t::X_RESOLUTION), raw.f64(t::Y_RESOLUTION)) {
        (Some(x), Some(y)) if x > 0.0 && y > 0.0 && (1.5..=4.0).contains(&(x / y)) => (x / y).round() as f32,
        _ => 1.0,
    }
}

fn raw_ifd(tiff: &Tiff) -> Option<&Ifd> {
    tiff.all_ifds()
        .into_iter()
        .filter(|i| i.u16(t::PHOTOMETRIC) == Some(photometric::CFA))
        .max_by_key(|i| i.u64(t::IMAGE_WIDTH).unwrap_or(0).saturating_mul(i.u64(t::IMAGE_LENGTH).unwrap_or(0)))
}

/// Rows the masked-column test reads, from the top of the frame. The header probe ([`Mode::Header`]) decodes
/// only these (a compressed strip is one sequential Huffman stream, so it can stop here; an uncompressed one reads
/// just its first strips), and the full decode judges the active width from the same rows, so
/// [`crate::probe_info`] equals [`RawImage::info`] of the decoded image (storytold/lightcraft#708). On every corpus NEF the
/// answer is the same for 8 to 512 rows as for the whole frame: the masked columns decode to 0 (compressed) or
/// ~19 (uncompressed) while live columns sit at the black level (~150 at 12 bit, ~600 at 14 bit) or above.
pub(crate) const MASK_ROWS: usize = 128;

/// Width without the optically masked columns some bodies append on the right (the D5100 and D7000 write 8, with
/// no maker-note tag that says so, in compressed and uncompressed files alike): trailing columns (at most 64) whose
/// mean over the first [`MASK_ROWS`] rows of `d` is below 1% of the nominal white level and a tenth of the image
/// interior. Kept even for CFA phase.
pub(crate) fn trailing_masked_columns(d: &[u16], w: usize, bits: u32) -> usize {
    let rows = d.len().checked_div(w).unwrap_or(0).min(MASK_ROWS);
    if w < 128 || rows == 0 {
        return w;
    }
    let col_mean = |x: usize| (0..rows).map(|y| d.get(y * w + x).copied().unwrap_or(0) as f64).sum::<f64>() / rows as f64;
    let interior = (w / 4..w * 3 / 4).step_by(w / 64).map(col_mean).sum::<f64>() / 32.0;
    let white = ((1u32 << bits.clamp(1, 16)) - 1) as f64;
    let dark = (white * 0.01).min(interior * 0.1);
    let mut aw = w;
    while aw > w - 64 && col_mean(aw - 1) < dark {
        aw -= 1;
    }
    aw & !1
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("TIFF without IFDs".into()))?;
    let raw = raw_ifd(&tiff).ok_or_else(|| RawError::Unsupported("NEF without a CFA image IFD".into()))?;
    let info = raw.image()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.bits() as u32;
    let make = ifd0.string(t::MAKE).unwrap_or_default();
    let mn =
        tiff.exif().and_then(|e| e.get(t::MAKER_NOTE)).and_then(|e| makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make));
    if let Some(strip) = small_raw_strip(&info, bytes.len(), mn.as_ref()) {
        return small_raw(bytes, &tiff, &info, strip, mode, limits);
    }
    // the header probe reads only the rows the masked-column test needs
    let rows = match mode {
        Mode::Full => h,
        Mode::Header => h.min(MASK_ROWS),
    };
    let data = if info.compression == t::compression::NIKON {
        compressed(bytes, &info, mn.as_ref(), rows, limits)?
    } else {
        uncompressed(bytes, &info, tiff.order, mn.as_ref(), rows, limits)?
    };
    let RawData::U16(ref samples) = data else { return Err(RawError::Unsupported("float NEF".into())) };

    // Maker note 0x003d is in 14-bit units whatever the sample depth: 12-bit files from the D750, D780, D850,
    // D7500 and Z 50 store 600 / 1008 / 400 / 400 / 1008 while their darkest samples are 150 / ~252 / 99 / 99 / ~250
    // (our measurements on CC0 raw.pixls.us samples; the same bodies' 14-bit files match the tag as is).
    let black_scale = if bits == 12 { 0.25 } else { 1.0 };
    let black = match mn.as_ref().and_then(|m| m.ifd.f64s(BLACK_LEVEL)).as_deref() {
        Some([a, b, c, d]) if [a, b, c, d].iter().all(|v| **v < 16384.0) => BlackLevel {
            repeat_rows: 2,
            repeat_cols: 2,
            values: [a, b, c, d].iter().map(|v| (**v * black_scale) as f32).collect(),
            ..Default::default()
        },
        _ => BlackLevel::uniform(nrw_black(mn.as_ref(), bits).unwrap_or_else(|| be16_black(&info, bytes.len() as u64, mn.as_ref(), samples))),
    };
    let wb = mn
        .as_ref()
        .and_then(|m| m.ifd.f64s(WB_RB_LEVELS))
        .filter(|v| v.len() >= 2 && v[0] > 0.1 && v[1] > 0.1 && v[0] < 10.0 && v[1] < 10.0)
        .map(|v| [v[0] as f32, 1.0, v[1] as f32]);
    let cfa = match (raw.u64s(t::CFA_REPEAT_PATTERN_DIM).as_deref(), raw.bytes(t::CFA_PATTERN_EP)) {
        (Some([2, 2]), Some(p)) if p.len() == 4 && p.iter().all(|&c| c <= 2) => Cfa { width: 2, height: 2, pattern: p.to_vec() },
        _ => Cfa::bayer_static("RGGB"),
    };
    let model = ifd0.string(t::MODEL).unwrap_or_default();
    let body = measured_body(&model, w, h, bits);
    let active_w = trailing_masked_columns(samples, w, bits);
    // the white level is measured from the whole image (or known for a measured body); the probe (no samples in its
    // result) takes the nominal one
    let white = match (body.and_then(|b| b.white), mode) {
        (Some(white), _) => white,
        (None, Mode::Full) => white_from_data(samples, bits),
        (None, Mode::Header) => ((1u32 << bits.clamp(1, 16)) - 1) as f32,
    };
    let crop = match body {
        Some(&MeasuredBody { crop: (x, y, cw, ch), .. }) if x + cw <= active_w && y + ch <= h => Rect::new(x, y, cw, ch),
        _ => default_crop(mn.as_ref(), active_w, h),
    };
    let pixel_aspect = pixel_aspect_of(raw);
    let img = RawImage {
        format: RawFormat::Nef,
        width: w,
        height: h,
        cpp: 1,
        data: match mode {
            Mode::Full => data,
            Mode::Header => RawData::U16(Vec::new()),
        },
        cfa: Some(cfa),
        bits,
        black,
        white: vec![white],
        active_area: Rect::new(0, 0, active_w, h),
        crop,
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: wb,
        linearized: false,
        pixel_aspect,
        // Nikon's in-file distortion correction, about the optical centre (which follows the sensor shift)
        opcodes: OpcodeLists {
            list3: crate::nef_lens::distortion(&model, raw, Rect::new(0, 0, active_w, h), crop).into_iter().collect(),
            ..Default::default()
        },
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// Uncompressed strips: 16-bit words or 12-bit packed, told apart by the strip size. Packed 12-bit data is an MSB-first
/// byte stream, except in files with an NRW data block (Coolpix), where it is MSB-first inside little-endian 32-bit
/// words (the rows are whole words). Reads the first `rows` rows (the strips that hold them) of the `info.height`.
fn uncompressed(
    bytes: &[u8],
    info: &ImageInfo,
    order: ByteOrder,
    mn: Option<&makernote::MakerNote>,
    rows: usize,
    limits: &Limits,
) -> Result<RawData> {
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.bits() as u32;
    let row_samples = w * info.samples_per_pixel as usize;
    let chunks = info.chunks(bytes.len() as u64);
    let rows_in_first = chunks.first().map(|c| c.height as usize).unwrap_or(h).max(1);
    let bytes_per_row = chunks.first().map(|c| c.len as usize / rows_in_first).unwrap_or(0);
    let packing = if bytes_per_row >= row_samples * 2 {
        Packing::Word16
    } else if bits == 12 && bytes_per_row * 8 >= row_samples * 12 && bytes_per_row * 8 < row_samples * 13 {
        if nrw_data(mn).is_some() && bytes_per_row.is_multiple_of(4) { Packing::Words32Msb } else { Packing::Msb }
    } else if info.compression == 1 && bits != 8 && bits != 16 {
        return Err(RawError::Unsupported(format!("NEF uncompressed packing ({bytes_per_row} bytes per {row_samples}-sample row)")));
    } else {
        Packing::Msb
    };
    if rows >= h {
        return read_image(bytes, info, order, packing, limits);
    }
    // the whole image's limits apply either way; then only the strips holding the first `rows` rows are unpacked
    check_image(bytes, info, limits)?;
    let top = ImageInfo { height: rows as u32, ..info.clone() };
    read_image(bytes, &top, order, packing, limits)
}

/// Packed layouts of strips that are labelled compression 34713 but have no linearization table (maker note
/// `0x0096`): they hold plain packed samples, not Huffman codes. The layouts were measured from CC0 raw.pixls.us
/// files by a clean-room black-box analysis (strip byte-count identity, same-colour smoothness and correlation with
/// the camera JPEG against at least eight alternative packings). Every layout is a single strip with
/// `stride = StripByteCounts / height`, selected only when `stride * height` equals the strip size exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackedLayout {
    /// LSB-first bit stream of `bits` per sample (12: 3 bytes per 2 samples, 14: 7 bytes per 4), rows byte aligned.
    /// 12-bit rows are `3w/2` bytes (older bodies) or padded to 16 bytes (Z 6, D6); 14-bit rows are `7w/4` bytes
    /// padded to 16 bytes.
    Lsb { stride: usize },
    /// D100: 16-byte chunks of fifteen data bytes holding ten MSB-first 12-bit samples, plus one zero pad byte. The
    /// rows carry more samples than the IFD width (3040 against 3034); the extra columns are not image.
    Chunked16 { stride: usize },
    /// Coolpix NRW: one big-endian 16-bit word per pixel, rows `2w` bytes with no header or padding; the 12-bit sample
    /// is the upper 12 bits of the word (the low nibble is zero, rarely 8).
    Be16,
}

fn packed_layout(w: usize, h: usize, bits: u32, strip: u64, stated: bool) -> Option<PackedLayout> {
    if w == 0 || h == 0 || !matches!(bits, 12 | 14) {
        return None;
    }
    let strip = usize::try_from(strip).ok()?;
    if !strip.is_multiple_of(h) {
        return None;
    }
    let stride = strip / h;
    let tight = w.checked_mul(bits as usize)?.div_ceil(8);
    let tight = if bits == 12 && w.is_multiple_of(2) || bits == 14 && w.is_multiple_of(4) { tight } else { return None };
    let padded = tight.checked_next_multiple_of(16)?;
    let lsb_ok = match bits {
        12 => stride == tight || stride == padded,
        _ => stride == padded || stated && stride == tight,
    };
    if lsb_ok {
        return Some(PackedLayout::Lsb { stride });
    }
    None
}

fn d100_layout(w: usize, h: usize, bits: u32, strip: u64) -> Option<PackedLayout> {
    let strip = usize::try_from(strip).ok()?;
    if bits != 12 || h == 0 || !strip.is_multiple_of(h) {
        return None;
    }
    let stride = strip / h;
    let per_row = stride / 16 * 10;
    (stride.is_multiple_of(16) && per_row >= w && per_row < w.checked_add(10)?).then_some(PackedLayout::Chunked16 { stride })
}

/// 12-bit strips of exactly `2 * w * h` bytes: big-endian 16-bit words (Coolpix P330, P7700, P7800 NRW, found by a
/// clean-room black-box analysis of the CC0 files). Tried after the other layouts, whose strides (`1.5w` to
/// `1.5w + 15`, or ten samples per 16 bytes) only reach `2w` for rows narrower than 40 samples, so it can't take a
/// strip they handle.
fn be16_layout(w: usize, h: usize, bits: u32, strip: u64) -> Option<PackedLayout> {
    let want = w.checked_mul(h)?.checked_mul(2)?;
    (bits == 12 && w > 0 && h > 0 && usize::try_from(strip).ok()? == want).then_some(PackedLayout::Be16)
}

/// Black level of the big-endian 16-bit layout, which carries neither `0x003d` nor an NRW block and has no masked
/// pixels (the borders of the frame are image). The darkest samples of the three measured bodies are 214 (P330),
/// 217 (P7700) and 201 (P7800, the only one with a dark enough frame: its per-channel 1st percentile is 209 / 225 /
/// 223), and the other 12-bit Coolpix NRWs state 200 in their NRW block; so 200, never above the darkest sample.
const BE16_BLACK: f32 = 200.0;

fn be16_black(info: &ImageInfo, file_len: u64, mn: Option<&makernote::MakerNote>, samples: &[u16]) -> f32 {
    let has_table = mn.is_some_and(|m| m.ifd.bytes(LINEARIZATION_TABLE).is_some());
    let single = matches!(info.chunks(file_len).as_slice(), [c] if be16_layout(info.width as usize, info.height as usize, info.bits() as u32, c.len).is_some());
    if info.compression != t::compression::NIKON || has_table || info.samples_per_pixel != 1 || !single {
        return 0.0;
    }
    let darkest = samples.iter().copied().min().unwrap_or(0);
    BE16_BLACK.min(f32::from(darkest))
}

/// Unpack a [`PackedLayout`] strip into `w * h` samples. Short input leaves the missing samples zero.
fn unpack_packed(src: &[u8], w: usize, h: usize, bits: u32, layout: PackedLayout) -> Vec<u16> {
    use rayon::prelude::*;
    let mut out = vec![0u16; w * h];
    let stride = match layout {
        PackedLayout::Lsb { stride } | PackedLayout::Chunked16 { stride } => stride,
        PackedLayout::Be16 => w.saturating_mul(2),
    };
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let Some(line) = src.get(y * stride..) else { return };
        let line = &line[..stride.min(line.len())];
        match layout {
            PackedLayout::Lsb { .. } => crate::unpack::unpack_lsb(line, bits, row),
            PackedLayout::Be16 => {
                let (words, _) = line.as_chunks::<2>();
                for (dst, word) in row.iter_mut().zip(words) {
                    *dst = u16::from_be_bytes(*word) >> 4;
                }
            }
            PackedLayout::Chunked16 { .. } => {
                let mut samples = vec![0u16; line.len() / 16 * 10];
                for (i, dst) in samples.chunks_mut(10).enumerate() {
                    if let Some(chunk) = line.get(i * 16..i * 16 + 15) {
                        crate::unpack::unpack_msb(chunk, 12, dst);
                    }
                }
                let n = row.len().min(samples.len());
                row[..n].copy_from_slice(&samples[..n]);
            }
        }
    });
    out
}

/// Nikon Huffman-compressed strip (see [`crate::nefc`]); the decode table is maker note `0x0096`. Without a table the
/// strip may be packed uncompressed data (see [`PackedLayout`]).
fn compressed(bytes: &[u8], info: &ImageInfo, mn: Option<&makernote::MakerNote>, rows: usize, limits: &Limits) -> Result<RawData> {
    let starts_with = |magic: &[u8]| {
        let start = info.chunks(bytes.len() as u64).first().and_then(|c| usize::try_from(c.offset).ok());
        start.and_then(|o| bytes.get(o..o.checked_add(magic.len())?)) == Some(magic)
    };
    // the maker note's own compression mode decides first: Z bodies label packed data 34713 and may still carry
    // a 0x0096 table
    let mode = nef_compression(mn);
    if starts_with(&JPEG_XS_START) || matches!(mode, Some(13 | 14)) {
        return Err(RawError::Unsupported(HIGH_EFFICIENCY.into()));
    }
    if let Some(depth) = match mode {
        Some(6) => Some(12),
        Some(10) => Some(14),
        _ => None,
    } {
        if info.bits() as u32 != depth {
            return Err(RawError::Unsupported(format!("Nikon packed {depth}-bit NEF with {}-bit samples", info.bits())));
        }
        return packed(bytes, info, rows, true, limits);
    }
    let Some(table) = mn.and_then(|m| Some((m.ifd.bytes(LINEARIZATION_TABLE)?, m.order))) else {
        return packed(bytes, info, rows, false, limits);
    };
    if info.samples_per_pixel != 1 {
        return Err(RawError::Unsupported(format!("Nikon compressed NEF with {} samples per pixel", info.samples_per_pixel)));
    }
    let bits = info.bits() as u32;
    if u64::from(info.width).saturating_mul(u64::from(info.height)) > limits.samples() as u64 {
        return Err(RawError::Limit("image too large"));
    }
    let table = nefc::parse_table(table.0, table.1, bits)?;
    // one strip in every sample; several would be contiguous parts of the same bit stream
    let chunks = info.chunks(bytes.len() as u64);
    let (Some(first), Some(last)) = (chunks.first(), chunks.last()) else { return Err(RawError::Corrupt("NEF: no image data".into())) };
    let end = last.offset.saturating_add(last.len).min(bytes.len() as u64);
    let src = usize::try_from(first.offset).ok().zip(usize::try_from(end).ok()).and_then(|(a, b)| bytes.get(a..b));
    let src = src.ok_or_else(|| RawError::Corrupt("NEF: image data outside the file".into()))?;
    Ok(RawData::U16(nefc::decode_rows(src, info.width as usize, info.height as usize, rows, bits, &table)?))
}

/// A "compressed" strip of packed samples: the strip size must match a measured layout exactly. `stated` is set
/// when the maker note names the packed mode, which also allows 14-bit rows without padding.
fn packed(bytes: &[u8], info: &ImageInfo, rows: usize, stated: bool, limits: &Limits) -> Result<RawData> {
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.bits() as u32;
    let chunks = info.chunks(bytes.len() as u64);
    let layout = match chunks.as_slice() {
        [c] if info.samples_per_pixel == 1 => {
            packed_layout(w, h, bits, c.len, stated).or_else(|| d100_layout(w, h, bits, c.len)).or_else(|| be16_layout(w, h, bits, c.len))
        }
        _ => None,
    };
    let (Some(layout), [c]) = (layout, chunks.as_slice()) else {
        return Err(RawError::Unsupported("Nikon compressed NEF without a linearization table (maker note 0x96)".into()));
    };
    let src = craft_tiff::image::chunk_bytes(bytes, c)
        .filter(|src| u64::try_from(src.len()).ok() == Some(c.len))
        .ok_or_else(|| RawError::Corrupt("NEF: incomplete image strip".into()))?;
    if (w as u64).saturating_mul(h as u64) > limits.samples() as u64 {
        return Err(RawError::Limit("image too large"));
    }
    Ok(RawData::U16(unpack_packed(src, w, rows.min(h), bits, layout)))
}

/// Nikon small raw (sRAW), measured clean-room from the CC0 raw.pixls.us D810 (2036) and D4S (2227) samples. The raw SubIFD is labelled compression 34713,
/// 12 bits, one CFA sample per pixel, but it carries no linearization table (maker note `0x0096`) and its single strip
/// is exactly `3 * width * height` bytes. It holds a YCbCr 4:2:2 image of the IFD size, not a CFA mosaic (the CFA
/// tags are dummies):
///
/// - the strip is an LSB-first 12-bit stream (`s0 = b0 | (b1 & 0x0f) << 8`, `s1 = b1 >> 4 | b2 << 4`), a row is `3 * width`
///   bytes with no padding, i.e. `2 * width` samples;
/// - the samples come in groups of four, `[Y_left, Y_right, Cb, Cr]`, for the pixel pair `(2k, 2k + 1)`: chroma is
///   halved horizontally and not vertically;
/// - luma is linear and already black-subtracted (the darkest samples are near 0) and never uses the top two bits
///   (maxima 873 and 1004 of 4095), so the white level is 1023; chroma is centred on 2048 (fitted neutral 2045-2049).
///
/// The chroma is a camera colour-difference scale, not JFIF Cb/Cr. In linear terms the stored values behave as
/// `R - Y = R_GAIN * (Cr - 2048)` and `B - Y = B_GAIN * (Cb - 2048)` on the luma scale: regressing the camera JPEG's
/// linear `(R - Y) / Y` and `(B - Y) / Y` on `(Cr - 2048) / Y` and `(Cb - 2048) / Y` over mid-tone pixels gave slopes
/// 4.92 / 5.09 (D810, correlation 0.94 / 0.88) and 3.52 / 3.73 (D4S, 0.98 / 0.97); the gains below are the rounded
/// means. They are approximate (the two bodies differ by about 30 %), the luma is not tone-mapped, and the data is
/// treated as already white balanced (neutral D4S areas have `Cb`, `Cr` within 2 of 2048 and equal JPEG R, G, B). G
/// follows from the luma equation with the BT.601 weights. The chroma is upsampled horizontally with linear
/// interpolation, centred between the two pixels of a pair. Output is linear RGB scaled by 16 (to a 14-bit range),
/// like Sony's downsized ARWs.
const SMALL_RAW_WHITE: f32 = 1023.0;
const SMALL_RAW_SCALE: f32 = 16.0;
const SMALL_RAW_CHROMA_ZERO: f32 = 2048.0;
const SMALL_RAW_R_GAIN: f32 = 4.2;
const SMALL_RAW_B_GAIN: f32 = 4.4;

/// The strip of a small raw: compression 34713, 12 bits, no maker note `0x0096`, one strip of exactly
/// `3 * width * height` bytes (even width).
fn small_raw_strip(info: &ImageInfo, file_len: usize, mn: Option<&makernote::MakerNote>) -> Option<craft_tiff::image::Chunk> {
    if info.compression != t::compression::NIKON || info.bits() != 12 || info.samples_per_pixel != 1 {
        return None;
    }
    if mn.is_some_and(|m| m.ifd.bytes(LINEARIZATION_TABLE).is_some()) {
        return None;
    }
    let (w, h) = (info.width as u64, info.height as u64);
    let chunks = info.chunks(file_len as u64);
    match chunks.as_slice() {
        [c] if w.is_multiple_of(2) && w > 0 && h > 0 && w.checked_mul(h).and_then(|n| n.checked_mul(3)) == Some(c.len) => Some(*c),
        _ => None,
    }
}

/// Decode one row of a small raw (`3 * width` bytes) into `3 * width` linear RGB values on the 14-bit scale.
/// Short input reads as zero bytes.
fn small_raw_row(src: &[u8], w: usize, out: &mut [u16]) {
    let Some(n) = w.checked_mul(3).filter(|n| *n <= out.len()) else { return };
    if w == 0 || !w.is_multiple_of(2) {
        return;
    }
    let mut s = vec![0u16; 2 * w];
    crate::unpack::unpack_lsb(&src[..src.len().min(n)], 12, &mut s);
    let pairs = w / 2;
    let chroma = |k: usize| {
        let g = &s[4 * k + 2..4 * k + 4];
        (f32::from(g[0]) - SMALL_RAW_CHROMA_ZERO, f32::from(g[1]) - SMALL_RAW_CHROMA_ZERO)
    };
    for k in 0..pairs {
        let (cb, cr) = chroma(k);
        for side in 0..2 {
            let (nb, nr) = if side == 0 { chroma(k.saturating_sub(1)) } else { chroma((k + 1).min(pairs - 1)) };
            let (u, v) = (0.75 * cb + 0.25 * nb, 0.75 * cr + 0.25 * nr);
            let y = f32::from(s[4 * k + side]);
            let r = y + SMALL_RAW_R_GAIN * v;
            let b = y + SMALL_RAW_B_GAIN * u;
            let g = (y - 0.299 * r - 0.114 * b) / 0.587;
            let dst = (2 * k + side) * 3;
            for (i, value) in [r, g, b].into_iter().enumerate() {
                out[dst + i] = (value * SMALL_RAW_SCALE).round().clamp(0.0, 65535.0) as u16;
            }
        }
    }
}

fn small_raw(bytes: &[u8], tiff: &Tiff, info: &ImageInfo, strip: craft_tiff::image::Chunk, mode: Mode, limits: &Limits) -> Result<RawImage> {
    use rayon::prelude::*;
    let (w, h) = (info.width as usize, info.height as usize);
    let stride = w.checked_mul(3).ok_or(RawError::Limit("image too large"))?;
    let count = stride.checked_mul(h).filter(|n| *n > 0 && *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let src = craft_tiff::image::chunk_bytes(bytes, &strip)
        .filter(|src| u64::try_from(src.len()).ok() == Some(strip.len))
        .ok_or_else(|| RawError::Corrupt("NEF: incomplete image strip".into()))?;
    let data = match mode {
        Mode::Full => {
            let mut data = vec![0u16; count];
            data.par_chunks_mut(stride).enumerate().for_each(|(y, out)| {
                small_raw_row(src.get(y * stride..).unwrap_or(&[]), w, out);
            });
            data
        }
        Mode::Header => Vec::new(),
    };
    let img = RawImage {
        format: RawFormat::Nef,
        width: w,
        height: h,
        cpp: 3,
        data: RawData::U16(data),
        cfa: None,
        bits: 14,
        black: BlackLevel::uniform(0.0),
        white: vec![SMALL_RAW_WHITE * SMALL_RAW_SCALE],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::from_exif(tiff.ifds.first().and_then(|i| i.u16(t::ORIENTATION)).unwrap_or(1)),
        color: ColorData::default(),
        // the stored colour is already white balanced (see above)
        wb_multipliers: Some([1.0; 3]),
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

    fn nef(compression: u16, bits: u16, strips: Vec<Vec<u8>>, w: u32, h: u32, rps: u32) -> Vec<u8> {
        nef_with_note(compression, bits, strips, w, h, rps, None)
    }

    /// [`nef`] with an optional maker note (`Nikon\0` v2 header + embedded big-endian TIFF holding `note`).
    fn nef_with_note(compression: u16, bits: u16, strips: Vec<Vec<u8>>, w: u32, h: u32, rps: u32, note: Option<IfdBuilder>) -> Vec<u8> {
        nef_in(ByteOrder::Big, compression, bits, strips, w, h, rps, note)
    }

    /// [`nef_with_note`] in either byte order (file and maker note).
    #[allow(clippy::too_many_arguments)]
    fn nef_in(order: ByteOrder, compression: u16, bits: u16, strips: Vec<Vec<u8>>, w: u32, h: u32, rps: u32, note: Option<IfdBuilder>) -> Vec<u8> {
        nef_model_in(order, "NIKON TEST", compression, bits, strips, w, h, rps, note)
    }

    /// [`nef_in`] from camera `model`.
    #[allow(clippy::too_many_arguments)]
    fn nef_model_in(
        order: ByteOrder,
        model: &str,
        compression: u16,
        bits: u16,
        strips: Vec<Vec<u8>>,
        w: u32,
        h: u32,
        rps: u32,
        note: Option<IfdBuilder>,
    ) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits]));
        raw.set(t::COMPRESSION, Value::Short(vec![compression]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![photometric::CFA]));
        raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![2, 1, 1, 0]));
        raw.set_image(ImageData::Strips { rows_per_strip: rps, strips });
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("NIKON CORPORATION".into()));
        ifd0.set(t::MODEL, Value::Ascii(model.into()));
        if let Some(mn) = note {
            let mut bytes = b"Nikon\0\x02\x10\0\0".to_vec();
            bytes.extend(TiffWriter::new(order, false).write(&[mn]).unwrap());
            let mut exif = IfdBuilder::new();
            exif.set(t::MAKER_NOTE, Value::Undefined(bytes));
            ifd0.set_child(t::EXIF_IFD, exif);
        }
        ifd0.add_sub_ifd(raw);
        TiffWriter::new(order, false).write(&[ifd0]).unwrap()
    }

    /// A Z 6II NEF carrying NikonNEFInfo gets Nikon's distortion correction as one warp, in the header probe and
    /// the full decode alike, and keeps it through DNG export; another body with the same data gets none.
    #[test]
    fn nikon_in_file_distortion_becomes_a_warp() {
        use crate::nef_lens::tests::{distortion_block, nef_info_blob, offset_block};
        let (w, h) = (48u32, 32u32);
        let file = |model: &str| {
            let words: Vec<u8> = (0..w * h).flat_map(|i| (600 + (i % 97) as u16 * 50).to_be_bytes()).collect();
            let mut raw = IfdBuilder::new();
            raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
            raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
            raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
            raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![14]));
            raw.set(t::COMPRESSION, Value::Short(vec![1]));
            raw.set(t::PHOTOMETRIC, Value::Short(vec![photometric::CFA]));
            raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
            raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
            let info = nef_info_blob(&[
                (0x0005, distortion_block(3, 21.6, 4, [0.039895, 0.019211, -0.127218], -0.025043)),
                (0x0008, offset_block(1.5, -0.75)),
            ]);
            raw.set(0xc7d5, Value::Undefined(info));
            raw.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![words] });
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii("NIKON CORPORATION".into()));
            ifd0.set(t::MODEL, Value::Ascii(model.into()));
            ifd0.add_sub_ifd(raw);
            TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
        };
        let bytes = file("NIKON Z 6_2");
        let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        let full = decode(&bytes, Mode::Full, &Limits::default()).unwrap();
        assert_eq!(header.info(), full.info());
        assert_eq!(header.opcodes.list3, full.opcodes.list3);
        let [crate::Opcode::WarpRectilinear { planes, center }] = full.opcodes.list3.as_slice() else { panic!("{:?}", full.opcodes.list3) };
        assert!((planes[0][0] - 1.0 / (1.0 - 0.025043)).abs() < 1e-5, "{planes:?}");
        assert!((center[0] * 47.0 - (23.5 + 1.5)).abs() < 0.01 && (center[1] * 31.0 - (15.5 - 0.75)).abs() < 0.01, "{center:?}");
        #[cfg(feature = "dng")]
        {
            let dng = crate::write_dng(&full, &Default::default()).unwrap();
            assert_eq!(crate::decode(&dng).unwrap().opcodes.list3, full.opcodes.list3);
        }
        assert!(decode(&file("NIKON D850"), Mode::Full, &Limits::default()).unwrap().opcodes.list3.is_empty());
    }

    #[test]
    fn preview_color_space_is_read_from_the_nikon_note() {
        for (value, expected) in [(1, Some(crate::PreviewColorSpace::Srgb)), (2, Some(crate::PreviewColorSpace::AdobeRgb)), (3, None)] {
            let mut mn = IfdBuilder::new();
            mn.set(0x001e, Value::Short(vec![value]));
            let bytes = nef_with_note(1, 14, vec![vec![0; 128]], 8, 8, 8, Some(mn));
            assert_eq!(crate::embedded_preview_color_space(&bytes), expected);
        }
        assert_eq!(crate::embedded_preview_color_space(&nef(1, 14, vec![vec![0; 128]], 8, 8, 8)), None);
        assert_eq!(crate::embedded_preview_color_space(b"not a TIFF"), None);
    }

    /// storytold/lightcraft#778: any make's raw falls back to its EXIF ColorSpace, with Uncalibrated + InteropIndex "R03" read as
    /// Adobe RGB; the Nikon note wins; a DNG is left alone.
    #[test]
    fn preview_color_space_falls_back_to_the_exif_color_space() {
        let file = |make: &str, cs: u16, interop: Option<&str>, note: Option<u16>, dng: bool| {
            let mut exif = IfdBuilder::new();
            exif.set(0xA001, Value::Short(vec![cs]));
            if let Some(i) = interop {
                let mut io = IfdBuilder::new();
                io.set(0x0001, Value::Ascii(i.into()));
                exif.set_child(t::INTEROP_IFD, io);
            }
            if let Some(v) = note {
                let mut mn = IfdBuilder::new();
                mn.set(0x001e, Value::Short(vec![v]));
                let mut bytes = b"Nikon   ".to_vec();
                bytes.extend(TiffWriter::new(ByteOrder::Little, false).write(&[mn]).unwrap());
                exif.set(t::MAKER_NOTE, Value::Undefined(bytes));
            }
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii(make.into()));
            if dng {
                ifd0.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
            }
            ifd0.set_child(t::EXIF_IFD, exif);
            TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
        };
        use crate::PreviewColorSpace::{AdobeRgb, Srgb};
        let f = crate::embedded_preview_color_space;
        assert_eq!(f(&file("SONY", 0xFFFF, Some("R03"), None, false)), Some(AdobeRgb));
        assert_eq!(f(&file("SONY", 1, Some("R98"), None, false)), Some(Srgb));
        assert_eq!(f(&file("SONY", 0xFFFF, Some("R98"), None, false)), None);
        assert_eq!(f(&file("SONY", 0xFFFF, None, None, false)), None);
        assert_eq!(f(&file("Canon", 0xFFFF, Some("R03"), None, false)), Some(AdobeRgb));
        assert_eq!(f(&file("NIKON CORPORATION", 1, Some("R98"), Some(2), false)), Some(AdobeRgb));
        assert_eq!(f(&file("NIKON CORPORATION", 0xFFFF, Some("R03"), None, false)), Some(AdobeRgb));
        assert_eq!(f(&file("Canon", 0xFFFF, Some("R03"), None, true)), None);
    }

    #[test]
    fn nikon_crop_area_preserves_sensor_origin_and_rejects_invalid_rectangles() {
        let (w, h) = (32, 24);
        let words: Vec<u8> = (0..w * h).flat_map(|i| (100 + i as u16).to_be_bytes()).collect();
        for (values, expected) in [
            (vec![2, 4, 28, 16], Rect::new(2, 4, 28, 16)),
            (vec![31, 4, 28, 16], Rect::new(0, 0, 32, 24)),
            (vec![2, 4, 0, 16], Rect::new(0, 0, 32, 24)),
            (vec![u32::MAX, 4, 28, 16], Rect::new(0, 0, 32, 24)),
            (vec![2, 4, 28], Rect::new(0, 0, 32, 24)),
        ] {
            let mut mn = IfdBuilder::new();
            mn.set(CROP_AREA, Value::Long(values));
            let bytes = nef_with_note(1, 14, vec![words.clone()], w, h, h, Some(mn));
            let raw = crate::decode(&bytes).unwrap();
            assert_eq!(raw.crop, expected);
            assert_eq!(raw.active_area, Rect::new(0, 0, 32, 24));
            assert_eq!((raw.crop.width, raw.crop.height), (expected.width, expected.height));
            assert_eq!(raw.info().developed_size(), (expected.width, expected.height));
        }
    }

    /// The D4 records no CropArea: its measured image area and saturation level apply to files of its data size and
    /// depth only.
    #[test]
    fn measured_d4_image_area_and_white_level() {
        let (w, h) = (4992usize, 3292usize);
        let decode_as = |model: &str, bits: u16| {
            let mut nef = nef(1, bits, vec![vec![0x10; w * h * 2]], w as u32, h as u32, h as u32);
            let at = nef.windows(10).position(|b| b == b"NIKON TEST").unwrap();
            let name = format!("{model:<10}");
            nef[at..at + 10].copy_from_slice(name.as_bytes());
            crate::decode(&nef).unwrap()
        };
        let d4 = decode_as("NIKON D4", 14);
        assert_eq!(d4.crop, Rect::new(8, 6, 4928, 3280));
        assert_eq!(d4.white, vec![15520.0]);
        assert_eq!((d4.crop.width, d4.crop.height), (4928, 3280));
        assert_eq!(d4.info().developed_size(), (4928, 3280));
        // the 12-bit mode reads out the same area, and its level is the 14-bit one shifted like its samples
        let d4_12 = decode_as("NIKON D4", 12);
        assert_eq!(d4_12.crop, Rect::new(8, 6, 4928, 3280));
        assert_eq!(d4_12.white, vec![3880.0]);
        let other = decode_as("NIKON D5", 14);
        assert_eq!(other.crop, Rect::new(0, 0, w, h));
        assert_ne!(other.white, vec![15520.0]);
    }

    /// storytold/lightcraft#976: the D7000 and D5100 record no CropArea, and their 36 masked columns at ~600 were developed as a
    /// coloured bar. Their measured area applies to the measured models, data sizes and depths only.
    #[test]
    fn measured_d7000_and_d5100_image_area() {
        let (w, h) = (4992usize, 3280usize);
        let decode_as = |model: &str, bits: u16| {
            let nef = nef_model_in(ByteOrder::Big, model, 1, bits, vec![vec![0x10; w * h * 2]], w as u32, h as u32, h as u32, None);
            crate::decode(&nef).unwrap()
        };
        for (model, bits) in [("NIKON D7000", 14), ("NIKON D7000", 12), ("NIKON D5100", 14)] {
            let raw = decode_as(model, bits);
            assert_eq!(raw.crop, Rect::new(8, 8, 4928, 3264), "{model} {bits}-bit");
            assert_eq!((raw.crop.width, raw.crop.height), (4928, 3264));
            assert_eq!(raw.info().developed_size(), (4928, 3264));
        }
        // another model, or the D5100's unmeasured 12-bit mode, keeps the whole data
        for (model, bits) in [("NIKON D7100", 14), ("NIKON D5100", 12), ("NIKON D7000 ", 16)] {
            assert_eq!(decode_as(model, bits).crop, Rect::new(0, 0, w, h), "{model} {bits}-bit");
        }
    }

    /// The D200, D3200 and Nikon 1 V1 keep masked or dark lines in their whole data (the D200 shares the D80's data
    /// size but frames it elsewhere), measured on public sample files beyond the CC0 ones; the D3100, D5000, D40, D50,
    /// D70 and D70s get best-guess frames (the D40 shares the D50 / D70 / D70s data size but not their frame).
    #[test]
    fn measured_older_dx_bodies_image_area() {
        let decode_as = |model: &str, (w, h): (usize, usize)| {
            let nef = nef_model_in(ByteOrder::Big, model, 1, 12, vec![vec![0x10; w * h * 2]], w as u32, h as u32, h as u32, None);
            crate::decode(&nef).unwrap()
        };
        for (model, size, crop) in [
            ("NIKON D200", (3904, 2616), Rect::new(16, 12, 3872, 2592)),
            ("NIKON D80", (3904, 2616), Rect::new(14, 9, 3872, 2592)),
            ("NIKON D3200", (6080, 4012), Rect::new(8, 6, 6016, 4000)),
            ("NIKON 1 V1", (3904, 2606), Rect::new(12, 6, 3872, 2592)),
            ("NIKON D3100", (4672, 3084), Rect::new(24, 6, 4608, 3072)),
            ("NIKON D5000", (4352, 2868), Rect::new(11, 10, 4288, 2848)),
            ("NIKON D40", (3040, 2014), Rect::new(16, 9, 3008, 2000)),
            ("NIKON D50", (3040, 2014), Rect::new(15, 7, 3008, 2000)),
            ("NIKON D70", (3040, 2014), Rect::new(15, 7, 3008, 2000)),
            ("NIKON D70s", (3040, 2014), Rect::new(15, 7, 3008, 2000)),
        ] {
            assert_eq!(decode_as(model, size).crop, crop, "{model}");
        }
        // the V1's area on the V2's data size: no entry, the whole data
        assert_eq!(decode_as("NIKON 1 V1", (4620, 3084)).crop, Rect::new(0, 0, 4620, 3084));
    }

    /// Every measured entry is a crop inside its data, once per model, data size and depth, with a white level (if
    /// any) inside its bit range.
    #[test]
    fn measured_bodies_are_consistent() {
        let mut seen = std::collections::HashSet::new();
        for b in MEASURED_BODIES {
            let (x, y, w, h) = b.crop;
            assert!(w > 0 && h > 0 && x + w <= b.size.0 && y + h <= b.size.1, "{}: crop outside its data", b.model);
            assert!(seen.insert((b.model, b.size, b.bits)), "{} {:?} {}-bit listed twice", b.model, b.size, b.bits);
            assert!(matches!(b.bits, 12 | 14), "{}: {}-bit", b.model, b.bits);
            assert!(b.white.is_none_or(|v| v > 0.0 && v < (1u32 << b.bits) as f32), "{}: white level", b.model);
            assert_eq!(b.model.trim(), b.model, "{}: model names are matched trimmed", b.model);
        }
    }

    /// storytold/lightcraft#1248: the D7100 records no CropArea, and its data holds picture beyond Lightroom's frame in both
    /// image areas (full frame and 1.3x crop), so renders came out larger and Lightroom crops landed off.
    #[test]
    fn measured_d7100_image_area() {
        let decode_as = |model: &str, (w, h): (usize, usize)| {
            let mut nef = nef(1, 14, vec![vec![0x10; w * h * 2]], w as u32, h as u32, h as u32);
            // the fixture's model is "NIKON TEST" plus its NUL: 11 bytes to overwrite
            let at = nef.windows(11).position(|b| b == b"NIKON TEST\0").unwrap();
            let name = format!("{model:\0<11}");
            nef[at..at + 11].copy_from_slice(name.as_bytes());
            crate::decode(&nef).unwrap()
        };
        for (size, crop) in [((6036, 4020), Rect::new(18, 10, 6000, 4000)), ((4812, 3212), Rect::new(6, 6, 4800, 3200))] {
            let d7100 = decode_as("NIKON D7100", size);
            assert_eq!(d7100.crop, crop);
            assert_eq!((d7100.crop.width, d7100.crop.height), (crop.width, crop.height));
            assert_ne!(d7100.white, vec![15520.0]);
            assert_eq!(d7100.info().developed_size(), (crop.width, crop.height));
            assert_eq!(decode_as("NIKON D7200", size).crop, Rect::new(0, 0, size.0, size.1));
        }
    }

    /// The D90's last 42 columns are filler at a flat ~254, above its dark image samples: without the measured
    /// area they stay in the image and develop as a pink band on the right.
    #[test]
    fn measured_d90_drops_the_filler_columns() {
        let (w, h) = (4352usize, 2868usize);
        let mut px = vec![40u16; w * h];
        for row in px.chunks_mut(w) {
            row[4310..].fill(254);
        }
        let mut nef = nef(1, 12, vec![px.iter().flat_map(|v| v.to_le_bytes()).collect()], w as u32, h as u32, h as u32);
        let at = nef.windows(10).position(|b| b == b"NIKON TEST").unwrap();
        nef[at..at + 10].copy_from_slice(b"NIKON D90 ");
        let d90 = crate::decode(&nef).unwrap();
        assert_eq!(d90.crop, Rect::new(10, 10, 4288, 2848));
        assert!(d90.crop.x + d90.crop.width <= 4310, "the filler columns are outside the image");
        assert_eq!(d90.white, vec![3830.0]);
        assert_eq!((d90.crop.width, d90.crop.height), (4288, 2848));
    }

    /// storytold/lightcraft#708: the active width is judged from the first [`MASK_ROWS`] rows in both modes, so the header probe
    /// (which reads only those rows' strips) and the full decode agree whether the masked columns are there or not.
    #[test]
    fn masked_columns_are_judged_from_the_top_rows_in_both_modes() {
        let (w, h, rps) = (256usize, MASK_ROWS + 64, 8u32);
        let file = |masked_rows: std::ops::Range<usize>| {
            let words: Vec<u8> = (0..w * h)
                .map(|i| if masked_rows.contains(&(i / w)) && i % w >= w - 8 { 0u16 } else { 1000 + (i % 7) as u16 })
                .flat_map(|v| v.to_be_bytes())
                .collect();
            nef(1, 14, words.chunks(w * 2 * rps as usize).map(|c| c.to_vec()).collect(), w as u32, h as u32, rps)
        };
        for (masked_rows, expected_width) in [(0..h, w - 8), (0..MASK_ROWS, w - 8), (MASK_ROWS..h, w), (0..0, w)] {
            let bytes = file(masked_rows.clone());
            let full = crate::decode(&bytes).unwrap();
            assert_eq!(full.active_area.width, expected_width, "masked rows {masked_rows:?}");
            assert_eq!(full.crop.width, expected_width);
            assert_eq!(crate::probe_info(&bytes).unwrap(), full.info(), "masked rows {masked_rows:?}");
        }
        // the shared rule on its own: nominal white, trailing columns at most 64, even width
        let d: Vec<u16> = (0..w * 4).map(|i| if i % w >= w - 9 { 0 } else { 600 }).collect();
        assert_eq!(trailing_masked_columns(&d, w, 14), w - 10, "nine dark columns trim to an even width");
        assert_eq!(trailing_masked_columns(&d, w, 12), w - 10);
        assert_eq!(trailing_masked_columns(&d[..w - 1], w, 14), w, "less than a row of samples leaves the width alone");
        assert_eq!(trailing_masked_columns(&[0; 64], 64, 14), 64, "narrow images are left alone");
        assert_eq!(trailing_masked_columns(&[], 0, 14), 0);
    }

    #[test]
    fn black_level_tag_is_in_14_bit_units() {
        let (w, h) = (16usize, 4usize);
        let words: Vec<u8> = (0..w * h).flat_map(|i| (100 + i as u16).to_be_bytes()).collect();
        let note = || {
            let mut mn = IfdBuilder::new();
            mn.set(BLACK_LEVEL, Value::Short(vec![400, 404, 408, 412]));
            mn
        };
        let black = |bits| crate::decode(&nef_with_note(1, bits, vec![words.clone()], w as u32, h as u32, h as u32, Some(note()))).unwrap().black;
        assert_eq!(black(12).values, vec![100.0, 101.0, 102.0, 103.0]);
        assert_eq!(black(14).values, vec![400.0, 404.0, 408.0, 412.0]);
    }

    #[test]
    fn uncompressed_word16_and_packed12() {
        let (w, h) = (16usize, 6usize);
        let px: Vec<u16> = (0..w * h).map(|i| (i * 131 % 4096) as u16).collect();
        let words: Vec<u8> = px.iter().flat_map(|v| v.to_be_bytes()).collect();
        let bytes = nef(1, 12, words.chunks(w * 2 * 3).map(|c| c.to_vec()).collect(), w as u32, h as u32, 3);
        assert_eq!(crate::probe(&bytes), Some(RawFormat::Nef));
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(r.data, RawData::U16(px.clone()));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "BGGR");
        // 12-bit MSB packed
        let mut packed = Vec::new();
        for pair in px.chunks(2) {
            let (a, b) = (pair[0] as u32, pair[1] as u32);
            let v = (a << 12) | b;
            packed.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        }
        let bytes = nef(1, 12, vec![packed], w as u32, h as u32, h as u32);
        assert_eq!(crate::decode(&bytes).unwrap().data, RawData::U16(px));
    }

    /// An NRW data block (`"NRW 0104"`, fields from byte 8) with `black` at byte `0x20`, as a maker note.
    fn nrw_note(black: u32) -> IfdBuilder {
        let mut block = b"NRW 0104".to_vec();
        block.resize(0x20, 0);
        block.extend_from_slice(&black.to_le_bytes());
        block.resize(0x40, 0);
        let mut mn = IfdBuilder::new();
        mn.set(NRW_DATA, Value::Undefined(block));
        mn
    }

    /// 12-bit samples as an MSB-first bit stream inside 32-bit words in `order` (rows are whole words).
    fn words32(px: &[u16], w: usize, order: ByteOrder) -> Vec<u8> {
        let mut out = Vec::new();
        for row in px.chunks(w) {
            let stream = row.iter().fold(Vec::new(), |mut bits: Vec<bool>, v| {
                bits.extend((0..12).rev().map(|b| v >> b & 1 == 1));
                bits
            });
            for word in stream.chunks(32) {
                let v = word.iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (31 - i));
                out.extend_from_slice(&match order {
                    ByteOrder::Little => v.to_le_bytes(),
                    ByteOrder::Big => v.to_be_bytes(),
                });
            }
        }
        out
    }

    #[test]
    fn nrw_block_selects_32_bit_word_packing_and_supplies_the_black_level() {
        let (w, h) = (16usize, 6usize);
        let px: Vec<u16> = (0..w * h).map(|i| (200 + i * 131 % 3800) as u16).collect();
        let packed = words32(&px, w, ByteOrder::Little);
        assert_eq!(packed.len(), w * h * 3 / 2);
        let decode = |note: Option<IfdBuilder>| {
            crate::decode(&nef_in(ByteOrder::Little, 1, 12, vec![packed.clone()], w as u32, h as u32, h as u32, note)).unwrap()
        };
        let r = decode(Some(nrw_note(200)));
        assert_eq!(r.data, RawData::U16(px.clone()));
        assert_eq!(r.black.values, vec![200.0]);
        assert_eq!(decode(Some(nrw_note(0))).black.values, vec![0.0]);
        // an implausible field is not a black level
        assert_eq!(decode(Some(nrw_note(60000))).black.values, vec![0.0]);
        // without the block the same bytes are an MSB-first byte stream
        let plain = decode(None);
        assert_ne!(plain.data, RawData::U16(px));
        assert_eq!(plain.black.values, vec![0.0]);
        // the maker-note black level (14-bit units) wins when present
        let mut both = nrw_note(200);
        both.set(BLACK_LEVEL, Value::Short(vec![400, 404, 408, 412]));
        assert_eq!(decode(Some(both)).black.values, vec![100.0, 101.0, 102.0, 103.0]);
    }

    #[test]
    fn compressed_without_table_is_unsupported() {
        let bytes = nef(34713, 14, vec![vec![0; 64]], 8, 8, 8);
        let Err(RawError::Unsupported(why)) = crate::decode(&bytes) else { panic!("expected unsupported") };
        assert!(why.contains("linearization table"), "{why}");
    }

    /// storytold/lightcraft#193: a High Efficiency NEF (Z f, Z 8: compression 34713, no `0x0096` table, a JPEG XS codestream) is
    /// named as such instead of being blamed on the missing table, and the header probe reports the same reason.
    #[test]
    fn high_efficiency_nef_is_named() {
        let mut strip = vec![0xff, 0x10, 0xff, 0x50, 0x00, 0x22];
        strip.resize(64, 0);
        let bytes = nef(34713, 14, vec![strip], 8, 8, 8);
        for result in [crate::decode(&bytes).map(|_| ()), crate::probe_info(&bytes).map(|_| ())] {
            let Err(RawError::Unsupported(why)) = result else { panic!("expected unsupported") };
            assert_eq!(why, HIGH_EFFICIENCY);
            assert!(why.starts_with("Nikon High Efficiency NEF ("), "{why}");
        }
        // a strip too short to hold the markers is still the missing-table case, not a panic
        let short = nef(34713, 14, vec![vec![0xff, 0x10]], 1, 1, 1);
        assert!(matches!(crate::decode(&short), Err(RawError::Unsupported(w)) if w.contains("linearization table")));
    }

    fn samples(n: usize, bits: u32) -> Vec<u16> {
        (0..n).map(|i| (((i * 2654435761usize) >> 7) as u32 % (1 << bits)) as u16).collect()
    }

    fn pack_lsb(row: &[u16], bits: u32) -> Vec<u8> {
        let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
        for &v in row {
            acc |= (v as u64) << n;
            n += bits;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    fn pack_msb12(row: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        for pair in row.chunks(2) {
            let (a, b) = (pair[0], pair.get(1).copied().unwrap_or(0));
            out.extend([(a >> 4) as u8, ((a & 15) << 4) as u8 | (b >> 8) as u8, b as u8]);
        }
        out
    }

    /// Rows of `samples` packed by `row`, each padded with `pad` bytes of 0xA5 (never decoded).
    fn strip(data: &[u16], w: usize, pad: usize, row: impl Fn(&[u16]) -> Vec<u8>) -> Vec<u8> {
        data.chunks(w).flat_map(|r| row(r).into_iter().chain(std::iter::repeat_n(0xA5, pad))).collect()
    }

    fn decoded(strip: Vec<u8>, bits: u16, w: u32, h: u32) -> Result<Vec<u16>> {
        let bytes = nef(34713, bits, vec![strip], w, h, h);
        match crate::decode(&bytes)?.data {
            RawData::U16(d) => Ok(d),
            _ => Err(RawError::Unsupported("float".into())),
        }
    }

    #[test]
    fn packed_34713_layouts_round_trip() {
        let (w, h) = (40usize, 6usize);
        let d12 = samples(w * h, 12);
        let d14 = samples(w * h, 14);
        // group A: 12-bit, 3w/2 bytes per row
        assert_eq!(decoded(strip(&d12, w, 0, |r| pack_lsb(r, 12)), 12, 40, 6).unwrap(), d12);
        // group B: 12-bit, rows padded to 16 bytes (60 -> 64)
        assert_eq!(decoded(strip(&d12, w, 4, |r| pack_lsb(r, 12)), 12, 40, 6).unwrap(), d12);
        // group C: 14-bit, 70 bytes -> 80
        assert_eq!(decoded(strip(&d14, w, 10, |r| pack_lsb(r, 14)), 14, 40, 6).unwrap(), d14);
        // group E: 16-byte chunks, 15 data bytes (ten samples) + a zero byte; 16 chunks = 160 samples (at 40 wide the
        // 64-byte stride would be group B's)
        let (w, h) = (160usize, 3usize);
        let d12 = samples(w * h, 12);
        let e = strip(&d12, w, 0, |r| r.chunks(10).flat_map(|c| pack_msb12(c).into_iter().chain([0u8])).collect());
        assert_eq!(e.len(), 256 * h);
        assert_eq!(decoded(e, 12, 160, 3).unwrap(), d12);
    }

    /// Z 6II "Packed 14 bits" (maker note 0x93 = 10, storytold/lightcraft#769): 6064x4040, 14-bit, one strip of 4040 rows of 10624
    /// bytes (6064 * 14 / 8 = 10612, padded to a multiple of 16). Four rows keep the real stride.
    #[test]
    fn packed_14_bit_z6ii_geometry_with_row_padding() {
        let (w, h) = (6064usize, 4usize);
        let d14 = samples(w * h, 14);
        let s = strip(&d14, w, 12, |r| pack_lsb(r, 14));
        assert_eq!(s.len(), 10624 * h);
        assert_eq!(decoded(s, 14, w as u32, h as u32).unwrap(), d14);
        assert_eq!(packed_layout(6064, 4040, 14, 42_920_960, false), Some(PackedLayout::Lsb { stride: 10624 }));
    }

    /// The maker note's compression mode routes the strip: 6 and 10 are packed samples even when a `0x0096`
    /// table is present (which the Huffman reader would otherwise take), in the old tag and in the `0x0051`
    /// record, in either byte order; a depth that contradicts the mode, and High Efficiency (13, 14), are refused.
    #[test]
    fn maker_note_compression_mode_routes_packed_and_high_efficiency() {
        let (w, h) = (40usize, 6usize);
        let note = |order: ByteOrder, modern: bool, mode: u16| {
            let mut n = IfdBuilder::new();
            n.set(LINEARIZATION_TABLE, Value::Undefined(vec![0x44, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
            if modern {
                let mut record = b"01000400\0\0".to_vec();
                record.extend(if order == ByteOrder::Big { mode.to_be_bytes() } else { mode.to_le_bytes() });
                n.set(NEF_COMPRESSION_RECORD, Value::Undefined(record));
            } else {
                n.set(NEF_COMPRESSION, Value::Short(vec![mode]));
            }
            n
        };
        let file = |order, modern, mode, bits: u16, image: Vec<u8>| {
            nef_in(order, 34713, bits, vec![image], w as u32, h as u32, h as u32, Some(note(order, modern, mode)))
        };
        for (bits, mode) in [(12u32, 6u16), (14, 10)] {
            let data = samples(w * h, bits);
            // rows padded to 16 bytes, and tight rows (14-bit tight rows only when the mode is stated)
            for pad in [(16 - (w * bits as usize / 8) % 16) % 16, 0] {
                let image = strip(&data, w, pad, |r| pack_lsb(r, bits));
                for order in [ByteOrder::Big, ByteOrder::Little] {
                    for modern in [false, true] {
                        let RawData::U16(got) = crate::decode(&file(order, modern, mode, bits as u16, image.clone())).unwrap().data else {
                            panic!("integer samples")
                        };
                        assert_eq!(got, data, "{bits}-bit, pad {pad}, {order:?}, modern {modern}");
                    }
                }
            }
        }
        let d14 = samples(w * h, 14);
        let image = strip(&d14, w, 10, |r| pack_lsb(r, 14));
        // tight 14-bit rows without a stated mode stay refused
        assert!(decoded(strip(&d14, w, 0, |r| pack_lsb(r, 14)), 14, w as u32, h as u32).is_err());
        // the mode says 12-bit packed, the IFD 14 bits
        assert!(matches!(crate::decode(&file(ByteOrder::Big, false, 6, 14, image.clone())), Err(RawError::Unsupported(_))));
        for mode in [13, 14] {
            for modern in [false, true] {
                let err = crate::decode(&file(ByteOrder::Big, modern, mode, 14, image.clone())).unwrap_err();
                assert!(matches!(&err, RawError::Unsupported(m) if m == HIGH_EFFICIENCY), "{err:?}");
            }
        }
    }

    #[test]
    fn packed_layout_refuses_overflowing_dimensions() {
        for bits in [12, 14] {
            assert!(packed_layout(usize::MAX - 3, 1, bits, u64::MAX, false).is_none());
        }
        assert!(d100_layout(usize::MAX, 1, 12, u64::MAX).is_none());
    }

    /// Header probes inspect only the same top rows that the masked-column test uses. Routing
    /// still validates the complete strip's layout, so header and full decode keep the same crop.
    #[test]
    fn packed_header_probe_reads_only_the_mask_rows() {
        let (w, h) = (160usize, MASK_ROWS + 7);
        let data = samples(w * h, 12);
        for image in [strip(&data, w, 0, |r| pack_lsb(r, 12)), be16(&data)] {
            let bytes = nef(34713, 12, vec![image], w as u32, h as u32, h as u32);
            let tiff = Tiff::parse(&bytes).unwrap();
            let info = raw_ifd(&tiff).unwrap().image().unwrap();
            let RawData::U16(top) = packed(&bytes, &info, MASK_ROWS, false, &Limits::default()).unwrap() else { panic!("integer samples") };
            assert_eq!(top, data[..w * MASK_ROWS]);
            let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
            let full = decode(&bytes, Mode::Full, &Limits::default()).unwrap();
            assert_eq!((header.active_area, header.crop), (full.active_area, full.crop));
            assert!(matches!(header.data, RawData::U16(ref samples) if samples.is_empty()));
        }
    }

    #[test]
    fn packed_d100_rows_carry_extra_columns() {
        // IFD width 154, rows hold 160 samples (16 chunks); the last 6 are dropped
        let (w, wide, h) = (154usize, 160usize, 3usize);
        let wide_data = samples(wide * h, 12);
        let e = strip(&wide_data, wide, 0, |r| r.chunks(10).flat_map(|c| pack_msb12(c).into_iter().chain([0u8])).collect());
        let got = decoded(e, 12, w as u32, h as u32).unwrap();
        let want: Vec<u16> = wide_data.chunks(wide).flat_map(|r| r[..w].iter().copied()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn packed_34713_wrong_sizes_stay_refused() {
        let d12 = samples(40 * 6, 12);
        let good = strip(&d12, 40, 0, |r| pack_lsb(r, 12));
        // one byte more or fewer, a stride that is neither tight nor 16-aligned, a 13-bit file
        for len in [good.len() - 1, good.len() + 1, good.len() + 6] {
            let mut s = good.clone();
            s.resize(len, 0);
            assert!(matches!(decoded(s, 12, 40, 6), Err(RawError::Unsupported(_))), "len {len}");
        }
        assert!(matches!(decoded(good.clone(), 13, 40, 6), Err(RawError::Unsupported(_))));
        // 14-bit rows must be padded to 16 bytes
        let d14 = samples(40 * 6, 14);
        assert!(matches!(decoded(strip(&d14, 40, 0, |r| pack_lsb(r, 14)), 14, 40, 6), Err(RawError::Unsupported(_))));
    }

    /// A small raw strip of `[Y0 Y1 Cb Cr]` groups, rows of `3 * w` bytes, LSB-first 12-bit.
    fn small_strip(groups: &[[u16; 4]], w: usize) -> Vec<u8> {
        let flat: Vec<u16> = groups.iter().flatten().copied().collect();
        assert_eq!(flat.len() % (2 * w), 0);
        flat.chunks(2 * w).flat_map(|r| pack_lsb(r, 12)).collect()
    }

    fn small(strip: Vec<u8>, bits: u16, w: u32, h: u32) -> Result<RawImage> {
        crate::decode(&nef(34713, bits, vec![strip], w, h, h))
    }

    #[test]
    fn small_raw_422_round_trips() {
        let (w, h) = (8usize, 3usize);
        // flat chroma per row so the interpolation is exact; distinct luma everywhere
        let mut groups = Vec::new();
        for y in 0..h {
            for k in 0..w / 2 {
                let (cb, cr) = (2048 + 10 * y as u16, 2048 - 5 * y as u16);
                groups.push([100 + (y * w + 2 * k) as u16, 100 + (y * w + 2 * k + 1) as u16, cb, cr]);
            }
        }
        let raw = small(small_strip(&groups, w), 12, w as u32, h as u32).unwrap();
        assert_eq!((raw.width, raw.height, raw.cpp, raw.cfa.is_none()), (w, h, 3, true));
        assert_eq!(raw.wb_multipliers, Some([1.0; 3]));
        let RawData::U16(d) = &raw.data else { panic!() };
        for y in 0..h {
            for x in 0..w {
                let luma = (100 + y * w + x) as f32;
                let (u, v) = (10.0 * y as f32, -5.0 * y as f32);
                let (r, b) = (luma + SMALL_RAW_R_GAIN * v, luma + SMALL_RAW_B_GAIN * u);
                let g = (luma - 0.299 * r - 0.114 * b) / 0.587;
                for (i, want) in [r, g, b].into_iter().enumerate() {
                    let got = f32::from(d[(y * w + x) * 3 + i]) / SMALL_RAW_SCALE;
                    assert!((got - want).abs() < 0.1, "({x},{y}) channel {i}: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn small_raw_neutral_chroma_is_grey_and_chroma_is_interpolated() {
        let w = 8usize;
        let groups: Vec<[u16; 4]> = (0..w / 2).map(|k| [200 + 40 * k as u16, 200 + 40 * k as u16, 2048, 2048]).collect();
        let raw = small(small_strip(&groups, w), 12, w as u32, 1).unwrap();
        let RawData::U16(d) = &raw.data else { panic!() };
        for x in 0..w {
            let px = &d[x * 3..x * 3 + 3];
            assert!(px.iter().all(|&v| v == px[0]), "{px:?}");
            assert_eq!(f32::from(px[0]), (200 + 40 * (x / 2)) as f32 * SMALL_RAW_SCALE);
        }
        // a step in Cb between the first two pairs is shared 3:1 by the pixels next to the boundary
        let mut groups = groups;
        groups[1][2] = 2048 + 100;
        let raw = small(small_strip(&groups, w), 12, w as u32, 1).unwrap();
        let RawData::U16(d) = &raw.data else { panic!() };
        let blue_minus_luma = |x: usize, luma: f32| f32::from(d[x * 3 + 2]) / SMALL_RAW_SCALE - luma;
        assert!((blue_minus_luma(1, 200.0) - SMALL_RAW_B_GAIN * 25.0).abs() < 0.1);
        assert!((blue_minus_luma(2, 240.0) - SMALL_RAW_B_GAIN * 75.0).abs() < 0.1);
    }

    #[test]
    fn small_raw_wrong_sizes_and_depths_are_refused() {
        let (w, h) = (8usize, 3usize);
        let good = small_strip(&vec![[100, 100, 2048, 2048]; w / 2 * h], w);
        assert!(small(good.clone(), 12, w as u32, h as u32).is_ok());
        for len in [good.len() - 1, good.len() + 1] {
            let mut s = good.clone();
            s.resize(len, 0);
            assert!(matches!(small(s, 12, w as u32, h as u32), Err(RawError::Unsupported(_))), "len {len}");
        }
        assert!(matches!(small(good.clone(), 14, w as u32, h as u32), Err(RawError::Unsupported(_))));
        // odd width: 3 bytes per pixel does not split into pixel pairs
        assert!(matches!(small(vec![0; 3 * 7 * 3], 12, 7, 3), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn small_raw_short_input_does_not_panic() {
        for len in [0, 1, 2, 3, 5, 11, 12, 13] {
            let mut out = vec![0u16; 12];
            small_raw_row(&vec![0xFF; len], 4, &mut out);
        }
        let mut bytes = nef(34713, 12, vec![vec![0x55; 3 * 8 * 3]], 8, 3, 3);
        bytes.truncate(bytes.len() - 40);
        assert!(crate::decode(&bytes).is_err(), "a truncated small-raw strip is corrupt");
    }

    #[test]
    fn small_raw_header_has_full_geometry_without_allocating_pixels() {
        let (w, h) = (8usize, 3usize);
        let bytes = nef(34713, 12, vec![small_strip(&vec![[100, 110, 2048, 2048]; w / 2 * h], w)], w as u32, h as u32, h as u32);
        let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        let full = decode(&bytes, Mode::Full, &Limits::default()).unwrap();
        assert_eq!(header.info(), full.info());
        assert!(matches!(header.data, RawData::U16(ref samples) if samples.is_empty()));
        assert!(matches!(full.data, RawData::U16(ref samples) if samples.len() == 3 * w * h));
    }

    #[test]
    fn small_raw_rejects_excessive_dimensions_before_allocation() {
        let bytes = nef(34713, 12, vec![vec![0; 72]], 8, 3, 3);
        let tiff = Tiff::parse(&bytes).unwrap();
        let mut info = raw_ifd(&tiff).unwrap().image().unwrap();
        let strip = info.chunks(bytes.len() as u64)[0];
        info.width = u32::MAX - 1;
        info.height = u32::MAX;
        assert!(matches!(small_raw(&bytes, &tiff, &info, strip, Mode::Header, &Limits::default()), Err(RawError::Limit(_))));
        let mut out = [0; 12];
        small_raw_row(&[], usize::MAX, &mut out);
        small_raw_row(&[], 3, &mut out);
        assert_eq!(out, [0; 12]);
    }

    /// Big-endian 16-bit words holding the 12-bit samples in their upper bits.
    fn be16(data: &[u16]) -> Vec<u8> {
        data.iter().flat_map(|v| (v << 4).to_be_bytes()).collect()
    }

    #[test]
    fn coolpix_be16_words_decode_and_wrong_sizes_are_refused() {
        let (w, h) = (40usize, 6usize);
        let d12 = samples(w * h, 12);
        let good = be16(&d12);
        assert_eq!(good.len(), 2 * w * h);
        assert_eq!(decoded(good.clone(), 12, 40, 6).unwrap(), d12);
        // a half-LSB in the low nibble does not change the sample
        let mut half = good.clone();
        for b in half.iter_mut().skip(1).step_by(2) {
            *b |= 8;
        }
        assert_eq!(decoded(half, 12, 40, 6).unwrap(), d12);
        // any other strip size, or another bit depth, is not this layout
        for len in [good.len() - 2, good.len() - 1, good.len() + 1, good.len() + 2, good.len() + 16] {
            let mut s = good.clone();
            s.resize(len, 0);
            assert!(matches!(decoded(s, 12, 40, 6), Err(RawError::Unsupported(_))), "len {len}");
        }
        assert!(matches!(decoded(good.clone(), 13, 40, 6), Err(RawError::Unsupported(_))));
        // the black level is 200, but never above the darkest sample
        let black = |data: &[u16]| crate::decode(&nef(34713, 12, vec![be16(data)], 40, 6, 6)).unwrap().black.values[0];
        let mut d = d12.clone();
        for v in &mut d {
            *v = (*v).max(300);
        }
        assert_eq!(black(&d), 200.0);
        d[5] = 150;
        assert_eq!(black(&d), 150.0);
    }

    #[test]
    fn coolpix_be16_does_not_take_other_layouts() {
        // every width where another group's stride equals 2w must still go to that group (the narrow rows where they
        // coincide are below the real-world widths; the other groups are tried first)
        for w in (2..=400usize).step_by(2) {
            let strip = 2 * w as u64 * 6;
            let other = packed_layout(w, 6, 12, strip, false).or_else(|| d100_layout(w, 6, 12, strip));
            if other.is_some() {
                assert_eq!(
                    packed_layout(w, 6, 12, strip, false).or_else(|| d100_layout(w, 6, 12, strip)).or_else(|| be16_layout(w, 6, 12, strip)),
                    other
                );
            }
        }
        assert_eq!(be16_layout(4032, 3024, 12, 24385536), Some(PackedLayout::Be16));
        assert!(packed_layout(4032, 3024, 12, 24385536, false).is_none() && d100_layout(4032, 3024, 12, 24385536).is_none());
        assert!(be16_layout(4032, 3024, 14, 24385536).is_none());
        assert!(be16_layout(usize::MAX, usize::MAX, 12, u64::MAX).is_none());
    }

    #[test]
    fn packed_34713_short_input_does_not_panic() {
        let (w, h) = (40usize, 6usize);
        for layout in [PackedLayout::Lsb { stride: 64 }, PackedLayout::Lsb { stride: 80 }, PackedLayout::Chunked16 { stride: 64 }, PackedLayout::Be16]
        {
            for len in [0, 1, 15, 16, 17, 63, 64, 65, 200] {
                let bits = if matches!(layout, PackedLayout::Lsb { stride: 80 }) { 14 } else { 12 };
                assert_eq!(unpack_packed(&vec![0xFF; len], w, h, bits, layout).len(), w * h);
            }
        }
        // a strip that claims more bytes than the file has
        let mut bytes = nef(34713, 12, vec![vec![0x55; 60 * 6]], 40, 6, 6);
        bytes.truncate(bytes.len() - 100);
        assert!(crate::decode(&bytes).is_err(), "a truncated packed strip is corrupt");
    }
    /// Keep every directory intact before a relocated tail strip, then remove its last rows.
    fn truncated_tail_strip(mut bytes: Vec<u8>, keep: usize) -> Vec<u8> {
        let tiff = Tiff::parse(&bytes).unwrap();
        let raw = raw_ifd(&tiff).unwrap();
        let info = raw.image().unwrap();
        let chunk = info.chunks(bytes.len() as u64)[0];
        let data = craft_tiff::image::chunk_bytes(&bytes, &chunk).unwrap().to_vec();
        let offset_entry = raw.get(t::STRIP_OFFSETS).unwrap().offset as usize;
        let start = bytes.len() as u32;
        bytes[offset_entry..offset_entry + 4].copy_from_slice(&start.to_be_bytes());
        bytes.extend_from_slice(&data[..keep]);
        bytes
    }

    #[test]
    fn packed_rejects_tail_truncation_with_intact_directories() {
        let bytes = nef(34713, 12, vec![vec![0x55; 360]], 40, 6, 6);
        for keep in [0, 60, 359] {
            let short = truncated_tail_strip(bytes.clone(), keep);
            for mode in [Mode::Header, Mode::Full] {
                assert!(matches!(decode(&short, mode, &Limits::default()), Err(RawError::Corrupt(_))));
            }
        }
    }

    #[test]
    fn small_raw_rejects_tail_truncation_with_intact_directories() {
        let bytes = nef(34713, 12, vec![vec![0x55; 72]], 8, 3, 3);
        for keep in [0, 24, 71] {
            let short = truncated_tail_strip(bytes.clone(), keep);
            for mode in [Mode::Header, Mode::Full] {
                assert!(matches!(decode(&short, mode, &Limits::default()), Err(RawError::Corrupt(_))));
            }
        }
    }
}
