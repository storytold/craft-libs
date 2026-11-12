//! Image resolution (pixels per inch) recorded in metadata blocks: EXIF / TIFF IFD0
//! (XResolution 282, YResolution 283, ResolutionUnit 296), XMP (`tiff:XResolution`,
//! `tiff:YResolution`, `tiff:ResolutionUnit`) and Photoshop's image resource
//! ResolutionInfo (`8BIM` resource 0x03ED); reading them, and rewriting the EXIF and XMP
//! copies on export so they agree with the resolution the file is written at.
//!
//! Implemented from TIFF 6.0 (section 8, "Baseline fields": XResolution / YResolution are
//! RATIONAL, ResolutionUnit is SHORT, 1 = no absolute unit, 2 = inch, 3 = centimetre,
//! default 2), EXIF 2.3 (same tags in the 0th IFD, same default) and the Adobe Photoshop File
//! Formats Specification (ResolutionInfo: Fixed 16.16 hRes in pixels per inch, then the
//! display units; the vertical resolution likewise).
//!
//! **Precedence** when a file carries several, as Photoshop resolves them: Photoshop's own
//! ResolutionInfo, then the XMP `tiff:` properties, then EXIF, and only then the format's own
//! density field when it is the generic JFIF one (see the JPEG decoder). JFIF density is last
//! because cameras record the real resolution in EXIF while many tools and libraries write a
//! default JFIF density of 72 (or a bare aspect ratio) without knowing it; Photoshop shows a
//! camera JPEG at its EXIF resolution whatever its JFIF header says. Formats whose own field is
//! written deliberately (PNG `pHYs`, TIFF tags) keep it first; the metadata blocks only fill in
//! when it is missing.
//!
//! Everything read here is untrusted: offsets and counts are bounds-checked, both byte orders
//! and BigTIFF-style EXIF are handled, and anything malformed reads as "no resolution" rather
//! than as a wrong one.

use std::borrow::Cow;

use super::{Order, ifd0_entries, tiff_body, upright_exif, upright_xmp};

const TAG_X_RESOLUTION: u16 = 282;
const TAG_Y_RESOLUTION: u16 = 283;
const TAG_RESOLUTION_UNIT: u16 = 296;
const TYPE_SHORT: u16 = 3;
const TYPE_RATIONAL: u16 = 5;
const CM_PER_INCH: f64 = 2.54;
/// Photoshop's ResolutionInfo image resource.
const RESOLUTION_INFO: u16 = 0x03ED;
/// Resolutions above this (pixels per inch) are treated as damage, not data.
const MAX_PPI: f64 = 1_000_000.0;
/// Two resolutions this close (pixels per inch) are the same.
const SAME_PPI: f64 = 1e-3;

/// A usable resolution pair, or `None`.
fn ppi_pair(x: f64, y: f64) -> Option<(f32, f32)> {
    let ok = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_PPI;
    (ok(x) && ok(y)).then_some((x as f32, y as f32))
}

fn same(a: (f32, f32), b: (f32, f32)) -> bool {
    (f64::from(a.0) - f64::from(b.0)).abs() < SAME_PPI && (f64::from(a.1) - f64::from(b.1)).abs() < SAME_PPI
}

/// `ppi` as a TIFF RATIONAL: a whole number over 1 when it is one, else over 1000.
fn rational(ppi: f32) -> (u32, u32) {
    let p = f64::from(ppi);
    if (p - p.round()).abs() < SAME_PPI { (p.round() as u32, 1) } else { ((p * 1000.0).round() as u32, 1000) }
}

/// One of the three resolution entries of IFD0.
#[derive(Clone, Copy)]
struct Field {
    /// Offset of the 12- (BigTIFF: 20-) byte entry.
    entry: usize,
    tag: u16,
    ty: u16,
    count: u64,
    /// Where the value bytes are, when the type is one we read and they lie inside the block.
    value: Option<usize>,
}

impl Field {
    fn short(&self, order: Order, b: &[u8]) -> Option<u16> {
        (self.ty == TYPE_SHORT && self.count == 1).then_some(())?;
        order.u16(b, self.value?)
    }

    fn rational(&self, order: Order, b: &[u8]) -> Option<f64> {
        (self.ty == TYPE_RATIONAL && self.count == 1).then_some(())?;
        let at = self.value?;
        let (n, d) = (order.u32(b, at)?, order.u32(b, at.checked_add(4)?)?);
        (d != 0).then(|| f64::from(n) / f64::from(d))
    }
}

/// IFD0's resolution entries.
struct Ifd0 {
    order: Order,
    big: bool,
    first: usize,
    count: usize,
    entry_bytes: usize,
    fields: Vec<Field>,
}

