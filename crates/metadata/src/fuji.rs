//! Fujifilm in-camera film recipe, read from the MakerNote.
//!
//! Tag numbers and value names follow the public [ExifTool FujiFilm table](https://exiftool.org/TagNames/FujiFilm.html).
//! No third-party metadata code was consulted. This does not
//! reproduce the camera JPEG or recover a custom recipe name: the file does not store one.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use craft_tiff::makernote::{MakerNoteKind, parse_makernote};
use craft_tiff::{Ifd, Tiff};
use serde::{Deserialize, Serialize};

use crate::strip_exif_header;

/// One labelled recipe row, in display order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FujiRow {
    /// MakerNote tag this row came from (the first, when a row joins two tags).
    pub tag: u16,
    pub label: String,
    pub value: String,
}

/// Settings recovered from a Fujifilm MakerNote. Empty means the note had none of these tags.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FujiRecipe {
    pub rows: Vec<FujiRow>,
}

impl FujiRecipe {
    /// Plain text for the clipboard, one `Label: value` line per row.
    pub fn text(&self) -> String {
        self.rows.iter().map(|r| format!("{}: {}", r.label, r.value)).collect::<Vec<_>>().join("\n")
    }

    fn push(&mut self, tag: u16, label: &str, value: String) {
        self.rows.push(FujiRow { tag, label: label.to_string(), value });
    }
}

/// Recipe in a file. A RAF contributes only the EXIF segment of its embedded JPEG, not the preview
/// and not the sensor data. A JPEG contributes only its EXIF segment. A TIFF, including a DNG, is
/// read only far enough to reach a Fujifilm MakerNote; a different make stops after a short sniff.
/// Anything else stops after the header. `Ok(None)` when the file has no Fujifilm recipe. A header
/// that claims to go on past the cap is also `Ok(None)`: retrying it would read the same bytes again.
pub fn read_recipe_file(path: &Path) -> Result<Option<FujiRecipe>, String> {
    let bytes = read_for_recipe(path)?;
    Ok(recipe_from_bytes(&bytes))
}

/// Whether a photo's file can hold a Fujifilm recipe. A known other make is skipped. The camera
/// string is empty on catalogs from before the make was stored, so JPEG and TIFF names are still
/// candidates; other raw extensions are not.
pub fn might_hold_fuji_recipe(camera: &str, format: &str, file_name: &str) -> bool {
    if kind_is(format, &["raf"]) || kind_is(file_name, &["raf"]) {
        return true;
    }
    if mentions_fuji(camera) {
        return true;
    }
    if !camera.trim().is_empty() {
        return false;
    }
    kind_is(format, &["jpg", "jpeg", "jpe", "tif", "tiff"]) || kind_is(file_name, &["jpg", "jpeg", "jpe", "tif", "tiff"])
}

fn kind_is(value: &str, opts: &[&str]) -> bool {
    let bare = value.rsplit(['/', '\\', '.']).next().unwrap_or("");
    opts.iter().any(|opt| bare.eq_ignore_ascii_case(opt))
}

fn mentions_fuji(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    if upper.contains("FUJIFILM") || upper.contains("FUJI PHOTO") || upper.contains("FUJI FILM") {
        return true;
    }
    upper.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| word == "FUJI")
}

/// Recipe in a JPEG, RAF, or TIFF-structured EXIF block already in memory.
pub fn recipe_from_bytes(bytes: &[u8]) -> Option<FujiRecipe> {
    if Tiff::sniff(bytes).is_some() {
        return recipe_from_exif(bytes);
    }
    let exif = crate::embedded(bytes).exif?;
    recipe_from_exif(&exif)
}

/// Recipe in a TIFF-structured EXIF block (with or without an `Exif\0\0` prefix).
pub fn recipe_from_exif(bytes: &[u8]) -> Option<FujiRecipe> {
    let bytes = strip_exif_header(bytes);
    let tiff = Tiff::parse(bytes).ok()?;
    let ifd0 = tiff.ifds.first()?;
    let exif = ifd0.exif.as_ref()?;
    let entry = exif.get(0x927c)?;
    let make = ifd0.string(0x010f).unwrap_or_default();
    let note = parse_makernote(bytes, entry.offset, entry.count() as u64, tiff.order, &make)?;
    if note.kind != MakerNoteKind::Fujifilm {
        return None;
    }
    let recipe = decode(&note.ifd);
    (!recipe.rows.is_empty()).then_some(recipe)
}

const HEADER_BYTES: usize = 92;
/// EXIF and the segments before it. One JPEG segment is at most 64 KiB; this stops a run of them.
const JPEG_META_MAX: usize = 8 * 1024 * 1024;
const TIFF_SNIFF: usize = 64 * 1024;
const TIFF_META_MAX: usize = 4 * 1024 * 1024;
const RAF_JPEG_MAX: u64 = 64 * 1024 * 1024;

fn read_for_recipe(path: &Path) -> Result<Vec<u8>, String> {
    let shown = path.display().to_string();
    let file = File::open(path).map_err(|e| format!("{shown}: {e}"))?;
    recipe_window(file, &shown)
}

/// The bytes a recipe can live in.
fn recipe_window<R: Read + Seek>(mut file: R, shown: &str) -> Result<Vec<u8>, String> {
    let mut head = Vec::with_capacity(HEADER_BYTES);
    fill(&mut file, &mut head, HEADER_BYTES, HEADER_BYTES, shown)?;
    let n = head.len();
    let got = head.as_slice();
    if got.starts_with(b"FUJIFILMCCD-RAW") {
        return raf_embedded_jpeg(&mut file, &head, n, shown);
    }
    if n >= 2 && got.first() == Some(&0xff) && got.get(1) == Some(&0xd8) {
        return jpeg_exif_window(&mut file, got, shown);
    }
    if tiff_magic(got) {
        return tiff_prefix(&mut file, got, shown);
    }
    Ok(Vec::new())
}

