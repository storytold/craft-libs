#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room Canon CRW (CIFF heap) raw decoder: the bodies before the CR2 era (EOS D30, D60,
//! 10D, 300D / Digital Rebel, PowerShot G-series).
//!
//! The container rules come from Canon's published CIFF specification and ExifTool's prose tag
//! documentation; the compression was derived from CC0 and maintainer-owned sample files by
//! black-box analysis against a reference decoder's command-line output, frozen in
//! `clean-room/02-crw-format.md` and verified sample-exact on 28 of 29 corpus files plus 1,516
//! further EOS D60 files (`clean-room/03-verification.md`). The code tables are frozen data
//! (`tables`, frozen from the clean-room derivation record retained in LightCraft). No other raw decoder's
//! source was consulted; the whole derivation record is in the crate's `clean-room/` folder.
//!
//! - [`probe`] recognises the CIFF signature.
//! - [`header`] reads geometry, bit depth, CFA phase, make and model, capture time, orientation
//!   and as-shot white balance without touching the entropy-coded data.
//! - [`decode`] also decodes the sensor samples and measures the black level (masked borders)
//!   and the white level (saturation plateau or the sensor's clip level).
//!
//! The decoder never panics on input: every offset read from the file is bounds-checked, the heap
//! walk is bounded by depth and by a per-file record budget, and the sample count is capped by
//! [`Limits`]. Malformed files return [`Error::Corrupt`]; unknown decoder tables and the
//! PowerShot G5 (whose entropy coding diverges from the derived model) return
//! [`Error::Unsupported`].
//!
//! Known gaps (documented in the freeze): the PowerShot G5 is refused (its embedded JPEG is the
//! app's fallback); PowerShot bodies without CC0 samples (G2, G3, Pro1, S-series) decode through
//! the same model unverified; the D30 and the PowerShots carry no white-balance record the
//! derivation deciphered, so `wb_multipliers` is `None` for them.
//!
//! Colour (camera matrices, profiles) is out of scope: the file carries none, and the app owns
//! its colour model.

mod tables;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

/// Why a file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes do not start with a CIFF `HEAPCCDR` header.
    #[error("not a Canon CRW (CIFF) file")]
    NotCrw,
    /// The file is a CRW but its structure or data is damaged.
    #[error("CRW: {0}")]
    Corrupt(String),
    /// A CRW the derived model does not cover (an unknown decoder table, the PowerShot G5).
    #[error("unsupported CRW: {0}")]
    Unsupported(String),
    /// The file declares more than [`Limits`] allow.
    #[error("CRW exceeds a limit: {0}")]
    LimitExceeded(&'static str),
}

/// Caps on what a file may ask the decoder to allocate, checked against the header before
/// anything is allocated. The struct is `#[non_exhaustive]`: start from [`Limits::default`] and
/// set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most sensor samples (`width × height`) a file may declare. The decoder holds about three
    /// `u16` buffers of this size at its peak. Default: 2²³ (8.4 million) on every target; real
    /// CRW frames hold at most 7.4 million samples.
    pub max_samples: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_samples: 1 << 23 }
    }
}

/// The 2×2 Bayer phase at sensor position (0, 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cfa {
    /// Red, green / green, blue: every verified body except the PowerShot G1.
    Rggb,
    /// Green, blue / red, green: the PowerShot G1.
    Gbrg,
}

impl Cfa {
    /// The conventional four-letter name.
    pub fn name(self) -> &'static str {
        match self {
            Cfa::Rggb => "RGGB",
            Cfa::Gbrg => "GBRG",
        }
    }

    /// The colour at each site of the 2×2 cell, row-major: 0 = red, 1 = green, 2 = blue.
    pub fn pattern(self) -> [u8; 4] {
        match self {
            Cfa::Rggb => [0, 1, 1, 2],
            Cfa::Gbrg => [1, 2, 0, 1],
        }
    }
}

/// A rectangle in sensor pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self { x, y, width, height }
    }
}

