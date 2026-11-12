//! Where the camera focused: the autofocus point a camera records in its maker notes, as a box on the
//! upright photo ([`FocusPoint`]). Sony only for now.
//!
//! **Sony** (the MakerNote IFD: header-less in ARWs, after a `SONY DSC` header in JPEGs). Tag identities and value names come from the public ExifTool
//! tag-name documentation (Sony table); every layout below was confirmed on real ILCE-1 (α1) and
//! ILCE-7SM3 (A7S III) ARWs, compared with ExifTool's output as a black box. No third-party code was read.
//!
//! | Tag | Layout (confirmed) | Use |
//! |---|---|---|
//! | `0x2027` FocusLocation | 4 × int16u: the width and height of the frame it is measured in, then the focus centre x, y in pixels of that frame (the sensor's landscape frame: a portrait shot still says 8640 × 5760) | the point |
//! | `0x2037` FocusFrameSize | 6 bytes: int16u width, int16u height of the frame the camera showed, then 2 bytes that are zero when there is no frame (manual focus) | the box size |
//! | `0x201b` FocusMode | int8u: 0 Manual, 2 AF-S, 3 AF-C, 4 AF-A, 6 DMF, 7 AF-D | manual = no point |
//! | `0x201c` AFAreaModeSetting | int8u: 0 Wide, 1 Center, 3 Flexible Spot, 11 Zone, 12 Expanded Flexible Spot, 13 Custom AF Area | shown |
//! | `0x2021` AFTracking | int8u: 0 Off, 1 Face tracking, 2 Lock On AF | shown |
//! | `0x9402` (enciphered block) byte 0x16, 0x17 | FocusMode (low 7 bits) and the AF area mode actually used: 0 Multi, 1 Center, 2 Spot, 3 Flexible Spot, 11 Zone, 12 Expanded Flexible Spot, 13 Custom AF Area, 14 Tracking, 15 Face Tracking, 20 Animal Eye Tracking, 21 Human Eye Tracking, 255 Manual | shown; read only when its focus mode agrees with `0x201b` |
//!
//! The `0x94xx` blocks are enciphered byte by byte (the documentation calls it "a simple substitution
//! cipher"). On the sample files each plain byte `p < 249` is stored as `p³ mod 249` and bytes 249–255
//! as themselves (e.g. a stored `0x20` reads as AFAreaMode 20, stored `0x77` as `0x83`, FocusMode AF-C
//! with its top bit set); the map is a bijection on 0..249 (cubing is invertible mod 3 and mod 83), so
//! it is undone by a table of its inverse.
//!
//! **No real point** (nothing is stored, so nothing is drawn and Z zooms as before):
//! - a body the layout hasn't been confirmed on ([`SONY_MODELS`]), a missing or malformed tag, a zero
//!   frame size, or a centre outside its frame;
//! - manual focus (`0x201b` = 0, or the block's area mode says Manual): the camera still records where
//!   the AF frame was parked, not where anything was in focus (on the samples these are off-centre);
//! - the exact frame centre while the area used was Multi (Wide with nothing tracked): on the samples
//!   the wide area reports the centre when it picked no particular spot. A centred Spot, Zone or
//!   Flexible Spot frame is a real choice and is kept.

pub use crate::focus_read::{HEADER_PREFIX, HeaderRead, MAX_HEADER_BYTES, read_header};
use crate::{Orientation, Rect};
use craft_tiff::{ByteOrder, Ifd, Tiff, makernote, tags as t};
use serde::{Deserialize, Serialize};

/// Where the camera focused, on the upright photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusPoint {
    /// The focus frame, normalized to the photo's full, EXIF-oriented frame (y down, 0..1 on both
    /// axes, clipped to it) — the frame face regions use, so it follows Rotate, flips and crops.
    pub rect: Rect,
    /// Focus mode as the camera names it (`AF-C`, `AF-S`, `DMF`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The AF area chosen on the camera (`Wide`, `Zone`, `Flexible Spot`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// What the camera followed, when it says (`Animal Eye Tracking`, `Face tracking`, `Lock On AF`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<String>,
}

impl FocusPoint {
    /// The box's centre (normalized, upright).
    pub fn center(&self) -> (f64, f64) {
        ((self.rect.x0 + self.rect.x1) / 2.0, (self.rect.y0 + self.rect.y1) / 2.0)
    }
    /// The words that describe it, in order (mode, area, tracking), for "AF-C · Wide · Animal Eye Tracking".
    pub fn words(&self) -> Vec<&str> {
        [&self.mode, &self.area, &self.tracking].into_iter().filter_map(|s| s.as_deref()).filter(|s| !s.is_empty()).collect()
    }
    /// The box clipped to the frame and checked: `None` when it is not a usable box (non-finite,
    /// empty, or entirely outside the photo).
    pub fn sanitized(mut self) -> Option<FocusPoint> {
        let r = self.rect;
        if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (x0, x1) = (r.x0.min(r.x1).clamp(0.0, 1.0), r.x0.max(r.x1).clamp(0.0, 1.0));
        let (y0, y1) = (r.y0.min(r.y1).clamp(0.0, 1.0), r.y0.max(r.y1).clamp(0.0, 1.0));
        if x1 - x0 <= 0.0 || y1 - y0 <= 0.0 {
            return None;
        }
        self.rect = Rect { x0, y0, x1, y1 };
        Some(self)
    }
}