fn tiff_magic(head: &[u8]) -> bool {
    head.starts_with(b"II*\0") || head.starts_with(b"MM\0*")
}

fn raf_embedded_jpeg<R: Read + Seek>(file: &mut R, head: &[u8], n: usize, shown: &str) -> Result<Vec<u8>, String> {
    if n < HEADER_BYTES {
        return Err(format!("{shown}: truncated RAF header"));
    }
    let off = u64::from(be_u32(head, 84).ok_or_else(|| format!("{shown}: truncated RAF header"))?);
    let len = u64::from(be_u32(head, 88).ok_or_else(|| format!("{shown}: truncated RAF header"))?);
    if off == 0 || len == 0 || len > RAF_JPEG_MAX {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(off)).map_err(|e| format!("{shown}: {e}"))?;
    // the length stops the read at the embedded JPEG: bytes after it are the raw image
    let mut jpeg = file.take(len);
    let mut jhead = Vec::with_capacity(HEADER_BYTES);
    fill(&mut jpeg, &mut jhead, HEADER_BYTES, HEADER_BYTES, shown)?;
    let jn = jhead.len();
    let got = jhead.as_slice();
    if jn < 2 || got.first() != Some(&0xff) || got.get(1) != Some(&0xd8) {
        return Ok(Vec::new());
    }
    jpeg_exif_window(&mut jpeg, got, shown)
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    let slice = bytes.get(at..end)?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(slice);
    Some(u32::from_be_bytes(buf))
}

/// SOI plus the first EXIF APP1 segment plus EOI. Stops at the start of scan. A segment that
/// would run past [`JPEG_META_MAX`] is treated as no recipe, so the panel does not open the file
/// again on a timer.
fn jpeg_exif_window<R: Read>(file: &mut R, head: &[u8], shown: &str) -> Result<Vec<u8>, String> {
    let mut buf = head.to_vec();
    let mut i = 2usize;
    while i < JPEG_META_MAX {
        let Some(marker_at) = i.checked_add(1) else { break };
        let need = marker_at.saturating_add(3);
        fill(file, &mut buf, need, JPEG_META_MAX, shown)?;
        let Some(marker0) = buf.get(i).copied() else { break };
        if marker0 != 0xff {
            break;
        }
        let Some(marker) = buf.get(marker_at).copied() else { break };
        if marker == 0xff {
            i = marker_at;
            continue;
        }
        // standalone markers: no length (fill bytes, restart, TEM)
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i = marker_at.saturating_add(1);
            continue;
        }
        // start of scan, or end of image: the recipe is not in the compressed data
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let Some(len_at) = marker_at.checked_add(1) else { break };
        let Some(len_end) = len_at.checked_add(1) else { break };
        let (Some(hi), Some(lo)) = (buf.get(len_at).copied(), buf.get(len_end).copied()) else { break };
        let len = usize::from(u16::from_be_bytes([hi, lo]));
        if len < 2 {
            break;
        }
        let Some(seg_start) = i.checked_add(2) else { break };
        let Some(end) = seg_start.checked_add(len) else { break };
        if end > JPEG_META_MAX {
            return Ok(Vec::new());
        }
        fill(file, &mut buf, end, JPEG_META_MAX, shown)?;
        if buf.len() < end {
            break;
        }
        let payload = buf.get(seg_start.saturating_add(2)..end).unwrap_or(&[]);
        if marker == 0xe1 && payload.starts_with(b"Exif\0") {
            let Some(segment) = buf.get(i..end) else { break };
            let mut out = Vec::with_capacity(segment.len().saturating_add(4));
            out.extend_from_slice(&[0xff, 0xd8]);
            out.extend_from_slice(segment);
            out.extend_from_slice(&[0xff, 0xd9]);
            return Ok(out);
        }
        i = end;
    }
    Ok(Vec::new())
}

enum TiffMake {
    Fuji,
    Other,
    /// No Make yet, or a truncated IFD: the bounded prefix may need more bytes.
    Unknown,
    /// Not a TIFF we can read from this prefix. Reading further does not help.
    Unreadable,
}

fn tiff_prefix<R: Read>(file: &mut R, head: &[u8], shown: &str) -> Result<Vec<u8>, String> {
    let mut buf = head.to_vec();
    fill(file, &mut buf, TIFF_SNIFF, TIFF_META_MAX, shown)?;
    if recipe_from_bytes(&buf).is_some() {
        return Ok(buf);
    }
    if matches!(tiff_make(&buf), TiffMake::Other | TiffMake::Unreadable) {
        return Ok(Vec::new());
    }
    fill(file, &mut buf, TIFF_META_MAX, TIFF_META_MAX, shown)?;
    Ok(buf)
}

fn tiff_make(bytes: &[u8]) -> TiffMake {
    let tiff = match Tiff::parse(bytes) {
        Ok(tiff) => tiff,
        Err(craft_tiff::TiffError::Truncated(offset)) if offset < TIFF_META_MAX as u64 => return TiffMake::Unknown,
        Err(_) => return TiffMake::Unreadable,
    };
    match tiff.ifds.first().and_then(|ifd| ifd.string(0x010f)) {
        Some(make) if mentions_fuji(&make) => TiffMake::Fuji,
        Some(_) => TiffMake::Other,
        None => TiffMake::Unknown,
    }
}

fn fill<R: Read>(file: &mut R, buf: &mut Vec<u8>, need: usize, cap: usize, shown: &str) -> Result<(), String> {
    let need = need.min(cap);
    while buf.len() < need {
        let mut tmp = [0u8; 8192];
        let want = (need - buf.len()).min(tmp.len());
        let Some(dest) = tmp.get_mut(..want) else { break };
        let n = match file.read(dest) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|e| format!("{shown}: {e}"))?,
        };
        if n == 0 {
            break;
        }
        let Some(chunk) = dest.get(..n) else { break };
        buf.extend_from_slice(chunk);
    }
    Ok(())
}

