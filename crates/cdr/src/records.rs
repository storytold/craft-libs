//! Hand-written decoders of the records a drawing's objects refer to: the `loda` argument table
//! (R6), transforms (R8), colors (R14), fills (R11) and outlines (R12, R13).

use crate::bytes::Rd;
use crate::layout::{ColorLayout, Profile};
use crate::model::{Blend, Cap, Color, Join, Matrix, Outline};
use crate::riff::push;
use crate::{Limits, Warning};

/// The tag of a record's current layout in tagged files (R11, R12).
const TAG_1300: u32 = 1300;
/// The most tagged records skipped before a fill's own (R11).
const MAX_SKIPPED: usize = 4;
/// The most records skipped before an outline's own (R12).
const MAX_OUTLINE_RECORDS: usize = 16;
/// The most UTF-16 units of a library color's name kept, and the most bytes or UTF-16 units of
/// a layer's.
pub(crate) const MAX_NAME: usize = 256;

/// A header field: u16 in short files, u32 otherwise.
fn word(r: &mut Rd, p: &Profile) -> Option<usize> {
    if p.short { r.u16().map(usize::from) } else { r.u32().map(|v| v as usize) }
}

/// A coordinate in points.
pub(crate) fn coord(r: &mut Rd, p: &Profile) -> Option<f64> {
    let raw = if p.short { f64::from(r.i16()?) } else { f64::from(r.i32()?) };
    Some(raw * p.unit())
}

/// An angle in degrees: tenths in short files, millionths otherwise (R7).
pub(crate) fn angle(r: &mut Rd, p: &Profile) -> Option<f64> {
    if p.short { Some(f64::from(r.i16()?) / 10.0) } else { Some(f64::from(r.i32()?) / 1_000_000.0) }
}

/// A `loda` record (R6): its object type and arguments as (type, bytes).
pub(crate) struct Loda<'a> {
    pub(crate) kind: u32,
    pub(crate) args: Vec<(u32, &'a [u8])>,
}

impl<'a> Loda<'a> {
    pub(crate) fn arg(&self, kind: u32) -> Option<&'a [u8]> {
        self.args.iter().find(|(k, _)| *k == kind).map(|(_, b)| *b)
    }
}

pub(crate) fn loda<'a>(b: &'a [u8], p: &Profile, limits: &Limits, warn: &mut Vec<Warning>) -> Option<Loda<'a>> {
    let mut r = Rd::new(b);
    let (_len, count, offsets_at, types_at, kind) = (word(&mut r, p)?, word(&mut r, p)?, word(&mut r, p)?, word(&mut r, p)?, word(&mut r, p)?);
    // The types table is indexed from the stored count, however many arguments are read.
    let stored = count;
    let count = if count > limits.arguments {
        push(warn, Warning::Damaged);
        limits.arguments
    } else {
        count
    };
    let table_bytes = count.checked_add(1)?.checked_mul(p.word())?;
    if offsets_at.checked_add(table_bytes)? > b.len() {
        return None;
    }
    let mut offsets = Vec::with_capacity(count + 1);
    let mut ro = Rd::at(b, offsets_at);
    for _ in 0..=count {
        offsets.push(word(&mut ro, p)?);
    }
    let mut args = Vec::with_capacity(count);
    for i in 0..count {
        // The types table runs the other way: offset i pairs with type stored - 1 - i.
        let kind = word(&mut Rd::at(b, types_at.checked_add((stored - 1 - i).checked_mul(p.word())?)?), p)? as u32;
        let (start, end) = (*offsets.get(i)?, *offsets.get(i + 1)?);
        match b.get(start..end) {
            Some(body) => args.push((kind, body)),
            None => push(warn, Warning::Damaged),
        }
    }
    Some(Loda { kind: kind as u32, args })
}

