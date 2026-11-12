//! Exif / TIFF tags → [`Metadata`] (CIPA DC-008 Exif 2.32, TIFF 6.0, DNG 1.7 tag semantics).

use crate::{DateTime, Flash, Gps, Metadata, Orientation};
use craft_tiff::{Ifd, ParseOptions, Tiff, TiffError, tags as t};

/// Strip an `Exif\0\0` (or `Exif\0\xff`) prefix as found in JPEG APP1 and some WebP `EXIF` chunks.
pub fn strip_exif_header(b: &[u8]) -> &[u8] {
    if b.len() >= 6 && &b[..4] == b"Exif" && b[4] == 0 { &b[6..] } else { b }
}

/// Read metadata from a TIFF-structured Exif block (a whole TIFF/DNG/raw file, or the payload of a JPEG APP1
/// Exif segment with or without the `Exif\0\0` prefix). Malformed input yields whatever could be read.
pub fn read_exif(bytes: &[u8]) -> Metadata {
    try_read_exif(bytes).unwrap_or_default()
}

/// Like [`read_exif`] but reports a structurally invalid TIFF header / IFD0.
pub fn try_read_exif(bytes: &[u8]) -> Result<Metadata, TiffError> {
    try_read_exif_with(bytes, &ParseOptions::default())
}

/// [`try_read_exif`] with the caller's parser options (its value allocation budget in particular).
/// A decoder working from a sparse buffer — only selected ranges of the logical file are actually
/// loaded, the rest is zeros — must not size its allocations by the logical file length, which
/// unrelated bytes of the file can inflate at no cost to the attacker.
pub fn try_read_exif_with(bytes: &[u8], opts: &ParseOptions) -> Result<Metadata, TiffError> {
    let data = strip_exif_header(bytes);
    let tiff = Tiff::parse_with(data, opts)?;
    let mut m = from_tiff(&tiff);
    // Panasonic (and some Canons) write no LensModel, only the name in their maker note
    if m.lens_model.is_none() {
        m.lens_model = panasonic_maker_note_lens(data, &tiff).or_else(|| canon_maker_note_lens(data, &tiff));
    }
    // the maker note's values are offsets into the stream, which `from_tiff` doesn't have
    m.focus = crate::focus::read(data, &tiff, m.make.as_deref(), m.model.as_deref(), m.orientation);
    Ok(m)
}

fn text(ifd: Option<&Ifd>, tag: u16) -> Option<String> {
    let s = ifd?.string(tag)?;
    let s = s.trim().to_string();
    (!s.is_empty() && !s.bytes().all(|b| b == b' ' || b == 0)).then_some(s)
}

fn pos(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite() && *x > 0.0)
}

/// The orientation the Exif's own first IFD states. [`from_tiff`] also reports one that only the XMP packet in
/// the Exif's tag 700 states; a viewer displays a photo by the Exif's alone, so that one can't tell how a photo
/// is stored.
pub(crate) fn ifd0_orientation(bytes: &[u8]) -> Option<Orientation> {
    let tiff = Tiff::parse_with(strip_exif_header(bytes), &ParseOptions::default()).ok()?;
    tiff.ifds.first().and_then(|i| i.u16(t::ORIENTATION)).filter(|v| (1..=8).contains(v)).map(Orientation::from_exif)
}

