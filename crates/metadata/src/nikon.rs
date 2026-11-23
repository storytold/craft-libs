//! Nikon maker note fields spelled out for the "All Metadata" rows, and the autofocus points a Nikon body records.
//!
//! Tag identities, offsets and value names come from the public ExifTool tag-name documentation (Nikon table,
//! <https://exiftool.org/TagNames/Nikon.html>). No third-party code was read. Every field decoded here was compared
//! with ExifTool's output, run as a black box, on 670 NIKON D800E NEFs of one photographer (FX and DX crop, landscape
//! and portrait, AF-C 3D-tracking, AF-S, manual focus, ISO 100 to Hi 0.5). Only type-3 notes (`Nikon\0`, a version,
//! then a TIFF of their own) are read ([`parse_note`]): the Coolpix type-1 note numbers its tags differently.
//!
//! **Autofocus (AFInfo2, `0x00b7`, version `0100`).** Byte 4 AFDetectionMethod (0 phase detect: the viewfinder's
//! points; live view's contrast detect has none of them), 5 AFAreaMode, 6 FocusPointSchema (1 = 51-point), 7
//! PrimaryAFPoint (the point the camera focused with; a 51-point index, named below), 8..15 AFPointsUsed (7 bytes, a
//! bit set per point the camera reports having used: bit `n` is PrimaryAFPoint value `n + 1`, confirmed on the D800E
//! files against ExifTool's point names). Version `0100` records no in-focus points: only [`AfPoints::primary`]
//! (selected) and [`AfPoints::used`] are known.
//!
//! **Where the points lie** ([`POINTS_51`]) is not in the documentation; it was measured. A contributor shot 51
//! frames of a small dark target on a plain wall with a D800E in AF-S, Single Area, FX, landscape: one per point, the
//! chosen point's viewfinder square laid on the target. The target's centre in each camera preview (the darkest
//! compact blob against its local background, every detection checked by eye) gives that point's place in the FX
//! frame the preview shows: the sensor's active area, 7360 × 4912 from (8, 6) of the 7424 × 4924 the maker note's
//! CropHiSpeed counts in ([`ACTIVE`], [`SENSOR`]). The places fall on 11 columns (A1–A9 and E1–E9 under columns
//! 2–10, C6 the centre) at an even pitch of about 370 sensor pixels both ways; the outer groups (columns 1–4 and
//! 8–11) hold their A, B, D and E rows nearer the middle than the centre group (columns 5–7) does. The table is each
//! column's median x and each row's median y within its group (residuals 0.3 % rms, at most 1.4 %, the aim of a
//! hand-held shot). Seven more frames (DX: C6, C1, C11, A5, E5; C1 and C11 held upright) put every target inside the
//! point drawn. The box drawn is one viewfinder point ([`POINT_BOX`]).
//!
//! Only bodies whose files were checked decode their AF points ([`AF_MODELS`]); another body may use another schema
//! or the same bytes for something else.

use crate::focus::FocusPoint;
use crate::tags::TagRow;
use crate::{Orientation, Rect};
use craft_tiff::makernote::{self, MakerNote, MakerNoteKind};
use craft_tiff::{ByteOrder, Ifd, Value};

/// Bodies whose AFInfo2 was checked against real files (51-point, version `0100`), as Exif Model names them.
pub const AF_MODELS: &[&str] = &["NIKON D800E"];

const FOCUS_MODE: u16 = 0x0007;
const QUALITY: u16 = 0x0004;
const WHITE_BALANCE: u16 = 0x0005;
const FLASH_SETTING: u16 = 0x0008;
const FLASH_TYPE: u16 = 0x0009;
const ISO: u16 = 0x0002;
const ISO_INFO: u16 = 0x0025;
const CROP_HI_SPEED: u16 = 0x001b;
const COLOR_SPACE: u16 = 0x001e;
const ACTIVE_D_LIGHTING: u16 = 0x0022;
const VIGNETTE_CONTROL: u16 = 0x002a;
const LENS_TYPE: u16 = 0x0083;
const LENS: u16 = 0x0084;
const FLASH_MODE: u16 = 0x0087;
const SHOOTING_MODE: u16 = 0x0089;
const NEF_COMPRESSION: u16 = 0x0093;
const SHUTTER_COUNT: u16 = 0x00a7;
const HIGH_ISO_NR: u16 = 0x00b1;
const AF_INFO2: u16 = 0x00b7;

/// Whether an Exif Make names Nikon (`NIKON CORPORATION`, `NIKON`), without allocating.
pub(crate) fn is_nikon_make(make: &str) -> bool {
    make.trim().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("NIKON"))
}

/// Whether an Exif Model is one of [`AF_MODELS`] (case and surrounding space aside).
pub(crate) fn af_model(model: &str) -> bool {
    AF_MODELS.iter().any(|m| m.eq_ignore_ascii_case(model.trim()))
}

/// A Nikon type-3 maker note: the `count` bytes at `offset` of `data` (the TIFF stream the Exif IFD belongs to, or
/// the note's own bytes with `offset` 0: a type-3 note counts its offsets from its own header, so either works).
/// `None` for another make or another kind of note.
pub(crate) fn parse_note(data: &[u8], offset: u64, count: u64, order: ByteOrder, make: &str) -> Option<MakerNote> {
    if !is_nikon_make(make) {
        return None;
    }
    let mn = makernote::parse_makernote(data, offset, count, order, make)?;
    (mn.kind == MakerNoteKind::NikonV3).then_some(mn)
}