/// Sony bodies whose focus tags have been checked against real files. Another body may store the same
/// tags differently (or not mean the same by them): it shows no focus point until its files are checked.
pub const SONY_MODELS: &[&str] = &["ILCE-1", "ILCE-7SM3"];

/// Whether a camera as the catalog names it (`"SONY ILCE-1"`: the Exif Make, a space, the Model) is
/// one whose files can carry a focus point, so a photo from any other camera is never opened to look.
pub fn camera_supported(camera: &str) -> bool {
    let camera = camera.trim();
    camera.get(..4).is_some_and(|m| m.eq_ignore_ascii_case("SONY")) && camera.rsplit(' ').next().is_some_and(|model| SONY_MODELS.contains(&model))
}

/// Names what [`read`] decodes (its bodies), so a record of "looked, found nothing" made by this
/// decoder is not trusted by one that knows more bodies.
pub fn decoder_tag() -> String {
    format!("sony-v1:{}", SONY_MODELS.join(","))
}

const FOCUS_MODE: u16 = 0x201b;
const AF_AREA_SETTING: u16 = 0x201c;
const AF_TRACKING: u16 = 0x2021;
const FOCUS_LOCATION: u16 = 0x2027;
const FOCUS_FRAME_SIZE: u16 = 0x2037;
const TAG_9402: u16 = 0x9402;

/// Side of the box drawn when the camera gave a point but no frame size, as a fraction of the frame's
/// short side (about the size of the α1's smallest frames).
const DEFAULT_SIDE: f64 = 0.03;

/// The focus point a file's maker note records, or `None` (see the module notes for when).
/// `data` is the TIFF stream the maker note's offsets refer to (a whole raw, or an Exif block).
pub fn read(data: &[u8], tiff: &Tiff, make: Option<&str>, model: Option<&str>, orientation: Option<Orientation>) -> Option<FocusPoint> {
    let make = make?;
    if !make.to_ascii_uppercase().starts_with("SONY") || !SONY_MODELS.contains(&model?.trim()) {
        return None;
    }
    let e = tiff.exif()?.get(t::MAKER_NOTE)?;
    let mn = makernote::parse_makernote(data, e.offset, e.count() as u64, tiff.order, make)?;
    // ARWs from these bodies carry a header-less IFD; their JPEGs the `SONY DSC` header
    if !matches!(mn.kind, makernote::MakerNoteKind::Sony | makernote::MakerNoteKind::Plain) {
        return None;
    }
    sony(&mn.ifd, mn.order, orientation.unwrap_or_default())
}

fn byte(ifd: &Ifd, tag: u16) -> Option<u8> {
    ifd.value(tag)?.get_u64(0).and_then(|v| u8::try_from(v).ok())
}

/// Decode the Sony tags of a parsed maker-note IFD.
pub(crate) fn sony(ifd: &Ifd, order: ByteOrder, orientation: Orientation) -> Option<FocusPoint> {
    let loc = ifd.value(FOCUS_LOCATION)?;
    let [w, h, x, y] = [0, 1, 2, 3].map(|i| loc.get_u64(i));
    let (w, h, x, y) = (w? as f64, h? as f64, x? as f64, y? as f64);
    if w < 1.0 || h < 1.0 || x > w || y > h {
        return None;
    }
    let mode = byte(ifd, FOCUS_MODE);
    if mode == Some(0) {
        return None;
    }
    // the enciphered block: its focus mode must agree with the plain one, else its layout is not the one we know
    let block = ifd.bytes(TAG_9402).and_then(|b| {
        let (m, area) = (decipher(*b.get(0x16)?) & 0x7f, decipher(*b.get(0x17)?));
        (Some(m) == mode).then_some(area)
    });
    if block == Some(255) {
        return None;
    }
    let setting = byte(ifd, AF_AREA_SETTING);
    let tracking = byte(ifd, AF_TRACKING);
    let wide_untracked = match block {
        Some(a) => a == 0,
        None => setting == Some(0) && tracking.is_none_or(|t| t == 0),
    };
    if wide_untracked && x * 2.0 == w && y * 2.0 == h {
        return None;
    }
    // frame size: two int16u in the note's byte order, then a non-zero marker when there is a frame
    let size = ifd.bytes(FOCUS_FRAME_SIZE).and_then(|b| {
        let (fw, fh) = (order.read_u16(b, 0)? as f64, order.read_u16(b, 2)? as f64);
        let shown = b.get(4..6).is_none_or(|m| m.iter().any(|&v| v != 0));
        (shown && fw >= 1.0 && fh >= 1.0).then_some((fw, fh))
    });
    let (fw, fh) = size.unwrap_or_else(|| {
        let side = w.min(h) * DEFAULT_SIDE;
        (side, side)
    });
    let sensor = Rect { x0: (x - fw / 2.0) / w, y0: (y - fh / 2.0) / h, x1: (x + fw / 2.0) / w, y1: (y + fh / 2.0) / h };
    let name = |v: Option<u8>, f: fn(u8) -> Option<&'static str>| v.and_then(f).map(str::to_string);
    let area = name(setting, area_setting_name);
    let used = name(block, area_used_name);
    // what it followed: the area mode used when it says more than the setting (Animal Eye Tracking on
    // Wide), else the tracking setting when it is on
    let tracking = used.filter(|u| Some(u) != area.as_ref() && u != "Multi").or_else(|| name(tracking, tracking_name).filter(|t| t != "Off"));
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode: name(mode, focus_mode_name), area, tracking }.sanitized()
}

