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
//!   read per file first. Without a known tag, observed model IDs provide the fallback; scene-derived
//!   measurements are not authoritative enough to establish sensor phase. The parity of the
//!   `SensorInfo` borders does *not* predict it (RGGB files come with both even and odd top borders).
//!   Header-only probes do not decode pixels for this fallback: without the tag their reported CFA
//!   is assumed and can refine on full decode, as indicated by `RawNotices::assumed_cfa`.
//! - `SensorInfo` (maker note `0x00e0`): sensor width/height and the left/top/right/bottom borders of the image
//!   area. Light already reaches some columns left of that border (EOS 6D: masked columns 0–70, border 84; 10–36
//!   such columns on every corpus body), so the masked columns that give the black level are measured from the
//!   data ([`masked_columns`]) rather than taken as the whole border.
//! - `ColorBalance` (maker note `0x4001`): as-shot `RGGB` levels at a model-dependent offset; we probe the known
//!   offsets and accept the first plausible quadruple. The array is 16-bit words; some models store it as UNDEFINED
//!   bytes (ColorData versions -3 and -4), which are paired up in the maker note's byte order first.
//! - The same array lists the levels of the camera's white-balance presets, each with the colour temperature Canon
//!   gives it ([`kelvin_presets_from_color_data`]): a Kelvin scale for a body without colour matrices.
//! - `AspectInfo` (maker note `0x009a`, as in CR3: aspect code, width, height, left, top): the in-camera aspect
//!   ratio (3:2, 4:3, 16:9, 1:1) is not applied to the sensor data, and the rectangle is measured from the top-left
//!   of the `SensorInfo` image area. It becomes the default crop (storytold/lightcraft#3313). Checked on the CC0
//!   PowerShot G12 and S120 files shot at 3:2 (3648 x 2432 at top 152 of a 3648 x 2736 area, 4000 x 2664 at top 168
//!   of 4000 x 3000: centred) and a G16 shot at 4:3 (the whole area); files shot at the sensor's own aspect store
//!   the whole area or no tag.
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
const CANON_MODEL_ID: u16 = 0x0010;
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

/// Fallback for files without CR2CFAPattern. Image content cannot establish sensor phase safely.
fn fallback_cfa(_data: &[u16], _width: usize, _area: Rect, _black: f32, _white: f32, model: Option<u64>) -> (Cfa, bool) {
    // Observed CC0 raw.pixls.us captures: 5D II, 7D, 550D, 600D and 1200D.
    let green_first = [0x8000_0218, 0x8000_0250, 0x8000_0270, 0x8000_0286, 0x8000_0327].into_iter().any(|id| model == Some(id));
    (Cfa::bayer_static(if green_first { "GBRG" } else { "RGGB" }), true)
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
/// Canon tag-name documentation's `ColorData1`–`ColorData8` tables, confirmed with ExifTool run as a black-box tool
/// on files of each version and by reading the words of 453 CR2 files (reporters' samples and the CC0 corpus): -4 on
/// a PowerShot G1 X Mark III (`ColorData5`: 8-word records, the second record after Daylight a repeat of it);
/// everything else is 5-word records `R G G B Kelvin` in the order Daylight (5200 K), Shade (7000), Cloudy (6000),
/// Tungsten (3200), Fluorescent, Kelvin, Flash (thirty words after Daylight): version 1 from word 78, 2–9 from 83, 10
/// from 103 on the EOS 1200D family and from 128 on the others, 11 from 128, 12–15 from 133. A version with two
/// layouts is read at the one whose Daylight record names 5200 K. Other versions give no presets.
fn preset_rows(version: i16) -> &'static [[(usize, usize); 5]] {
    const fn records(daylight: usize) -> [(usize, usize); 5] {
        preset_records(daylight, daylight + 30)
    }
    const G1X3: [(usize, usize); 5] = [(95, 102), (111, 118), (119, 126), (127, 134), (151, 158)];
    const AT_78: [(usize, usize); 5] = records(78);
    const AT_83: [(usize, usize); 5] = records(83);
    const AT_103: [(usize, usize); 5] = records(103);
    const AT_128: [(usize, usize); 5] = records(128);
    const AT_133: [(usize, usize); 5] = records(133);
    match version {
        -4 => &[G1X3],
        1 => &[AT_78],
        2..=9 => &[AT_83],
        10 => &[AT_103, AT_128],
        11 => &[AT_128],
        12..=15 => &[AT_133],
        _ => &[],
    }
}