impl Ifd0 {
    fn parse(b: &[u8]) -> Option<Ifd0> {
        let (order, first, count, entry_bytes, big) = ifd0_entries(b)?;
        let mut fields = Vec::new();
        for i in 0..count {
            let e = first.checked_add(i.checked_mul(entry_bytes)?)?;
            let tag = order.u16(b, e)?;
            if !matches!(tag, TAG_X_RESOLUTION | TAG_Y_RESOLUTION | TAG_RESOLUTION_UNIT) {
                continue;
            }
            let ty = order.u16(b, e.checked_add(2)?)?;
            let count = if big { order.u64(b, e.checked_add(4)?)? } else { u64::from(order.u32(b, e.checked_add(4)?)?) };
            let value = value_at(order, b, e, big, ty, count);
            fields.push(Field { entry: e, tag, ty, count, value });
        }
        Some(Ifd0 { order, big, first, count, entry_bytes, fields })
    }

    fn first_of(&self, tag: u16) -> Option<&Field> {
        self.fields.iter().find(|f| f.tag == tag)
    }
}

/// Where an entry's SHORT or RATIONAL value bytes are: inline in the entry when they fit,
/// else at the offset it holds. `None` for other types or when they leave the block.
fn value_at(order: Order, b: &[u8], e: usize, big: bool, ty: u16, count: u64) -> Option<usize> {
    let size = match ty {
        TYPE_SHORT => 2u64,
        TYPE_RATIONAL => 8,
        _ => return None,
    }
    .checked_mul(count)?;
    let (field, inline) = if big { (e.checked_add(12)?, 8) } else { (e.checked_add(8)?, 4) };
    let at = if size <= inline {
        field
    } else if big {
        usize::try_from(order.u64(b, field)?).ok()?
    } else {
        usize::try_from(order.u32(b, field)?).ok()?
    };
    (at.checked_add(usize::try_from(size).ok()?)? <= b.len()).then_some(at)
}

/// The resolution (pixels per inch, horizontal and vertical) recorded in IFD0 of an EXIF block
/// (with or without the JPEG `Exif\0\0` prefix) or a TIFF structure.
///
/// Both XResolution and YResolution must be one RATIONAL each with a non-zero denominator.
/// ResolutionUnit 2 (inch, also when the tag is absent) is used as is, 3 (centimetre) is
/// converted; 1 (no absolute unit), any other value or a malformed unit entry gives `None`.
/// The first of duplicated entries wins.
pub fn exif_resolution(exif: &[u8]) -> Option<(f32, f32)> {
    let b = tiff_body(exif);
    let ifd = Ifd0::parse(b)?;
    let unit = match ifd.first_of(TAG_RESOLUTION_UNIT) {
        None => 2,
        Some(f) => f.short(ifd.order, b)?,
    };
    let scale = match unit {
        2 => 1.0,
        3 => CM_PER_INCH,
        _ => return None,
    };
    let x = ifd.first_of(TAG_X_RESOLUTION)?.rational(ifd.order, b)?;
    let y = ifd.first_of(TAG_Y_RESOLUTION)?.rational(ifd.order, b)?;
    ppi_pair(x * scale, y * scale)
}

/// `exif` with its IFD0 resolution set to `ppi` (pixels per inch), borrowed when it has no
/// resolution entries, already records that resolution, or cannot be parsed.
///
/// Existing well-formed XResolution / YResolution (one RATIONAL) entries are rewritten in place,
/// and every well-formed ResolutionUnit (one SHORT) becomes 2 (inch). Malformed entries are
/// removed from IFD0 instead, so no stale value survives; all other bytes are kept.
pub fn exif_with_resolution(exif: &[u8], ppi: (f32, f32)) -> Cow<'_, [u8]> {
    exif_with_resolution_inner(exif, ppi, false)
}

/// Like [`exif_with_resolution`], but adds X/Y resolution tags when a valid IFD0 has none.
/// Existing TIFF data stays in place, so its offsets remain valid. Used by CMYK JPEG, which
/// cannot carry a JFIF density marker.
pub fn exif_with_resolution_adding_missing(exif: &[u8], ppi: (f32, f32)) -> Cow<'_, [u8]> {
    exif_with_resolution_inner(exif, ppi, true)
}