/// Build [`Metadata`] from a parsed TIFF stream (IFD0 + Exif + GPS; DNG tags as fallbacks).
pub fn from_tiff(tiff: &Tiff) -> Metadata {
    let ifd0 = tiff.ifds.first();
    let exif = tiff.exif();
    let gps = tiff.gps();
    let either = |tag: u16| exif.and_then(|e| e.value(tag)).or_else(|| ifd0.and_then(|i| i.value(tag)));
    let mut m = Metadata {
        make: text(ifd0, t::MAKE),
        model: text(ifd0, t::MODEL).or_else(|| text(ifd0, t::UNIQUE_CAMERA_MODEL)),
        software: text(ifd0, t::SOFTWARE),
        artist: text(ifd0, t::ARTIST),
        copyright: text(ifd0, t::COPYRIGHT),
        caption: text(ifd0, t::IMAGE_DESCRIPTION),
        serial_number: text(exif, t::BODY_SERIAL_NUMBER).or_else(|| text(ifd0, t::CAMERA_SERIAL_NUMBER)),
        lens_make: text(exif, t::LENS_MAKE),
        lens_model: text(exif, t::LENS_MODEL),
        lens_serial_number: text(exif, t::LENS_SERIAL_NUMBER),
        ..Default::default()
    };
    // Exif requires LensSpecification/LensInfo as 4 rationals.
    let spec = exif.and_then(|e| e.f64s(t::LENS_SPECIFICATION)).or_else(|| ifd0.and_then(|i| i.f64s(t::LENS_INFO)));
    if let Some(s) = spec.filter(|s| s.len() == 4 && s[0] > 0.0) {
        m.lens_spec = Some([s[0], s[1], s[2], s[3]]);
    }
    // Nikon DSLRs write neither LensModel nor LensSpecification, only the same four values in their maker note
    if m.lens_spec.is_none() {
        m.lens_spec = nikon_lens_spec(exif, m.make.as_deref(), tiff.order);
    }
    m.orientation = ifd0.and_then(|i| i.u16(t::ORIENTATION)).filter(|v| (1..=8).contains(v)).map(Orientation::from_exif);
    m.rating = ifd0.and_then(|i| i.i64(t::RATING)).map(|r| r.clamp(-1, 5) as i8);

    m.exposure_time = pos(either(t::EXPOSURE_TIME).and_then(|v| v.get_f64(0)))
        .or_else(|| exif.and_then(|e| e.f64(t::SHUTTER_SPEED_VALUE)).filter(|v| v.abs() < 64.0).map(|tv| 2f64.powf(-tv)));
    m.f_number = pos(either(t::F_NUMBER).and_then(|v| v.get_f64(0)))
        .or_else(|| exif.and_then(|e| e.f64(t::APERTURE_VALUE)).filter(|v| v.abs() < 64.0).map(|av| 2f64.powf(av / 2.0)));
    let iso = either(t::ISO_SPEED).and_then(|v| v.get_u64(0)).filter(|&v| v > 0);
    let iso_ext = exif.and_then(|e| e.u64(t::RECOMMENDED_EXPOSURE_INDEX).or_else(|| e.u64(t::ISO_SPEED_RATINGS_EXT))).filter(|&v| v > 0);
    m.iso = match (iso, iso_ext) {
        (Some(65535), Some(x)) | (None, Some(x)) => Some(x),
        (Some(v), _) => Some(v),
        _ => None,
    }
    .map(|v| v.min(u32::MAX as u64) as u32);
    m.focal_length = pos(either(t::FOCAL_LENGTH).and_then(|v| v.get_f64(0)));
    m.focal_length_35mm = pos(exif.and_then(|e| e.f64(t::FOCAL_LENGTH_35MM)));
    m.exposure_bias = exif.and_then(|e| e.f64(t::EXPOSURE_BIAS)).filter(|v| v.is_finite() && v.abs() < 100.0);
    m.exposure_program = exif.and_then(|e| e.u16(t::EXPOSURE_PROGRAM));
    m.metering_mode = exif.and_then(|e| e.u16(t::METERING_MODE));
    m.white_balance = exif.and_then(|e| e.u16(t::WHITE_BALANCE));
    m.flash = exif.and_then(|e| e.u16(t::FLASH)).map(|raw| Flash { fired: raw & 1 != 0, raw });

    // capture time: DateTimeOriginal, then DateTimeDigitized, then IFD0 DateTime
    let candidates = [
        (exif, t::DATE_TIME_ORIGINAL, t::OFFSET_TIME_ORIGINAL, t::SUBSEC_TIME_ORIGINAL),
        (exif, t::DATE_TIME_DIGITIZED, t::OFFSET_TIME_DIGITIZED, t::SUBSEC_TIME_DIGITIZED),
        (ifd0, t::DATE_TIME, t::OFFSET_TIME, t::SUBSEC_TIME),
    ];
    for (ifd, dt_tag, off_tag, sub_tag) in candidates {
        if let Some(mut dt) = text(ifd, dt_tag).and_then(|s| DateTime::parse_exif(&s)) {
            if let Some(s) = text(exif, sub_tag) {
                dt = dt.with_subsec(&s);
            }
            dt.offset_minutes = text(exif, off_tag).and_then(|s| DateTime::parse_offset(&s));
            m.capture_time = Some(dt);
            break;
        }
    }

    m.width = exif.and_then(|e| e.u32(t::PIXEL_X_DIMENSION)).filter(|&v| v > 0).or_else(|| ifd0.and_then(|i| i.u32(t::IMAGE_WIDTH)));
    m.height = exif.and_then(|e| e.u32(t::PIXEL_Y_DIMENSION)).filter(|&v| v > 0).or_else(|| ifd0.and_then(|i| i.u32(t::IMAGE_LENGTH)));

    m.gps = gps.and_then(read_gps);
    if let Some(iptc) = ifd0.and_then(|i| i.value(t::IPTC_NAA)) {
        // IPTC-NAA is often typed LONG; re-serialise to bytes in file order.
        let bytes = match iptc {
            craft_tiff::Value::Byte(b) | craft_tiff::Value::Undefined(b) => b.clone(),
            other => craft_tiff::writer::encode(tiff.order, other),
        };
        m.fill_missing(&crate::parse_iptc(&bytes));
    }
    if let Some(x) = ifd0.and_then(|i| i.bytes(t::XMP)).map(String::from_utf8_lossy)
        && let Ok(x) = crate::parse_xmp(&x)
    {
        // (the size stays the one Exif states: `extract` turns regions by it, and a size only the XMP states may be the upright one)
        let size = (m.width, m.height);
        m.overlay_user_fields(&x.metadata);
        m.fill_missing(&x.metadata);
        (m.width, m.height) = size;
    }
    m
}

/// Degrees/minutes/seconds triplet as decimal degrees. A component that is present but not a number (a rational
/// with a zero denominator, as in the placeholder GPS block phones write without a fix) makes the whole value unusable.
fn dms(ifd: &Ifd, tag: u16) -> Option<f64> {
    let v = ifd.value(tag)?;
    let part = |i: usize| if i < v.count() { v.get_f64(i) } else { Some(0.0) };
    let d = v.get_f64(0)?;
    let deg = d + part(1)? / 60.0 + part(2)? / 3600.0;
    deg.is_finite().then_some(deg)
}

/// `Some(true)` for the negative hemisphere (`neg`), `Some(false)` for `pos`, `None` when the ref is missing or invalid.
fn hemisphere(g: &Ifd, tag: u16, pos: char, neg: char) -> Option<bool> {
    match g.string(tag)?.trim_start().chars().next()?.to_ascii_uppercase() {
        c if c == neg => Some(true),
        c if c == pos => Some(false),
        _ => None,
    }
}