/// The 7424 × 4924 frame the D800E's CropHiSpeed counts in.
const SENSOR: (f64, f64) = (7424.0, 4924.0);
/// The active area an FX photo shows within [`SENSOR`] (x, y, width, height): the camera's preview and the raw
/// decoder's crop.
const ACTIVE: (f64, f64, f64, f64) = (8.0, 6.0, 7360.0, 4912.0);

/// The 51-point schema by PrimaryAFPoint value − 1: each point's name (5 rows A–E, 11 columns, C6 the centre) and its
/// centre in the [`ACTIVE`] frame, normalized (x right, y down, landscape), as measured on the D800E (module notes).
const POINTS_51: [(&str, f64, f64); 51] = [
    ("C6", 0.4969, 0.5019),
    ("B6", 0.4969, 0.4255),
    ("A5", 0.4969, 0.3476),
    ("D6", 0.4969, 0.5850),
    ("E5", 0.4969, 0.6570),
    ("C7", 0.5541, 0.5019),
    ("B7", 0.5541, 0.4255),
    ("A6", 0.5541, 0.3476),
    ("D7", 0.5541, 0.5850),
    ("E6", 0.5541, 0.6570),
    ("C5", 0.4427, 0.5019),
    ("B5", 0.4427, 0.4255),
    ("A4", 0.4427, 0.3476),
    ("D5", 0.4427, 0.5850),
    ("E4", 0.4427, 0.6570),
    ("C8", 0.6080, 0.5040),
    ("B8", 0.6080, 0.4377),
    ("A7", 0.6080, 0.3728),
    ("D8", 0.6080, 0.5684),
    ("E7", 0.6080, 0.6301),
    ("C9", 0.6541, 0.5040),
    ("B9", 0.6541, 0.4377),
    ("A8", 0.6541, 0.3728),
    ("D9", 0.6541, 0.5684),
    ("E8", 0.6541, 0.6301),
    ("C10", 0.7001, 0.5040),
    ("B10", 0.7001, 0.4377),
    ("A9", 0.7001, 0.3728),
    ("D10", 0.7001, 0.5684),
    ("E9", 0.7001, 0.6301),
    ("C11", 0.7510, 0.5040),
    ("B11", 0.7510, 0.4377),
    ("D11", 0.7510, 0.5684),
    ("C4", 0.3887, 0.5040),
    ("B4", 0.3887, 0.4377),
    ("A3", 0.3887, 0.3728),
    ("D4", 0.3887, 0.5684),
    ("E3", 0.3887, 0.6301),
    ("C3", 0.3449, 0.5040),
    ("B3", 0.3449, 0.4377),
    ("A2", 0.3449, 0.3728),
    ("D3", 0.3449, 0.5684),
    ("E2", 0.3449, 0.6301),
    ("C2", 0.2968, 0.5040),
    ("B2", 0.2968, 0.4377),
    ("A1", 0.2968, 0.3728),
    ("D2", 0.2968, 0.5684),
    ("E1", 0.2968, 0.6301),
    ("C1", 0.2483, 0.5040),
    ("B1", 0.2483, 0.4377),
    ("D1", 0.2483, 0.5684),
];

/// One AF point as the viewfinder draws it, in [`SENSOR`] pixels: a rectangle about 0.7 of the point pitch wide and
/// 0.5 of it tall (the proportions of the 51-point diagram in the public D800/D800E documentation).
const POINT_BOX: (f64, f64) = (260.0, 185.0);

/// A maker-note string with its padding (spaces, NULs) taken off; `None` when nothing is left.
fn clean(s: Option<String>) -> Option<String> {
    let s = s?.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
    (!s.is_empty()).then_some(s)
}

/// The frame a photo was recorded in, within [`SENSOR`] (x, y, width, height): the [`ACTIVE`] area without a crop,
/// else CropHiSpeed's crop. `None` for a crop record that is not one we know (the wrong type, too short, another
/// full size, a crop past it).
fn recorded_frame(note: &Ifd) -> Option<(f64, f64, f64, f64)> {
    let Some(v) = note.value(CROP_HI_SPEED) else { return Some(ACTIVE) };
    let Value::Short(c) = v else { return None };
    match c.as_slice() {
        [0, ..] => Some(ACTIVE),
        [_, fw, fh, cw, ch, cx, cy, ..] => {
            let [fw, fh, cw, ch, cx, cy] = [*fw, *fh, *cw, *ch, *cx, *cy].map(f64::from);
            ((fw, fh) == SENSOR && cw >= 1.0 && ch >= 1.0 && cx + cw <= fw && cy + ch <= fh).then_some((cx, cy, cw, ch))
        }
        _ => None,
    }
}

/// The focus point a checked Nikon body records, as a box on the upright photo: the primary AF point's place, carried
/// into the frame the photo was recorded in ([`recorded_frame`]: FX, DX, 1.2×, 5:4…) and turned by its orientation.
/// `None` without a primary point (manual focus, live view, an unchecked body), with a crop record we don't know, or
/// when the point lies outside the recorded frame. Its words are the focus mode and the AF area mode; the point's
/// name is [`FocusPoint::af_point`].
pub(crate) fn focus(note: &Ifd, model: &str, orientation: Orientation) -> Option<FocusPoint> {
    let af = af_points(note, Some(model))?;
    let (name, x, y) = POINTS_51.get(af.primary_index?)?;
    let mode = clean(note.string(FOCUS_MODE));
    if mode.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("MANUAL") || m.eq_ignore_ascii_case("MF")) {
        return None;
    }
    let (fx, fy, fw, fh) = recorded_frame(note)?;
    let (u, v) = ((ACTIVE.0 + x * ACTIVE.2 - fx) / fw, (ACTIVE.1 + y * ACTIVE.3 - fy) / fh);
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    let (hw, hh) = (POINT_BOX.0 / 2.0 / fw, POINT_BOX.1 / 2.0 / fh);
    let sensor = Rect { x0: u - hw, y0: v - hh, x1: u + hw, y1: v + hh };
    let area = af.area.map(str::to_string);
    FocusPoint { rect: orientation.map_norm_rect(sensor), mode, area, tracking: None, af_point: Some((*name).to_string()) }.sanitized()
}