/// A minimal little-endian TIFF laid out as an α1 ARW's metadata is (IFD0 with Make `SONY` and Model `ILCE-1` →
/// Exif IFD → a header-less maker note with the focus tags: AF-C or the given FocusMode byte, Wide, Face tracking,
/// Animal Eye Tracking at (3226, 1980) of 8640 × 5760), followed by `samples` bytes standing in for sensor data. For
/// tests of the readers and the backfill in other crates; not a real raw (it has no image).
#[doc(hidden)]
pub fn synthetic_arw(mode: u8, samples: usize) -> Vec<u8> {
    fn entry(f: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: [u8; 4]) {
        f.extend_from_slice(&tag.to_le_bytes());
        f.extend_from_slice(&typ.to_le_bytes());
        f.extend_from_slice(&count.to_le_bytes());
        f.extend_from_slice(&value);
    }
    let encipher = |p: u8| if p >= 249 { p } else { (u32::from(p).pow(3) % 249) as u8 };
    let mut f = b"II*\0".to_vec();
    f.extend_from_slice(&8u32.to_le_bytes());
    // IFD0 at 8: Make (at 62), Model (at 68), Orientation, the Exif IFD (at 76)
    f.extend_from_slice(&4u16.to_le_bytes());
    entry(&mut f, t::MAKE, 2, 5, 62u32.to_le_bytes());
    entry(&mut f, t::MODEL, 2, 7, 68u32.to_le_bytes());
    entry(&mut f, t::ORIENTATION, 3, 1, [1, 0, 0, 0]);
    entry(&mut f, t::EXIF_IFD, 4, 1, 76u32.to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(b"SONY\0\0ILCE-1\0\0");
    // the Exif IFD: the maker note at 94, 156 bytes
    f.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut f, t::MAKER_NOTE, 7, 156, 94u32.to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    // the maker note's IFD, its values from 172
    f.extend_from_slice(&6u16.to_le_bytes());
    entry(&mut f, FOCUS_MODE, 1, 1, [mode, 0, 0, 0]);
    entry(&mut f, AF_AREA_SETTING, 1, 1, [0, 0, 0, 0]);
    entry(&mut f, AF_TRACKING, 1, 1, [1, 0, 0, 0]);
    entry(&mut f, FOCUS_LOCATION, 3, 4, 172u32.to_le_bytes());
    entry(&mut f, FOCUS_FRAME_SIZE, 7, 6, 180u32.to_le_bytes());
    entry(&mut f, TAG_9402, 7, 64, 186u32.to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    for v in [8640u16, 5760, 3226, 1980] {
        f.extend_from_slice(&v.to_le_bytes());
    }
    f.extend_from_slice(&[0xaf, 0, 0xb4, 0, 1, 1]);
    let mut block = [0u8; 64];
    block[0x16] = encipher(0x80 | mode);
    block[0x17] = encipher(20);
    f.extend_from_slice(&block);
    f.extend((0..samples).map(|i| (i % 251) as u8));
    f
}

/// Undo the `0x94xx` byte substitution: the stored byte is `p³ mod 249` for `p < 249`.
fn decipher(b: u8) -> u8 {
    if b >= 249 {
        return b;
    }
    (0..249u32).find(|p| (p * p * p) % 249 == u32::from(b)).and_then(|p| u8::try_from(p).ok()).unwrap_or(b)
}

fn focus_mode_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Manual",
        2 => "AF-S",
        3 => "AF-C",
        4 => "AF-A",
        6 => "DMF",
        7 => "AF-D",
        _ => return None,
    })
}

fn area_setting_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Wide",
        1 | 9 => "Center",
        3 | 4 => "Flexible Spot",
        11 => "Zone",
        12 => "Expanded Flexible Spot",
        13 => "Custom AF Area",
        _ => return None,
    })
}

fn tracking_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        1 => "Face tracking",
        2 => "Lock On AF",
        _ => return None,
    })
}

fn area_used_name(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Multi",
        1 => "Center",
        2 => "Spot",
        3 => "Flexible Spot",
        11 => "Zone",
        12 => "Expanded Flexible Spot",
        13 => "Custom AF Area",
        14 => "Tracking",
        15 => "Face Tracking",
        20 => "Animal Eye Tracking",
        21 => "Human Eye Tracking",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