fn decode(ifd: &Ifd) -> FujiRecipe {
    let mut out = FujiRecipe::default();
    let film = ifd.u16(0x1401).map(film_mode);
    let filter = ifd.u16(0x1003).and_then(bw_filter);
    match (film, filter) {
        (Some(film), Some(filter)) => out.push(0x1401, "Film simulation", format!("{film}, {filter}")),
        (Some(film), None) => out.push(0x1401, "Film simulation", film),
        (None, Some(filter)) => out.push(0x1003, "Film simulation", filter.to_string()),
        (None, None) => {}
    }
    if let Some(mode) = ifd.u16(0x1443) {
        let strength = match mode {
            // X-H1 files can put the strength in the other mode's tag.
            0 => ifd.u16(0x1444).or_else(|| ifd.u16(0x1445)).map(|v| named(v, &[(1, "Weak"), (2, "Strong"), (3, "Plus")])),
            1 => ifd.u16(0x1445).or_else(|| ifd.u16(0x1444)).map(|v| named(v, &[(1, "Weak"), (2, "Strong")])),
            _ => None,
        };
        let priority = match (mode, strength) {
            (0, Some(strength)) => format!("Auto ({strength})"),
            (1, Some(strength)) => strength,
            _ => named(mode, &[(0, "Auto"), (1, "Fixed")]),
        };
        out.push(0x1443, "D-Range priority", priority);
    } else if ifd.u16(0x1402) == Some(1)
        && let Some(dr) = ifd.u16(0x1403)
    {
        out.push(0x1403, "Dynamic range", named(dr, &[(100, "DR100"), (200, "DR200"), (400, "DR400")]));
    } else if let Some(dr) = ifd.u16(0x1402).map(dynamic_range_setting).or_else(|| ifd.u16(0x1400).map(dynamic_range_legacy)) {
        let tag = if ifd.contains(0x1402) { 0x1402 } else { 0x1400 };
        out.push(tag, "Dynamic range", dr);
    }
    if let Some(code) = ifd.u16(0x1002) {
        out.push(0x1002, "White balance", white_balance(code, ifd.i64(0x1005)));
    }
    if let Some(v) = ifd.value(0x100a)
        && let (Some(red), Some(blue)) = (v.get_i64(0), v.get_i64(1))
    {
        let (red, blue) = scale_shift(red, blue);
        out.push(0x100a, "White balance shift", format!("R {} / B {}", signed(red), signed(blue)));
    }
    let grain = match (ifd.i64(0x1047).map(grain_roughness), ifd.u16(0x104c).map(grain_size)) {
        (Some(rough), Some(size)) => Some((0x1047_u16, format!("{rough} / {size}"))),
        (Some(rough), None) => Some((0x1047, rough)),
        (None, Some(size)) => Some((0x104c, size)),
        (None, None) => None,
    };
    if let Some((tag, value)) = grain {
        out.push(tag, "Grain", value);
    }
    match (ifd.i64(0x1041).map(tone_dial), ifd.i64(0x1040).map(tone_dial)) {
        (Some(high), Some(shadow)) => out.push(0x1041, "Highlight / Shadow", format!("{high} / {shadow}")),
        (Some(high), None) => out.push(0x1041, "Highlight", high),
        (None, Some(shadow)) => out.push(0x1040, "Shadow", shadow),
        (None, None) => {}
    }
    if let Some(v) = ifd.i64(0x1048) {
        out.push(0x1048, "Color Chrome", chrome(v));
    }
    if let Some(v) = ifd.i64(0x104e) {
        out.push(0x104e, "Color Chrome FX Blue", chrome(v));
    }
    if let Some(v) = ifd.u16(0x1001) {
        out.push(0x1001, "Sharpness", sharpness(v));
    }
    if let Some(v) = ifd.u16(0x1003).filter(|v| bw_filter(*v).is_none()) {
        out.push(0x1003, "Color", color_step(v));
    }
    if let Some(v) = ifd.u16(0x100e).map(noise_reduction).or_else(|| ifd.u16(0x100b).map(noise_reduction_legacy)) {
        let tag = if ifd.contains(0x100e) { 0x100e } else { 0x100b };
        out.push(tag, "Noise reduction", v);
    }
    if let Some(v) = ifd.i64(0x100f) {
        out.push(0x100f, "Clarity", clarity(v));
    }
    out
}

fn named(v: u16, table: &[(u16, &str)]) -> String {
    table.iter().find(|(k, _)| *k == v).map(|(_, n)| (*n).to_string()).unwrap_or_else(|| format!("Unknown ({v})"))
}

fn film_mode(v: u16) -> String {
    named(
        v,
        &[
            (0x000, "F0/Standard (Provia)"),
            (0x100, "F1/Studio Portrait"),
            (0x110, "F1a/Studio Portrait Enhanced Saturation"),
            (0x120, "F1b/Studio Portrait Smooth Skin Tone (Astia)"),
            (0x130, "F1c/Studio Portrait Increased Sharpness"),
            (0x200, "Velvia/Vivid"),
            (0x300, "F3/Studio Portrait Ex"),
            (0x400, "F4/Velvia"),
            (0x500, "Pro Neg. Std"),
            (0x501, "Pro Neg. Hi"),
            (0x600, "Classic Chrome"),
            (0x700, "Eterna"),
            (0x800, "Classic Negative"),
            (0x900, "Bleach Bypass"),
            (0xa00, "Nostalgic Neg"),
            (0xb00, "Reala ACE"),
        ],
    )
}