/// Rewrite valid IFD0 resolution tags and optionally append tags that are missing or malformed.
fn exif_with_resolution_inner(exif: &[u8], ppi: (f32, f32), add_missing: bool) -> Cow<'_, [u8]> {
    if ppi_pair(f64::from(ppi.0), f64::from(ppi.1)).is_none() {
        return Cow::Borrowed(exif);
    }
    if exif.is_empty() {
        if !add_missing {
            return Cow::Borrowed(exif);
        }
        // An image with DPI but no source EXIF still needs a valid EXIF block when the output
        // format cannot carry resolution in its own header (CMYK JPEG has no valid JFIF marker).
        return Cow::Owned(exif_with_resolution_inner(b"II*\0\x08\0\0\0\0\0\0\0\0\0\0", ppi, true).into_owned());
    }
    let body = tiff_body(exif);
    let prefix = exif.len() - body.len();
    let Some(ifd) = Ifd0::parse(body) else { return Cow::Borrowed(exif) };
    if exif_resolution(body).is_some_and(|r| same(r, ppi)) {
        return Cow::Borrowed(exif);
    }
    let missing_xy = ifd.first_of(TAG_X_RESOLUTION).and_then(|f| f.rational(ifd.order, body)).is_none()
        || ifd.first_of(TAG_Y_RESOLUTION).and_then(|f| f.rational(ifd.order, body)).is_none();
    if add_missing && missing_xy {
        let Some(body) = append_resolution_ifd(body, &ifd, ppi) else { return Cow::Borrowed(exif) };
        let mut full = exif.get(..prefix).unwrap_or_default().to_vec();
        full.extend_from_slice(&body);
        return Cow::Owned(full);
    }
    if ifd.fields.is_empty() {
        return Cow::Borrowed(exif);
    }
    let order = ifd.order;
    let mut out = body.to_vec();
    let mut removed = Vec::new();
    for f in &ifd.fields {
        let bytes = match f.tag {
            TAG_RESOLUTION_UNIT if f.short(order, body).is_some() => put_u16(order, 2).to_vec(),
            TAG_X_RESOLUTION | TAG_Y_RESOLUTION if f.rational(order, body).is_some() => {
                let (n, d) = rational(if f.tag == TAG_X_RESOLUTION { ppi.0 } else { ppi.1 });
                [put_u32(order, n), put_u32(order, d)].concat()
            }
            _ => {
                removed.push(f.entry);
                continue;
            }
        };
        let slot = f.value.and_then(|at| out.get_mut(at..at.checked_add(bytes.len())?));
        match slot {
            Some(slot) => slot.copy_from_slice(&bytes),
            None => removed.push(f.entry),
        }
    }
    if !removed.is_empty() {
        remove_entries(&mut out, &ifd, &removed);
    }
    let mut full = exif.get(..prefix).unwrap_or_default().to_vec();
    full.extend_from_slice(&out);
    if full == exif { Cow::Borrowed(exif) } else { Cow::Owned(full) }
}

/// Append a replacement IFD0 with fresh resolution tags. Every other entry keeps its original
/// bytes and offsets, since the TIFF block's existing data remains where it was.
fn append_resolution_ifd(body: &[u8], ifd: &Ifd0, ppi: (f32, f32)) -> Option<Vec<u8>> {
    let (count_bytes, next_bytes, alignment) = if ifd.big { (8usize, 8usize, 8usize) } else { (2usize, 4usize, 2usize) };
    let mut entries = Vec::with_capacity(ifd.count.saturating_add(3));
    for index in 0..ifd.count {
        let entry = ifd.first.checked_add(index.checked_mul(ifd.entry_bytes)?)?;
        let tag = ifd.order.u16(body, entry)?;
        if matches!(tag, TAG_X_RESOLUTION | TAG_Y_RESOLUTION | TAG_RESOLUTION_UNIT) {
            continue;
        }
        entries.push((tag, body.get(entry..entry.checked_add(ifd.entry_bytes)?)?.to_vec()));
    }
    let count = entries.len().checked_add(3)?;
    if !ifd.big && count > usize::from(u16::MAX) {
        return None;
    }

    let new_ifd = align_up(body.len(), alignment)?;
    let table_end = new_ifd.checked_add(count_bytes)?.checked_add(count.checked_mul(ifd.entry_bytes)?)?;
    let next_at = ifd.first.checked_add(ifd.count.checked_mul(ifd.entry_bytes)?)?;
    let next_ifd = body.get(next_at..next_at.checked_add(next_bytes)?)?;
    let rational_at = align_up(table_end.checked_add(next_bytes)?, if ifd.big { 8 } else { 4 })?;
    let y_rational_at = rational_at.checked_add(8)?;
    if !ifd.big && (new_ifd > u32::MAX as usize || y_rational_at.checked_add(8)? > u32::MAX as usize) {
        return None;
    }

    entries.push((TAG_X_RESOLUTION, resolution_entry(ifd.order, ifd.big, TAG_X_RESOLUTION, u64::try_from(rational_at).ok()?)?));
    entries.push((TAG_Y_RESOLUTION, resolution_entry(ifd.order, ifd.big, TAG_Y_RESOLUTION, u64::try_from(y_rational_at).ok()?)?));
    entries.push((TAG_RESOLUTION_UNIT, resolution_entry(ifd.order, ifd.big, TAG_RESOLUTION_UNIT, 2)?));
    entries.sort_by_key(|(tag, _)| *tag);

    let mut out = body.to_vec();
    out.resize(new_ifd, 0);
    if ifd.big {
        out.extend_from_slice(&put_u64(ifd.order, count as u64));
    } else {
        out.extend_from_slice(&put_u16(ifd.order, count as u16));
    }
    for (_, entry) in entries {
        out.extend_from_slice(&entry);
    }
    out.extend_from_slice(next_ifd);
    out.resize(rational_at, 0);
    let (xn, xd) = rational(ppi.0);
    let (yn, yd) = rational(ppi.1);
    out.extend_from_slice(&put_u32(ifd.order, xn));
    out.extend_from_slice(&put_u32(ifd.order, xd));
    out.extend_from_slice(&put_u32(ifd.order, yn));
    out.extend_from_slice(&put_u32(ifd.order, yd));

    let offset_at: usize = if ifd.big { 8 } else { 4 };
    if ifd.big {
        out.get_mut(offset_at..offset_at.checked_add(8)?)?.copy_from_slice(&put_u64(ifd.order, new_ifd as u64));
    } else {
        out.get_mut(offset_at..offset_at.checked_add(4)?)?.copy_from_slice(&put_u32(ifd.order, u32::try_from(new_ifd).ok()?));
    }
    Some(out)
}