/// The autofocus points a Nikon maker note records (51-point schema).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfPoints {
    /// The AF area mode set on the camera ("Dynamic Area (3D-tracking)", "Single Area"…).
    pub area: Option<&'static str>,
    /// The point the camera focused with (selected by the photographer, or by tracking), e.g. "C6".
    pub primary: Option<&'static str>,
    /// The points the camera reports having used, in PrimaryAFPoint order. Empty when it reports none.
    pub used: Vec<&'static str>,
    /// [`Self::primary`]'s place in [`POINTS_51`].
    primary_index: Option<usize>,
}

/// The AF points of a checked body's maker note, or `None`: another body, no AFInfo2, another version or schema
/// (manual focus and live view write schema 0), contrast-detect AF (live view: none of these points), or no point.
pub fn af_points(note: &Ifd, model: Option<&str>) -> Option<AfPoints> {
    if !af_model(model?) {
        return None;
    }
    let b = note.bytes(AF_INFO2)?;
    if b.get(0..4)? != b"0100" || *b.get(4)? != 0 || *b.get(6)? != 1 {
        return None;
    }
    let primary_index = usize::from(*b.get(7)?).checked_sub(1).filter(|&i| i < POINTS_51.len());
    let mask = b.get(8..15)?;
    let used: Vec<&str> = (0..POINTS_51.len()).filter(|&n| mask[n / 8] >> (n % 8) & 1 == 1).map(|n| POINTS_51[n].0).collect();
    if primary_index.is_none() && used.is_empty() {
        return None;
    }
    Some(AfPoints { area: af_area_phase(*b.get(5)?), primary: primary_index.map(|i| POINTS_51[i].0), used, primary_index })
}

/// The ISO a Nikon maker note records: `0x0002`'s second value (int16u[2], the ISO used), else ISOInfo's byte 0
/// (`0x0025`: `100 · 2^(raw/12 − 5)`). On D800E NEFs shot at Hi 0.3 and Hi 0.5, Exif ISOSpeedRatings and `0x0002` are
/// both absent and ISOInfo alone holds the ISO (ExifTool reads 8063 and 9051). A zero, an int16u overflow marker or
/// an unset `0xff` byte states nothing.
pub(crate) fn iso(note: &Ifd) -> Option<u32> {
    stated_iso(note).or_else(|| info_iso(note))
}

fn stated_iso(note: &Ifd) -> Option<u32> {
    note.value(ISO).and_then(|v| v.get_u64(1)).filter(|&v| v > 0 && v < u64::from(u16::MAX)).map(|v| v as u32)
}

fn info_iso(note: &Ifd) -> Option<u32> {
    let raw = *note.bytes(ISO_INFO)?.first()?;
    (raw > 0 && raw < 0xff).then(|| (100.0 * 2f64.powf(f64::from(raw) / 12.0 - 5.0)).round() as u32)
}

/// The lens a Nikon maker note records (`0x0084`, Lens: shortest and longest focal length, then the widest aperture at
/// each, like Exif LensSpecification), when plausible. Zero for a lens that doesn't report itself.
pub(crate) fn lens_spec(note: &Ifd) -> Option<[f64; 4]> {
    let s = note.f64s(LENS)?;
    let [short, long, wide, narrow] = s.as_slice() else { return None };
    let focal = |f: f64| f.is_finite() && f > 0.0 && f < 10_000.0;
    let aperture = |a: f64| a.is_finite() && a > 0.0 && a < 100.0;
    (focal(*short) && focal(*long) && long >= short && aperture(*wide) && aperture(*narrow)).then_some([*short, *long, *wide, *narrow])
}

fn af_area_phase(v: u8) -> Option<&'static str> {
    Some(match v {
        0 => "Single Area",
        1 => "Dynamic Area",
        2 => "Dynamic Area (closest subject)",
        3 => "Group Dynamic",
        4 => "Dynamic Area (9 points)",
        5 => "Dynamic Area (21 points)",
        6 => "Dynamic Area (51 points)",
        7 => "Dynamic Area (51 points, 3D-tracking)",
        8 => "Auto-area",
        9 => "Dynamic Area (3D-tracking)",
        10 => "Single Area (wide)",
        11 => "Dynamic Area (wide)",
        12 => "Dynamic Area (wide, 3D-tracking)",
        13 => "Group Area",
        _ => return None,
    })
}