/// The five preset records (Daylight, Shade, Cloudy, Tungsten, Flash) of a `ColorData` array whose Daylight record
/// starts at word `daylight` and Flash record at `flash`: Daylight, Shade, Cloudy and Tungsten follow each other
/// in 5-word records `R G G B Kelvin`. Also used by CR3 (`ColorData` words of the same record layout).
pub(crate) const fn preset_records(daylight: usize, flash: usize) -> [(usize, usize); 5] {
    [(daylight, daylight + 4), (daylight + 5, daylight + 9), (daylight + 10, daylight + 14), (daylight + 15, daylight + 19), (flash, flash + 4)]
}

/// The camera's planckian white-balance presets from the `ColorData` words ([`preset_rows`]) as a Kelvin curve;
/// `None` for an unknown version, a short array or presets that don't make a curve.
fn kelvin_presets_from_color_data(v: &[u64]) -> Option<crate::kelvin::KelvinPresets> {
    let version = *v.first()? as u16 as i16;
    let rows = match preset_rows(version) {
        [one] => one,
        many => many.iter().find(|rows| v.get(rows[0].1) == Some(&5200))?,
    };
    presets_at(v, rows)
}

/// The Kelvin curve of the preset records at `rows` ((levels word, temperature word) of each) in `v`.
pub(crate) fn presets_at(v: &[u64], rows: &[(usize, usize); 5]) -> Option<crate::kelvin::KelvinPresets> {
    let mut points = Vec::with_capacity(rows.len());
    for &(at, temp_at) in rows {
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
    let tagged_cfa = cfa_from_tag(raw.u64(CR2_CFA_PATTERN));
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
            let (Some(w0), Some(w1)) = (usize::try_from(s[1]).ok(), usize::try_from(s[2]).ok()) else {
                return Err(RawError::Corrupt("CR2 slice width exceeds address space".into()));
            };
            let mut v = vec![w0; n];
            v.push(w1);
            v
        }
        None => {
            let w = (fw as u64)
                .checked_mul(nc as u64)
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(|| RawError::Corrupt("bad CR2 frame size".into()))?;
            vec![w]
        }
    };
    let width: usize = widths.iter().try_fold(0usize, |acc, &w| acc.checked_add(w)).ok_or(RawError::Limit("CR2 slice widths overflow"))?;
    if width == 0 || total % width != 0 {
        return Err(RawError::Corrupt(format!("CR2 slices ({width}) do not divide the frame ({total} samples)")));
    }
    let height = total / width;
    if width > u32::MAX as usize || height > u32::MAX as usize {
        return Err(RawError::Limit("CR2 dimensions exceed u32::MAX"));
    }
    let mut data = Vec::new();
    if let Some(frame) = &frame {
        data = vec![0u16; total];
        let mut i = 0;
        let mut x0 = 0;
        for &sw in &widths {
            for y in 0..height {
                let row_start = y.checked_mul(width).and_then(|s| s.checked_add(x0)).ok_or(RawError::Limit("CR2 row index overflow"))?;
                let row_end = row_start.checked_add(sw).ok_or(RawError::Limit("CR2 row index overflow"))?;
                data[row_start..row_end].copy_from_slice(&frame.data[i..i + sw]);
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
    let mut crop = Rect::new(0, 0, width, height);
    if let Some(si) = mn.as_ref().and_then(|m| m.ifd.u64s(SENSOR_INFO)).filter(|v| v.len() >= 9) {
        let coords = [si[5], si[6], si[7], si[8]].map(|v| usize::try_from(v).ok());
        if let [Some(l), Some(tp), Some(r), Some(b)] = coords
            && r > l
            && b > tp
            && r < width
            && b < height
        {
            let cw = r - l + 1;
            let ch = b - tp + 1;
            if cw <= width - l && ch <= height - tp {
                active = Rect::new(l, tp, cw, ch);
                // `AspectInfo` is measured from the image area's origin, so it only applies when that area is known.
                let image = Rect::new(0, 0, active.width, active.height);
                crop = crate::shared::aspect_crop(mn.as_ref().map(|m| &m.ifd), image).unwrap_or(image);
            }
        }
    }
    if active.width == 0 || active.height == 0 {
        active = Rect::new(0, 0, width, height);
    }
    if crop.width == 0 || crop.height == 0 {
        crop = Rect::new(0, 0, active.width, active.height);
    }
    let color_data = mn.as_ref().and_then(|m| m.ifd.value(COLOR_BALANCE).map(|v| words(v, m.order)));
    let kelvin_presets = color_data.as_deref().and_then(kelvin_presets_from_color_data);
    let wb = color_data.as_deref().and_then(wb_from_color_balance).or_else(|| wb_from_wb_info(mn.as_ref()?));
    let (white, white_source) = white_from_data_sourced(&active_rows(&data, width, active), prec as u32);
    let (black, black_source) = match masked_columns(&data, width, active, white) {
        cols if cols.len() >= 2 => black_from_columns_sourced(&data, width, cols, active.y..active.y + active.height, active),
        _ => (BlackLevel::uniform(0.0), crate::LevelSource::Fallback),
    };
    let model = mn.as_ref().and_then(|m| m.ifd.u64(CANON_MODEL_ID));
    let (cfa, assumed_cfa) = match tagged_cfa {
        Some(cfa) => (cfa, false),
        None => {
            let (cfa, guessed) = fallback_cfa(&data, width, active, black.mean(), white, model);
            (cfa, guessed || mode == Mode::Header)
        }
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
        crop,
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData { kelvin_presets, ..Default::default() },
        wb_multipliers: wb,
        linearized: false,
        linearization_table: None,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: black_source, white: white_source },
        notices: crate::RawNotices { assumed_cfa, ..Default::default() },
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
    fn absent_cfa_tag_uses_safe_phase_without_shifting_for_crop_parity() {
        let (w, h) = (192usize, 96usize);
        for red_row in [0usize, 1] {
            let samples: Vec<u16> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let texture = ((x / 2 + y / 2) % 17) as u16;
                    if i % 97 == 0 {
                        return 13000;
                    }
                    100 + if (y & 1) == red_row {
                        if x & 1 == 0 { 500 + texture } else { 1000 + texture }
                    } else if x & 1 == 0 {
                        1000 + texture
                    } else {
                        667 + texture
                    }
                })
                .collect();
            for (left, top) in [(16, 4), (17, 4), (16, 5), (17, 5)] {
                let area = Rect::new(left, top, w - left, h - top);
                let (cfa, assumed) = fallback_cfa(&samples, w, area, 100.0, 4000.0, None);
                assert_eq!(cfa.name(), "RGGB");
                assert!(assumed);
            }
            let mut bytes = synthetic_cr2_of(&samples, w, h, None, None);
            let marker = [0xe0, 0xc5, 4, 0, 1, 0, 0, 0];
            let at = bytes.windows(marker.len()).position(|b| b == marker).unwrap();
            // The explicit tag remains authoritative even when the observed sensor phase disagrees.
            bytes[at + 8..at + 12].copy_from_slice(&4u32.to_le_bytes());
            let explicit = crate::decode(&bytes).unwrap();
            assert_eq!(explicit.cfa.as_ref().unwrap().name(), "GRBG");
            assert_eq!(explicit.info(), crate::probe_info(&bytes).unwrap());
            assert!(!explicit.notices.assumed_cfa);
            bytes[at..at + 2].copy_from_slice(&0xc5dfu16.to_le_bytes());
            let full = crate::decode(&bytes).unwrap();
            assert_eq!(full.cfa.as_ref().unwrap().name(), "RGGB");
            assert!(full.notices.assumed_cfa);
            let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
            assert!(header.notices.assumed_cfa);
            assert!(matches!(header.data, RawData::U16(ref v) if v.is_empty()));
            assert_eq!(full.data, RawData::U16(samples));
        }
    }

    #[test]
    fn inconclusive_cfa_measurement_keeps_observed_model_fallback_and_safe_geometry() {
        let data = vec![512; 64 * 32];
        let area = Rect::new(0, 0, 64, 32);
        assert_eq!(fallback_cfa(&data, 64, area, 512.0, 16000.0, Some(0x8000_0250)).0.name(), "GBRG");
        assert!(fallback_cfa(&data, 64, area, 512.0, 16000.0, Some(0x8000_0250)).1);
        assert_eq!(fallback_cfa(&data, 64, area, 512.0, 16000.0, None).0.name(), "RGGB");
        assert!(fallback_cfa(&data, 64, area, 512.0, 16000.0, None).1);
        let gradient: Vec<u16> = (0..192 * 96)
            .map(|i| {
                let (x, y) = (i % 192, i / 192);
                if y % 2 == 0 {
                    if x % 2 == 0 { 500 } else { 1000 + 8 * x as u16 }
                } else if x % 2 == 0 {
                    1000 + 8 * x as u16
                } else {
                    500
                }
            })
            .collect();
        let (cfa, assumed) = fallback_cfa(&gradient, 192, Rect::new(0, 0, 192, 96), 0.0, 4000.0, None);
        assert_eq!(cfa.name(), "RGGB");
        assert!(assumed);
        let mut bytes = synthetic_cr2_of(&gradient, 192, 96, None, None);
        let marker = [0xe0, 0xc5, 4, 0, 1, 0, 0, 0];
        let at = bytes.windows(marker.len()).position(|b| b == marker).unwrap();
        // Exercise the decoder's missing-tag fallback, not the fixture's default CFA tag.
        bytes[at..at + 2].copy_from_slice(&0xc5dfu16.to_le_bytes());
        let decoded = crate::decode(&bytes).unwrap();
        assert_eq!(decoded.cfa.as_ref().unwrap().name(), "RGGB");
        assert!(decoded.notices.assumed_cfa);
    }

    #[test]
    fn model_cfa_fallback_is_disclosed_in_header_and_full_decodes() {
        // A single inline LONG maker-note entry needs no external offsets.
        let mut note = Vec::new();
        note.extend_from_slice(&1u16.to_le_bytes());
        note.extend_from_slice(&CANON_MODEL_ID.to_le_bytes());
        note.extend_from_slice(&4u16.to_le_bytes());
        note.extend_from_slice(&1u32.to_le_bytes());
        note.extend_from_slice(&0x8000_0250u32.to_le_bytes());
        note.extend_from_slice(&0u32.to_le_bytes());
        let samples = vec![512u16; 64 * 32];
        let mut bytes = synthetic_cr2_of(&samples, 64, 32, None, Some(note));
        let marker = [0xe0, 0xc5, 4, 0, 1, 0, 0, 0];
        let at = bytes.windows(marker.len()).position(|b| b == marker).unwrap();
        // Remove only the CFA tag; the model ID and encoded samples stay intact.
        bytes[at..at + 2].copy_from_slice(&0xc5dfu16.to_le_bytes());
        let header = decode(&bytes, Mode::Header, &Limits::default()).unwrap();
        let full = decode(&bytes, Mode::Full, &Limits::default()).unwrap();
        for image in [&header, &full] {
            assert_eq!(image.cfa.as_ref().unwrap().name(), "GBRG");
            assert!(image.notices.assumed_cfa);
        }
        assert_eq!(header.info(), full.info());
        assert_eq!(full.data, RawData::U16(samples));
        assert!(header.data.is_empty());
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
        decode_sized_with_note(64, 10, entries)
    }

    /// [`decode_with_note`] with a `w` x `h` sensor.
    fn decode_sized_with_note(w: usize, h: usize, entries: &[(u16, u16, Vec<u8>)]) -> RawImage {
        let ifd_len = 2 + 12 * entries.len() + 4;
        let (mut note, mut data, mut at) = (Vec::new(), Vec::new(), Vec::new());
        note.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, ty, value) in entries {
            let count = match ty {
                3 => value.len() / 2,
                4 => value.len() / 4,
                _ => value.len(),
            } as u32;
            note.extend_from_slice(&tag.to_le_bytes());
            note.extend_from_slice(&ty.to_le_bytes());
            note.extend_from_slice(&count.to_le_bytes());
            at.push((note.len(), ifd_len + data.len()));
            note.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(value);
        }
        note.extend_from_slice(&0u32.to_le_bytes());
        note.extend_from_slice(&data);
        let (mut bytes, _) = synthetic_cr2_note(w, h, None, Some(note));
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
        for ((at, temp_at), &(r, b, k)) in preset_rows(version).first().unwrap().iter().copied().zip(presets) {
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
        for version in [1, 2, 3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15] {
            assert!(kelvin_presets_from_color_data(&color_data_with_presets(version, &eos)).is_some(), "{version}");
        }
        // version 10 has two layouts: the one whose Daylight record says 5200 K is read
        let mut b = vec![0u64; 400];
        b[0] = 10;
        for (&(at, temp_at), &(r, bl, k)) in preset_rows(10)[1].iter().zip(&eos) {
            b[at..at + 4].copy_from_slice(&[r.into(), 1024, 1024, bl.into()]);
            b[temp_at] = k.into();
        }
        assert!(kelvin_presets_from_color_data(&b).is_some(), "10, second layout");
        b[132] = 5000;
        assert!(kelvin_presets_from_color_data(&b).is_none(), "10, neither layout names Daylight at 5200 K");
        assert!(kelvin_presets_from_color_data(&[10]).is_none());
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
                    if i == 0 { [0xfffc, 1, 10, 13, 14, 15][(seed % 6) as usize] } else { seed % 70000 }
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

    /// A 68 x 50 sensor whose `SensorInfo` image area is the 64 x 48 (4:3) at (4, 2), with `aspect` as its
    /// `AspectInfo` (LONG x 5), decoded.
    fn decode_with_aspect(aspect: Option<[u32; 5]>) -> RawImage {
        let si: Vec<u8> = [68u16, 50, 0, 0, 0, 4, 2, 67, 49].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut entries = vec![(SENSOR_INFO, 3, si)];
        if let Some(a) = aspect {
            entries.push((crate::shared::ASPECT_INFO, 4, a.iter().flat_map(|v| v.to_le_bytes()).collect()));
        }
        decode_sized_with_note(68, 50, &entries)
    }

    /// storytold/lightcraft#3313: a CR2 shot at 3:2 on a 4:3 sensor (PowerShot G12: `AspectInfo` 3648 x 2432 at
    /// top 152 of the 3648 x 2736 image area) opens with that rectangle as its crop, as a CR3 does; the active
    /// area is unchanged.
    #[test]
    fn aspect_info_replaces_the_crop() {
        let image = decode_with_aspect(Some([0, 64, 42, 0, 3]));
        assert_eq!(image.active_area, Rect::new(4, 2, 64, 48));
        assert_eq!(image.crop, Rect::new(0, 3, 64, 42));
        assert_eq!(image.info().developed_size(), (64, 42));
        // 1:1 with a left offset
        assert_eq!(decode_with_aspect(Some([1, 48, 48, 8, 0])).crop, Rect::new(8, 0, 48, 48));
        // the whole image area, as stored for a shot at the sensor's own aspect (G16 at 4:3), or no tag: unchanged
        for aspect in [Some([2, 64, 48, 0, 0]), None] {
            assert_eq!(decode_with_aspect(aspect).crop, Rect::new(0, 0, 64, 48), "{aspect:?}");
        }
    }

    /// Rectangles that don't fit the image area leave the crop alone and cannot overflow; without a usable
    /// `SensorInfo` the origin `AspectInfo` is measured from is unknown, so it isn't applied either.
    #[test]
    fn aspect_info_that_does_not_fit_is_ignored() {
        for bad in [[0, 65, 42, 0, 3], [0, 64, 42, 0, 7], [0, 64, 42, 1, 3], [0, 1, 42, 0, 0], [0, 64, 0, 0, 0], [0, u32::MAX, 42, 0, 0]] {
            assert_eq!(decode_with_aspect(Some(bad)).crop, Rect::new(0, 0, 64, 48), "{bad:?}");
        }
        let aspect = [0u32, 32, 8, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
        let image = decode_with_note(&[(crate::shared::ASPECT_INFO, 4, aspect)]);
        assert_eq!((image.active_area, image.crop), (Rect::new(0, 0, 64, 10), Rect::new(0, 0, 64, 10)));
    }

    /// Active sensor area dimensions that are missing, zero-sized or inverted fall back to the full sensor frame.
    #[test]
    fn active_area_fallback_on_missing_or_corrupt_sensor_info() {
        let (w, h) = (68, 50);
        // Zero-sized: right == left
        let si_zero: Vec<u8> = [68u16, 50, 0, 0, 0, 4, 2, 4, 49].iter().flat_map(|v| v.to_le_bytes()).collect();
        let img = decode_sized_with_note(w, h, &[(SENSOR_INFO, 3, si_zero)]);
        assert_eq!((img.active_area, img.crop), (Rect::new(0, 0, w, h), Rect::new(0, 0, w, h)));

        // Inverted borders: right < left
        let si_inv: Vec<u8> = [68u16, 50, 0, 0, 0, 10, 2, 4, 49].iter().flat_map(|v| v.to_le_bytes()).collect();
        let img = decode_sized_with_note(w, h, &[(SENSOR_INFO, 3, si_inv)]);
        assert_eq!((img.active_area, img.crop), (Rect::new(0, 0, w, h), Rect::new(0, 0, w, h)));

        // Without SensorInfo: falls back to full sensor frame
        let img = decode_sized_with_note(w, h, &[]);
        assert_eq!((img.active_area, img.crop), (Rect::new(0, 0, w, h), Rect::new(0, 0, w, h)));
    }
}