/// One IFD entry. Rational fields store an offset; the SHORT unit is inline.
fn resolution_entry(order: Order, big: bool, tag: u16, value: u64) -> Option<Vec<u8>> {
    let entry_bytes = if big { 20 } else { 12 };
    let count_at = 4;
    let value_at = if big { 12 } else { 8 };
    let mut entry = vec![0; entry_bytes];
    entry.get_mut(..2)?.copy_from_slice(&put_u16(order, tag));
    entry.get_mut(2..4)?.copy_from_slice(&put_u16(order, if tag == TAG_RESOLUTION_UNIT { TYPE_SHORT } else { TYPE_RATIONAL }));
    if big {
        entry.get_mut(count_at..count_at + 8)?.copy_from_slice(&put_u64(order, 1));
        if tag == TAG_RESOLUTION_UNIT {
            entry.get_mut(value_at..value_at + 2)?.copy_from_slice(&put_u16(order, value as u16));
        } else {
            entry.get_mut(value_at..value_at + 8)?.copy_from_slice(&put_u64(order, value));
        }
    } else {
        entry.get_mut(count_at..count_at + 4)?.copy_from_slice(&put_u32(order, 1));
        if tag == TAG_RESOLUTION_UNIT {
            entry.get_mut(value_at..value_at + 2)?.copy_from_slice(&put_u16(order, value as u16));
        } else {
            entry.get_mut(value_at..value_at + 4)?.copy_from_slice(&put_u32(order, u32::try_from(value).ok()?));
        }
    }
    Some(entry)
}

/// Round `value` up to the next `alignment` boundary, returning `None` on overflow.
fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value.checked_add((alignment - value % alignment) % alignment)
}

fn put_u16(order: Order, v: u16) -> [u8; 2] {
    match order {
        Order::Little => v.to_le_bytes(),
        Order::Big => v.to_be_bytes(),
    }
}

fn put_u32(order: Order, v: u32) -> [u8; 4] {
    match order {
        Order::Little => v.to_le_bytes(),
        Order::Big => v.to_be_bytes(),
    }
}

fn put_u64(order: Order, v: u64) -> [u8; 8] {
    match order {
        Order::Little => v.to_le_bytes(),
        Order::Big => v.to_be_bytes(),
    }
}

/// Removes the IFD0 entries at offsets `removed` (see [`exif_with_resolution`]).
///
/// `removed` is strictly increasing: `Ifd0::parse` visits entries in physical order and the
/// caller records each malformed field at most once. Walk those offsets once alongside the
/// table, keeping compaction linear even when many duplicate resolution entries are malformed.
/// The private offset type lets tests observe real equality checks in this same loop; production
/// instantiates it with `usize`, without a counter or a change to the exported codec API.
fn remove_entries<Offset: Copy + PartialEq + From<usize>>(b: &mut [u8], ifd: &Ifd0, removed: &[Offset]) {
    let eb = ifd.entry_bytes;
    let next_bytes = if ifd.big { 8 } else { 4 };
    let count_bytes = if ifd.big { 8 } else { 2 };
    let Some(table_end) = ifd.count.checked_mul(eb).and_then(|n| ifd.first.checked_add(n)) else { return };
    let Some(old) = b.get(ifd.first..table_end.saturating_add(next_bytes)).map(<[u8]>::to_vec) else { return };
    let mut table = Vec::with_capacity(old.len());
    let mut kept = 0usize;
    let mut removed = removed.iter().copied().peekable();
    for (i, entry) in old.chunks_exact(eb).take(ifd.count).enumerate() {
        if removed.peek() == Some(&Offset::from(ifd.first + i * eb)) {
            removed.next();
        } else {
            table.extend_from_slice(entry);
            kept += 1;
        }
    }
    table.extend_from_slice(old.get(ifd.count * eb..).unwrap_or_default());
    table.resize(old.len(), 0);
    let count: Vec<u8> = if ifd.big { put_u64(ifd.order, kept as u64).to_vec() } else { put_u16(ifd.order, kept as u16).to_vec() };
    let Some(count_at) = ifd.first.checked_sub(count_bytes) else { return };
    if let Some(slot) = b.get_mut(count_at..ifd.first) {
        slot.copy_from_slice(&count);
    }
    if let Some(slot) = b.get_mut(ifd.first..ifd.first + table.len()) {
        slot.copy_from_slice(&table);
    }
}