/// Black level per 2×2 CFA site, row-major, anchored at the active area's origin.
///
/// The anchor differs from [`Header::cfa`]'s: `cfa` is the phase at sensor (0, 0), `values` is
/// indexed from the active area's origin. The active area can start at an odd sensor row (the
/// PowerShot G1 starts at y = 15), so `values[i]` does not line up with `cfa.pattern()[i]`: read a
/// level through [`BlackLevel::at`] rather than pairing the two by index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackLevel {
    pub values: [f32; 4],
}

impl BlackLevel {
    /// The same level at every site.
    pub const fn uniform(v: f32) -> Self {
        Self { values: [v; 4] }
    }

    /// The level at sensor position (`x`, `y`), whose CFA site is counted from `active`'s origin.
    pub fn at(&self, active: Rect, x: usize, y: usize) -> f32 {
        let px = (x as isize - active.x as isize).rem_euclid(2) as usize;
        let py = (y as isize - active.y as isize).rem_euclid(2) as usize;
        self.values.get(py * 2 + px).copied().unwrap_or(self.values[0])
    }
}

/// Everything a CRW tells about its image short of the samples: what [`header`] returns, and
/// what [`Image::header`] carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Sensor width in samples (SensorInfo), masked borders included.
    pub width: usize,
    /// Sensor height in samples.
    pub height: usize,
    /// Sample depth: 12 when the file carries a low-bits plane, else 10.
    pub bits: u32,
    /// The sensor area that holds the image (SensorInfo's inclusive crop); the rest is masked.
    pub active_area: Rect,
    /// Bayer phase at sensor (0, 0).
    pub cfa: Cfa,
    /// The DecoderTable number (0, 1 or 2): which frozen code tables the stream uses.
    pub selector: u32,
    /// Camera make from the MakeModel record (`Canon`).
    pub make: Option<String>,
    /// Camera model from the MakeModel record (`Canon EOS D60`).
    pub model: Option<String>,
    /// Capture time as Unix seconds on the camera's local clock (the TimeStamp record); the
    /// file carries no UTC offset. Convert with a calendar library (chrono), not by hand.
    pub capture_unix: Option<i64>,
    /// Exif orientation (1, 3, 6 or 8) from the ImageInfo rotation.
    pub exif_orientation: u8,
    /// As-shot white balance as (R, G, B) multipliers with G = 1, from the ColorBalance record
    /// slot the ShotInfo white-balance setting selects; `None` on bodies without the record
    /// (D30, PowerShots).
    pub wb_multipliers: Option<[f32; 3]>,
}

/// A decoded CRW.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub header: Header,
    /// `width × height` sensor samples, row-major, `bits` deep (10 or 12), black not subtracted.
    pub samples: Vec<u16>,
    /// Black level measured from the masked borders.
    pub black: BlackLevel,
    /// White level: the saturation plateau, the sensor's clip level, or the full range.
    pub white: f32,
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn corrupt(why: &str) -> Error {
    Error::Corrupt(why.to_string())
}

/// Heap recursion bound: the deepest directory observed in real files is 4.
const MAX_DEPTH: usize = 16;

/// Records visited per file, across every heap level: real files hold a few dozen. Depth alone
/// doesn't bound the walk — a sub-heap record may point back at its own parent block, and N such
/// records per block multiply into N^MAX_DEPTH visits — so this shared budget does.
const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Order {
    Le,
    Be,
}

fn u16_at(b: &[u8], at: usize, o: Order) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(match o {
        Order::Le => u16::from_le_bytes(s),
        Order::Be => u16::from_be_bytes(s),
    })
}

fn u32_at(b: &[u8], at: usize, o: Order) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(match o {
        Order::Le => u32::from_le_bytes(s),
        Order::Be => u32::from_be_bytes(s),
    })
}

