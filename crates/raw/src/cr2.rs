//! Canon CR2.
//!
//! Source: Laurent Clévy, "Understanding what is stored in a Canon RAW .CR2 file" (prose sections on the file
//! structure, IFD#3, the `0xc640` slice layout and the maker-note `SensorInfo` table), plus the ExifTool Canon
//! tag-name documentation for maker-note tag meanings. Only the structural description was used.
//!
//! - The raw data is IFD#3's single strip: a lossless-JPEG frame (usually 2 or 4 components).
//! - The decoded sample stream fills vertical slices left to right: `cr2_slice = [n, w, last]` means `n` slices of
//!   width `w` then one of width `last`; each slice is filled top to bottom, row by row.
//! - Colour filter layout: IFD#3 tag `0xc5e0` (`CR2CFAPattern` in the ExifTool EXIF tag-name docs): 1 = RGGB,
//!   2 = BGGR, 3 = GBRG, 4 = GRBG, anchored at the top-left of the full decoded sensor (masked borders included).
//!   It differs by model (e.g. 3 on the 50D/60D/7D/550D/5D Mark II, 1 on the 40D/5D Mark III/6D/5DS R), so it is
//!   read per file (no model table); files without the tag fall back to RGGB (storytold/lightcraft#85). The parity of the
//!   `SensorInfo` borders does *not* predict it (RGGB files come with both even and odd top borders).
//! - `SensorInfo` (maker note `0x00e0`): sensor width/height and the left/top/right/bottom borders of the image
//!   area. Light already reaches some columns left of that border (EOS 6D: masked columns 0–70, border 84; 10–36
//!   such columns on every corpus body), so the masked columns that give the black level are measured from the
//!   data ([`masked_columns`]) rather than taken as the whole border.
//! - `ColorBalance` (maker note `0x4001`): as-shot `RGGB` levels at a model-dependent offset; we probe the known
//!   offsets and accept the first plausible quadruple. The array is 16-bit words; some models store it as UNDEFINED
//!   bytes (ColorData versions -3 and -4), which are paired up in the maker note's byte order first.
//! - The same array lists the levels of the camera's white-balance presets, each with the colour temperature Canon
//!   gives it ([`kelvin_presets_from_color_data`]): a Kelvin scale for a body without colour matrices.
//! - sRAW / mRAW (raw IFD tag `0xc6c5` = 4) hold subsampled YCbCr instead of a mosaic: see [`sraw`].

mod sraw;

use crate::shared::{active_rows, black_from_columns_sourced, white_from_data_sourced};
use crate::{
    BlackLevel, Camera, Cfa, ColorData, Limits, Mode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result, ljpeg,
};
use craft_tiff::image::chunk_bytes;
use craft_tiff::{ByteOrder, Ifd, Tiff, Value, makernote, tags as t};
use std::ops::Range;

const CR2_SLICE: u16 = 0xc640;
const SRAW_TYPE: u16 = 0xc6c5;
/// `SRAW_TYPE` value of the YCbCr frames of sRAW / mRAW (IFD2's preview image carries 3).
const SRAW_YCC: u32 = 4;
const CR2_CFA_PATTERN: u16 = 0xc5e0;
const SENSOR_INFO: u16 = 0x00e0;
const COLOR_BALANCE: u16 = 0x4001;
/// ShotInfo, whose value 7 is the white-balance setting.
const SHOT_INFO: u16 = 0x0004;
/// WBInfo, the PowerShot G9's preset levels (it writes no `ColorData`).
const WB_INFO: u16 = 0x0029;

fn raw_ifd(tiff: &Tiff) -> Option<&Ifd> {
    tiff.ifds.get(3).or_else(|| tiff.ifds.iter().rev().find(|i| i.contains(CR2_SLICE)))
}

/// Bayer layout from the raw IFD's `CR2CFAPattern` (`0xc5e0`) value; `None` for missing/unknown values.
fn cfa_from_tag(v: Option<u64>) -> Option<Cfa> {
    let name = match v? {
        1 => "RGGB",
        2 => "BGGR",
        3 => "GBRG",
        4 => "GRBG",
        _ => return None,
    };
    Some(Cfa::bayer_static(name))
}

/// The 16-bit words of a maker-note array. Canon writes `ColorData` as SHORT in most models, but as UNDEFINED
/// bytes (count = byte length, e.g. 5120) in the PowerShot / EOS M models with ColorData versions -3 and -4;
/// those bytes are the same 16-bit words in the maker note's byte order, so they are paired up, not read one by
/// one. Other types are returned value by value.
fn words(v: &Value, order: ByteOrder) -> Vec<u64> {
    match v {
        Value::Byte(b) | Value::Undefined(b) => b.as_chunks::<2>().0.iter().map(|c| u64::from(order.u16(*c))).collect(),
        _ => v.to_u64_vec(),
    }
}