/// The resolution in a Photoshop image resource block (a sequence of `8BIM` resources, as in a
/// JPEG's APP13 "Photoshop 3.0" segment or TIFF tag 34377): its ResolutionInfo (0x03ED), whose
/// Fixed 16.16 hRes / vRes are pixels per inch whatever display unit follows them.
pub fn photoshop_resolution(resources: &[u8]) -> Option<(f32, f32)> {
    let be16 = |at: usize| resources.get(at..at.checked_add(2)?).map(|s| u16::from_be_bytes([s[0], s[1]]));
    let be32 = |at: usize| resources.get(at..at.checked_add(4)?).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
    let mut i = 0usize;
    while i < resources.len() {
        let signature = resources.get(i..i.checked_add(4)?)?;
        let id = be16(i.checked_add(4)?)?;
        // Pascal name: length byte plus text, padded to an even size.
        let name_len = usize::from(*resources.get(i.checked_add(6)?)?);
        let name = (name_len + 1).next_multiple_of(2);
        let size_at = i.checked_add(6)?.checked_add(name)?;
        let size = usize::try_from(be32(size_at)?).ok()?;
        let data = size_at.checked_add(4)?;
        if signature == b"8BIM" && id == RESOLUTION_INFO {
            let info = resources.get(data..data.checked_add(size)?)?;
            if info.len() < 16 {
                return None;
            }
            let fixed = |at: usize| be32(data + at).map(|v| f64::from(v) / 65536.0);
            return ppi_pair(fixed(0)?, fixed(8)?);
        }
        i = data.checked_add(size)?.checked_add(size % 2)?;
    }
    None
}

/// The text ranges of every value of XMP property `key` (`key="v"`, `key='v'` or
/// `<key>v</key>`), in order.
fn xmp_values(xmp: &str, key: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = xmp.get(from..).and_then(|s| s.find(key)).map(|k| k + from) {
        from = k + key.len();
        // An attribute or an opening tag of exactly this name (not `</key>`, not `xkey`).
        if !xmp.get(..k).and_then(|h| h.chars().next_back()).is_some_and(|c| c == '<' || c.is_ascii_whitespace()) {
            continue;
        }
        let rest = xmp.get(from..).unwrap_or_default();
        let after_key = rest.trim_start();
        let (value, close) = if let Some(r) = after_key.strip_prefix('=') {
            let r = r.trim_start();
            match r.chars().next() {
                Some(q @ ('"' | '\'')) => (r.get(1..).unwrap_or_default(), q),
                _ => continue,
            }
        } else if let Some(r) = rest.strip_prefix('>') {
            (r, '<')
        } else {
            continue;
        };
        let start = xmp.len() - value.len();
        // Advance past this value before searching for the next key. Without this, `from`
        // only moved past the key text, so property-like text repeated inside an unrelated
        // attribute re-ran this full-suffix `find(close)` for every occurrence, which is
        // quadratic in the packet size. A missing close means the value is unterminated, so
        // no later value can parse either; stop scanning in that case.
        let Some(len) = value.find(close) else { break };
        out.push((start, start + len));
        from = start + len + close.len_utf8();
    }
    out
}

/// An XMP rational (`300/1`) or plain number.
fn xmp_number(s: &str) -> Option<f64> {
    let s = s.trim();
    match s.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
            (d != 0.0).then(|| n / d)
        }
        None => s.parse().ok(),
    }
}

const XMP_X: &str = "tiff:XResolution";
const XMP_Y: &str = "tiff:YResolution";
const XMP_UNIT: &str = "tiff:ResolutionUnit";

/// The resolution (pixels per inch) in an XMP packet's `tiff:XResolution` / `tiff:YResolution`
/// (with `tiff:ResolutionUnit`, 2 when absent, 3 converted from centimetres), or `None`.
pub fn xmp_resolution(xmp: &str) -> Option<(f32, f32)> {
    let first = |key: &str| xmp_values(xmp, key).first().and_then(|&(a, b)| xmp.get(a..b));
    let unit = match first(XMP_UNIT) {
        None => 2.0,
        Some(u) => xmp_number(u)?,
    };
    let scale = if unit == 2.0 {
        1.0
    } else if unit == 3.0 {
        CM_PER_INCH
    } else {
        return None;
    };
    ppi_pair(xmp_number(first(XMP_X)?)? * scale, xmp_number(first(XMP_Y)?)? * scale)
}