/// Walk a heap block (`start..start + len` of `bytes`) depth-first in record-table order,
/// recording each in-heap record's tag (typeCode & 0x3fff), absolute offset and length — first
/// occurrence wins. In-record values and reserved storage classes are skipped; dataType 5/6
/// records are sub-heaps. `budget` is the number of records the whole file may still visit;
/// the walk stops wherever it runs out.
fn walk(bytes: &[u8], start: usize, len: usize, o: Order, depth: usize, budget: &mut usize, out: &mut BTreeMap<u16, (usize, usize)>) {
    if depth > MAX_DEPTH || len < 6 {
        return;
    }
    let Some(tbl) = start.checked_add(len).and_then(|e| e.checked_sub(4)).and_then(|at| u32_at(bytes, at, o)) else { return };
    let tbl = tbl as usize;
    // the offset table must fit between the block start and its own trailing pointer
    if tbl.checked_add(6).is_none_or(|e| e > len) {
        return;
    }
    let Some(abs_tbl) = start.checked_add(tbl) else { return };
    let Some(n) = u16_at(bytes, abs_tbl, o) else { return };
    let Some(records_end) = (n as usize).checked_mul(10).and_then(|r| tbl.checked_add(2)?.checked_add(r)) else { return };
    if records_end > len {
        return;
    }
    for k in 0..n as usize {
        let Some(left) = budget.checked_sub(1) else { return };
        *budget = left;
        let at = abs_tbl + 2 + k * 10;
        let (Some(tc), Some(rl), Some(ro)) = (u16_at(bytes, at, o), u32_at(bytes, at + 2, o), u32_at(bytes, at + 6, o)) else { return };
        if tc & 0xc000 != 0 {
            continue; // in-record value or reserved storage: nothing we need
        }
        let (rl, ro) = (rl as usize, ro as usize);
        if ro.checked_add(rl).is_none_or(|e| e > len) {
            continue;
        }
        let Some(abs) = start.checked_add(ro) else { continue };
        if matches!((tc >> 11) & 7, 5 | 6) {
            walk(bytes, abs, rl, o, depth + 1, budget, out);
        } else {
            out.entry(tc & 0x3fff).or_insert((abs, rl));
        }
    }
}

/// CIFF records the decoder consumes.
const RAW_DATA: u16 = 0x2005;
const SENSOR_INFO: u16 = 0x1031;
const DECODER_TABLE: u16 = 0x1835;
const MAKE_MODEL: u16 = 0x080a;
const IMAGE_INFO: u16 = 0x1810;
const TIME_STAMP: u16 = 0x180e;
const COLOR_BALANCE: u16 = 0x10a9;
const SHOT_INFO: u16 = 0x102a;

/// Whether `bytes` start with a CIFF raw header: `II` or `MM`, then `HEAPCCDR` at offset 6.
pub fn probe(bytes: &[u8]) -> bool {
    matches!(bytes.get(0..2), Some(b"II") | Some(b"MM")) && bytes.get(6..14) == Some(b"HEAPCCDR")
}

/// Read the header: geometry, bit depth, CFA phase, metadata and white balance, without
/// decoding the samples. Fails the same way [`decode`] would on the records it needs.
pub fn header(bytes: &[u8], limits: &Limits) -> Result<Header> {
    Ok(parse(bytes, limits, false)?.header)
}

/// Decode a CRW: the header plus every sensor sample, the black level and the white level.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image> {
    parse(bytes, limits, true)
}