fn bw_filter(v: u16) -> Option<&'static str> {
    Some(match v {
        0x300 => "B&W",
        0x301 => "B&W Red Filter",
        0x302 => "B&W Yellow Filter",
        0x303 => "B&W Green Filter",
        0x310 => "B&W Sepia",
        0x500 => "Acros",
        0x501 => "Acros Red Filter",
        0x502 => "Acros Yellow Filter",
        0x503 => "Acros Green Filter",
        _ => return None,
    })
}

fn dynamic_range_setting(v: u16) -> String {
    if v == 0x8000 {
        return "As film simulation".to_string();
    }
    named(v, &[(0x0, "Auto"), (0x1, "Manual"), (0x100, "DR100"), (0x200, "DR200"), (0x201, "DR400")])
}

fn dynamic_range_legacy(v: u16) -> String {
    named(v, &[(1, "DR100"), (3, "Wide")])
}

fn white_balance(code: u16, kelvin: Option<i64>) -> String {
    if code == 0xff0 {
        return match kelvin.filter(|k| *k > 0) {
            Some(k) => format!("Kelvin ({k} K)"),
            None => "Kelvin".to_string(),
        };
    }
    named(
        code,
        &[
            (0x0, "Auto"),
            (0x1, "Auto (white priority)"),
            (0x2, "Auto (ambiance priority)"),
            (0x100, "Daylight"),
            (0x200, "Cloudy"),
            (0x300, "Daylight Fluorescent"),
            (0x301, "Day White Fluorescent"),
            (0x302, "White Fluorescent"),
            (0x303, "Warm White Fluorescent"),
            (0x304, "Living Room Warm White Fluorescent"),
            (0x400, "Incandescent"),
            (0x500, "Flash"),
            (0x600, "Underwater"),
            (0xf00, "Custom"),
            (0xf01, "Custom 2"),
            (0xf02, "Custom 3"),
            (0xf03, "Custom 4"),
            (0xf04, "Custom 5"),
        ],
    )
}

/// Newer cameras store the shift times 20. Older cameras store about `-9..=9`. Divide only when
/// the pair is on the newer scale and both components are multiples of 20; otherwise keep the
/// stored pair so a mixed value is not shown as a wrong shift.
fn scale_shift(red: i64, blue: i64) -> (i64, i64) {
    let wide = red.unsigned_abs() > 9 || blue.unsigned_abs() > 9;
    if wide && red % 20 == 0 && blue % 20 == 0 { (red / 20, blue / 20) } else { (red, blue) }
}

/// Highlight and shadow use 16 stored units per dial step, with the opposite sign
/// (stored −16 is +1, stored 16 is −1). Half steps are multiples of 8, so stored 24 is −1.5.
/// Values that are not on that grid, or outside the camera's −4..=+4 dial, stay unknown.
fn tone_dial(v: i64) -> String {
    if v % 8 != 0 {
        return format!("Unknown ({v})");
    }
    let Some(half_steps) = v.checked_div(8).and_then(i64::checked_neg) else {
        return format!("Unknown ({v})");
    };
    if half_steps.unsigned_abs() > 8 {
        return format!("Unknown ({v})");
    }
    if half_steps % 2 == 0 {
        return signed(half_steps / 2);
    }
    let sign = if half_steps > 0 { "+" } else { "−" };
    format!("{sign}{}.5", half_steps.unsigned_abs() / 2)
}

fn grain_roughness(v: i64) -> String {
    match v {
        0 => "Off".to_string(),
        32 => "Weak".to_string(),
        64 => "Strong".to_string(),
        other => format!("Unknown ({other})"),
    }
}

fn grain_size(v: u16) -> String {
    match v {
        0 => "Off".to_string(),
        16 => "Small".to_string(),
        32 => "Large".to_string(),
        other => format!("Unknown ({other})"),
    }
}

fn chrome(v: i64) -> String {
    match v {
        0 => "Off".to_string(),
        32 => "Weak".to_string(),
        64 => "Strong".to_string(),
        other => format!("Unknown ({other})"),
    }
}

fn sharpness(v: u16) -> String {
    if v == 0x8000 {
        return "As film simulation".to_string();
    }
    match v {
        0x0 => signed(-4),
        0x1 => signed(-3),
        0x2 => signed(-2),
        0x3 => "0".to_string(),
        0x4 => "+2".to_string(),
        0x5 => "+3".to_string(),
        0x6 => "+4".to_string(),
        0x82 => signed(-1),
        0x84 => "+1".to_string(),
        0xffff => "n/a".to_string(),
        other => format!("Unknown ({other})"),
    }
}

fn color_step(v: u16) -> String {
    if v == 0x8000 {
        return "As film simulation".to_string();
    }
    match v {
        0x0 => "0".to_string(),
        0x80 => "+1".to_string(),
        0xc0 => "+3".to_string(),
        0xe0 => "+4".to_string(),
        0x100 => "+2".to_string(),
        0x180 => signed(-1),
        0x200 => "Low".to_string(),
        0x400 => signed(-2),
        0x4c0 => signed(-3),
        0x4e0 => signed(-4),
        other => format!("Unknown ({other})"),
    }
}

fn noise_reduction(v: u16) -> String {
    match v {
        0x0 => "0".to_string(),
        0x100 => "+2".to_string(),
        0x180 => "+1".to_string(),
        0x1c0 => "+3".to_string(),
        0x1e0 => "+4".to_string(),
        0x200 => signed(-2),
        0x280 => signed(-1),
        0x2c0 => signed(-3),
        0x2e0 => signed(-4),
        other => format!("Unknown ({other})"),
    }
}

fn noise_reduction_legacy(v: u16) -> String {
    named(v, &[(0x40, "Low"), (0x80, "Normal"), (0x100, "n/a")])
}

fn clarity(v: i64) -> String {
    if v % 1000 == 0 && (-5000..=5000).contains(&v) { signed(v / 1000) } else { format!("Unknown ({v})") }
}