/// The matrices of a `trfd` record composed in file order, in points (R8). Each entry ends where
/// the next offset points, and the last at the offset after it (R51); an entry too short for its
/// matrix makes the record unreadable.
pub(crate) fn trfd(b: &[u8], p: &Profile) -> Option<Matrix> {
    let mut r = Rd::new(b);
    let (_len, count, offsets_at) = (word(&mut r, p)?, word(&mut r, p)?, word(&mut r, p)?);
    let mut m = Matrix::IDENTITY;
    let k = p.unit();
    // Each entry takes at least its offset's bytes, which bounds the count by the record's size.
    for i in 0..count.min(b.len() / p.word()) {
        let slot = offsets_at.checked_add(i.checked_mul(p.word())?)?;
        let start = word(&mut Rd::at(b, slot), p)?;
        // A table that stops before the last entry's end offset leaves that entry the rest of the
        // record.
        let end = word(&mut Rd::at(b, slot.checked_add(p.word())?), p).unwrap_or(b.len());
        let mut e = Rd::at(b.get(start..end)?, p.trfd_head);
        if e.u16()? != 8 {
            continue;
        }
        e.skip(p.matrix_skip)?;
        let next = if p.fixed_matrix {
            fixed_matrix(&mut e, k)?
        } else {
            let (a, c, tx, bb, d, ty) = (e.f64()?, e.f64()?, e.f64()?, e.f64()?, e.f64()?, e.f64()?);
            Matrix([a, bb, c, d, tx * k, ty * k])
        };
        m = next.after(&m);
    }
    Some(m)
}

/// A number in 16.16 fixed point.
pub(crate) fn fixed(r: &mut Rd) -> Option<f64> {
    Some(f64::from(r.i32()?) / 65536.0)
}

/// A matrix of versions 3 and 4 (R37): six i32 `a c tx b d ty`, the factors in 16.16 fixed point
/// and `tx` and `ty` in coordinate units of `k` points.
pub(crate) fn fixed_matrix(r: &mut Rd, k: f64) -> Option<Matrix> {
    let (a, c, tx) = (fixed(r)?, fixed(r)?, f64::from(r.i32()?));
    let (b, d, ty) = (fixed(r)?, fixed(r)?, f64::from(r.i32()?));
    Some(Matrix([a, b, c, d, tx * k, ty * k]))
}

/// A color in the layout of the file's version (R14, R36). Unknown models read as black with a
/// warning.
pub(crate) fn color(r: &mut Rd, p: &Profile, warn: &mut Vec<Warning>) -> Option<Color> {
    match p.color {
        ColorLayout::Record12 => {
            let model = r.u16()?;
            let palette = r.u16()?;
            r.skip(4)?;
            let v: [u8; 4] = r.bytes(4)?.try_into().ok()?;
            Some(model_color(model, palette, v, warn))
        }
        ColorLayout::Words12 => {
            let model = r.u16()?;
            let w = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
            r.skip(2)?;
            // The models take 8-bit values, as in the other layouts; a larger value reads as 255.
            Some(early_color(model, w.map(|x| u8::try_from(x).unwrap_or(u8::MAX)), w[0], w[1], warn))
        }
        ColorLayout::Bytes5 => Some(color5(r.bytes(5)?.try_into().ok()?, warn)),
    }
}

/// A 5-byte color (R36): a model byte and four value bytes.
pub(crate) fn color5(b: [u8; 5], warn: &mut Vec<Warning>) -> Color {
    let [model, v @ ..] = b;
    early_color(u16::from(model), v, u16::from_le_bytes([v[0], v[1]]), u16::from_le_bytes([v[2], v[3]]), warn)
}

/// A color of versions 1 to 4 (R36). Model 1 is CMYK in percent. Model 0 is a numbered ink and
/// its tint in percent: 0 is black, 1 is white, and other numbers are inks of a library the file
/// doesn't store. Other models read as in later versions.
fn early_color(model: u16, v: [u8; 4], index: u16, tint: u16, warn: &mut Vec<Warning>) -> Color {
    let pct = |x: f32| (x / 100.0).clamp(0.0, 1.0);
    match (model, index) {
        (1, _) => Color::Cmyk(v.map(|x| pct(f32::from(x)))),
        (0, 0) => Color::Gray(pct(f32::from(tint))),
        (0, 1) => Color::Gray(0.0),
        (0, number) => {
            push(warn, Warning::SpotLooks);
            Color::Spot { name: format!("Spot 0:{number}"), tint: pct(f32::from(tint)), look: None }
        }
        _ => model_color(model, 0, v, warn),
    }
}