fn parse(bytes: &[u8], limits: &Limits, full: bool) -> Result<Image> {
    let o = match bytes.get(0..2) {
        Some(b"II") => Order::Le,
        Some(b"MM") => Order::Be,
        _ => return Err(Error::NotCrw),
    };
    if bytes.get(6..14) != Some(b"HEAPCCDR") {
        return Err(Error::NotCrw);
    }
    let hl = u32_at(bytes, 2, o).ok_or_else(|| corrupt("truncated header"))? as usize;
    let heap_len = bytes.len().checked_sub(hl).filter(|&l| l >= 6).ok_or_else(|| corrupt("header length beyond the file"))?;
    let mut rec = BTreeMap::new();
    let mut budget = MAX_RECORDS;
    walk(bytes, hl, heap_len, o, 0, &mut budget, &mut rec);

    let &(so, sl) = rec.get(&SENSOR_INFO).ok_or_else(|| corrupt("no SensorInfo record"))?;
    let sensor: Vec<u16> = (0..sl / 2).map_while(|i| u16_at(bytes, so + i * 2, o)).collect();
    let (w, h) = match (sensor.get(1), sensor.get(2)) {
        (Some(&w), Some(&h)) => (w as usize, h as usize),
        _ => return Err(corrupt("SensorInfo too short")),
    };
    if w == 0 || h == 0 {
        return Err(corrupt("zero sensor dimensions"));
    }
    let n = w.checked_mul(h).filter(|&n| n <= limits.max_samples).ok_or(Error::LimitExceeded("image too large"))?;

    let &(dto, dtl) = rec.get(&DECODER_TABLE).ok_or_else(|| corrupt("no DecoderTable record"))?;
    if dtl < 16 {
        return Err(corrupt("DecoderTable too short"));
    }
    let selector = u32_at(bytes, dto, o).ok_or_else(|| corrupt("truncated DecoderTable"))?;
    let ent_off = u32_at(bytes, dto + 8, o).ok_or_else(|| corrupt("truncated DecoderTable"))? as usize;
    let tables = tables::tables(selector).ok_or_else(|| Error::Unsupported(format!("decoder table {selector}")))?;
    if ent_off > bytes.len() {
        return Err(corrupt("entropy offset beyond the file"));
    }

    // "Canon\0<model>\0"
    let (make, model) = rec
        .get(&MAKE_MODEL)
        .and_then(|&(mo, ml)| bytes.get(mo..mo.checked_add(ml)?))
        .map(|v| {
            let mut parts = v.split(|&b| b == 0).map(|s| String::from_utf8_lossy(s).trim().to_string());
            (parts.next().unwrap_or_default(), parts.next().unwrap_or_default())
        })
        .unwrap_or_default();
    if model.contains("PowerShot G5") {
        return Err(Error::Unsupported(
            "Canon PowerShot G5 entropy coding (a documented anomaly of the derivation, clean-room/02-crw-format.md §4); use the embedded JPEG"
                .into(),
        ));
    }

    // active area from the SensorInfo crop (inclusive left, top, right, bottom)
    let active = match (sensor.get(5), sensor.get(6), sensor.get(7), sensor.get(8)) {
        (Some(&l), Some(&t), Some(&r), Some(&b))
            if (l as usize) < (r as usize) && (r as usize) < w && (t as usize) < (b as usize) && (b as usize) < h =>
        {
            Rect::new(l as usize, t as usize, r as usize - l as usize + 1, b as usize - t as usize + 1)
        }
        _ => Rect::new(0, 0, w, h),
    };

    let &(ro, rl) = rec.get(&RAW_DATA).ok_or_else(|| corrupt("no raw data record"))?;
    let payload =
        bytes.get(ro..ro.checked_add(rl).ok_or_else(|| corrupt("raw record size overflow"))?).ok_or_else(|| corrupt("raw data outside the file"))?;

    // Low-bits plane discriminator, two signs of a plane: (a) its raw bytes may hold an 0xff not
    // followed by 0x00, which the stuffed entropy stream never does; (b) the zero header of
    // `ent_off` bytes sits right after it, where a file without a plane is deep in entropy data.
    // (b) catches the D60 files whose lowest bit is almost always 0, so no plane byte is 0xff.
    let q = n.div_ceil(4);
    let span = q.saturating_add(ent_off);
    let head = payload.get(..span.min(payload.len())).unwrap_or(payload);
    // Both signs need a payload long enough to hold a plane and the header behind it. A shorter
    // payload ends in the PowerShot `ff d9 00` trailer, and `ff d9` is a 0xff not followed by
    // 0x00: without this gate a flat no-plane G1 frame read as 12-bit and was then refused.
    let plane_fits = payload.len() > span;
    let header_after_plane = plane_fits && ent_off >= 64 && payload.get(q..span).is_some_and(|z| z.iter().all(|&b| b == 0));
    let has_low = header_after_plane || (plane_fits && head.windows(2).any(|p| p[0] == 0xff && p[1] != 0x00));
    let bits = if has_low { 12 } else { 10 };
    let low_len = if has_low { q } else { 0 };
    let samples = if full {
        let entropy = payload.get(low_len.saturating_add(ent_off)..).ok_or_else(|| corrupt("payload shorter than its header"))?;
        let low = has_low.then(|| payload.get(..q)).flatten();
        if has_low && low.is_none() {
            return Err(corrupt("payload shorter than its low-bits plane"));
        }
        let hi = decode_entropy(entropy, w, h, &tables)?;
        let mut samples = vec![0u16; n];
        // `samples` is documented as `bits` deep, so a damaged stream whose chain value runs past
        // the range is clamped here: a consumer indexing a `1 << bits` LUT must not see more.
        let top = ((1u32 << bits) - 1) as u16;
        for (i, (dst, &hi)) in samples.iter_mut().zip(hi.iter()).enumerate() {
            *dst = match low {
                Some(low) => {
                    let lb = low.get(i >> 2).map_or(0, |&b| (b >> ((i & 3) * 2)) & 3);
                    let v = (((hi as u32) << 2) | lb as u32) & 0xffff;
                    v.min(u32::from(top)) as u16
                }
                None => hi.min(top),
            };
        }
        samples
    } else {
        Vec::new()
    };

    // CFA phase at sensor (0, 0): RGGB on every verified body except the PowerShot G1 (GBRG),
    // established by chroma correlation against each body's embedded JPEG (derivation log).
    let cfa = if model.contains("PowerShot G1") { Cfa::Gbrg } else { Cfa::Rggb };

    let black = if full { black_from_borders(&samples, w, h, active) } else { BlackLevel::uniform(0.0) };
    let white = if full { white_level(&samples, bits) } else { ((1u32 << bits) - 1) as f32 };

    // ImageInfo rotation: degrees; 270 observed on portrait files whose scene is upright after a
    // 90° counter-clockwise turn (Exif 8), so 90 is its mirror (Exif 6)
    // The record holds a signed degree count, so a file may write −90 for 270: take it modulo
    // 360 before mapping, and leave anything off-axis (0, ±360) at the identity.
    let rotation_deg =
        rec.get(&IMAGE_INFO).and_then(|&(io, il)| (il >= 16).then(|| u32_at(bytes, io + 12, o)).flatten()).map(|r| (r as i32).rem_euclid(360));
    let exif_orientation = match rotation_deg {
        Some(90) => 6,
        Some(180) => 3,
        Some(270) => 8,
        _ => 1,
    };

    let capture_unix = rec.get(&TIME_STAMP).and_then(|&(to, tl)| (tl >= 4).then(|| u32_at(bytes, to, o)).flatten()).map(i64::from);

    let header = Header {
        width: w,
        height: h,
        bits,
        active_area: active,
        cfa,
        selector,
        make: (!make.is_empty()).then_some(make),
        model: (!model.is_empty()).then_some(model),
        capture_unix,
        exif_orientation,
        wb_multipliers: wb_from_color_balance(bytes, &rec, o),
    };
    Ok(Image { header, samples, black, white })
}