fn read_gps(g: &Ifd) -> Option<Gps> {
    let mut lat = dms(g, t::GPS_LATITUDE)?;
    let mut lon = dms(g, t::GPS_LONGITUDE)?;
    if hemisphere(g, t::GPS_LATITUDE_REF, 'N', 'S')? {
        lat = -lat;
    }
    if hemisphere(g, t::GPS_LONGITUDE_REF, 'E', 'W')? {
        lon = -lon;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let altitude = g.f64(t::GPS_ALTITUDE).filter(|a| a.is_finite()).map(|a| if g.u32(t::GPS_ALTITUDE_REF) == Some(1) { -a } else { a });
    Some(Gps { latitude: lat, longitude: lon, altitude })
}

/// Panasonic maker note tag `0x0051` (LensType in the ExifTool Panasonic tag names): the lens name, ASCII.
const PANASONIC_LENS_TYPE: u16 = 0x0051;
/// RW2/RWL IFD0 tag `0x002E` (JpgFromRaw): a JPEG whose APP1 holds the full Exif, maker note included.
const PANASONIC_JPG_FROM_RAW: u16 = 0x002E;

/// The lens name in the Panasonic maker note (tag `0x0051`) of this Exif or RW2 stream (`data` starts at its
/// TIFF header): the note of its own Exif IFD, else, for a raw, the note in its JpgFromRaw's Exif.
pub(crate) fn panasonic_maker_note_lens(data: &[u8], tiff: &Tiff) -> Option<String> {
    panasonic_lens(data, tiff).or_else(|| panasonic_jpg_from_raw_lens(tiff))
}

/// The lens name in a `Panasonic\0` maker note of this Exif stream (`data` starts at its TIFF header).
fn panasonic_lens(data: &[u8], tiff: &Tiff) -> Option<String> {
    let exif = tiff.exif()?;
    if !exif.bytes(t::MAKER_NOTE)?.starts_with(b"Panasonic\0") {
        return None;
    }
    let e = exif.get(t::MAKER_NOTE)?;
    let make = tiff.ifds.first().and_then(|i| i.string(t::MAKE)).unwrap_or_default();
    let mn = craft_tiff::makernote::parse_makernote(data, e.offset, e.count() as u64, tiff.order, &make)?;
    text(Some(&mn.ifd), PANASONIC_LENS_TYPE)
}

/// The lens name from the Exif of a RW2's (or Leica RWL's) embedded JpgFromRaw.
fn panasonic_jpg_from_raw_lens(tiff: &Tiff) -> Option<String> {
    let jpg = tiff.ifds.first()?.bytes(PANASONIC_JPG_FROM_RAW)?;
    let exif = crate::jpeg_segments(jpg).exif?;
    let data = strip_exif_header(&exif);
    panasonic_lens(data, &Tiff::parse(data).ok()?)
}

/// Canon maker note tag `0x0095` (LensModel in the ExifTool Canon tag names): the lens name as ASCII. Older
/// bodies instead record a numeric LensType (CameraSettings, tag `0x0001`, index 22), which is no name to read.
const CANON_LENS_MODEL: u16 = 0x0095;

/// The lens name in a Canon maker note of this Exif stream (`data` starts at its TIFF header): the note of
/// its own Exif IFD. Only consulted when Make is Canon, so other vendors' plain notes are never read as lenses.
fn canon_maker_note_lens(data: &[u8], tiff: &Tiff) -> Option<String> {
    let make = tiff.ifds.first().and_then(|i| i.string(t::MAKE)).unwrap_or_default();
    if !make.trim().to_ascii_uppercase().starts_with("CANON") {
        return None;
    }
    let exif = tiff.exif()?;
    let e = exif.get(t::MAKER_NOTE)?;
    let mn = craft_tiff::makernote::parse_makernote(data, e.offset, e.count() as u64, tiff.order, &make)?;
    text(Some(&mn.ifd), CANON_LENS_MODEL)
}

/// Nikon maker note tag `0x0084` (Lens in the ExifTool Nikon tag names): shortest and longest focal length, then the
/// widest aperture at each, as rationals, like Exif LensSpecification. Zero for a lens that doesn't report itself.
const NIKON_LENS: u16 = 0x0084;

/// The lens a Nikon camera records in its maker note, when it's plausible.
fn nikon_lens_spec(exif: Option<&Ifd>, make: Option<&str>, order: craft_tiff::ByteOrder) -> Option<[f64; 4]> {
    let make = make?;
    if !make.trim().to_ascii_uppercase().starts_with("NIKON") {
        return None;
    }
    let note = exif?.bytes(t::MAKER_NOTE)?;
    let mn = craft_tiff::makernote::parse_makernote(note, 0, note.len() as u64, order, make)?;
    let s = mn.ifd.f64s(NIKON_LENS)?;
    let [short, long, wide, narrow] = s.as_slice() else { return None };
    let focal = |f: f64| f.is_finite() && f > 0.0 && f < 10_000.0;
    let aperture = |a: f64| a.is_finite() && a > 0.0 && a < 100.0;
    (focal(*short) && focal(*long) && long >= short && aperture(*wide) && aperture(*narrow)).then_some([*short, *long, *wide, *narrow])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};

    fn capture_time_tiff(original: Option<&str>, digitized: Option<&str>, ifd0_time: Option<&str>, xmp: Option<&str>) -> Vec<u8> {
        let mut exif = IfdBuilder::new();
        if let Some(value) = original {
            exif.set(t::DATE_TIME_ORIGINAL, Value::Ascii(value.into()));
        }
        if let Some(value) = digitized {
            exif.set(t::DATE_TIME_DIGITIZED, Value::Ascii(value.into()));
        }
        let mut ifd0 = IfdBuilder::new();
        if let Some(value) = ifd0_time {
            ifd0.set(t::DATE_TIME, Value::Ascii(value.into()));
        }
        ifd0.set_child(t::EXIF_IFD, exif);
        if let Some(packet) = xmp {
            ifd0.set(t::XMP, Value::Byte(packet.as_bytes().to_vec()));
        }
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn exif_clock_invalid_original_uses_the_next_valid_exif_time() {
        let invalid = "2026:01:01 12:34:56:99";
        let digitized = capture_time_tiff(Some(invalid), Some("2026:01:02 10:00:00"), Some("2026:01:03 11:00:00"), None);
        assert_eq!(read_exif(&digitized).capture_time.map(|date| date.to_iso()), Some("2026-01-02T10:00:00".into()));

        let ifd0 = capture_time_tiff(Some(invalid), Some("2026:01:02 10:00:00:"), Some("2026:01:03 11:00:00"), None);
        assert_eq!(read_exif(&ifd0).capture_time.map(|date| date.to_iso()), Some("2026-01-03T11:00:00".into()));

        let none = capture_time_tiff(Some(invalid), Some("2026:01:02 10:00:00:junk"), Some("2026:01:03 11:00:00:"), None);
        assert_eq!(read_exif(&none).capture_time, None);

        let original = capture_time_tiff(Some("2026:01:04 12:00:00"), Some("2026:01:02 10:00:00"), Some("2026:01:03 11:00:00"), None);
        assert_eq!(read_exif(&original).capture_time.map(|date| date.to_iso()), Some("2026-01-04T12:00:00".into()));
    }

    #[test]
    fn exif_clock_invalid_original_allows_xmp_fallback_and_keeps_raw_tag() {
        let capture = DateTime::parse_iso("2026-01-02T10:00:00Z");
        let packet = crate::write_xmp(&Metadata { capture_time: capture, ..Default::default() });
        let bytes = capture_time_tiff(Some("2026:01:01 12:34:56:99"), None, None, Some(&packet));
        assert_eq!(crate::extract(&bytes).capture_time.map(|date| date.to_iso()), Some("2026-01-02T10:00:00+00:00".into()));

        let without_fallback = capture_time_tiff(Some("2026:01:01 12:34:56:junk"), None, None, None);
        assert_eq!(crate::extract(&without_fallback).capture_time, None);
        assert!(crate::embedded(&without_fallback).exif.is_some(), "rejecting the projection must retain the embedded EXIF block");
    }

    /// An Exif block from a Panasonic body: no LensModel, the lens name in maker note tag 0x0051, whose value
    /// lies outside the note's IFD at an offset relative to the Exif TIFF header (as Panasonic writes it).
    pub(crate) fn panasonic_exif(order: ByteOrder, lens: Option<&str>, lens_model: Option<&str>) -> Vec<u8> {
        let note = |at: u32| {
            let mut v = b"Panasonic\0\0\0".to_vec();
            order.put_u16(&mut v, 1);
            order.put_u16(&mut v, if lens.is_some() { PANASONIC_LENS_TYPE } else { 0x0001 });
            order.put_u16(&mut v, 2);
            let name = format!("{:<34}\0", lens.unwrap_or("0210"));
            order.put_u32(&mut v, name.len() as u32);
            order.put_u32(&mut v, at + 30);
            order.put_u32(&mut v, 0);
            v.extend_from_slice(name.as_bytes());
            v
        };
        let build = |at: u32| {
            let mut exif = IfdBuilder::new();
            exif.set(t::MAKER_NOTE, Value::Undefined(note(at)));
            exif.set(t::FOCAL_LENGTH, Value::Rational(vec![(25, 1)]));
            if let Some(l) = lens_model {
                exif.set(t::LENS_MODEL, Value::Ascii(l.into()));
            }
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii("Panasonic".into()));
            ifd0.set_child(t::EXIF_IFD, exif);
            TiffWriter::new(order, false).write(&[ifd0]).unwrap()
        };
        // the note's own offset is only known once it is laid out; its length doesn't change
        let first = build(0);
        let at = Tiff::parse(&first).unwrap().exif().unwrap().get(t::MAKER_NOTE).unwrap().offset as u32;
        build(at)
    }

    #[test]
    fn panasonic_maker_note_names_the_lens() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let m = read_exif(&panasonic_exif(order, Some("LUMIX G 25/F1.7"), None));
            assert_eq!(m.lens_model.as_deref(), Some("LUMIX G 25/F1.7"));
            assert_eq!(m.focal_length, Some(25.0));
        }
        // Exif's own LensModel wins, and a note without 0x0051 leaves the lens empty
        let m = read_exif(&panasonic_exif(ByteOrder::Little, Some("LUMIX G 25/F1.7"), Some("Other 25mm")));
        assert_eq!(m.lens_model.as_deref(), Some("Other 25mm"));
        assert_eq!(read_exif(&panasonic_exif(ByteOrder::Little, None, None)).lens_model, None);
    }

    /// A RW2 has the full Exif only in the APP1 of its JpgFromRaw (IFD0 0x002E), not in its own Exif IFD.
    pub(crate) fn panasonic_rw2(lens: &str) -> Vec<u8> {
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe1];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(panasonic_exif(ByteOrder::Little, Some(lens), None));
        jpg.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        jpg.extend(app1);
        jpg.extend_from_slice(&[0xff, 0xda, 0, 2, 0xff, 0xd9]);
        let mut exif = IfdBuilder::new();
        exif.set(t::FOCAL_LENGTH, Value::Rational(vec![(85, 1)]));
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("Panasonic".into()));
        ifd0.set(PANASONIC_JPG_FROM_RAW, Value::Undefined(jpg));
        ifd0.set_child(t::EXIF_IFD, exif);
        TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn rw2_reads_the_lens_from_its_jpg_from_raw() {
        let rw2 = panasonic_rw2("LUMIX S 85/F1.8");
        let m = read_exif(&rw2);
        assert_eq!(m.lens_model.as_deref(), Some("LUMIX S 85/F1.8"));
        assert_eq!(m.focal_length, Some(85.0));
        assert_eq!(crate::extract(&rw2).lens_model.as_deref(), Some("LUMIX S 85/F1.8"));
    }

    /// Canon maker note tags: `0x0095` holds the lens name as ASCII; `0x0001` is CameraSettings, a table of
    /// int16s whose index 22 is the numeric LensType.
    const CANON_LENS_MODEL_TAG: u16 = 0x0095;
    const CANON_CAMERA_SETTINGS: u16 = 0x0001;

    /// An Exif block from a Canon EOS 5D: `Canon` / `Canon EOS 5D` in IFD0, no LensModel tag, and a plain maker
    /// note whose IFD (value offsets relative to the enclosing TIFF header, as Canon writes it) records
    /// CameraSettings with the numeric LensType at index 22 and, when `lens` is given, the name at `0x0095`.
    fn canon_exif(order: ByteOrder, lens: Option<&str>) -> Vec<u8> {
        let note = |at: u32| {
            // The plain IFD: entry count, one or two 12-byte entries and the next-IFD pointer, then the
            // out-of-line values (CameraSettings, then the lens name when present).
            // pointer, then the out-of-line values (CameraSettings, then the lens name when present).
            let settings_at = at + 2 + 12 * (1 + lens.is_some() as u32) + 4;
            let name_at = settings_at + 60;
            let mut v = Vec::new();
            let entry = |v: &mut Vec<u8>, tag: u16, kind: u16, count: u32, off: u32| {
                order.put_u16(v, tag);
                order.put_u16(v, kind);
                order.put_u32(v, count);
                order.put_u32(v, off);
            };
            order.put_u16(&mut v, 1 + lens.is_some() as u16);
            entry(&mut v, CANON_CAMERA_SETTINGS, 3, 30, settings_at);
            if let Some(name) = lens {
                entry(&mut v, CANON_LENS_MODEL_TAG, 2, name.len() as u32, name_at);
            }
            order.put_u32(&mut v, 0);
            for i in 0..30u16 {
                order.put_u16(&mut v, if i == 22 { 178 } else { 0 }); // 22: numeric LensType, no name
            }
            if let Some(name) = lens {
                v.extend_from_slice(name.as_bytes());
            }
            v
        };
        let build = |at: u32| {
            let mut exif = IfdBuilder::new();
            exif.set(t::MAKER_NOTE, Value::Undefined(note(at)));
            exif.set(t::FOCAL_LENGTH, Value::Rational(vec![(50, 1)]));
            let mut ifd0 = IfdBuilder::new();
            ifd0.set(t::MAKE, Value::Ascii("Canon".into()));
            ifd0.set(t::MODEL, Value::Ascii("Canon EOS 5D".into()));
            ifd0.set_child(t::EXIF_IFD, exif);
            TiffWriter::new(order, false).write(&[ifd0]).unwrap()
        };
        // the note's own offset is only known once it is laid out; its length doesn't change
        let first = build(0);
        let at = Tiff::parse(&first).unwrap().exif().unwrap().get(t::MAKER_NOTE).unwrap().offset as u32;
        build(at)
    }

    #[test]
    fn canon_maker_note_lens_fills_lens_model() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let m = read_exif(&canon_exif(order, Some("Canon EF 50mm f/1.8 II")));
            assert_eq!(m.lens_model.as_deref(), Some("Canon EF 50mm f/1.8 II"));
            // a note that only records the numeric LensType has no name to fill in
            assert_eq!(read_exif(&canon_exif(order, None)).lens_model, None);
        }
    }

    /// An Exif block from a Nikon body whose maker note records the lens (tag 0x0084) and no LensModel or
    /// LensSpecification, as the D700, D750, D80 and D5300 write it.
    fn nikon_exif(make: &str, lens: Option<[(u32, u32); 4]>, spec: Option<[(u32, u32); 4]>) -> Vec<u8> {
        let mut note = IfdBuilder::new();
        note.set(0x0001, Value::Undefined(b"0210".to_vec()));
        if let Some(l) = lens {
            note.set(NIKON_LENS, Value::Rational(l.to_vec()));
        }
        let mut blob = b"Nikon\0\x02\x10\0\0".to_vec();
        blob.extend(TiffWriter::new(ByteOrder::Little, false).write(&[note]).unwrap());
        let mut exif = IfdBuilder::new();
        exif.set(t::MAKER_NOTE, Value::Undefined(blob));
        if let Some(sp) = spec {
            exif.set(t::LENS_SPECIFICATION, Value::Rational(sp.to_vec()));
        }
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii(make.into()));
        ifd0.set_child(t::EXIF_IFD, exif);
        TiffWriter::new(ByteOrder::Big, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn nikon_maker_note_lens_fills_the_lens_spec() {
        let zoom = [(18, 1), (135, 1), (35, 10), (56, 10)];
        assert_eq!(read_exif(&nikon_exif("NIKON CORPORATION", Some(zoom), None)).lens_spec, Some([18.0, 135.0, 3.5, 5.6]));
        // Exif's own LensSpecification wins, another make's note is not read, and a lens that reports nothing is left out
        let prime = [(50, 1), (50, 1), (18, 10), (18, 10)];
        assert_eq!(read_exif(&nikon_exif("NIKON CORPORATION", Some(zoom), Some(prime))).lens_spec, Some([50.0, 50.0, 1.8, 1.8]));
        assert_eq!(read_exif(&nikon_exif("Canon", Some(zoom), None)).lens_spec, None);
        assert_eq!(read_exif(&nikon_exif("NIKON CORPORATION", Some([(0, 1), (0, 1), (0, 1), (0, 1)]), None)).lens_spec, None);
        assert_eq!(read_exif(&nikon_exif("NIKON CORPORATION", Some([(200, 1), (80, 1), (28, 10), (28, 10)]), None)).lens_spec, None);
        assert_eq!(read_exif(&nikon_exif("NIKON CORPORATION", None, None)).lens_spec, None);
    }

    pub(crate) fn sample_exif(order: ByteOrder) -> Vec<u8> {
        let mut exif = IfdBuilder::new();
        exif.set(t::EXPOSURE_TIME, Value::Rational(vec![(1, 250)]));
        exif.set(t::F_NUMBER, Value::Rational(vec![(28, 10)]));
        exif.set(t::ISO_SPEED, Value::Short(vec![400]));
        exif.set(t::DATE_TIME_ORIGINAL, Value::Ascii("2023:07:14 18:30:05".into()));
        exif.set(t::OFFSET_TIME_ORIGINAL, Value::Ascii("+02:00".into()));
        exif.set(t::SUBSEC_TIME_ORIGINAL, Value::Ascii("42".into()));
        exif.set(t::EXPOSURE_BIAS, Value::SRational(vec![(-2, 3)]));
        exif.set(t::METERING_MODE, Value::Short(vec![5]));
        exif.set(t::FLASH, Value::Short(vec![0x19]));
        exif.set(t::FOCAL_LENGTH, Value::Rational(vec![(50, 1)]));
        exif.set(t::FOCAL_LENGTH_35MM, Value::Short(vec![75]));
        exif.set(t::WHITE_BALANCE, Value::Short(vec![0]));
        exif.set(t::EXPOSURE_PROGRAM, Value::Short(vec![3]));
        exif.set(t::LENS_MAKE, Value::Ascii("LensCo".into()));
        exif.set(t::LENS_MODEL, Value::Ascii("50mm F1.8".into()));
        exif.set(t::BODY_SERIAL_NUMBER, Value::Ascii("12345".into()));
        exif.set(t::LENS_SPECIFICATION, Value::Rational(vec![(50, 1), (50, 1), (18, 10), (18, 10)]));
        exif.set(t::PIXEL_X_DIMENSION, Value::Long(vec![6000]));
        exif.set(t::PIXEL_Y_DIMENSION, Value::Long(vec![4000]));
        let mut gps = IfdBuilder::new();
        gps.set(t::GPS_LATITUDE_REF, Value::Ascii("S".into()));
        gps.set(t::GPS_LATITUDE, Value::Rational(vec![(33, 1), (51, 1), (3564, 100)]));
        gps.set(t::GPS_LONGITUDE_REF, Value::Ascii("E".into()));
        gps.set(t::GPS_LONGITUDE, Value::Rational(vec![(151, 1), (12, 1), (4032, 100)]));
        gps.set(t::GPS_ALTITUDE_REF, Value::Byte(vec![1]));
        gps.set(t::GPS_ALTITUDE, Value::Rational(vec![(125, 10)]));
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::MAKE, Value::Ascii("Maker Inc.".into()));
        ifd0.set(t::MODEL, Value::Ascii("Model X ".into()));
        ifd0.set(t::ORIENTATION, Value::Short(vec![6]));
        ifd0.set(t::ARTIST, Value::Ascii("Jane".into()));
        ifd0.set(t::COPYRIGHT, Value::Ascii("(c) Jane".into()));
        ifd0.set(t::RATING, Value::Short(vec![3]));
        ifd0.set_child(t::EXIF_IFD, exif);
        ifd0.set_child(t::GPS_IFD, gps);
        TiffWriter::new(order, false).write(&[ifd0]).unwrap()
    }

    #[test]
    fn reads_all_fields() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let m = read_exif(&sample_exif(order));
            assert_eq!(m.make.as_deref(), Some("Maker Inc."));
            assert_eq!(m.model.as_deref(), Some("Model X"));
            assert_eq!(m.exposure_time, Some(0.004));
            assert_eq!(m.f_number, Some(2.8));
            assert_eq!(m.iso, Some(400));
            let dt = m.capture_time.unwrap();
            assert_eq!(dt.to_iso(), "2023-07-14T18:30:05.420+02:00");
            assert!((m.exposure_bias.unwrap() + 0.6667).abs() < 1e-3);
            assert_eq!(m.metering_mode, Some(5));
            assert_eq!(m.flash, Some(Flash { fired: true, raw: 0x19 }));
            assert_eq!(m.focal_length, Some(50.0));
            assert_eq!(m.focal_length_35mm, Some(75.0));
            assert_eq!(m.orientation, Some(Orientation::Rotate90));
            assert_eq!(m.lens_model.as_deref(), Some("50mm F1.8"));
            assert_eq!(m.lens_make.as_deref(), Some("LensCo"));
            assert_eq!(m.serial_number.as_deref(), Some("12345"));
            assert_eq!(m.lens_spec, Some([50.0, 50.0, 1.8, 1.8]));
            assert_eq!((m.width, m.height), (Some(6000), Some(4000)));
            assert_eq!(m.artist.as_deref(), Some("Jane"));
            assert_eq!(m.rating, Some(3));
            let g = m.gps.unwrap();
            assert!((g.latitude + 33.8599).abs() < 1e-4, "{g:?}");
            assert!((g.longitude - 151.2112).abs() < 1e-4);
            assert_eq!(g.altitude, Some(-12.5));
        }
    }

    #[test]
    fn placeholder_gps_block_is_no_location() {
        // Android phones without a fix write NUL refs and 0/0 rationals.
        let read = |lat_ref: &str, lat: Vec<(u32, u32)>| {
            let mut gps = IfdBuilder::new();
            gps.set(t::GPS_LATITUDE_REF, Value::Ascii(lat_ref.into()));
            gps.set(t::GPS_LATITUDE, Value::Rational(lat));
            gps.set(t::GPS_LONGITUDE_REF, Value::Ascii("E".into()));
            gps.set(t::GPS_LONGITUDE, Value::Rational(vec![(151, 1), (12, 1), (4032, 100)]));
            let mut ifd0 = IfdBuilder::new();
            ifd0.set_child(t::GPS_IFD, gps);
            read_exif(&TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap()).gps
        };
        let valid = vec![(33, 1), (51, 1), (3564, 100)];
        assert!(read("N", valid.clone()).is_some());
        assert_eq!(read("N", vec![(0, 0), (0, 0), (0, 0)]), None, "0/0 rationals");
        assert_eq!(read("N", vec![(33, 1), (0, 0), (0, 0)]), None, "0/0 minutes");
        assert_eq!(read(" ", valid.clone()), None, "NUL ref");
        assert_eq!(read("", valid.clone()), None, "empty ref");
        assert_eq!(read("X", valid), None, "invalid ref");
    }

    #[test]
    fn exif_prefix_and_garbage() {
        let mut b = b"Exif\0\0".to_vec();
        b.extend(sample_exif(ByteOrder::Little));
        assert_eq!(read_exif(&b).iso, Some(400));
        assert_eq!(read_exif(b"garbage"), Metadata::default());
        assert!(try_read_exif(b"garbage").is_err());
    }

    #[test]
    fn apex_fallbacks_and_extended_iso() {
        let mut exif = IfdBuilder::new();
        exif.set(t::SHUTTER_SPEED_VALUE, Value::SRational(vec![(8, 1)]));
        exif.set(t::APERTURE_VALUE, Value::Rational(vec![(4, 1)]));
        exif.set(t::ISO_SPEED, Value::Short(vec![65535]));
        exif.set(t::RECOMMENDED_EXPOSURE_INDEX, Value::Long(vec![102400]));
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::DATE_TIME, Value::Ascii("2020:01:02 03:04:05".into()));
        ifd0.set_child(t::EXIF_IFD, exif);
        let m = read_exif(&TiffWriter::default().write(&[ifd0]).unwrap());
        assert_eq!(m.exposure_time, Some(1.0 / 256.0));
        assert_eq!(m.f_number, Some(4.0));
        assert_eq!(m.iso, Some(102400));
        assert_eq!(m.capture_time.unwrap().to_exif(), "2020:01:02 03:04:05");
    }

    #[test]
    fn xmp_in_ifd0_overrides_user_fields() {
        let mut meta = Metadata { rating: Some(5), title: Some("From XMP".into()), ..Default::default() };
        meta.keywords = vec!["a".into()];
        let x = crate::write_xmp(&meta);
        let mut ifd0 = IfdBuilder::new();
        ifd0.set(t::RATING, Value::Short(vec![1]));
        ifd0.set(t::XMP, Value::Byte(x.into_bytes()));
        let m = read_exif(&TiffWriter::default().write(&[ifd0]).unwrap());
        assert_eq!(m.rating, Some(5));
        assert_eq!(m.title.as_deref(), Some("From XMP"));
    }
}