fn iso_expansion(v: u16) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        0x101 => "Hi 0.3",
        0x102 => "Hi 0.5",
        0x103 => "Hi 0.7",
        0x104 => "Hi 1.0",
        0x105 => "Hi 1.3",
        0x106 => "Hi 1.5",
        0x107 => "Hi 1.7",
        0x108 => "Hi 2.0",
        0x109 => "Hi 2.3",
        0x10a => "Hi 2.5",
        0x10b => "Hi 2.7",
        0x10c => "Hi 3.0",
        0x10d => "Hi 3.3",
        0x10e => "Hi 3.5",
        0x10f => "Hi 3.7",
        0x110 => "Hi 4.0",
        0x111 => "Hi 4.3",
        0x112 => "Hi 4.5",
        0x113 => "Hi 4.7",
        0x114 => "Hi 5.0",
        0x201 => "Lo 0.3",
        0x202 => "Lo 0.5",
        0x203 => "Lo 0.7",
        0x204 => "Lo 1.0",
        _ => return None,
    })
}

fn crop_name(v: u16) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        1 => "1.3x Crop",
        2 => "DX Crop",
        3 => "5:4 Crop",
        4 => "3:2 Crop",
        6 => "16:9 Crop",
        8 => "2.7x Crop",
        9 => "DX Movie 16:9 Crop",
        10 => "1.3x Movie Crop",
        11 => "FX Uncropped",
        12 => "DX Uncropped",
        13 => "2.8x Movie Crop",
        14 => "1.4x Movie Crop",
        15 => "1.5x Movie Crop",
        17 => "FX 1:1 Crop",
        18 => "DX 1:1 Crop",
        _ => return None,
    })
}

fn active_d_lighting(v: u64) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        1 => "Low",
        3 => "Normal",
        5 => "High",
        7 => "Extra High",
        65535 => "Auto",
        _ => return None,
    })
}

fn high_iso_nr(v: u64) -> Option<&'static str> {
    Some(match v {
        0 => "Off",
        1 => "Minimal",
        2 => "Low",
        3 => "Medium Low",
        4 => "Normal",
        5 => "Medium High",
        6 => "High",
        _ => return None,
    })
}

fn flash_mode(v: u64) -> Option<&'static str> {
    Some(match v {
        0 => "Did Not Fire",
        1 => "Fired, Manual",
        3 => "Not Ready",
        7 => "Fired, External",
        8 => "Fired, Commander Mode",
        9 => "Fired, TTL Mode",
        18 => "LED Light",
        _ => return None,
    })
}

/// Nikon maker note `NEFCompression` (`0x0093`) spelled out.
pub(crate) fn nef_compression(v: u64) -> Option<&'static str> {
    Some(match v {
        1 => "Lossy (type 1)",
        2 => "Uncompressed",
        3 => "Lossless",
        4 => "Lossy (type 2)",
        5 => "Striped packed 12 bits",
        6 => "Uncompressed (reduced to 12 bit)",
        7 => "Unpacked 12 bits",
        8 => "Small",
        9 => "Packed 12 bits",
        10 => "Packed 14 bits",
        13 => "High Efficiency",
        14 => "High Efficiency*",
        _ => return None,
    })
}

/// [`nef_compression`] of a note.
pub(crate) fn note_nef_compression(note: &Ifd) -> Option<&'static str> {
    note.value(NEF_COMPRESSION).and_then(|v| v.get_u64(0)).and_then(nef_compression)
}

const SHOOTING_BITS: [(u16, &str); 10] = [
    (0, "Continuous"),
    (1, "Delay"),
    (2, "PC Control"),
    (3, "Self-timer"),
    (4, "Exposure Bracketing"),
    (5, "Auto ISO"),
    (6, "White-Balance Bracketing"),
    (7, "IR Control"),
    (8, "D-Lighting Bracketing"),
    (11, "Pre-capture"),
];

const LENS_TYPE_BITS: [(u8, &str); 8] = [(0, "MF"), (1, "D"), (2, "G"), (3, "VR"), (4, "1"), (5, "FT-1"), (6, "E"), (7, "AF-P")];

/// "50", "3.5" or "18-200": a focal length or aperture range, one number when both ends agree.
fn range(a: f64, b: f64) -> String {
    let f = |v: f64| if (v - v.round()).abs() < 0.05 { format!("{}", v.round()) } else { format!("{v:.1}") };
    if (a - b).abs() < 0.05 { f(a) } else { format!("{}-{}", f(a), f(b)) }
}