/// The color of `model` with the four value bytes `v`, from version 5 (R14).
fn model_color(model: u16, palette: u16, v: [u8; 4], warn: &mut Vec<Warning>) -> Color {
    let (v0, v1, v2, v3) = (f32::from(v[0]), f32::from(v[1]), f32::from(v[2]), f32::from(v[3]));
    let pct = |x: f32| (x / 100.0).clamp(0.0, 1.0);
    let byte = |x: f32| x / 255.0;
    match model {
        2 => Color::Cmyk([pct(v0), pct(v1), pct(v2), pct(v3)]),
        3 | 17 => Color::Cmyk([byte(v0), byte(v1), byte(v2), byte(v3)]),
        4 => Color::Cmyk([byte(v0), byte(v1), byte(v2), 0.0]),
        5 | 21 => Color::Rgb([byte(v2), byte(v1), byte(v0)]),
        9 => Color::Gray(1.0 - byte(v0)),
        20 => Color::Registration,
        1 | 14 | 25 => {
            let number = u16::from_le_bytes([v[0], v[1]]);
            let tint = pct(f32::from(u16::from_le_bytes([v[2], v[3]])));
            push(warn, Warning::SpotLooks);
            // Each library numbers its own colors: the name holds the palette and the number.
            Color::Spot { name: format!("Spot {palette}:{number}"), tint, look: None }
        }
        _ => {
            push(warn, Warning::UnknownColors);
            Color::Rgb([0.0; 3])
        }
    }
}

/// A UTF-16 name stored as a character count and its code units. Every shape that uses the color
/// gets its own copy of the name: a name longer than [`MAX_NAME`] units keeps its first ones and a
/// hash of the whole name, with a warning, so two long names that start alike stay apart.
fn counted_utf16(r: &mut Rd, warn: &mut Vec<Warning>) -> Option<String> {
    let n = r.u32()? as usize;
    let b = r.bytes(n.checked_mul(2)?)?;
    // Trailing NULs are padding.
    let all = b.as_chunks::<2>().0;
    let all = all.get(..all.iter().rposition(|c| *c != [0, 0]).map_or(0, |i| i + 1)).unwrap_or_default();
    let units = all.iter().map(|c| u16::from_le_bytes(*c));
    if all.len() <= MAX_NAME {
        let units: Vec<u16> = units.collect();
        return Some(String::from_utf16_lossy(&units));
    }
    push(warn, Warning::TooDeep);
    // FNV-1a over the code units.
    let hash = units.clone().fold(0xcbf2_9ce4_8422_2325_u64, |h, u| (h ^ u64::from(u)).wrapping_mul(0x0100_0000_01b3));
    let kept: Vec<u16> = units.take(MAX_NAME).collect();
    Some(format!("{} {hash:016x}", String::from_utf16_lossy(&kept)))
}