/// An exact `1/N` rational for a sub-second value whose reciprocal is a whole number (shutter speeds such as
/// 1/12800 s, which [`craft_tiff::writer::rational`]'s decimal denominators cannot represent).
fn reciprocal(v: Option<f64>) -> Option<craft_tiff::Value> {
    let v = v.filter(|v| v.is_finite() && *v > 0.0 && *v < 1.0)?;
    let inv = 1.0 / v;
    ((inv - inv.round()).abs() < 1e-6 && inv.round() <= u32::MAX as f64).then(|| craft_tiff::Value::Rational(vec![(1, inv.round() as u32)]))
}

/// Serialise the interchange subset of `meta` as a TIFF-structured Exif block (little-endian, no
/// `Exif\0\0` prefix): IFD0 (make, model, software, artist, copyright, description, orientation,
/// date), the Exif IFD (capture settings, lens) and, when present, the GPS IFD.
pub fn write_exif(meta: &Metadata) -> Vec<u8> {
    use craft_tiff::writer::{rational, srational};
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};
    let ascii = |s: &Option<String>| s.as_ref().filter(|v| !v.is_empty()).map(|v| Value::Ascii(v.clone()));
    let mut ifd0 = IfdBuilder::new();
    for (tag, v) in [
        (t::MAKE, ascii(&meta.make)),
        (t::MODEL, ascii(&meta.model)),
        (t::SOFTWARE, ascii(&meta.software)),
        (t::ARTIST, ascii(&meta.artist)),
        (t::COPYRIGHT, ascii(&meta.copyright)),
        (t::IMAGE_DESCRIPTION, ascii(&meta.caption)),
        (t::DATE_TIME, meta.capture_time.map(|d| Value::Ascii(d.to_exif()))),
        (t::ORIENTATION, meta.orientation.map(|o| Value::Short(vec![o.to_exif()]))),
    ] {
        if let Some(v) = v {
            ifd0.set(tag, v);
        }
    }
    let mut ex = IfdBuilder::new();
    ex.set(t::EXIF_VERSION, Value::Undefined(b"0232".to_vec()));
    let r = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0).map(|v| Value::Rational(vec![rational(v)]));
    for (tag, v) in [
        (t::DATE_TIME_ORIGINAL, meta.capture_time.map(|d| Value::Ascii(d.to_exif()))),
        (t::EXPOSURE_TIME, reciprocal(meta.exposure_time).or_else(|| r(meta.exposure_time))),
        (t::F_NUMBER, r(meta.f_number)),
        (t::FOCAL_LENGTH, r(meta.focal_length)),
        (t::ISO_SPEED, meta.iso.map(|i| Value::Short(vec![i.min(65_535) as u16]))),
        (t::ISO_SPEED_RATINGS_EXT, meta.iso.filter(|&i| i > 65_534).map(|i| Value::Long(vec![i]))),
        (t::LENS_MAKE, ascii(&meta.lens_make)),
        (t::LENS_MODEL, ascii(&meta.lens_model)),
        (t::EXPOSURE_BIAS, meta.exposure_bias.map(|v| Value::SRational(vec![srational(v)]))),
    ] {
        if let Some(v) = v {
            ex.set(tag, v);
        }
    }
    if let Some(d) = meta.capture_time {
        // an offset past 18 h is not a real one (a deserialized value can be anything): write none
        let offset = d.offset_minutes.filter(|o| o.unsigned_abs() <= 18 * 60).map(|o| {
            let sign = if o < 0 { '-' } else { '+' };
            format!("{sign}{:02}:{:02}", o.unsigned_abs() / 60, o.unsigned_abs() % 60)
        });
        let subsec = (d.millis != 0).then(|| format!("{:03}", d.millis));
        for (tag, v) in [(t::OFFSET_TIME, &offset), (t::OFFSET_TIME_ORIGINAL, &offset), (t::SUBSEC_TIME, &subsec), (t::SUBSEC_TIME_ORIGINAL, &subsec)]
        {
            if let Some(v) = v {
                ex.set(tag, Value::Ascii(v.clone()));
            }
        }
    }
    ifd0.set_child(t::EXIF_IFD, ex);
    if let Some(g) = meta.gps {
        let dms = |v: f64| {
            let v = v.abs();
            let d = v.floor();
            let m = ((v - d) * 60.0).floor();
            let s = (v - d - m / 60.0) * 3600.0;
            Value::Rational(vec![(d as u32, 1), (m as u32, 1), ((s * 1000.0).round() as u32, 1000)])
        };
        let mut gps = IfdBuilder::new();
        gps.set(t::GPS_VERSION_ID, Value::Byte(vec![2, 3, 0, 0]));
        gps.set(t::GPS_LATITUDE_REF, Value::Ascii(if g.latitude < 0.0 { "S" } else { "N" }.into()));
        gps.set(t::GPS_LATITUDE, dms(g.latitude));
        gps.set(t::GPS_LONGITUDE_REF, Value::Ascii(if g.longitude < 0.0 { "W" } else { "E" }.into()));
        gps.set(t::GPS_LONGITUDE, dms(g.longitude));
        if let Some(a) = g.altitude {
            gps.set(t::GPS_ALTITUDE_REF, Value::Byte(vec![u8::from(a < 0.0)]));
            gps.set(t::GPS_ALTITUDE, Value::Rational(vec![rational(a.abs())]));
        }
        ifd0.set_child(t::GPS_IFD, gps);
    }
    TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap_or_default()
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn capture_time_offset_and_subsec_round_trip() {
        for iso in ["2026-09-20T05:40:00.250+02:00", "2026-09-20T05:40:00.005-03:30", "2026-09-20T05:40:00"] {
            let m = Metadata { capture_time: DateTime::parse_iso(iso), ..Default::default() };
            assert_eq!(read_exif(&write_exif(&m)).capture_time, m.capture_time, "{iso}");
        }
    }

    /// A deserialized offset can be any i16, `i16::MIN` included: writing must not overflow.
    #[test]
    fn extreme_capture_time_offsets_do_not_panic() {
        for o in [i16::MIN, i16::MAX, -1081, 1081] {
            let mut dt = DateTime::parse_iso("2026-09-20T05:40:00").unwrap();
            dt.offset_minutes = Some(o);
            let m = Metadata { capture_time: Some(dt), ..Default::default() };
            assert_eq!(read_exif(&write_exif(&m)).capture_time.map(|d| d.offset_minutes), Some(None), "{o}");
        }
    }

    #[test]
    fn exif_round_trips() {
        let m = Metadata {
            make: Some("Synthetic".into()),
            model: Some("X2".into()),
            copyright: Some("© 2026 A. Person".into()),
            artist: Some("A. Person".into()),
            capture_time: DateTime::parse_iso("2026-09-20T05:40:00"),
            exposure_time: Some(1.0 / 250.0),
            f_number: Some(2.8),
            iso: Some(400),
            focal_length: Some(35.0),
            lens_model: Some("35mm F2".into()),
            gps: Some(Gps { latitude: 43.7904, longitude: -110.6818, altitude: Some(2000.0) }),
            ..Default::default()
        };
        let back = read_exif(&write_exif(&m));
        assert_eq!(back.make, m.make);
        assert_eq!(back.model, m.model);
        assert_eq!(back.copyright, m.copyright);
        assert_eq!(back.artist, m.artist);
        assert_eq!(back.capture_time.map(|d| d.to_exif()), m.capture_time.map(|d| d.to_exif()));
        assert_eq!(back.iso, Some(400));
        assert!((back.f_number.unwrap() - 2.8).abs() < 1e-6);
        assert!((back.exposure_time.unwrap() - 0.004).abs() < 1e-9);
        assert_eq!(back.lens_model, m.lens_model);
        let g = back.gps.unwrap();
        assert!((g.latitude - 43.7904).abs() < 1e-5 && (g.longitude + 110.6818).abs() < 1e-5, "{g:?}");
    }

    #[test]
    fn iso_above_65534_round_trips() {
        for iso in [65_534, 65_535, 102_400, 409_600] {
            let back = read_exif(&write_exif(&Metadata { iso: Some(iso), ..Default::default() }));
            assert_eq!(back.iso, Some(iso));
        }
    }

    #[test]
    fn fast_shutter_speeds_round_trip_exactly() {
        for den in [8000.0, 12_800.0, 16_000.0, 32_000.0] {
            let back = read_exif(&write_exif(&Metadata { exposure_time: Some(1.0 / den), ..Default::default() }));
            let got = back.exposure_time.unwrap();
            assert!((1.0 / got - den).abs() < 0.5, "1/{den} s came back as 1/{}", 1.0 / got);
        }
    }
}