/// A readable row for each Nikon maker note field this module knows, in a fixed order; a field the note lacks, or
/// whose value is not one the documentation names, is left out (nothing is guessed).
pub fn rows(note: &Ifd, order: ByteOrder, model: Option<&str>) -> Vec<TagRow> {
    let mut out = Vec::new();
    let mut push = |tag: u16, name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            out.push(TagRow { group: "Nikon", tag, name: name.into(), value });
        }
    };
    let text = |tag| clean(note.string(tag));
    let num = |tag| note.value(tag).and_then(|v| v.get_u64(0));
    push(QUALITY, "Quality", text(QUALITY));
    push(FOCUS_MODE, "Focus Mode", text(FOCUS_MODE));
    push(WHITE_BALANCE, "White Balance", text(WHITE_BALANCE));
    let stated = stated_iso(note);
    push(ISO, "ISO", stated.map(|v| v.to_string()));
    if let Some(info) = note.bytes(ISO_INFO) {
        // the ISO this record states, only when 0x0002 has none (Hi ISO): the same ISO to a fraction otherwise
        push(ISO_INFO, "ISO", stated.is_none().then(|| info_iso(note)).flatten().map(|v| v.to_string()));
        push(ISO_INFO, "ISO Expansion", order.read_u16(info, 4).and_then(iso_expansion).map(str::to_string));
    }
    if let Some(bits) = num(SHOOTING_MODE) {
        // bit 0 clear is single frame, so the drive mode always leads
        let mut names: Vec<&str> = SHOOTING_BITS.iter().filter(|(b, _)| bits >> b & 1 == 1).map(|(_, n)| *n).collect();
        if bits & 1 == 0 {
            names.insert(0, "Single-Frame");
        }
        push(SHOOTING_MODE, "Shooting Mode", Some(names.join(", ")));
    }
    if let Some(Value::Short(c)) = note.value(CROP_HI_SPEED)
        && let Some(&kind) = c.first()
    {
        let size = match c.as_slice() {
            [k, fw, fh, cw, ch, ..] if *k != 0 && [fw, fh, cw, ch].iter().all(|&&v| v > 0) => format!(" ({cw} × {ch} of {fw} × {fh})"),
            _ => String::new(),
        };
        push(CROP_HI_SPEED, "Image Area", crop_name(kind).map(|n| format!("{n}{size}")));
    }
    push(ACTIVE_D_LIGHTING, "Active D-Lighting", num(ACTIVE_D_LIGHTING).and_then(active_d_lighting).map(str::to_string));
    push(HIGH_ISO_NR, "High ISO Noise Reduction", num(HIGH_ISO_NR).and_then(high_iso_nr).map(str::to_string));
    push(
        VIGNETTE_CONTROL,
        "Vignette Control",
        num(VIGNETTE_CONTROL).and_then(|v| [(0, "Off"), (1, "Low"), (3, "Normal"), (5, "High")].iter().find(|p| p.0 == v)).map(|p| p.1.to_string()),
    );
    push(
        COLOR_SPACE,
        "Color Space",
        num(COLOR_SPACE).and_then(|v| match v {
            1 => Some("sRGB".to_string()),
            2 => Some("Adobe RGB".to_string()),
            _ => None,
        }),
    );
    push(NEF_COMPRESSION, "NEF Compression", note_nef_compression(note).map(str::to_string));
    push(FLASH_SETTING, "Flash Setting", text(FLASH_SETTING));
    push(FLASH_TYPE, "Flash Type", text(FLASH_TYPE));
    push(FLASH_MODE, "Flash Mode", num(FLASH_MODE).and_then(flash_mode).map(str::to_string));
    if let Some([short, long, wide, narrow]) = lens_spec(note) {
        push(LENS, "Lens", Some(format!("{} mm f/{}", range(short, long), range(wide, narrow))));
    }
    if let Some(bits) = num(LENS_TYPE).and_then(|v| u8::try_from(v).ok()) {
        // no bit set: an autofocus lens of none of the named kinds
        let names: Vec<&str> = LENS_TYPE_BITS.iter().filter(|(b, _)| bits >> b & 1 == 1).map(|(_, n)| *n).collect();
        push(LENS_TYPE, "Lens Type", Some(if names.is_empty() { "AF".into() } else { names.join(" ") }));
    }
    push(SHUTTER_COUNT, "Shutter Count", num(SHUTTER_COUNT).filter(|&v| v > 0).map(|v| v.to_string()));
    if let Some(af) = af_points(note, model) {
        push(AF_INFO2, "AF Area Mode", af.area.map(str::to_string));
        push(AF_INFO2, "Primary AF Point", af.primary.map(str::to_string));
        push(AF_INFO2, "AF Points Used", Some(if af.used.is_empty() { "(none reported)".into() } else { af.used.join(", ") }));
    }
    out
}