/// The color of the property list of a tagged uniform fill (R11) or outline (R12): its color
/// record, named by the library name it carries.
pub(crate) fn property_color(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> Option<Color> {
    let mut r = Rd::new(b);
    let (mut found, mut name) = (None, None);
    while r.left() >= 5 {
        let kind = r.u8()?;
        let len = r.u32()? as usize;
        if kind == 0 {
            break;
        }
        let body = r.bytes(len)?;
        let mut br = Rd::new(body);
        match kind {
            1 => found = color(&mut br, p, warn),
            8 => name = br.skip(16).and_then(|_| counted_utf16(&mut br, warn)),
            3 => name = name.or_else(|| counted_utf16(&mut br, warn)),
            _ => {}
        }
    }
    match (found?, name.filter(|n| !n.is_empty())) {
        // A library color with its name: the record's sRGB has the tint applied already.
        (Color::Rgb(rgb), Some(name)) => {
            let tint = b_tint(b).unwrap_or(1.0);
            Some(Color::Spot { name, tint, look: Some(untinted(rgb, tint)) })
        }
        (c, _) => Some(c),
    }
}

/// The tint byte of the first color property of a property list (model 21 keeps it last).
fn b_tint(b: &[u8]) -> Option<f32> {
    let mut r = Rd::new(b);
    while r.left() >= 5 {
        let kind = r.u8()?;
        let len = r.u32()? as usize;
        let body = r.bytes(len)?;
        if kind == 1 {
            let mut c = Rd::new(body);
            return (c.u16()? == 21).then(|| f32::from(*body.get(11).unwrap_or(&100)) / 100.0).map(|t| t.clamp(0.0, 1.0));
        }
    }
    None
}

/// The full-strength color behind sRGB `rgb` shown at `tint` (a tint mixes toward white).
pub(crate) fn untinted(rgb: [f32; 3], tint: f32) -> [f32; 3] {
    if tint <= 0.0 {
        return rgb;
    }
    rgb.map(|v| (1.0 - (1.0 - v) / tint).clamp(0.0, 1.0))
}

/// A fill record: its id, and its color (`None`: no fill). Fills of other kinds read as none with
/// a warning (R11).
pub(crate) fn fill(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> Option<(u32, Option<Color>)> {
    let mut r = Rd::new(b);
    let id = r.u32()?;
    let body = if p.tagged { tagged_body(&mut r)? } else { r.bytes(r.left())? };
    let mut r = Rd::new(body);
    let kind = r.u16()?;
    let color = match kind {
        0 => None,
        1 if p.tagged => {
            if r.u32()? != TAG_1300 {
                push(warn, Warning::Damaged);
                return Some((id, None));
            }
            // A uniform fill whose color can't be read is damage, not a fill of none.
            let len = r.u32()? as usize;
            Some(property_color(r.bytes(len.min(r.left()))?, p, warn)?)
        }
        1 => {
            r.skip(2)?;
            Some(color(&mut r, p, warn)?)
        }
        _ => {
            push(warn, Warning::UnsupportedFills);
            None
        }
    };
    Some((id, color))
}

/// The body of the record tagged 1300 that follows tagged records of later layouts (R11).
fn tagged_body<'a>(r: &mut Rd<'a>) -> Option<&'a [u8]> {
    for _ in 0..=MAX_SKIPPED {
        let (tag, len) = (r.u32()?, r.u32()? as usize);
        let body = r.bytes(len.min(r.left()))?;
        if tag == TAG_1300 {
            return Some(body);
        }
    }
    None
}