/// `xmp` with its `tiff:` resolution properties set to `ppi` (and the unit to 2, inch),
/// borrowed when it has none or already says so.
pub fn xmp_with_resolution(xmp: &str, ppi: (f32, f32)) -> Cow<'_, str> {
    if ppi_pair(f64::from(ppi.0), f64::from(ppi.1)).is_none() || xmp_resolution(xmp).is_some_and(|r| same(r, ppi)) {
        return Cow::Borrowed(xmp);
    }
    let text = |v: f32| {
        let (n, d) = rational(v);
        format!("{n}/{d}")
    };
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (key, value) in [(XMP_X, text(ppi.0)), (XMP_Y, text(ppi.1)), (XMP_UNIT, "2".to_owned())] {
        edits.extend(xmp_values(xmp, key).into_iter().map(|(a, b)| (a, b, value.clone())));
    }
    if edits.is_empty() {
        return Cow::Borrowed(xmp);
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(xmp.len() + 16);
    let mut at = 0;
    for (a, b, v) in edits {
        // Overlapping ranges can only come from malformed markup; keep the first edit.
        if a < at {
            continue;
        }
        out.push_str(xmp.get(at..a).unwrap_or_default());
        out.push_str(&v);
        at = b;
    }
    out.push_str(xmp.get(at..).unwrap_or_default());
    if out == xmp { Cow::Borrowed(xmp) } else { Cow::Owned(out) }
}

/// `exif` without its embedded thumbnail, or `None` when it has none (or doesn't parse).
///
/// EXIF keeps the thumbnail in IFD1, the IFD IFD0's next-IFD pointer names. Finder, Android's
/// gallery and upload pickers show that thumbnail instead of decoding the image, so a copied one
/// keeps showing the photo as it was before it was edited. The pointer is cleared;
/// nothing else moves, so every other offset in the block stays valid.
fn exif_without_thumbnail(exif: &[u8]) -> Option<Vec<u8>> {
    let body = tiff_body(exif);
    let prefix = exif.len() - body.len();
    let (order, first, count, entry_bytes, big_tiff) = ifd0_entries(body)?;
    let next = prefix.checked_add(first)?.checked_add(count.checked_mul(entry_bytes)?)?;
    let width = if big_tiff { 8 } else { 4 };
    // In bounds: `ifd0_entries` checked that the whole pointer is present.
    if exif.get(next..next.checked_add(width)?)?.iter().all(|b| *b == 0) {
        return None;
    }
    // Classic TIFF: also blank the thumbnail JPEG itself (IFD1's JPEGInterchangeFormat/Length),
    // which stays recoverable in the block once nothing points at it.
    // Only a range clear of the TIFF header, IFD0's table and values, and its Exif/GPS sub-IFDs: one
    // that covers them is not a thumbnail.
    let thumb = (!big_tiff)
        .then(|| thumbnail_range(order, body, next - prefix))
        .flatten()
        .filter(|&(off, len)| off >= 8 && !overlaps_ifd0(order, body, first - 2, count, off, len));
    let mut out = exif.to_vec();
    out.get_mut(next..next + width)?.fill(0);
    if let Some((off, len)) = thumb
        && let Some(jpeg) = prefix.checked_add(off).and_then(|a| out.get_mut(a..a.checked_add(len)?))
    {
        jpeg.fill(0);
    }
    Some(out)
}

/// The Exif IFD pointer tag in IFD0.
const TAG_EXIF_IFD: u16 = 34665;
/// PixelXDimension and PixelYDimension in the Exif IFD (EXIF 2.3, SHORT or LONG).
const TAG_PIXEL_X: u16 = 40962;
const TAG_PIXEL_Y: u16 = 40963;
const TYPE_LONG: u16 = 4;

/// Where the Exif IFD of a TIFF stream starts, from IFD0's pointer (LONG or IFD; in BigTIFF also
/// LONG8 or IFD8). The first pointer wins; one that doesn't hold exactly one value is not followed.
fn exif_ifd(order: Order, b: &[u8], first: usize, count: usize, entry_bytes: usize, big: bool) -> Option<usize> {
    let e = (0..count).filter_map(|i| first.checked_add(i.checked_mul(entry_bytes)?)).find(|&e| order.u16(b, e) == Some(TAG_EXIF_IFD))?;
    let count_at = e.checked_add(4)?;
    let values = if big { order.u64(b, count_at)? } else { u64::from(order.u32(b, count_at)?) };
    if values != 1 {
        return None;
    }
    let ty = order.u16(b, e.checked_add(2)?)?;
    let at = e.checked_add(if big { 12 } else { 8 })?;
    let offset = match ty {
        4 | 13 => u64::from(order.u32(b, at)?),
        16 | 18 if big => order.u64(b, at)?,
        _ => return None,
    };
    usize::try_from(offset).ok()
}