/// As-shot white-balance multipliers from the ColorBalance record (0x10a9, D60/10D/300D): a
/// byte-length word, then (R, G, G, B) level quadruples per white-balance setting, the last
/// quadruple being black levels (ExifTool's prose Canon/CanonRaw tag documentation names the
/// slots; the 10D/300D order is Auto, Daylight, Shade, Cloudy, Tungsten, Fluorescent, Flash,
/// Custom, Kelvin, and the D60's seven slots lack Shade and Kelvin). The slot is chosen by the
/// ShotInfo (0x102a) WhiteBalance value; multipliers are `level / green`, confirmed against the
/// cameras' own JPEG renderings (derivation log). Bodies without the record (D30, PowerShots)
/// return `None`.
fn wb_from_color_balance(bytes: &[u8], rec: &BTreeMap<u16, (usize, usize)>, o: Order) -> Option<[f32; 3]> {
    let &(bo, bl) = rec.get(&COLOR_BALANCE)?;
    let vals: Vec<u16> = (0..bl / 2).map_while(|i| u16_at(bytes, bo + i * 2, o)).collect();
    // quadruples after the length word; the last one is black levels
    let slots = vals.len().checked_sub(1)? / 4;
    let usable = slots.checked_sub(1)?;
    if usable == 0 {
        return None;
    }
    let setting = rec.get(&SHOT_INFO).and_then(|&(so, sl)| (sl / 2 > 7).then(|| u16_at(bytes, so + 14, o)).flatten()).unwrap_or(0);
    // Canon WhiteBalance values: 0 Auto, 1 Daylight, 2 Cloudy, 3 Tungsten, 4 Fluorescent,
    // 5 Flash, 6 Custom, 8 Shade, 9 Kelvin
    let slot = if usable >= 9 {
        match setting {
            1 => 1,
            8 => 2,
            2 => 3,
            3 => 4,
            4 => 5,
            5 => 6,
            6 => 7,
            9 => 8,
            _ => 0,
        }
    } else {
        match setting {
            s @ 1..=6 if (s as usize) < usable => s as usize,
            _ => 0,
        }
    };
    let q = vals.get(1 + slot.min(usable - 1) * 4..5 + slot.min(usable - 1) * 4)?;
    let g = (q.get(1).copied()? as f32 + q.get(2).copied()? as f32) / 2.0;
    let (r, b) = (q.first().copied()? as f32, q.get(3).copied()? as f32);
    if g <= 0.0 || r <= 0.0 || b <= 0.0 {
        return None;
    }
    let (mr, mb) = (r / g, b / g);
    ((0.2..=8.0).contains(&mr) && (0.2..=8.0).contains(&mb)).then_some([mr, 1.0, mb])
}