fn signed(v: i64) -> String {
    if v > 0 {
        format!("+{v}")
    } else if v < 0 {
        format!("−{}", v.unsigned_abs())
    } else {
        "0".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::tags;
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};

    fn inline(tag: u16, ty: u16, count: u32, val: [u8; 4]) -> [u8; 12] {
        let mut e = [0u8; 12];
        e[0..2].copy_from_slice(&tag.to_le_bytes());
        e[2..4].copy_from_slice(&ty.to_le_bytes());
        e[4..8].copy_from_slice(&count.to_le_bytes());
        e[8..12].copy_from_slice(&val);
        e
    }

    fn i32_inline(tag: u16, v: i32) -> [u8; 12] {
        inline(tag, 9, 1, v.to_le_bytes())
    }

    fn u16_inline(tag: u16, v: u16) -> [u8; 12] {
        let mut val = [0u8; 4];
        val[..2].copy_from_slice(&v.to_le_bytes());
        inline(tag, 3, 1, val)
    }

    /// A Fujifilm note: little-endian IFD at byte 12, the white-balance shift stored out of line.
    fn fuji_note(film: u16, shift: [i32; 2]) -> Vec<u8> {
        let mut entries = vec![
            inline(0x100a, 9, 2, [0, 0, 0, 0]),
            i32_inline(0x1040, -32),
            i32_inline(0x1041, -16),
            i32_inline(0x1047, 32),
            i32_inline(0x1048, 64),
            u16_inline(0x104c, 16),
            u16_inline(0x1401, film),
            u16_inline(0x1402, 0x200),
        ];
        let ifd_len = 2 + entries.len() * 12 + 4;
        let data_at = 12 + ifd_len;
        entries[0][8..12].copy_from_slice(&(data_at as u32).to_le_bytes());
        let mut note = b"FUJIFILM".to_vec();
        note.extend_from_slice(&12u32.to_le_bytes());
        note.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for e in &entries {
            note.extend_from_slice(e);
        }
        note.extend_from_slice(&0u32.to_le_bytes());
        note.extend_from_slice(&shift[0].to_le_bytes());
        note.extend_from_slice(&shift[1].to_le_bytes());
        note
    }

    fn exif_with_note(note: Vec<u8>) -> Vec<u8> {
        let mut exif = IfdBuilder::new();
        exif.set(tags::MAKER_NOTE, Value::Undefined(note));
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(tags::MAKE, Value::Ascii("FUJIFILM".into()));
        ifd0.set(tags::MODEL, Value::Ascii("X-S10".into()));
        ifd0.set_child(tags::EXIF_IFD, exif);
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
    }

    fn jpeg_with_exif(exif: &[u8]) -> Vec<u8> {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(exif);
        let len = (payload.len() + 2) as u16;
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&len.to_be_bytes());
        jpeg.extend_from_slice(&payload);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }

    fn sample_jpeg() -> Vec<u8> {
        jpeg_with_exif(&exif_with_note(fuji_note(0x600, [40, -80])))
    }

    fn sample_raf(tail: usize) -> Vec<u8> {
        let jpeg = sample_jpeg();
        let mut raf = b"FUJIFILMCCD-RAW ".to_vec();
        raf.resize(120, 0);
        raf[84..88].copy_from_slice(&120u32.to_be_bytes());
        raf[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
        raf.extend_from_slice(&jpeg);
        raf.extend(std::iter::repeat_n(0x33, tail));
        raf
    }

    fn value<'a>(recipe: &'a FujiRecipe, label: &str) -> &'a str {
        recipe.rows.iter().find(|r| r.label == label).map(|r| r.value.as_str()).unwrap_or("")
    }

    #[test]
    fn tone_half_steps() {
        assert_eq!(tone_dial(-16), "+1");
        assert_eq!(tone_dial(-32), "+2");
        assert_eq!(tone_dial(16), "−1");
        assert_eq!(tone_dial(24), "−1.5");
        assert_eq!(tone_dial(-8), "+0.5");
        assert_eq!(tone_dial(0), "0");
        assert_eq!(tone_dial(7), "Unknown (7)");
    }

    #[test]
    fn classic_chrome_recipe_from_jpeg_and_raf() {
        for bytes in [sample_jpeg(), sample_raf(64), exif_with_note(fuji_note(0x600, [40, -80]))] {
            let recipe = recipe_from_bytes(&bytes).unwrap();
            assert_eq!(value(&recipe, "Film simulation"), "Classic Chrome");
            assert_eq!(value(&recipe, "Dynamic range"), "DR200");
            // Public table: stored −16 is dial +1, stored −32 is dial +2.
            assert_eq!(value(&recipe, "Highlight / Shadow"), "+1 / +2");
            assert_eq!(value(&recipe, "Grain"), "Weak / Small");
            assert_eq!(value(&recipe, "Color Chrome"), "Strong");
            assert_eq!(value(&recipe, "White balance shift"), "R +2 / B −4");
            let text = recipe.text();
            assert!(text.contains("Film simulation: Classic Chrome"), "{text}");
            assert!(crate::file_tag_rows(&bytes).iter().any(|r| r.group == "Fujifilm" && r.value == "Classic Chrome"));
            assert!(crate::file_tag_rows(&bytes).iter().any(|r| r.name == "Maker Note"));
            let rows = crate::file_tag_rows(&bytes);
            let fuji_rows: Vec<_> = rows.iter().filter(|r| r.group == "Fujifilm").collect();
            assert_eq!(fuji_rows.len(), recipe.rows.len());
            for (actual, expected) in fuji_rows.iter().zip(&recipe.rows) {
                assert_eq!((actual.tag, &actual.name, &actual.value), (expected.tag, &expected.label, &expected.value));
            }
        }
        assert_eq!(crate::extract(&sample_jpeg()).fuji_recipe.as_ref().map(|r| value(r, "Dynamic range")), Some("DR200"));
    }

    #[test]
    fn old_shift_scale_is_not_divided() {
        let recipe = recipe_from_exif(&exif_with_note(fuji_note(0x600, [2, -4]))).unwrap();
        assert_eq!(value(&recipe, "White balance shift"), "R +2 / B −4");
    }

    #[test]
    fn xs10_velvia_uses_the_camera_menu_name() {
        let recipe = recipe_from_exif(&exif_with_note(fuji_note(0x200, [0, 0]))).unwrap();
        assert_eq!(value(&recipe, "Film simulation"), "Velvia/Vivid");
    }

    fn recipe_with_tags(tags: &[(u16, u16)]) -> (FujiRecipe, Vec<u8>) {
        let mut note = b"FUJIFILM".to_vec();
        note.extend_from_slice(&12u32.to_le_bytes());
        note.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for &(tag, code) in tags {
            note.extend_from_slice(&u16_inline(tag, code));
        }
        note.extend_from_slice(&0u32.to_le_bytes());
        let exif = exif_with_note(note);
        (recipe_from_exif(&exif).unwrap(), exif)
    }

    #[test]
    fn manual_dynamic_range_uses_the_development_percentage() {
        for (percentage, expected) in [(100, "DR100"), (200, "DR200"), (400, "DR400"), (123, "Unknown (123)")] {
            let (recipe, exif) = recipe_with_tags(&[(0x1400, 1), (0x1402, 1), (0x1403, percentage)]);
            assert_eq!(value(&recipe, "Dynamic range"), expected);
            assert_eq!(recipe.rows[0].tag, 0x1403);
            let rows = crate::file_tag_rows(&jpeg_with_exif(&exif));
            assert!(rows.iter().any(|r| r.group == "Fujifilm" && r.name == "Dynamic range" && r.value == expected));
        }
        let (recipe, _) = recipe_with_tags(&[(0x1402, 1)]);
        assert_eq!(value(&recipe, "Dynamic range"), "Manual");
        let (recipe, _) = recipe_with_tags(&[(0x1402, 0), (0x1403, 200)]);
        assert_eq!(value(&recipe, "Dynamic range"), "Auto");
    }

    #[test]
    fn priority_suppresses_legacy_dynamic_range_and_reads_camera_strength() {
        for (tags, expected) in [
            (vec![(0x1443, 1), (0x1444, 1)], "Weak"),
            (vec![(0x1443, 1), (0x1444, 2)], "Strong"),
            (vec![(0x1443, 1), (0x1444, 1), (0x1445, 2)], "Strong"),
            (vec![(0x1443, 0), (0x1444, 1), (0x1445, 2)], "Auto (Weak)"),
            (vec![(0x1443, 0), (0x1445, 1)], "Auto (Weak)"),
            (vec![(0x1443, 0), (0x1444, 3)], "Auto (Plus)"),
            (vec![(0x1443, 0)], "Auto"),
            (vec![(0x1443, 1)], "Fixed"),
            (vec![(0x1443, 1), (0x1445, 42)], "Unknown (42)"),
            (vec![(0x1443, 42)], "Unknown (42)"),
        ] {
            let mut tags = tags;
            tags.extend_from_slice(&[(0x1400, 1), (0x1402, 1), (0x1403, 100)]);
            tags.sort_unstable();
            let (recipe, _) = recipe_with_tags(&tags);
            assert_eq!(value(&recipe, "D-Range priority"), expected);
            assert_eq!(value(&recipe, "Dynamic range"), "");
            assert!(recipe.text().contains(&format!("D-Range priority: {expected}")));
        }
    }

    #[test]
    fn monochrome_variants_need_no_film_mode_tag() {
        for (code, name) in [
            (0x300, "B&W"),
            (0x301, "B&W Red Filter"),
            (0x302, "B&W Yellow Filter"),
            (0x303, "B&W Green Filter"),
            (0x310, "B&W Sepia"),
            (0x500, "Acros"),
            (0x501, "Acros Red Filter"),
            (0x502, "Acros Yellow Filter"),
            (0x503, "Acros Green Filter"),
        ] {
            let mut note = b"FUJIFILM".to_vec();
            note.extend_from_slice(&12u32.to_le_bytes());
            note.extend_from_slice(&1u16.to_le_bytes());
            note.extend_from_slice(&u16_inline(0x1003, code));
            note.extend_from_slice(&0u32.to_le_bytes());
            let recipe = recipe_from_exif(&exif_with_note(note)).unwrap();
            assert_eq!(value(&recipe, "Film simulation"), name);
            assert_eq!(recipe.rows[0].tag, 0x1003);
            assert!(!recipe.rows.iter().any(|row| row.label == "Color"));
        }
    }

    #[test]
    fn unknown_film_code_is_labelled() {
        let recipe = recipe_from_exif(&exif_with_note(fuji_note(0x1234, [0, 0]))).unwrap();
        assert_eq!(value(&recipe, "Film simulation"), "Unknown (4660)");
    }

    #[test]
    fn acros_filter_is_appended() {
        let only = {
            let entry = u16_inline(0x1003, 0x501);
            let mut n = b"FUJIFILM".to_vec();
            n.extend_from_slice(&12u32.to_le_bytes());
            n.extend_from_slice(&1u16.to_le_bytes());
            n.extend_from_slice(&entry);
            n.extend_from_slice(&0u32.to_le_bytes());
            n
        };
        let recipe = recipe_from_exif(&exif_with_note(only)).unwrap();
        assert_eq!(value(&recipe, "Film simulation"), "Acros Red Filter");
    }

    #[test]
    fn truncated_and_non_fuji_notes_are_absent() {
        assert!(recipe_from_exif(&exif_with_note(b"FUJIFILM".to_vec())).is_none());
        let mut plain = Vec::new();
        ByteOrder::Little.put_u16(&mut plain, 1);
        ByteOrder::Little.put_u16(&mut plain, 1);
        ByteOrder::Little.put_u16(&mut plain, 3);
        ByteOrder::Little.put_u32(&mut plain, 1);
        ByteOrder::Little.put_u16(&mut plain, 7);
        ByteOrder::Little.put_u16(&mut plain, 0);
        ByteOrder::Little.put_u32(&mut plain, 0);
        assert!(recipe_from_exif(&exif_with_note(plain)).is_none());
        assert!(recipe_from_bytes(b"not an image").is_none());
    }

    #[test]
    fn read_recipe_file_uses_the_raf_jpeg_window() {
        let dir = std::env::temp_dir().join(format!("lc-fuji-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let raf = dir.join("dscf0001.raf");
        let jpeg = dir.join("dscf0001.jpg");
        std::fs::write(&raf, sample_raf(200_000)).unwrap();
        std::fs::write(&jpeg, sample_jpeg()).unwrap();
        let from_raf = read_recipe_file(&raf).unwrap().unwrap();
        let from_jpeg = read_recipe_file(&jpeg).unwrap().unwrap();
        assert_eq!(from_raf, from_jpeg);
        assert_eq!(value(&from_raf, "Film simulation"), "Classic Chrome");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reader that fails once more than `limit` bytes have been copied out. Seeking does not count:
    /// a RAF seeks to its embedded JPEG and must not then walk the sensor data.
    struct Budget {
        data: Vec<u8>,
        pos: usize,
        served: usize,
        limit: usize,
    }

    impl Budget {
        fn new(data: Vec<u8>, limit: usize) -> Self {
            Self { data, pos: 0, served: 0, limit }
        }
    }

    impl Read for Budget {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.data.len() - self.pos);
            if self.served.saturating_add(n) > self.limit {
                return Err(std::io::Error::other("read past the recipe window"));
            }
            let Some(src) = self.data.get(self.pos..self.pos + n) else {
                return Ok(0);
            };
            let Some(dst) = buf.get_mut(..n) else {
                return Ok(0);
            };
            dst.copy_from_slice(src);
            self.pos += n;
            self.served += n;
            Ok(n)
        }
    }

    impl Seek for Budget {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            let len = self.data.len() as u64;
            let at = self.pos as u64;
            let next = match pos {
                SeekFrom::Start(p) => p,
                SeekFrom::Current(d) if d >= 0 => at.saturating_add(d as u64),
                SeekFrom::Current(d) => at.checked_sub(d.unsigned_abs()).ok_or_else(|| std::io::Error::other("seek"))?,
                SeekFrom::End(d) if d >= 0 => len.saturating_add(d as u64),
                SeekFrom::End(d) => len.checked_sub(d.unsigned_abs()).ok_or_else(|| std::io::Error::other("seek"))?,
            };
            if next > len {
                return Err(std::io::Error::other("seek past end"));
            }
            self.pos = next as usize;
            Ok(next)
        }
    }

    /// Legal short reads and transient interruptions, including inside the RAF preview.
    struct Fragmented {
        inner: Budget,
        chunk: usize,
        interrupt: bool,
        retry: bool,
    }

    impl Read for Fragmented {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.interrupt && !self.retry {
                self.retry = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.retry = false;
            let n = buf.len().min(self.chunk);
            self.inner.read(&mut buf[..n])
        }
    }

    impl Seek for Fragmented {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    /// Fragmentation must not change format detection or read into the sensor tail.
    #[test]
    fn fragmented_recipe_reads_preserve_recipes_and_budgets() {
        for chunk in [1, 7, HEADER_BYTES] {
            for interrupt in [false, true] {
                let tiff = exif_with_note(fuji_note(0x600, [40, -80]));
                for (bytes, limit) in
                    [(sample_jpeg(), sample_jpeg().len()), (sample_raf(3_000_000), HEADER_BYTES + sample_jpeg().len()), (tiff.clone(), tiff.len())]
                {
                    let expected = recipe_from_bytes(&bytes).unwrap();
                    let reader = Fragmented { inner: Budget::new(bytes, limit), chunk, interrupt, retry: false };
                    let window = recipe_window(reader, "fragmented").unwrap();
                    assert_eq!(recipe_from_bytes(&window), Some(expected), "chunk={chunk}, interrupt={interrupt}");
                }
            }
        }
    }

    #[test]
    fn tiff_recipe_beyond_the_sniff_window_is_read_within_the_cap() {
        let mut bytes = exif_with_note(fuji_note(0x600, [0, 0]));
        let root = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(bytes[root..root + 2].try_into().unwrap()) as usize;
        let ifd = bytes[root..root + 2 + count * 12 + 4].to_vec();
        let relocated = TIFF_SNIFF + 1024;
        bytes.resize(relocated, 0);
        bytes.extend_from_slice(&ifd);
        bytes[4..8].copy_from_slice(&(relocated as u32).to_le_bytes());
        let expected = recipe_from_bytes(&bytes).unwrap();
        bytes.resize(TIFF_META_MAX + 1024, 0);
        let window = recipe_window(Budget::new(bytes, TIFF_META_MAX), "late TIFF IFD").unwrap();
        assert_eq!(recipe_from_bytes(&window), Some(expected));
    }

    #[test]
    fn candidates_are_raf_jpeg_tiff_or_a_fuji_camera() {
        assert!(!might_hold_fuji_recipe("", "NEF", "DSC_0001.NEF"));
        assert!(!might_hold_fuji_recipe("NIKON Z 9", "JPEG", "DSC_0001.JPG"));
        assert!(might_hold_fuji_recipe("", "JPEG", "DSC_0001.JPG"));
        assert!(might_hold_fuji_recipe("", "", "DSCF0001.RAF"));
        assert!(might_hold_fuji_recipe("NIKON Z 9", "RAF", "DSCF0001.RAF"));
        assert!(might_hold_fuji_recipe("FUJIFILM X-T5", "JPEG", "DSCF0001.JPG"));
        assert!(might_hold_fuji_recipe("Fuji Photo Film Co., Ltd.", "JPEG", "a.jpg"));
        assert!(might_hold_fuji_recipe("", "TIF", "DSCF0001.TIF"));
        assert!(!might_hold_fuji_recipe("", "CR3", "IMG_0001.CR3"));
        assert!(!might_hold_fuji_recipe("", "DNG", "IMG_0001.DNG"));
        assert!(might_hold_fuji_recipe("FUJIFILM GFX100", "DNG", "a.dng"));
        assert!(!might_hold_fuji_recipe("GFX 100", "DNG", "a.dng"), "the model alone is not a make");
    }

    #[test]
    fn recipe_reads_do_not_pull_the_rest_of_the_file() {
        let jpeg = sample_jpeg();
        let mut jpeg_tail = jpeg.clone();
        jpeg_tail.extend(std::iter::repeat_n(0x11, 2_000_000));
        let from_jpeg = recipe_window(Budget::new(jpeg_tail, jpeg.len()), "a.jpg").unwrap();
        assert!(from_jpeg.len() <= jpeg.len(), "kept {} bytes of the jpeg", from_jpeg.len());
        assert_eq!(value(&recipe_from_bytes(&from_jpeg).unwrap(), "Film simulation"), "Classic Chrome");

        let raf = sample_raf(2_000_000);
        let from_raf = recipe_window(Budget::new(raf, HEADER_BYTES + jpeg.len()), "a.raf").unwrap();
        assert_eq!(value(&recipe_from_bytes(&from_raf).unwrap(), "Film simulation"), "Classic Chrome");

        // the embedded preview is a whole JPEG after the EXIF segment; only the segment is read
        let mut fat = jpeg.clone();
        fat.extend(std::iter::repeat_n(0x22, 2_000_000));
        let mut fat_raf = b"FUJIFILMCCD-RAW ".to_vec();
        fat_raf.resize(120, 0);
        fat_raf[84..88].copy_from_slice(&120u32.to_be_bytes());
        let fat_len = u32::try_from(fat.len()).expect("preview length");
        fat_raf[88..92].copy_from_slice(&fat_len.to_be_bytes());
        fat_raf.extend_from_slice(&fat);
        let from_fat = recipe_window(Budget::new(fat_raf, HEADER_BYTES + jpeg.len()), "fat.raf").unwrap();
        assert_eq!(value(&recipe_from_bytes(&from_fat).unwrap(), "Film simulation"), "Classic Chrome");

        let mut plain = b"hello".to_vec();
        plain.resize(2_000_000, 0xab);
        let from_plain = recipe_window(Budget::new(plain, HEADER_BYTES), "a.nef").unwrap();
        assert!(from_plain.is_empty(), "a file that is not JPEG, RAF or TIFF stops at the header");

        let mut garbage = b"II*\0canon-raw".to_vec();
        garbage.resize(2_000_000, 0xab);
        let from_garbage = recipe_window(Budget::new(garbage, TIFF_SNIFF), "a.nef").unwrap();
        assert!(from_garbage.is_empty(), "a TIFF header that does not parse is not read to the end");

        let mut note = IfdBuilder::new();
        note.set(tags::MAKE, Value::Ascii("Canon".into()));
        let canon = TiffWriter::new(ByteOrder::Little, false).write(&[note]).unwrap();
        let mut canon_tail = canon;
        canon_tail.resize(2_000_000, 0xcd);
        let from_tiff = recipe_window(Budget::new(canon_tail, TIFF_SNIFF), "a.dng").unwrap();
        assert!(from_tiff.is_empty(), "a Canon TIFF is not loaded past the header sniff");

        let mut fuji = exif_with_note(fuji_note(0x700, [0, 0]));
        let fuji_len = fuji.len();
        fuji.resize(fuji_len + 2_000_000, 0xef);
        let from_fuji_tiff = recipe_window(Budget::new(fuji, TIFF_SNIFF), "a.dng").unwrap();
        assert!(from_fuji_tiff.len() <= TIFF_SNIFF);
        assert_eq!(value(&recipe_from_bytes(&from_fuji_tiff).unwrap(), "Film simulation"), "Eterna");
    }

    #[test]
    fn a_header_past_the_cap_is_absent_without_reading_the_rest() {
        let mut jpeg = vec![0xff, 0xd8];
        let seg = 65535u16;
        let seg_len = usize::from(seg);
        while jpeg.len().saturating_add(4).saturating_add(seg_len) <= JPEG_META_MAX {
            jpeg.extend_from_slice(&[0xff, 0xe0]);
            jpeg.extend_from_slice(&seg.to_be_bytes());
            jpeg.extend(std::iter::repeat_n(0, seg_len - 2));
        }
        jpeg.extend_from_slice(&[0xff, 0xe0]);
        jpeg.extend_from_slice(&seg.to_be_bytes());
        jpeg.extend(std::iter::repeat_n(0, seg_len - 2));
        let window = recipe_window(Budget::new(jpeg, JPEG_META_MAX.saturating_add(64)), "big.jpg").unwrap();
        assert!(window.is_empty());

        let mut raf = b"FUJIFILMCCD-RAW ".to_vec();
        raf.resize(HEADER_BYTES, 0);
        raf[84..88].copy_from_slice(&92u32.to_be_bytes());
        let too_big = u32::try_from(RAF_JPEG_MAX + 1).expect("cap fits in u32");
        raf[88..92].copy_from_slice(&too_big.to_be_bytes());
        let from_raf = recipe_window(Budget::new(raf, HEADER_BYTES), "big.raf").unwrap();
        assert!(from_raf.is_empty());
    }
}