/// An outline record: its id and the outline (`None`: no outline) (R12, R13).
pub(crate) fn outline(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> Option<(u32, Option<Outline>)> {
    let mut r = Rd::new(b);
    let id = r.u32()?;
    // A library color named in a record of type 2, with the warnings its read gave.
    let mut named = None;
    if p.tagged {
        let mut found = false;
        for _ in 0..MAX_OUTLINE_RECORDS {
            let (rec, len) = (r.u32()?, r.u32()? as usize);
            if rec == 1 {
                found = true;
                break;
            }
            let body = r.bytes(len)?;
            // Type 2 holds the outline's color as a property list, as a uniform fill does (R12).
            // Only a library color with its name is taken from it; any other color comes from
            // the fixed field below.
            if rec == 2 {
                let mut w = Vec::new();
                if let Some(c @ Color::Spot { look: Some(_), .. }) = property_color(body, p, &mut w) {
                    named = Some((c, w));
                }
            }
        }
        if !found {
            return None;
        }
    }
    let (line_type, cap, join) = (r.u16()?, r.u16()?, r.u16()?);
    r.skip(p.outl_after_join)?;
    let width = coord(&mut r, p)?.abs();
    let _stretch = r.u16()?;
    r.skip(p.outl_after_stretch)?;
    let _nib = angle(&mut r, p)?;
    r.skip(p.outl_after_angle)?;
    let color = match named {
        Some((named, w)) => {
            color(&mut r, p, &mut Vec::new())?;
            for x in w {
                push(warn, x);
            }
            named
        }
        None => color(&mut r, p, warn)?,
    };
    r.skip(p.outl_after_color)?;
    let count = usize::from(r.u16()?);
    let first_dash = r.pos();
    let fits = r.left() / 2;
    if count > fits {
        push(warn, Warning::Damaged);
    }
    // At most 65,535; the read's dash limit is applied where shapes use them.
    let dashes: Vec<f64> = (0..count.min(fits)).filter_map(|_| r.u16().map(f64::from)).collect();
    if let Some(after) = p.marker_after_dash {
        let mut m = Rd::at(b, first_dash + after);
        if matches!((m.u32(), m.u32()), (Some(s), Some(e)) if s != 0 || e != 0) {
            push(warn, Warning::Arrowheads);
        }
    }
    // Bit 0 of the line type: no outline (R13).
    if line_type & 1 != 0 {
        return Some((id, None));
    }
    let cap = match cap {
        1 => Cap::Round,
        2 => Cap::Square,
        _ => Cap::Butt,
    };
    let join = match join {
        1 => Join::Round,
        2 => Join::Bevel,
        _ => Join::Miter,
    };
    // Bit 2 of the line type marks the outline dashed; without it the dashes stored are not drawn
    // (R48).
    let dashes = if line_type & 4 != 0 && dashes.iter().any(|d| *d > 0.0) { dashes } else { Vec::new() };
    // Bit 4 of the line type: behind the fill (R49).
    let behind_fill = line_type & 0x10 != 0;
    // Bit 5 of the line type: scaled with the shape (R50).
    let scale_with = line_type & 0x20 != 0;
    Some((id, Some(Outline { width, color, cap, join, dashes, behind_fill, scale_with })))
}

/// The opacity and blend of argument 8000 from version 13 (R47): records of a u32 version, a u32
/// length and a body. A body of version 1300 that is [`UNIFORM_LEN`] bytes long is a uniform
/// transparency: an f64 at byte 61 and the merge mode at byte 77. VectorCraft reads the f64 as the
/// transparency in percent, 0 opaque and 100 clear. `None` for any other argument, such as a lens,
/// and for a value out of range or a merge mode without a VectorCraft blend mode.
pub(crate) fn transparency(b: &[u8]) -> Option<(f32, Blend)> {
    let mut r = Rd::new(b);
    while r.left() >= 8 {
        let (since, len) = (r.u32()?, r.u32()? as usize);
        let body = r.bytes(len)?;
        if since != TAG_1300 {
            continue;
        }
        if body.len() != UNIFORM_LEN {
            return None;
        }
        let value = Rd::at(body, 61).f64()?;
        if !(0.0..=100.0).contains(&value) {
            return None;
        }
        return Some(((1.0 - value / 100.0) as f32, blend(*body.get(77)?)?));
    }
    None
}

/// The length of a uniform transparency's record body (R47).
const UNIFORM_LEN: usize = 117;

/// The blend mode of a transparency's merge mode byte (R47); `None` for the modes VectorCraft has
/// no blend mode for.
fn blend(mode: u8) -> Option<Blend> {
    Some(match mode {
        0 => Blend::Normal,
        9 => Blend::Multiply,
        11 => Blend::Lighten,
        12 => Blend::Darken,
        14 => Blend::Color,
        15 => Blend::Hue,
        16 => Blend::Saturation,
        17 => Blend::Luminosity,
        24 => Blend::Difference,
        28 => Blend::Screen,
        29 => Blend::Overlay,
        30 => Blend::SoftLight,
        31 => Blend::HardLight,
        33 => Blend::ColorDodge,
        34 => Blend::ColorBurn,
        36 => Blend::Exclusion,
        _ => return None,
    })
}

/// A name argument (R9): Windows-1252 up to a NUL before version 12, UTF-16LE from it. A name
/// longer than [`MAX_NAME`] bytes or UTF-16 units keeps its first ones (without half of a
/// surrogate pair) and a hash of the whole name, with a warning, as a library color's name does.
pub(crate) fn name(b: &[u8], p: &Profile, warn: &mut Vec<Warning>) -> String {
    if p.utf16_names {
        let units = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0);
        let (mut kept, hash) = capped(units, warn);
        if hash.is_some() && kept.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            kept.pop();
        }
        with_hash(String::from_utf16_lossy(&kept), hash)
    } else {
        let bytes = b.iter().copied().take_while(|c| *c != 0);
        let (kept, hash) = capped(bytes, warn);
        with_hash(kept.into_iter().map(windows_1252).collect(), hash)
    }
}