/// Fewest samples at the top of the data that count as the sensor's clip level (hot pixels are
/// fewer).
const CLIP_SAMPLES: usize = 64;

/// White level: the saturation plateau when [`saturation_plateau`] finds one, else the top of
/// the data when it sits just under the full range.
///
/// These bodies clip below the full range at low ISO: on 1,409 D60 files at ISO 100–400 no
/// sample exceeds 4009 (most stop at 4000–4008), while ISO 800–1000 files reach 4095; the 10D
/// and 300D samples stop at 4003–4008 too. A photo with only a small blown area has no plateau
/// for [`saturation_plateau`], which then answered 4095 (373 of 925 ISO 100 files, 283 of 287 at
/// ISO 400): the blown pixels sat at 0.977 of white, under the clip threshold, and kept a
/// colour after white balance. So when at least [`CLIP_SAMPLES`] samples reach within 3 % of the
/// full range, the level they reach is the white level, lowered by the same 1/256 band the
/// plateau estimate uses.
fn white_level(data: &[u16], bits: u32) -> f32 {
    let full = (1u32 << bits.clamp(1, 16)) - 1;
    let plateau = saturation_plateau(data, bits);
    if plateau < full as f32 {
        return plateau;
    }
    let mut hist = vec![0usize; full as usize + 1];
    for &v in data {
        if let Some(h) = hist.get_mut(v as usize) {
            *h += 1;
        }
    }
    let mut above = 0usize;
    let top = hist.iter().enumerate().rev().find_map(|(v, &n)| {
        above += n;
        (above >= CLIP_SAMPLES).then_some(v as u32)
    });
    match top {
        Some(top) if top < full && top as f32 >= 0.97 * full as f32 => (top - (top / 256).max(1)) as f32,
        _ => full as f32,
    }
}

/// White level estimate shared by the Craft raw decoders (LightCraft's `white_from_data`): the
/// saturation plateau if a noticeable number of samples sit at the maximum value, otherwise the
/// full `bits` range. A few stray samples above the plateau (hot pixels) don't hide it.
fn saturation_plateau(data: &[u16], bits: u32) -> f32 {
    let full_level = (1u32 << bits.clamp(1, 16)) - 1;
    let full = full_level as f32;
    let step = (data.len() / 2_000_000).max(1);
    // samples above the nominal range are padding / invalid, not saturation
    let mut hist = vec![0usize; full_level as usize + 1];
    for &v in data.iter().step_by(step) {
        if let Some(h) = hist.get_mut(v as usize) {
            *h += 1;
        }
    }
    let min_plateau = (data.len() / 20_000).max(16);
    // the top of the data, skipping up to half a plateau's worth of strays so a real plateau can't be skipped
    let strays = min_plateau / 2;
    let mut above = 0usize;
    let Some(mx) = hist.iter().enumerate().rev().find_map(|(v, &n)| {
        above += n * step;
        (above > strays).then_some(v as u32)
    }) else {
        return full;
    };
    let band = (mx / 256).max(1);
    let near = mx.saturating_sub(band);
    let below = near.saturating_sub(band);
    let count = |from: u32, to: u32| hist.get(from as usize..to as usize).map_or(0, |h| h.iter().sum::<usize>());
    // the strays count as saturated too
    let top = count(near, full_level + 1);
    let under = count(below, near);
    // a saturation plateau is a spike: many more samples in the top band than in the band just below it
    if mx as f32 > full * 0.5 && top * step >= min_plateau && top >= 4 * under + 4 {
        // plateau: use its lower edge so everything at saturation maps to ≥ 1.0
        near as f32
    } else {
        full
    }
}