/// `exif` with the Exif IFD's PixelXDimension and PixelYDimension set to `size`, the pixels the
/// file is written with; borrowed when there is nothing to change (no such tags, already right,
/// or unparseable). A camera records its sensor's size there, which after flattening a cropped,
/// resized or rotated document names a size the file doesn't have. Only a tag with one
/// SHORT or LONG value is rewritten, in place and as a LONG (EXIF allows either), so no offset
/// moves.
pub fn exif_with_pixel_size(exif: &[u8], (width, height): (u32, u32)) -> Cow<'_, [u8]> {
    let body = tiff_body(exif);
    let prefix = exif.len() - body.len();
    let Some((order, first, count, entry_bytes, big)) = ifd0_entries(body) else {
        return Cow::Borrowed(exif);
    };
    let Some(dir) = exif_ifd(order, body, first, count, entry_bytes, big) else {
        return Cow::Borrowed(exif);
    };
    let (count_bytes, value_offset, field) = if big { (8, 12, 8) } else { (2, 8, 4) };
    let entries = if big { order.u64(body, dir).and_then(|n| usize::try_from(n).ok()) } else { order.u16(body, dir).map(usize::from) };
    let Some(start) = dir.checked_add(count_bytes) else { return Cow::Borrowed(exif) };
    // Every entry must be present: a truncated directory is left as it is.
    let Some(entries) = entries.filter(|n| n.checked_mul(entry_bytes).and_then(|len| len.checked_add(start)).is_some_and(|end| end <= body.len()))
    else {
        return Cow::Borrowed(exif);
    };
    let put = |v: u32| match order {
        Order::Little => v.to_le_bytes(),
        Order::Big => v.to_be_bytes(),
    };
    let long = match order {
        Order::Little => TYPE_LONG.to_le_bytes(),
        Order::Big => TYPE_LONG.to_be_bytes(),
    };
    // Where entry `i`'s type and value sit in `exif`, and the value to write, when it is a
    // PixelX/YDimension entry with one SHORT or LONG value that differs from it.
    let stale = |i: usize| -> Option<(usize, usize, u32)> {
        let e = start.checked_add(i.checked_mul(entry_bytes)?)?;
        let want = match order.u16(body, e)? {
            TAG_PIXEL_X => width,
            TAG_PIXEL_Y => height,
            _ => return None,
        };
        let count_at = e.checked_add(4)?;
        let n = if big { order.u64(body, count_at)? } else { u64::from(order.u32(body, count_at)?) };
        let type_at = e.checked_add(2)?;
        let at = e.checked_add(value_offset)?;
        let now = match order.u16(body, type_at)? {
            TYPE_SHORT => u32::from(order.u16(body, at)?),
            TYPE_LONG => order.u32(body, at)?,
            _ => return None,
        };
        (n == 1 && now != want).then_some((prefix.checked_add(type_at)?, prefix.checked_add(at)?, want))
    };
    let mut out: Option<Vec<u8>> = None;
    for (type_at, at, want) in (0..entries).filter_map(stale) {
        let o = out.get_or_insert_with(|| exif.to_vec());
        let mut value = [0u8; 8];
        value[..4].copy_from_slice(&put(want));
        if let Some(slot) = type_at.checked_add(2).and_then(|end| o.get_mut(type_at..end)) {
            slot.copy_from_slice(&long);
        }
        if let Some(slot) = at.checked_add(field).and_then(|end| o.get_mut(at..end)) {
            slot.copy_from_slice(&value[..field]);
        }
    }
    out.map_or(Cow::Borrowed(exif), Cow::Owned)
}

/// Most IFD entries `overlaps_ifd0` examines in total: a hostile block with thousands of Exif/GPS
/// pointers to large sub-IFDs would otherwise cost billions of checks.
const MAX_OVERLAP_ENTRIES: usize = 1 << 20;

/// Whether `off..off + len` of `body` covers part of classic IFD0 (`ifd0` is its entry count's
/// offset) or of its Exif / GPS sub-IFDs. Past [`MAX_OVERLAP_ENTRIES`] checks it answers `true`
/// (the range is not trusted as a thumbnail, so its bytes stay).
fn overlaps_ifd0(order: Order, body: &[u8], ifd0: usize, count: usize, off: usize, len: usize) -> bool {
    let mut budget = MAX_OVERLAP_ENTRIES;
    overlaps_ifd(order, body, ifd0, count, off, len, &mut budget)
        || (0..count).any(|i| {
            let e = ifd0 + 2 + i * 12;
            // A LONG pointer (tag 0x8769 ExifIFD, 0x8825 GPSIFD) to a sub-IFD, whose own count says its size.
            matches!(order.u16(body, e), Some(0x8769 | 0x8825))
                && order
                    .u32(body, e + 8)
                    .and_then(|v| usize::try_from(v).ok())
                    .is_some_and(|sub| overlaps_sub_ifd(order, body, sub, off, len, &mut budget))
        })
}

/// [`overlaps_ifd`] for the Exif / GPS sub-IFD at `sub`, and for the Interoperability IFD (tag
/// 0xA005) it points to, which `jpeg.fill(0)` would otherwise erase from under its pointer.
fn overlaps_sub_ifd(order: Order, body: &[u8], sub: usize, off: usize, len: usize, budget: &mut usize) -> bool {
    let Some(n) = order.u16(body, sub).map(usize::from) else { return false };
    if overlaps_ifd(order, body, sub, n, off, len, budget) {
        return true;
    }
    (0..n).any(|i| {
        let Some(left) = budget.checked_sub(1) else { return true };
        *budget = left;
        let e = sub + 2 + i * 12;
        matches!(order.u16(body, e), Some(0xA005))
            && order
                .u32(body, e + 8)
                .and_then(|v| usize::try_from(v).ok())
                .is_some_and(|interop| order.u16(body, interop).is_some_and(|m| overlaps_ifd(order, body, interop, usize::from(m), off, len, budget)))
    })
}