/// The first [`MAX_NAME`] of `units`, and when there are more, an FNV-1a hash of them all, with a
/// warning.
fn capped<T: Copy + Into<u64>>(units: impl Iterator<Item = T> + Clone, warn: &mut Vec<Warning>) -> (Vec<T>, Option<u64>) {
    if units.clone().nth(MAX_NAME).is_none() {
        return (units.collect(), None);
    }
    push(warn, Warning::TooDeep);
    let hash = units.clone().fold(0xcbf2_9ce4_8422_2325_u64, |h, u| (h ^ u.into()).wrapping_mul(0x0100_0000_01b3));
    (units.take(MAX_NAME).collect(), Some(hash))
}

/// `name`, followed by `hash` in hexadecimal when there is one.
fn with_hash(name: String, hash: Option<u64>) -> String {
    match hash {
        Some(h) => format!("{name} {h:016x}"),
        None => name,
    }
}

/// The characters Windows-1252 puts at 0x80 to 0x9F (U+FFFD where it has none); the other bytes
/// are Latin-1.
pub(crate) const CP1252_HIGH: [char; 32] = [
    '€', '\u{FFFD}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{FFFD}', 'Ž', '\u{FFFD}', '\u{FFFD}', '‘', '’', '“', '”', '•', '–',
    '—', '˜', '™', 'š', '›', 'œ', '\u{FFFD}', 'ž', 'Ÿ',
];