/// WB multipliers (R, G, B; G = 1) from one set of levels, when they are plausible.
fn wb_from_levels(r: f32, g1: f32, g2: f32, b: f32) -> Option<[f32; 3]> {
    if g1 < 256.0 || g2 < 256.0 || r < 64.0 || b < 64.0 || g1 > 16384.0 || (g1 - g2).abs() > 0.05 * g1 {
        return None;
    }
    let g = (g1 + g2) / 2.0;
    let (mr, mb) = (r / g, b / g);
    ((0.25..6.0).contains(&mr) && (0.25..6.0).contains(&mb)).then_some([mr, 1.0, mb])
}

/// As-shot WB multipliers (R, G, B; G = 1) from the ColorBalance array.
fn wb_from_color_balance(v: &[u64]) -> Option<[f32; 3]> {
    for off in [63usize, 25, 24, 34, 71, 85, 105, 69, 77] {
        let q = v.get(off..off + 4)?;
        if let Some(wb) = wb_from_levels(q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32) {
            return Some(wb);
        }
    }
    None
}

/// Where the planckian white-balance presets sit in `ColorData`, by `ColorDataVersion` (word 0, signed): the word
/// index of each preset's `R G G B` levels and of its colour temperature. Daylight, Shade, Cloudy, Tungsten and
/// Flash, in that order. Fluorescent is left out (a green light off the locus) and so is the Kelvin row (it holds the
/// dial's last setting, and on some bodies a temperature that doesn't match its own levels: 6036 K with the camera at
/// 5500 K on the PowerShot G1 X Mark III, Daylight's levels at 5213 K on the EOS 2000D). Positions from the ExifTool
/// Canon tag-name documentation's `ColorData5` / `ColorData8` tables, confirmed with ExifTool run as a black-box tool
/// on files of each version: -4 on a PowerShot G1 X Mark III (`ColorData5`: 8-word records, the second record after
/// Daylight a repeat of it), 13, 14 and 15 on the CC0 corpus samples of the EOS 5D Mark IV, 2000D and 6D Mark II
/// (`ColorData8`: 5-word records). Other versions give no presets.
fn preset_rows(version: i16) -> Option<[(usize, usize); 5]> {
    match version {
        -4 => Some([(95, 102), (111, 118), (119, 126), (127, 134), (151, 158)]),
        13..=15 => Some([(133, 137), (138, 142), (143, 147), (148, 152), (163, 167)]),
        _ => None,
    }
}

/// The camera's planckian white-balance presets from the `ColorData` words ([`preset_rows`]) as a Kelvin curve;
/// `None` for an unknown version, a short array or presets that don't make a curve.
fn kelvin_presets_from_color_data(v: &[u64]) -> Option<crate::kelvin::KelvinPresets> {
    let version = *v.first()? as u16 as i16;
    let rows = preset_rows(version)?;
    let mut points = Vec::with_capacity(rows.len());
    for (at, temp_at) in rows {
        let q = v.get(at..at + 4)?;
        let temp = *v.get(temp_at)? as f32;
        if let Some(wb) = wb_from_levels(q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32)
            && (1000.0..=20000.0).contains(&temp)
        {
            points.push((temp, wb));
        }
    }
    crate::kelvin::KelvinPresets::from_unordered(points)
}

/// As-shot WB multipliers of a body without `ColorData` (the PowerShot G9): the WBInfo row of the preset that
/// ShotInfo's white-balance value names. WBInfo is int32 values in the maker note's byte order, four per preset in
/// G R B G order, at the rows the ExifTool Canon tag-name documentation gives (2 Auto, 10 Daylight, 18 Cloudy,
/// 26 Tungsten, 34 Fluorescent, 50 Flash, 58 Underwater, 66 Custom 1, 74 Custom 2); other settings (14, Daylight
/// Fluorescent, has no documented row) give none. Measured on the CC0 file IMG_1188.CR2 (Custom 1, row 66:
/// 848 1747 1676 848), whose camera JPEG's neutral areas imply the same gains within 0.1 %.
fn wb_from_wb_info(m: &makernote::MakerNote) -> Option<[f32; 3]> {
    let row = match m.ifd.u64s(SHOT_INFO)?.get(7)? {
        0 => 2,
        1 => 10,
        2 => 18,
        3 => 26,
        4 => 34,
        5 => 50,
        15 => 66,
        16 => 74,
        17 => 58,
        _ => return None,
    };
    let b = m.ifd.bytes(WB_INFO)?;
    let level = |i: usize| Some(m.order.u32(b.get(4 * (row + i)..4 * (row + i + 1))?.try_into().ok()?) as i32 as f32);
    wb_from_levels(level(1)?, level(0)?, level(3)?, level(2)?)
}