/// Whether `off..off + len` of `body` covers the table of the classic IFD at `ifd` (its entry
/// count's offset) with the next-IFD pointer, or an out-of-line value of one of its entries. Each
/// entry examined is charged to `budget`; when it runs out the answer is `true`.
fn overlaps_ifd(order: Order, body: &[u8], ifd: usize, count: usize, off: usize, len: usize, budget: &mut usize) -> bool {
    let end = off.saturating_add(len);
    let hits = |a: usize, size: usize| a < end && off < a.saturating_add(size);
    if hits(ifd, 2 + count * 12 + 4) {
        return true;
    }
    (0..count).any(|i| {
        let Some(left) = budget.checked_sub(1) else { return true };
        *budget = left;
        let e = ifd + 2 + i * 12;
        let unit = match order.u16(body, e + 2) {
            Some(1 | 2 | 6 | 7) => 1,
            Some(3 | 8) => 2,
            Some(4 | 9 | 11) => 4,
            Some(5 | 10 | 12) => 8,
            _ => return false,
        };
        let size = usize::try_from(order.u32(body, e + 4).unwrap_or(0)).unwrap_or(usize::MAX).saturating_mul(unit);
        size > 4 && order.u32(body, e + 8).and_then(|v| usize::try_from(v).ok()).is_some_and(|v| hits(v, size))
    })
}

/// IFD1's thumbnail JPEG as (offset, length) in `body`, from the next-IFD pointer at `ptr`.
fn thumbnail_range(order: Order, body: &[u8], ptr: usize) -> Option<(usize, usize)> {
    let ifd1 = usize::try_from(order.u32(body, ptr)?).ok()?;
    let (mut off, mut len) = (None, None);
    for i in 0..usize::from(order.u16(body, ifd1)?) {
        let e = ifd1.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        let tag = order.u16(body, e)?;
        if !matches!(tag, 0x0201 | 0x0202) {
            continue;
        }
        // One SHORT or LONG value; anything else holds an unrelated number.
        let ty = order.u16(body, e + 2)?;
        if order.u32(body, e + 4)? != 1 || !matches!(ty, 3 | 4) {
            return None;
        }
        let value = if ty == 3 { usize::from(order.u16(body, e + 8)?) } else { usize::try_from(order.u32(body, e + 8)?).ok()? };
        if tag == 0x0201 {
            off = Some(value);
        } else {
            len = Some(value);
        }
    }
    Some((off?, len?))
}

/// The EXIF to write next to upright pixels at resolution `ppi`: Orientation 1 (see
/// [`upright_exif`]), no thumbnail of the source image (see [`exif_without_thumbnail`]) and,
/// when `ppi` is known, the resolution (see [`exif_with_resolution`]).
pub fn export_exif(exif: &[u8], ppi: Option<(f32, f32)>) -> Cow<'_, [u8]> {
    export_exif_inner(exif, ppi, false)
}

/// Export EXIF for a format without its own resolution field, adding missing X/Y resolution
/// entries to valid metadata when necessary.
pub fn export_exif_adding_missing_resolution(exif: &[u8], ppi: Option<(f32, f32)>) -> Cow<'_, [u8]> {
    export_exif_inner(exif, ppi, true)
}

/// Normalize EXIF orientation and thumbnails, optionally adding missing output resolution tags.
fn export_exif_inner(exif: &[u8], ppi: Option<(f32, f32)>, add_missing: bool) -> Cow<'_, [u8]> {
    let up = upright_exif(exif);
    let up = match exif_without_thumbnail(&up) {
        Some(v) => Cow::Owned(v),
        None => up,
    };
    let Some(ppi) = ppi else { return up };
    let result = if add_missing { exif_with_resolution_adding_missing(&up, ppi) } else { exif_with_resolution(&up, ppi) };
    if let Cow::Owned(v) = result {
        return Cow::Owned(v);
    }
    up
}

/// The XMP to write next to upright pixels at resolution `ppi` (see [`upright_xmp`] and
/// [`xmp_with_resolution`]).
pub fn export_xmp(xmp: &str, ppi: Option<(f32, f32)>) -> Cow<'_, str> {
    let up = upright_xmp(xmp);
    let Some(ppi) = ppi else { return up };
    if let Cow::Owned(v) = xmp_with_resolution(&up, ppi) {
        return Cow::Owned(v);
    }
    up
}

/// The resolution a decoded file's metadata blocks record, in precedence order: XMP, then
/// EXIF (Photoshop's ResolutionInfo is format-specific and read by the decoder).
pub fn resolution(exif: Option<&[u8]>, xmp: Option<&str>) -> Option<(f32, f32)> {
    xmp.and_then(xmp_resolution).or_else(|| exif.and_then(exif_resolution))
}

#[cfg(test)]
mod complexity_tests;