/// A minimal little-endian TIFF laid out as a D800E NEF's metadata is (IFD0 with Make `NIKON CORPORATION` and Model
/// `NIKON D800E` -> Exif IFD -> a type-3 `Nikon` maker note holding FocusMode AF-C and AFInfo2: phase detect,
/// 3D-tracking, 51-point, PrimaryAFPoint `primary` (1 = C6), FX), followed by `samples` bytes standing in for sensor
/// data. For tests of the readers and the backfill in other crates; not a real raw (it has no image).
#[doc(hidden)]
pub fn synthetic_nef(primary: u8, samples: usize) -> Vec<u8> {
    use craft_tiff::{IfdBuilder, TiffWriter, tags as t};
    let mut note = IfdBuilder::new();
    note.set(0x0001, Value::Undefined(b"0210".to_vec()));
    note.set(FOCUS_MODE, Value::Ascii("AF-C".into()));
    let mut af = b"0100".to_vec();
    af.extend([0, 9, 1, primary]);
    af.resize(30, 0);
    note.set(AF_INFO2, Value::Undefined(af));
    let mut blob = b"Nikon\0\x02\x10\0\0".to_vec();
    blob.extend(TiffWriter::new(ByteOrder::Big, false).write(&[note]).unwrap_or_default());
    let mut exif = IfdBuilder::new();
    exif.set(t::MAKER_NOTE, Value::Undefined(blob));
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii("NIKON CORPORATION".into()));
    ifd0.set(t::MODEL, Value::Ascii("NIKON D800E".into()));
    ifd0.set_child(t::EXIF_IFD, exif);
    let mut f = TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap_or_default();
    f.extend((0..samples).map(|i| (i % 251) as u8));
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::tags as t;
    use craft_tiff::{IfdBuilder, Tiff, TiffWriter};

    const D800E: Option<&str> = Some("NIKON D800E");

    /// A maker-note IFD in `order` with what `fill` sets, parsed back as the readers see it.
    fn note_with(order: ByteOrder, fill: impl FnOnce(&mut IfdBuilder)) -> Ifd {
        let mut b = IfdBuilder::new();
        fill(&mut b);
        let bytes = TiffWriter::new(order, false).write(&[b]).unwrap();
        Tiff::parse(&bytes).unwrap().ifds.remove(0)
    }

    fn note(af: Option<Vec<u8>>) -> Ifd {
        note_with(ByteOrder::Big, |b| {
            b.set(FOCUS_MODE, Value::Ascii("AF-C  ".into()));
            if let Some(af) = af {
                b.set(AF_INFO2, Value::Undefined(af));
            }
        })
    }

    /// AFInfo2 `0100` as a D800E writes it: phase detect, 3D-tracking, 51-point, `primary`, then the used-points mask.
    fn af_info2(primary: u8, used: [u8; 7]) -> Vec<u8> {
        let mut b = b"0100".to_vec();
        b.extend([0, 9, 1, primary]);
        b.extend(used);
        b.resize(30, 0);
        b
    }

    #[test]
    fn af_points_name_the_primary_and_used_points_and_tell_them_apart() {
        // C7 (value 6) primary, the camera reports using C7 (bit 5) and C6 (bit 0)
        let af = af_points(&note(Some(af_info2(6, [0x21, 0, 0, 0, 0, 0, 0]))), D800E).unwrap();
        assert_eq!(af.primary, Some("C7"));
        assert_eq!(af.used, ["C6", "C7"]);
        assert_eq!(af.area, Some("Dynamic Area (3D-tracking)"));
        // a primary point with no used points reported keeps the two apart
        let af = af_points(&note(Some(af_info2(1, [0; 7]))), D800E).unwrap();
        assert_eq!((af.primary, af.used.is_empty()), (Some("C6"), true));
        // the last bit of the mask is point 51 (D1)
        assert_eq!(af_points(&note(Some(af_info2(0, [0, 0, 0, 0, 0, 0, 0x04]))), D800E).unwrap().used, ["D1"]);
        // the model as another program may have rewritten it
        assert!(af_points(&note(Some(af_info2(1, [0; 7]))), Some(" Nikon D800E ")).is_some());
    }

    #[test]
    fn af_points_refuse_what_was_not_checked() {
        assert_eq!(af_points(&note(Some(af_info2(6, [0; 7]))), Some("NIKON D850")), None, "an unchecked body");
        assert_eq!(af_points(&note(None), D800E), None, "no AFInfo2");
        let mut schema0 = af_info2(0, [0; 7]);
        schema0[6] = 0;
        assert_eq!(af_points(&note(Some(schema0)), D800E), None, "schema Off (manual focus, live view)");
        let mut contrast = af_info2(6, [0; 7]);
        contrast[4] = 1;
        assert_eq!(af_points(&note(Some(contrast)), D800E), None, "contrast detect (live view): none of these points");
        let mut v0101 = af_info2(6, [0; 7]);
        v0101[3] = b'1';
        assert_eq!(af_points(&note(Some(v0101)), D800E), None, "another version's layout");
        assert_eq!(af_points(&note(Some(af_info2(0, [0; 7]))), D800E), None, "no point at all");
        assert_eq!(af_points(&note(Some(af_info2(52, [0; 7]))), D800E), None, "a value past 51");
        // every truncation of the record is refused without a panic, the whole record read
        let whole = af_info2(6, [0x20, 0, 0, 0, 0, 0, 0]);
        for n in 0..whole.len() {
            let got = af_points(&note(Some(whole[..n].to_vec())), D800E);
            assert_eq!(got.is_some(), n >= 15, "{n} bytes");
        }
    }

    /// A TIFF laid out as a D800E NEF's metadata is: IFD0 (Make, Model, Orientation, and `pad` bytes of private data
    /// that push the rest past the first read) -> Exif -> a maker note with `header` and then its own big-endian
    /// TIFF holding FocusMode, CropHiSpeed (when `crop`) and AFInfo2 with `primary` and `schema`; then `filler`
    /// bytes standing in for the image.
    struct Nef<'a> {
        primary: u8,
        schema: u8,
        mode: &'a str,
        crop: Option<Value>,
        orientation: u16,
        header: &'a [u8],
        pad: usize,
        filler: usize,
    }

    impl Default for Nef<'_> {
        fn default() -> Self {
            Nef { primary: 1, schema: 1, mode: "AF-S", crop: None, orientation: 1, header: b"Nikon\0\x02\x10\0\0", pad: 0, filler: 0 }
        }
    }

    impl Nef<'_> {
        fn bytes(&self) -> Vec<u8> {
            let mut n = IfdBuilder::new();
            n.set(0x0001, Value::Undefined(b"0210".to_vec()));
            n.set(FOCUS_MODE, Value::Ascii(self.mode.into()));
            if let Some(c) = &self.crop {
                n.set(CROP_HI_SPEED, c.clone());
            }
            let mut af = af_info2(self.primary, [0; 7]);
            af[6] = self.schema;
            n.set(AF_INFO2, Value::Undefined(af));
            let mut blob = self.header.to_vec();
            blob.extend(TiffWriter::new(ByteOrder::Big, false).write(&[n]).unwrap());
            let mut exif = IfdBuilder::new();
            exif.set(t::MAKER_NOTE, Value::Undefined(blob));
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii("NIKON CORPORATION".into()));
            ifd0.set(t::MODEL, Value::Ascii("NIKON D800E".into()));
            ifd0.set(t::ORIENTATION, Value::Short(vec![self.orientation]));
            if self.pad > 0 {
                ifd0.set(0xc634, Value::Byte(vec![0x5a; self.pad]));
            }
            ifd0.set_child(t::EXIF_IFD, exif);
            let mut f = TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap();
            f.extend((0..self.filler).map(|i| (i % 251) as u8));
            f
        }
    }

    fn focus_of(nef: &Nef) -> Option<FocusPoint> {
        crate::read_exif(&nef.bytes()).focus
    }

    fn centre(nef: &Nef) -> Option<(f64, f64)> {
        focus_of(nef).map(|f| f.center())
    }

    /// The value (1-based) of a point name.
    fn value(name: &str) -> u8 {
        POINTS_51.iter().position(|p| p.0 == name).map(|i| i as u8 + 1).unwrap()
    }

    fn at(name: &str) -> Nef<'static> {
        Nef { primary: value(name), ..Nef::default() }
    }

    fn dx() -> Option<Value> {
        Some(Value::Short(vec![2, 7424, 4924, 4864, 3212, 1256, 856]))
    }

    #[test]
    fn the_primary_point_lands_where_the_grid_puts_it() {
        let c = |p: &str| centre(&at(p)).unwrap();
        let (c6, a1, e9, b1, d11) = (c("C6"), c("A1"), c("E9"), c("B1"), c("D11"));
        assert!((c6.0 - 0.5).abs() < 0.01 && (c6.1 - 0.5).abs() < 0.01, "C6 is the centre: {c6:?}");
        assert!(a1.0 < c6.0 && a1.1 < c6.1 && e9.0 > c6.0 && e9.1 > c6.1, "A1 upper left, E9 lower right");
        assert!(b1.0 < a1.0 && d11.0 > e9.0, "columns 1 and 11 lie outside A1-A9 and E1-E9");
        // the outer groups hold their rows nearer the middle than the centre group does
        assert!(c("A1").1 > c("A5").1 && c("E1").1 < c("E5").1);
        // the point's name is its own field, not a word about tracking
        let f = focus_of(&at("C6")).unwrap();
        assert_eq!((f.mode.as_deref(), f.area.as_deref(), f.tracking.as_deref()), (Some("AF-S"), Some("Dynamic Area (3D-tracking)"), None));
        assert_eq!(f.af_point.as_deref(), Some("C6"));
        // a point is wider than tall, as the viewfinder draws it
        assert!((f.rect.x1 - f.rect.x0) * ACTIVE.2 > (f.rect.y1 - f.rect.y0) * ACTIVE.3);
    }

    #[test]
    fn a_crop_spreads_the_points_and_cuts_off_those_outside_it() {
        let fx = centre(&at("C9")).unwrap();
        let dx_c9 = centre(&Nef { crop: dx(), ..at("C9") }).unwrap();
        assert!(dx_c9.0 - 0.5 > fx.0 - 0.5, "a DX frame is narrower, so the same point sits further out: {fx:?} {dx_c9:?}");
        // the DX crop counts from the sensor's corner, not the active area's: C1 and C11 sit as far in from either side
        let (c1, c11) = (centre(&Nef { crop: dx(), ..at("C1") }).unwrap(), centre(&Nef { crop: dx(), ..at("C11") }).unwrap());
        assert!((c1.0 - (1.0 - c11.0)).abs() < 0.004, "{c1:?} {c11:?}");
        // a crop that leaves column 11 out has no point for D11
        let narrow = Some(Value::Short(vec![2, 7424, 4924, 3000, 2000, 2212, 1462]));
        assert_eq!(centre(&Nef { crop: narrow, ..at("D11") }), None);
    }

    #[test]
    fn a_crop_record_we_dont_know_draws_nothing() {
        for (why, crop) in [
            ("past the frame", Value::Short(vec![2, 7424, 4924, 7000, 3212, 1256, 856])),
            ("cut short before its origin", Value::Short(vec![2, 7424, 4924, 4864, 3212])),
            ("another full size", Value::Short(vec![2, 6080, 4028, 4864, 3212, 600, 400])),
            ("the wrong type", Value::Long(vec![2, 7424, 4924, 4864, 3212, 1256, 856])),
        ] {
            assert_eq!(centre(&Nef { crop: Some(crop), ..at("C6") }), None, "{why}");
        }
        // CropHiSpeed Off is the FX frame
        assert_eq!(centre(&Nef { crop: Some(Value::Short(vec![0, 7424, 4924, 7424, 4924, 0, 0])), ..at("C6") }), centre(&at("C6")));
    }

    #[test]
    fn the_box_turns_with_the_photo() {
        // E9 is lower right on the sensor; a shot held with its right side up (orientation 8) shows it upper right
        let landscape = centre(&at("E9")).unwrap();
        let portrait = centre(&Nef { orientation: 8, ..at("E9") }).unwrap();
        assert!(landscape.0 > 0.5 && landscape.1 > 0.5);
        assert!(portrait.0 > 0.5 && portrait.1 < 0.5, "{portrait:?}");
    }

    #[test]
    fn no_point_is_made_up() {
        assert_eq!(centre(&Nef { mode: "MANUAL", ..at("C6") }), None, "manual focus");
        assert_eq!(centre(&Nef { mode: "MF\0\0", ..at("C6") }), None, "manual focus, padded");
        assert_eq!(centre(&Nef { schema: 0, ..at("C6") }), None, "schema 0 (live view, manual)");
        assert_eq!(centre(&Nef { primary: 0, ..at("C6") }), None, "no primary point");
        // a type-1 (Coolpix) note numbers its tags differently: nothing is read from it
        let v1 = Nef { header: b"Nikon\0\x01\0", ..at("C6") };
        assert_eq!(centre(&v1), None);
        assert!(!crate::file_tag_rows(&v1.bytes()).iter().any(|r| r.group == "Nikon"));
        // padding is no part of the mode
        assert_eq!(focus_of(&Nef { mode: "AF-S\0\0", ..at("C6") }).and_then(|f| f.mode).as_deref(), Some("AF-S"));
    }

    fn header_read(file: &[u8]) -> crate::focus::HeaderRead {
        crate::focus::read_header(file.len() as u64, &mut |off, buf| {
            let s = usize::try_from(off).unwrap();
            buf.copy_from_slice(&file[s..s + buf.len()]);
            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn the_header_read_finds_the_same_point_without_reading_the_file() {
        for nef in [at("A1"), Nef { crop: dx(), ..at("D11") }, Nef { orientation: 8, ..at("E9") }, Nef { header: b"Nikon\0\x01\0", ..at("C6") }] {
            let file = Nef { filler: 400_000, ..nef }.bytes();
            let r = header_read(&file);
            assert_eq!(r.focus, crate::read_exif(&file).focus);
            assert!(r.bytes_read < file.len() as u64 / 4, "{} of {}", r.bytes_read, file.len());
        }
        // a note that starts past the first read is fetched in further rounds, with the same result
        let file = Nef { pad: 3 * crate::focus::HEADER_PREFIX as usize, filler: 400_000, ..at("B8") }.bytes();
        let r = header_read(&file);
        assert!(r.bytes_read > crate::focus::HEADER_PREFIX, "{}", r.bytes_read);
        assert!(r.focus.is_some() && r.focus == crate::read_exif(&file).focus);
    }

    #[test]
    fn only_checked_nikon_bodies_are_opened() {
        assert!(crate::focus::camera_supported("NIKON CORPORATION NIKON D800E"));
        assert!(crate::focus::camera_supported("Nikon Corporation Nikon D800E"), "case aside");
        assert!(!crate::focus::camera_supported("NIKON CORPORATION NIKON D850"));
        assert!(!crate::focus::camera_supported("NIKON CORPORATION FOO_NIKON D800E"), "the model's whole words only");
        assert!(crate::focus::camera_supported("SONY ILCE-1"), "Sony unchanged");
    }

    #[test]
    fn rows_leave_out_what_the_note_does_not_say() {
        let rows = rows(&note(Some(af_info2(1, [1, 0, 0, 0, 0, 0, 0]))), ByteOrder::Big, D800E);
        let get = |n: &str| rows.iter().find(|r| r.name == n).map(|r| r.value.as_str());
        assert_eq!(get("Focus Mode"), Some("AF-C"));
        assert_eq!(get("Primary AF Point"), Some("C6"));
        assert_eq!(get("AF Points Used"), Some("C6"));
        assert_eq!(get("ISO"), None);
        assert_eq!(get("Shutter Count"), None);
        assert!(rows.iter().all(|r| r.group == "Nikon"));
    }

    #[test]
    fn rows_spell_out_the_values_as_documented() {
        let rows_of = |order: ByteOrder, fill: &dyn Fn(&mut IfdBuilder)| {
            let rows = rows(&note_with(order, fill), order, D800E);
            move |n: &str| rows.iter().find(|r| r.name == n).map(|r| r.value.clone())
        };
        // ISOInfo's ISOExpansion is read in the note's byte order: 0x0102 is Hi 0.5 either way round
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let expansion = match order {
                ByteOrder::Big => [1, 2],
                ByteOrder::Little => [2, 1],
            };
            let get = rows_of(order, &|b| {
                b.set(ISO_INFO, Value::Undefined(vec![138, 1, 12, 0, expansion[0], expansion[1], 60, 1, 12, 0, 0, 0, 0, 1]));
            });
            assert_eq!(get("ISO Expansion").as_deref(), Some("Hi 0.5"), "{order:?}");
            assert_eq!(get("ISO").as_deref(), Some("9051"), "{order:?}");
        }
        let get = rows_of(ByteOrder::Big, &|b| {
            b.set(ISO_INFO, Value::Undefined(vec![0xff, 1, 12, 0, 0, 0, 60, 1, 12, 0, 0, 0, 0, 1]));
            b.set(LENS_TYPE, Value::Byte(vec![0]));
            b.set(LENS, Value::Rational(vec![(50, 1), (5001, 100), (18, 10), (18, 10)]));
            b.set(CROP_HI_SPEED, Value::Short(vec![2, 0, 0, 0, 0, 0, 0]));
        });
        assert_eq!(get("ISO"), None, "an unset ISOInfo byte states no ISO");
        assert_eq!(get("Lens Type").as_deref(), Some("AF"), "a lens with no kind bit set is a plain AF lens");
        assert_eq!(get("Lens").as_deref(), Some("50 mm f/1.8"), "a prime reads as one focal length");
        assert_eq!(get("Image Area").as_deref(), Some("DX Crop"), "no size of zeros");
        let get = rows_of(ByteOrder::Big, &|b| {
            b.set(LENS, Value::Rational(vec![(200, 1), (80, 1), (28, 10), (28, 10)]));
        });
        assert_eq!(get("Lens"), None, "a lens record that can't be is left out");
    }
}