/// The optically black columns left of the image area: from column 2 up to two columns before the first column
/// whose mean (over the image rows) rises above the leftmost columns' level by more than 1/256 of the remaining
/// range (at least 16), else up to two columns before the image area. Empty when the border is too narrow.
fn masked_columns(data: &[u16], width: usize, active: Rect, white: f32) -> Range<usize> {
    let limit = active.x.saturating_sub(2).min(width);
    if limit < 4 || active.height == 0 {
        return 0..0;
    }
    let step = (active.height / 256).max(1);
    let column_mean = |x: usize| {
        let (mut sum, mut n) = (0.0, 0usize);
        for y in (active.y..active.y.saturating_add(active.height)).step_by(step) {
            if let Some(&v) = y.checked_mul(width).and_then(|i| data.get(i.checked_add(x)?)) {
                sum += f64::from(v);
                n += 1;
            }
        }
        (n > 0).then(|| sum / n as f64)
    };
    let means: Vec<f64> = (2..limit).map_while(column_mean).collect();
    let mut first: Vec<f64> = means.iter().take(8).copied().collect();
    first.sort_by(f64::total_cmp);
    let Some(&level) = first.get(first.len() / 2) else { return 0..0 };
    let tolerance = ((f64::from(white) - level) / 256.0).max(16.0);
    let end = match means.iter().position(|&m| m > level + tolerance) {
        // `means[k]` is column `2 + k`: stop two columns before it
        Some(k) => k,
        None => 2 + means.len(),
    };
    2..end.max(2)
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("TIFF without IFDs".into()))?;
    let raw = raw_ifd(&tiff).ok_or_else(|| RawError::Corrupt("CR2 without raw IFD".into()))?;
    match raw.u32(SRAW_TYPE) {
        Some(SRAW_YCC) => return sraw::decode(bytes, &tiff, raw, mode, limits),
        Some(v) if v != 1 => return Err(RawError::Unsupported(format!("Canon raw type {v}"))),
        _ => {}
    }
    let off = raw.u64(t::STRIP_OFFSETS).ok_or(RawError::Tiff(craft_tiff::TiffError::MissingTag(t::STRIP_OFFSETS)))?;
    let len = raw.u64(t::STRIP_BYTE_COUNTS).unwrap_or(bytes.len() as u64 - off.min(bytes.len() as u64));
    let chunk = craft_tiff::image::Chunk { index: 0, x: 0, y: 0, width: 0, height: 0, plane: 0, offset: off, len };
    let src = chunk_bytes(bytes, &chunk).ok_or_else(|| RawError::Corrupt("raw strip outside file".into()))?;
    let (fw, fh, nc, prec) = ljpeg::frame_info(src)?;
    let frame = match mode {
        Mode::Full => {
            let total = (fw as u64)
                .checked_mul(fh as u64)
                .and_then(|v| v.checked_mul(nc as u64))
                .filter(|t| *t > 0 && *t <= limits.samples() as u64)
                .ok_or_else(|| RawError::Corrupt("bad CR2 frame size".into()))?;
            Some(ljpeg::decode(src, total as usize)?)
        }
        Mode::Header => None,
    };
    let total = match &frame {
        Some(f) => f.data.len(),
        None => {
            let total = fw
                .checked_mul(fh)
                .and_then(|v| v.checked_mul(nc))
                .filter(|t| *t > 0)
                .ok_or_else(|| RawError::Corrupt("bad CR2 frame size".into()))?;
            if total > limits.samples() {
                return Err(RawError::Limit("lossless JPEG frame larger than expected"));
            }
            total
        }
    };
    let slices = raw.u64s(CR2_SLICE).filter(|s| s.len() == 3 && s[1] > 0 && s[2] > 0);
    let widths: Vec<usize> = match &slices {
        Some(s) => {
            let n = s[0].min(64) as usize;
            let mut v = vec![s[1] as usize; n];
            v.push(s[2] as usize);
            v
        }
        None => {
            let w = (fw as u64).checked_mul(nc as u64).ok_or_else(|| RawError::Corrupt("bad CR2 frame size".into()))?;
            vec![w as usize]
        }
    };
    let width: usize = widths.iter().sum();
    if width == 0 || total % width != 0 {
        return Err(RawError::Corrupt(format!("CR2 slices ({width}) do not divide the frame ({total} samples)")));
    }
    let height = total / width;
    let mut data = Vec::new();
    if let Some(frame) = &frame {
        data = vec![0u16; total];
        let mut i = 0;
        let mut x0 = 0;
        for &sw in &widths {
            for y in 0..height {
                data[y * width + x0..y * width + x0 + sw].copy_from_slice(&frame.data[i..i + sw]);
                i += sw;
            }
            x0 += sw;
        }
    }
    drop(frame);

    // maker note: sensor borders and white balance
    let make = ifd0.string(t::MAKE).unwrap_or_default();
    let mn =
        tiff.exif().and_then(|e| e.get(t::MAKER_NOTE)).and_then(|e| makernote::parse_makernote(bytes, e.offset, e.count() as u64, tiff.order, &make));
    let mut active = Rect::new(0, 0, width, height);
    if let Some(si) = mn.as_ref().and_then(|m| m.ifd.u64s(SENSOR_INFO)).filter(|v| v.len() >= 9) {
        let (l, tp, r, b) = (si[5] as usize, si[6] as usize, si[7] as usize, si[8] as usize);
        if r > l && b > tp && r < width && b < height {
            active = Rect::new(l, tp, r - l + 1, b - tp + 1);
        }
    }
    let color_data = mn.as_ref().and_then(|m| m.ifd.value(COLOR_BALANCE).map(|v| words(v, m.order)));
    let kelvin_presets = color_data.as_deref().and_then(kelvin_presets_from_color_data);
    let wb = color_data.as_deref().and_then(wb_from_color_balance).or_else(|| wb_from_wb_info(mn.as_ref()?));
    let cfa = cfa_from_tag(raw.u64(CR2_CFA_PATTERN)).unwrap_or_else(|| Cfa::bayer_static("RGGB"));
    let (white, white_source) = white_from_data_sourced(&active_rows(&data, width, active), prec as u32);
    let (black, black_source) = match masked_columns(&data, width, active, white) {
        cols if cols.len() >= 2 => black_from_columns_sourced(&data, width, cols, active.y..active.y + active.height, active),
        _ => (BlackLevel::uniform(0.0), crate::LevelSource::Fallback),
    };
    let img = RawImage {
        format: RawFormat::Cr2,
        width,
        height,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits: prec as u32,
        black,
        white: vec![white],
        active_area: active,
        crop: Rect::new(0, 0, active.width, active.height),
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData { kelvin_presets, ..Default::default() },
        wb_multipliers: wb,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: black_source, white: white_source },
    };
    img.validate_for(mode)?;
    Ok(img)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

    /// Build a synthetic CR2: IFD0..IFD2 placeholders + IFD3 holding a sliced 2-component LJ92 frame.
    pub(crate) fn synthetic_cr2(w: usize, h: usize, slices: Option<[u16; 3]>) -> (Vec<u8>, Vec<u16>) {
        synthetic_cr2_note(w, h, slices, None)
    }

    /// Like [`synthetic_cr2`], with `note` as the Exif maker note (bytes as stored).
    fn synthetic_cr2_note(w: usize, h: usize, slices: Option<[u16; 3]>, note: Option<Vec<u8>>) -> (Vec<u8>, Vec<u16>) {
        let img: Vec<u16> = (0..w * h).map(|i| 1024 + ((i * 7919) % 12000) as u16).collect();
        (synthetic_cr2_of(&img, w, h, slices, note), img)
    }

    /// A synthetic CR2 whose sensor samples are `img` (`w`×`h`, 14-bit), with `note` as the Exif maker note.
    fn synthetic_cr2_of(img: &[u16], w: usize, h: usize, slices: Option<[u16; 3]>, note: Option<Vec<u8>>) -> Vec<u8> {
        let widths: Vec<usize> = match slices {
            Some([n, sw, last]) => {
                let mut v = vec![sw as usize; n as usize];
                v.push(last as usize);
                v
            }
            None => vec![w],
        };
        let mut stream = Vec::with_capacity(w * h);
        let mut x0 = 0;
        for &sw in &widths {
            for y in 0..h {
                stream.extend_from_slice(&img[y * w + x0..y * w + x0 + sw]);
            }
            x0 += sw;
        }
        let enc = crate::ljpeg::encode(&stream, w / 2, h, 2, 14, 1, 0);
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("Canon".into()));
        ifd0.set(t::MODEL, Value::Ascii("Canon EOS Test".into()));
        ifd0.set(t::ORIENTATION, Value::Short(vec![8]));
        // maker note: SensorInfo + ColorBalance, plain IFD with offsets relative to the TIFF header
        let mut exif = IfdBuilder::new();
        exif.set(t::ISO_SPEED, Value::Short(vec![400]));
        if let Some(n) = note {
            exif.set(t::MAKER_NOTE, Value::Undefined(n));
        }
        ifd0.set_child(t::EXIF_IFD, exif);
        let mut ifd3 = IfdBuilder::new();
        ifd3.set(t::COMPRESSION, Value::Short(vec![6]));
        ifd3.set(CR2_CFA_PATTERN, Value::Long(vec![3]));
        if let Some(s) = slices {
            ifd3.set(CR2_SLICE, Value::Short(s.to_vec()));
        }
        ifd3.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![enc] });
        // (real files also carry a "CR\x02\0" signature at offset 8; probe recognises the structure without it)
        TiffWriter::new(ByteOrder::Little, false)
            .write(&[ifd0, IfdBuilder::new().with(1, Value::Short(vec![0])), IfdBuilder::new().with(1, Value::Short(vec![0])), ifd3])
            .unwrap()
    }

    /// A frame with no masked border and no saturation plateau says so: both levels are fallbacks.
    #[test]
    fn levels_without_masked_columns_or_plateau_are_fallbacks() {
        let (bytes, _) = synthetic_cr2(64, 10, None);
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Fallback, white: crate::LevelSource::Fallback });
        assert_eq!(r.white, vec![16383.0]);
    }

    #[test]
    fn synthetic_roundtrip() {
        for slices in [None, Some([2u16, 24, 16]), Some([1, 40, 24])] {
            let (bytes, img) = synthetic_cr2(64, 10, slices);
            assert_eq!(crate::probe(&bytes), Some(RawFormat::Cr2));
            let r = crate::decode(&bytes).unwrap();
            assert_eq!((r.width, r.height), (64, 10));
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
            assert_eq!(r.data, RawData::U16(img));
            assert_eq!(r.orientation, Orientation::Rotate270);
            assert_eq!(r.cfa.as_ref().map(Cfa::name).as_deref(), Some("GBRG"), "CR2CFAPattern 3");
        }
    }

    #[test]
    fn cfa_pattern_tag() {
        let name = |v| cfa_from_tag(v).map(|c| c.name());
        assert_eq!(name(Some(1)).as_deref(), Some("RGGB"));
        assert_eq!(name(Some(2)).as_deref(), Some("BGGR"));
        assert_eq!(name(Some(3)).as_deref(), Some("GBRG"));
        assert_eq!(name(Some(4)).as_deref(), Some("GRBG"));
        assert_eq!(name(Some(0)), None);
        assert_eq!(name(Some(7)), None);
        assert_eq!(name(None), None);
    }

    #[test]
    fn wb_probe() {
        let mut v = vec![0u64; 120];
        v[63..67].copy_from_slice(&[2000, 1024, 1026, 1500]);
        let wb = wb_from_color_balance(&v).unwrap();
        assert!((wb[0] - 2000.0 / 1025.0).abs() < 1e-4 && (wb[2] - 1500.0 / 1025.0).abs() < 1e-4);
        assert!(wb_from_color_balance(&[0; 10]).is_none());
    }

    #[test]
    fn wb_from_undefined_bytes() {
        // hand-coded: ColorData of an S110-like file, UNDEFINED bytes, little-endian words; word 71.. = 1988 745 745 1641
        let mut b = vec![0u8; 2 * 120];
        b[0..2].copy_from_slice(&[0xfd, 0xff]); // version -3
        b[142..150].copy_from_slice(&[0xc4, 0x07, 0xe9, 0x02, 0xe9, 0x02, 0x69, 0x06]);
        let le = words(&Value::Undefined(b.clone()), ByteOrder::Little);
        assert_eq!(&le[71..75], &[1988, 745, 745, 1641]);
        let wb = wb_from_color_balance(&le).unwrap();
        assert!((wb[0] - 1988.0 / 745.0).abs() < 1e-5 && wb[1] == 1.0 && (wb[2] - 1641.0 / 745.0).abs() < 1e-5);
        // read as single bytes (the old behaviour) nothing is plausible
        assert!(wb_from_color_balance(&b.iter().map(|&x| u64::from(x)).collect::<Vec<_>>()).is_none());
        // big-endian note: same words, bytes swapped
        let mut be = b.clone();
        be.as_chunks_mut::<2>().0.iter_mut().for_each(|c| c.swap(0, 1));
        assert_eq!(words(&Value::Undefined(be), ByteOrder::Big), le);
        // SHORT values pass through, an odd trailing byte is ignored
        assert_eq!(words(&Value::Short(vec![1, 2, 3]), ByteOrder::Little), vec![1, 2, 3]);
        assert_eq!(words(&Value::Undefined(vec![1, 0, 2]), ByteOrder::Little), vec![1]);
    }

    /// Light reaches columns left of the `SensorInfo` border (as on the EOS 6D): the black level comes from the
    /// masked columns only, not from the image columns between them and the border.
    #[test]
    fn black_from_masked_columns_only() {
        let (w, h, border, light) = (40, 12, 20, 14);
        let data: Vec<u16> = (0..w * h).map(|i| if i % w < light { 2048 + (i % 2) as u16 } else { 6000 }).collect();
        let active = Rect::new(border, 2, w - border, h - 2);
        let cols = masked_columns(&data, w, active, 15000.0);
        assert_eq!(cols, 2..light - 2);
        let black = crate::shared::black_from_columns(&data, w, cols, 2..h, active);
        assert!(black.values.iter().all(|v| (2048.0..=2049.0).contains(v)), "{:?}", black.values);
        // the whole border masked: every column but the two next to the image
        let dark: Vec<u16> = (0..w * h).map(|i| if i % w < border { 2048 } else { 6000 }).collect();
        assert_eq!(masked_columns(&dark, w, active, 15000.0), 2..border - 2);
        // too narrow a border, or no samples (headers-only decode): none
        assert!(masked_columns(&dark, w, Rect::new(5, 0, 30, h), 15000.0).is_empty());
        assert!(masked_columns(&[], w, active, 15000.0).is_empty());
    }

    /// A maker note holding only `tag` as `ty` (3 = SHORT, 7 = UNDEFINED) with `data`, a plain IFD whose value
    /// offset is relative to the TIFF header (patched by [`patch_note`] once the note's position is known).
    fn note_with(tag: u16, ty: u16, data: &[u8]) -> Vec<u8> {
        let count = if ty == 3 { data.len() / 2 } else { data.len() } as u32;
        let mut note = Vec::new();
        note.extend_from_slice(&1u16.to_le_bytes());
        note.extend_from_slice(&tag.to_le_bytes());
        note.extend_from_slice(&ty.to_le_bytes());
        note.extend_from_slice(&count.to_le_bytes());
        note.extend_from_slice(&0u32.to_le_bytes()); // value offset, patched below
        note.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        note.extend_from_slice(data);
        note
    }

    fn patch_note(bytes: &mut [u8]) {
        let tiff = Tiff::parse(bytes).unwrap();
        let e = tiff.exif().unwrap().get(t::MAKER_NOTE).unwrap();
        let off = e.offset as usize;
        bytes[off + 10..off + 14].copy_from_slice(&(e.offset as u32 + 18).to_le_bytes());
    }

    fn decode_with_color_data(ty: u16, data: &[u8]) -> RawImage {
        let (mut bytes, _) = synthetic_cr2_note(64, 10, None, Some(note_with(COLOR_BALANCE, ty, data)));
        patch_note(&mut bytes);
        crate::decode(&bytes).unwrap()
    }

    /// A maker note of `entries` (tag, TIFF type, value bytes), each value stored after the IFD, decoded; the value
    /// offsets (relative to the TIFF header) are patched once the note's position is known.
    fn decode_with_note(entries: &[(u16, u16, Vec<u8>)]) -> RawImage {
        let ifd_len = 2 + 12 * entries.len() + 4;
        let (mut note, mut data, mut at) = (Vec::new(), Vec::new(), Vec::new());
        note.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, ty, value) in entries {
            let count = if *ty == 3 { value.len() / 2 } else { value.len() } as u32;
            note.extend_from_slice(&tag.to_le_bytes());
            note.extend_from_slice(&ty.to_le_bytes());
            note.extend_from_slice(&count.to_le_bytes());
            at.push((note.len(), ifd_len + data.len()));
            note.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(value);
        }
        note.extend_from_slice(&0u32.to_le_bytes());
        note.extend_from_slice(&data);
        let (mut bytes, _) = synthetic_cr2_note(64, 10, None, Some(note));
        let tiff = Tiff::parse(&bytes).unwrap();
        let off = tiff.exif().unwrap().get(t::MAKER_NOTE).unwrap().offset as usize;
        for (field, value) in at {
            bytes[off + field..off + field + 4].copy_from_slice(&((off + value) as u32).to_le_bytes());
        }
        crate::decode(&bytes).unwrap()
    }

    /// storytold/lightcraft#1273: the PowerShot G9 has no `ColorData`; its preset levels are WBInfo (`0x0029`: BYTE, int32 values,
    /// four per preset in G R B G order), and ShotInfo (`0x0004`) value 7 says which preset was used (15 = Custom 1,
    /// row 66: 848 1747 1676 848 on the CC0 file IMG_1188.CR2).
    #[test]
    fn white_balance_from_wb_info_when_there_is_no_color_data() {
        let shot_info = |wb: u16| {
            let mut v = [0u16; 34];
            v[7] = wb;
            v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()
        };
        let mut rows = vec![0i32; 512];
        rows[2..6].copy_from_slice(&[850, 1805, 1684, 850]);
        rows[66..70].copy_from_slice(&[848, 1747, 1676, 848]);
        rows[74..78].copy_from_slice(&[820, 1901, 4000, 820]);
        rows[18..22].copy_from_slice(&[850, 1805, 6000, 850]);
        let wb_info: Vec<u8> = rows.iter().flat_map(|x| x.to_le_bytes()).collect();
        let wb = |si: u16| decode_with_note(&[(0x0004, 3, shot_info(si)), (0x0029, 1, wb_info.clone())]).wb_multipliers;
        assert_eq!(wb(15), Some([1747.0 / 848.0, 1.0, 1676.0 / 848.0]));
        assert_eq!(wb(0), Some([1805.0 / 850.0, 1.0, 1684.0 / 850.0]));
        // each preset its own row (Custom 2: row 74); an empty row (Daylight here), an implausible one (Cloudy:
        // B 7.1) or a value without a documented row (14) gives none
        assert_eq!(wb(16), Some([1901.0 / 820.0, 1.0, 4000.0 / 820.0]));
        assert_eq!(wb(1), None);
        assert_eq!(wb(2), None);
        assert_eq!(wb(14), None);
        // without ShotInfo, or with WBInfo too short for the row
        assert_eq!(decode_with_note(&[(0x0029, 1, wb_info.clone())]).wb_multipliers, None);
        assert_eq!(decode_with_note(&[(0x0004, 3, shot_info(15)), (0x0029, 1, wb_info[..4 * 68].to_vec())]).wb_multipliers, None);
        // ColorData, when it gives levels, wins
        let mut color_data = vec![0u8; 2 * 120];
        color_data[0..2].copy_from_slice(&[0xfd, 0xff]);
        color_data[142..150].copy_from_slice(&[0xc4, 0x07, 0xe9, 0x02, 0xe9, 0x02, 0x69, 0x06]);
        let both = decode_with_note(&[(0x0004, 3, shot_info(15)), (0x0029, 1, wb_info.clone()), (COLOR_BALANCE, 7, color_data)]);
        assert_eq!(both.wb_multipliers, Some([1988.0 / 745.0, 1.0, 1641.0 / 745.0]));
    }

    /// `ColorData` stored as UNDEFINED bytes (versions -3 / -4: PowerShot, EOS M) gives the as-shot WB from word 71,
    /// exactly as the same words stored as SHORT do.
    #[test]
    fn color_data_stored_as_bytes() {
        let mut bytes = vec![0u8; 2 * 120];
        bytes[0..2].copy_from_slice(&[0xfd, 0xff]); // version -3
        bytes[142..150].copy_from_slice(&[0xc4, 0x07, 0xe9, 0x02, 0xe9, 0x02, 0x69, 0x06]); // 1988 745 745 1641
        for ty in [7, 3] {
            let wb = decode_with_color_data(ty, &bytes).wb_multipliers.unwrap_or_else(|| panic!("type {ty}: no WB"));
            assert!((wb[0] - 1988.0 / 745.0).abs() < 1e-5 && wb[1] == 1.0 && (wb[2] - 1641.0 / 745.0).abs() < 1e-5, "type {ty}: {wb:?}");
        }
    }

    /// `ColorData` words of `version` with the planckian presets (R, B over G = 1024, Kelvin) at its
    /// [`preset_rows`], a Fluorescent-like green row between them, and the as-shot levels at word 71 / 63.
    fn color_data_with_presets(version: i16, presets: &[(u16, u16, u16)]) -> Vec<u64> {
        let mut v = vec![0u64; 400];
        v[0] = u64::from(version as u16);
        for ((at, temp_at), &(r, b, k)) in preset_rows(version).unwrap().into_iter().zip(presets) {
            v[at..at + 4].copy_from_slice(&[r.into(), 1024, 1024, b.into()]);
            v[temp_at] = k.into();
        }
        v
    }

    /// storytold/lightcraft#157 of a downstream fork: the PowerShot G1 X Mark III (`ColorData` -4) gets a Kelvin curve from its
    /// Daylight, Shade, Cloudy, Tungsten and Flash presets (levels and temperatures as its files carry them), which
    /// puts a preset's own levels at the preset's temperature; the EOS bodies of versions 13–15 likewise.
    #[test]
    fn kelvin_presets_from_color_data_presets() {
        let g1x3 = [(1418, 1357, 5651), (1653, 1171, 7505), (1533, 1259, 6594), (963, 2016, 3174), (1807, 1080, 8529)];
        let v = color_data_with_presets(-4, &g1x3);
        let p = kelvin_presets_from_color_data(&v).unwrap();
        // every preset is on the curve at its own temperature, in Kelvin order
        let mut want: Vec<(f32, [f32; 3])> = g1x3.iter().map(|&(r, b, k)| (k as f32, [r as f32 / 1024.0, 1.0, b as f32 / 1024.0])).collect();
        want.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(p.points(), want);
        let eos = [(2013, 1479, 5200), (2300, 1280, 7000), (2153, 1374, 6000), (1442, 2089, 3200), (2203, 1371, 6156)];
        for version in [13, 14, 15] {
            assert!(kelvin_presets_from_color_data(&color_data_with_presets(version, &eos)).is_some(), "{version}");
        }
        // a preset that would turn the curve back (Flash cooler than Shade's levels say) is dropped, not fatal
        let mut odd = g1x3;
        odd[4] = (1500, 1300, 9000);
        assert!(kelvin_presets_from_color_data(&color_data_with_presets(-4, &odd)).is_some());
    }

    /// Unknown versions, short or garbled arrays give no presets and never panic.
    #[test]
    fn kelvin_presets_from_hostile_color_data() {
        let g1x3 = [(1418, 1357, 5651), (1653, 1171, 7505), (1533, 1259, 6594), (963, 2016, 3174), (1807, 1080, 8529)];
        let good = color_data_with_presets(-4, &g1x3);
        let mut other = good.clone();
        other[0] = u64::from(-3i16 as u16);
        assert!(kelvin_presets_from_color_data(&other).is_none(), "unverified version");
        assert!(kelvin_presets_from_color_data(&[]).is_none());
        for len in [1, 96, 120, 157, 158] {
            assert!(kelvin_presets_from_color_data(&good[..len]).is_none(), "{len}");
        }
        let mut zeros = good.clone();
        for at in [102, 118, 126] {
            zeros[at] = 0;
        }
        assert!(kelvin_presets_from_color_data(&zeros).is_none(), "two points left");
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let v: Vec<u64> = (0..200)
                .map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    if i == 0 { [0xfffc, 13, 14, 15][(seed % 4) as usize] } else { seed % 70000 }
                })
                .collect();
            let _ = kelvin_presets_from_color_data(&v);
        }
        // through a whole decode, ColorData as UNDEFINED bytes
        let bytes: Vec<u8> = good.iter().flat_map(|&w| (w as u16).to_le_bytes()).collect();
        assert!(decode_with_color_data(7, &bytes).color.kelvin_presets.is_some());
        assert!(decode_with_color_data(7, &bytes[..200]).color.kelvin_presets.is_none());
    }

    /// Masked rows above the image full of samples at 16383 (EOS 50D, storytold/lightcraft#1272): the white level is the image's
    /// own saturation plateau, not the edge of that strip, so clipped highlights still reach 1.0.
    #[test]
    fn white_from_active_area_only() {
        let (w, h, top, left) = (64, 40, 4, 8);
        let img: Vec<u16> = (0..w * h)
            .map(|i| match (i % w, i / w) {
                (_, y) if y < top => 16383,
                (x, _) if x < left => 1024,
                _ if i % 7 == 0 => 13433,
                _ => 1024 + ((i * 7919) % 11000) as u16,
            })
            .collect();
        // SensorInfo: width, height, three unused words, then the inclusive left/top/right/bottom borders
        let si: Vec<u8> = [w, h, 0, 0, 0, left, top, w - 1, h - 1].iter().flat_map(|&v| (v as u16).to_le_bytes()).collect();
        let mut bytes = synthetic_cr2_of(&img, w, h, None, Some(note_with(SENSOR_INFO, 3, &si)));
        patch_note(&mut bytes);
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(r.active_area, Rect::new(left, top, w - left, h - top));
        assert!((13300.0..=13433.0).contains(&r.white[0]), "{:?}", r.white);
    }
}