/// Black level from the masked sensor borders: the darkest flat strip among 8-column groups of
/// the left border and 4-row groups of the top border, with the strips within 8 DN of it averaged
/// in. A strip whose mean is ≤ 1 is clamped padding (observed on the D60's first rows), not a
/// black reference, and bright transition strips next to the active area only raise means, so the
/// minimum stays on true dark pixels.
fn black_from_borders(data: &[u16], w: usize, h: usize, active: Rect) -> BlackLevel {
    let mean_of = |xs: std::ops::Range<usize>, ys: std::ops::Range<usize>| -> Option<f64> {
        let (mut sum, mut cnt) = (0f64, 0u64);
        for y in ys {
            let row = data.get(y.checked_mul(w)?..y.checked_mul(w)?.checked_add(w)?)?;
            for &v in row.get(xs.clone())? {
                sum += v as f64;
                cnt += 1;
            }
        }
        (cnt > 0).then(|| sum / cnt as f64)
    };
    let mut strips: Vec<(f64, std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    let rows = active.y..active.y.saturating_add(active.height).min(h);
    let mut x = 2usize;
    while x + 8 <= active.x {
        if let Some(m) = mean_of(x..x + 8, rows.clone()) {
            strips.push((m, x..x + 8, rows.clone()));
        }
        x += 8;
    }
    let cols = active.x..active.x.saturating_add(active.width).min(w);
    let mut y = 2usize;
    while y + 4 <= active.y {
        if let Some(m) = mean_of(cols.clone(), y..y + 4) {
            strips.push((m, cols.clone(), y..y + 4));
        }
        y += 4;
    }
    let Some(darkest) = strips.iter().map(|s| s.0).filter(|&m| m > 1.0).reduce(f64::min) else {
        return BlackLevel::uniform(0.0);
    };
    // per-CFA-site means over every strip near the darkest
    let (mut sum, mut cnt) = ([0f64; 4], [0u64; 4]);
    for (m, xs, ys) in &strips {
        // clamped padding (mean ≤ 1) is no black reference: counting it dragged the D60's level
        // from 128 down to 117 and lifted every shadow
        if *m <= 1.0 || *m > darkest + 8.0 {
            continue;
        }
        for y in ys.clone() {
            let py = (y as isize - active.y as isize).rem_euclid(2) as usize;
            for x in xs.clone() {
                let px = (x as isize - active.x as isize).rem_euclid(2) as usize;
                if let Some(&v) = data.get(y * w + x) {
                    sum[py * 2 + px] += v as f64;
                    cnt[py * 2 + px] += 1;
                }
            }
        }
    }
    if cnt.contains(&0) {
        return BlackLevel::uniform(darkest as f32);
    }
    let mut values = [0f32; 4];
    for (v, (s, c)) in values.iter_mut().zip(sum.iter().zip(&cnt)) {
        *v = (s / *c as f64) as f32;
    }
    BlackLevel { values }
}

/// A code-table lookup built for 16-bit MSB-first peeks: entry = `len << 16 | run << 8 | size`,
/// zero where no code matches.
fn build_lut(codes: &[(u16, u8, u8, u8)]) -> Vec<u32> {
    let mut lut = vec![0u32; 1 << 16];
    for &(code, len, r, s) in codes {
        if len == 0 || len > 16 {
            continue;
        }
        let span = 1usize << (16 - len);
        let base = (code as usize) << (16 - len);
        for e in lut.iter_mut().skip(base).take(span) {
            *e = (len as u32) << 16 | (r as u32) << 8 | s as u32;
        }
    }
    lut
}

/// Decode the ff00-stuffed entropy stream into the `w × h` high-plane samples, per the frozen
/// model (`clean-room/02-crw-format.md` §2): 64-sample blocks, first/main code tables,
/// (run, size) leaves with JPEG-rule magnitudes, EOB, block-DC carry, and two parity DPCM chains
/// initialised to 512 and reset at every row start. A stream that ends early leaves the remaining
/// samples at their chain values (the frozen stream-end rule).
fn decode_entropy(entropy: &[u8], w: usize, h: usize, tables: &tables::SelTables) -> Result<Vec<u16>> {
    let d = unstuff(entropy);
    let nbits = d.len().saturating_mul(8);
    let peek16 = |pos: usize| -> u16 {
        let (byte, sh) = (pos >> 3, pos & 7);
        let b = |k: usize| d.get(byte + k).copied().unwrap_or(0) as u32;
        let w24 = b(0) << 16 | b(1) << 8 | b(2);
        ((w24 >> (8 - sh)) & 0xffff) as u16
    };

    let luts = [build_lut(tables.first), build_lut(tables.main)];
    let n = w.checked_mul(h).ok_or(Error::LimitExceeded("image too large"))?;
    let mut out = Chains { dec: vec![0u16; n], w, qi: 0, ce: 512, co: 512 };
    let mut carry: i32 = 0;
    let mut pos = 0usize;
    while out.qi < n {
        let block_end = n.min((out.qi / 64 + 1) * 64);
        let mut tok = 0u32;
        while out.qi < block_end {
            // stream exhausted: every remaining sample keeps its chain value
            if pos >= nbits.saturating_sub(1) {
                out.fill(n);
                break;
            }
            let e = luts[usize::from(tok > 0)].get(peek16(pos) as usize).copied().unwrap_or(0);
            if e == 0 {
                return Err(corrupt("undecodable entropy code"));
            }
            let (len, mut rr, s) = ((e >> 16) as usize, ((e >> 8) & 0xff) as usize, e & 0xff);
            if pos.saturating_add(len).saturating_add(s as usize) > nbits {
                out.fill(n);
                break;
            }
            pos += len;
            tok += 1;
            if s == 0 && rr == 0 && tok > 1 {
                // end of block
                out.fill(block_end);
                continue;
            }
            let diff: i32 = if s > 0 {
                let m = (peek16(pos) >> (16 - s)) as i32;
                pos += s as usize;
                // JPEG magnitude rule
                if m >= 1 << (s - 1) { m } else { m - (1 << s) + 1 }
            } else {
                0
            };
            let mut first = out.qi.is_multiple_of(64);
            if first && rr > 0 {
                // the block's first sample covered by the skip: its diff is exactly the carry
                out.emit(carry);
                rr -= 1;
                first = false;
            }
            out.fill(block_end.min(out.qi + rr));
            if out.qi >= block_end {
                continue;
            }
            if first {
                carry = carry.wrapping_add(diff);
                out.emit(carry);
            } else {
                out.emit(diff);
            }
        }
    }
    Ok(out.dec)
}

/// Drop the stuffing byte: an 0x00 after an 0xff.
fn unstuff(entropy: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(entropy.len());
    let mut i = 0;
    while let Some(&b) = entropy.get(i) {
        d.push(b);
        i += if b == 0xff && entropy.get(i + 1) == Some(&0x00) { 2 } else { 1 };
    }
    d
}

/// The decoded samples and the two parity DPCM chains that produce them.
struct Chains {
    dec: Vec<u16>,
    w: usize,
    qi: usize,
    ce: i32,
    co: i32,
}

impl Chains {
    /// Emit one diff: row-start chain reset, parity chain update, 16-bit masked store.
    fn emit(&mut self, d: i32) {
        if self.qi.is_multiple_of(self.w) {
            self.ce = 512;
            self.co = 512;
        }
        let c = if self.qi.is_multiple_of(2) { &mut self.ce } else { &mut self.co };
        *c = c.wrapping_add(d);
        if let Some(p) = self.dec.get_mut(self.qi) {
            *p = (*c & 0xffff) as u16;
        }
        self.qi += 1;
    }

    /// Emit zero diffs up to sample `end`.
    fn fill(&mut self, end: usize) {
        while self.qi < end {
            self.emit(0);
        }
    }
}