fn windows_1252(c: u8) -> char {
    match c {
        0x80..=0x9F => CP1252_HIGH.get(usize::from(c - 0x80)).copied().unwrap_or('\u{FFFD}'),
        _ => char::from(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An outline that names an arrowhead says so with a warning, in each version whose arrowhead
    /// ids have a known place: 22 bytes after the first dash in versions 6 to 12, 20 from 13.
    #[test]
    fn arrowheads_are_named_from_version_6() {
        let pen = crate::synth::Pen { line_type: 0, width: 1.0, cap: 0, join: 0 };
        let color = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100];
        for (version, place) in [
            (500, None),
            (600, Some(22)),
            (700, Some(22)),
            (800, Some(22)),
            (900, Some(22)),
            (1000, Some(22)),
            (1100, Some(22)),
            (1200, Some(22)),
            (1300, Some(20)),
            (1700, Some(20)),
        ] {
            let p = crate::layout::for_version(version).unwrap();
            let plain = crate::synth::V(version).outline(7, pen, &color, &[]);
            // The record ends with 32 bytes after its dashes, and this one has none.
            let first_dash = plain.len() - 32;
            let mut w = vec![];
            assert!(outline(&plain, &p, &mut w).is_some_and(|(id, o)| id == 7 && o.is_some()), "{version}");
            assert!(w.is_empty(), "{version}: {w:?}");
            let Some(place) = place else { continue };
            let mut arrow = plain.clone();
            arrow[first_dash + place..first_dash + place + 4].copy_from_slice(&9u32.to_le_bytes());
            let mut w = vec![];
            assert!(outline(&arrow, &p, &mut w).is_some(), "{version}");
            assert_eq!(w, vec![Warning::Arrowheads], "{version}");
        }
    }

    #[test]
    fn single_byte_names_read_as_windows_1252() {
        let p = crate::layout::for_version(1100).unwrap();
        assert_eq!(name(b"Caf\xe9 \x80\x96\0junk", &p, &mut vec![]), "Café €–");
        let p = crate::layout::for_version(1200).unwrap();
        assert_eq!(name(&[0x41, 0, 0x42, 0, 0, 0, 0x43, 0], &p, &mut vec![]), "AB");
    }

    /// A layer name longer than [`MAX_NAME`] bytes or UTF-16 units keeps its first ones and a hash
    /// of the whole name, with a warning, in both encodings. A cut never keeps half of a surrogate
    /// pair.
    #[test]
    fn long_layer_names_are_cut_with_a_hash() {
        for version in [1100, 1200] {
            let p = crate::layout::for_version(version).unwrap();
            let bytes =
                |s: &str| if p.utf16_names { s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>() } else { s.as_bytes().to_vec() };
            let mut w = vec![];
            let a = name(&bytes(&"N".repeat(100_000)), &p, &mut w);
            let b = name(&bytes(&format!("{}M", "N".repeat(99_999))), &p, &mut vec![]);
            assert_ne!(a, b, "{version}");
            assert!(a.starts_with(&"N".repeat(MAX_NAME)) && a.chars().count() == MAX_NAME + 17, "{version}: {}", a.len());
            assert_eq!(w, vec![Warning::TooDeep], "{version}");
            let mut w = vec![];
            assert_eq!(name(&bytes(&"N".repeat(MAX_NAME)), &p, &mut w), "N".repeat(MAX_NAME), "{version}");
            assert!(w.is_empty(), "{version}");
        }
        let p = crate::layout::for_version(1200).unwrap();
        let emoji: Vec<u8> = format!("N{}", "\u{1F600}".repeat(200)).encode_utf16().flat_map(u16::to_le_bytes).collect();
        let cut = name(&emoji, &p, &mut vec![]);
        assert!(!cut.contains('\u{FFFD}'), "{cut}");
        assert_eq!(cut.chars().filter(|c| *c == '\u{1F600}').count(), (MAX_NAME - 1) / 2);
    }

    #[test]
    fn colors_read_in_their_models() {
        let p = crate::layout::for_version(500).unwrap();
        let rec = |model: u16, v: [u8; 4]| {
            let mut b = model.to_le_bytes().to_vec();
            b.extend([0; 6]);
            b.extend(v);
            let mut w = vec![];
            (color(&mut Rd::new(&b), &p, &mut w), w)
        };
        assert_eq!(rec(2, [100, 50, 0, 10]).0, Some(Color::Cmyk([1.0, 0.5, 0.0, 0.1])));
        assert_eq!(rec(5, [255, 0, 0, 0]).0, Some(Color::Rgb([0.0, 0.0, 1.0])));
        assert_eq!(rec(9, [255, 0, 0, 0]).0, Some(Color::Gray(0.0)));
        assert_eq!(rec(25, [7, 0, 40, 0]).0, Some(Color::Spot { name: "Spot 0:7".into(), tint: 0.4, look: None }));
        // The same number in another library is another color.
        let mut other = 25u16.to_le_bytes().to_vec();
        other.extend(3u16.to_le_bytes());
        other.extend([0; 4]);
        other.extend([7, 0, 40, 0]);
        assert_eq!(color(&mut Rd::new(&other), &p, &mut vec![]), Some(Color::Spot { name: "Spot 3:7".into(), tint: 0.4, look: None }));
        let (c, w) = rec(6, [1, 2, 3, 4]);
        assert_eq!((c, w), (Some(Color::Rgb([0.0; 3])), vec![Warning::UnknownColors]));
    }

    #[test]
    fn colors_of_versions_3_and_4_read_models_0_and_1() {
        let read = |version: u16, model: u8, v: [u16; 4]| {
            let p = crate::layout::for_version(version).unwrap();
            let b: Vec<u8> = if version < 400 {
                std::iter::once(model).chain(v.map(|x| x as u8)).collect()
            } else {
                u16::from(model).to_le_bytes().into_iter().chain(v.iter().flat_map(|x| x.to_le_bytes())).chain([0, 0]).collect()
            };
            let mut r = Rd::new(&b);
            let mut w = vec![];
            let c = color(&mut r, &p, &mut w);
            assert_eq!(r.left(), 0, "{version}");
            (c, w)
        };
        for version in [300, 400] {
            // Model 1 is CMYK in percent, not a library color as from version 5.
            assert_eq!(read(version, 1, [0, 100, 50, 10]).0, Some(Color::Cmyk([0.0, 1.0, 0.5, 0.1])), "{version}");
            assert_eq!(read(version, 2, [0, 100, 50, 10]).0, Some(Color::Cmyk([0.0, 1.0, 0.5, 0.1])), "{version}");
        }
        // Model 0: an ink number and a tint. Version 3 stores each as a u16 in its four bytes.
        assert_eq!(read(300, 0, [0, 0, 60, 0]).0, Some(Color::Gray(0.6)));
        assert_eq!(read(300, 0, [1, 0, 60, 0]).0, Some(Color::Gray(0.0)));
        assert_eq!(read(300, 0, [44, 1, 60, 0]), (Some(Color::Spot { name: "Spot 0:300".into(), tint: 0.6, look: None }), vec![Warning::SpotLooks]));
        assert_eq!(read(400, 0, [0, 60, 0, 0]).0, Some(Color::Gray(0.6)));
        assert_eq!(read(400, 0, [1, 60, 0, 0]).0, Some(Color::Gray(0.0)));
        assert_eq!(read(400, 0, [300, 60, 0, 0]), (Some(Color::Spot { name: "Spot 0:300".into(), tint: 0.6, look: None }), vec![Warning::SpotLooks]));
    }

    #[test]
    fn a_fixed_point_matrix_reads_its_factors_and_offsets() {
        let raw: Vec<u8> = [65536i32, -32768, 1000, 131072, 16384, -500].iter().flat_map(|v| v.to_le_bytes()).collect();
        let m = fixed_matrix(&mut Rd::new(&raw), 0.5).unwrap();
        assert_eq!(m, Matrix([1.0, 2.0, -0.5, 0.25, 500.0, -250.0]));
        assert!(fixed_matrix(&mut Rd::new(&raw[..23]), 0.5).is_none());
    }

    #[test]
    fn long_names_that_start_alike_stay_apart() {
        let read = |name: &str| {
            let mut b = (name.encode_utf16().count() as u32).to_le_bytes().to_vec();
            b.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
            let mut w = vec![];
            (counted_utf16(&mut Rd::new(&b), &mut w).unwrap(), w)
        };
        let (a, w) = read(&"N".repeat(300));
        let (b, _) = read(&format!("{}M", "N".repeat(299)));
        assert_ne!(a, b);
        assert!(a.starts_with(&"N".repeat(MAX_NAME)) && a.chars().count() == MAX_NAME + 17, "{a}");
        assert_eq!(w, vec![Warning::TooDeep]);
        assert_eq!(read("Sky Blue"), ("Sky Blue".to_string(), vec![]));
        // Padding doesn't count toward the length or the name.
        let (padded, w) = read(&format!("{}\0\0", "N".repeat(MAX_NAME - 1)));
        assert_eq!((padded, w), ("N".repeat(MAX_NAME - 1), vec![]));
    }

    #[test]
    fn a_tint_mixes_toward_white_and_comes_off_again() {
        let shown = [0.6, 0.8, 1.0];
        let base = untinted(shown, 0.5);
        assert!((base[0] - 0.2).abs() < 1e-6 && (base[1] - 0.6).abs() < 1e-6 && base[2] == 1.0);
    }
}
