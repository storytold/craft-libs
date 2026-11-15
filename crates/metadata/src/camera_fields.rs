//! Unnormalized camera-field EXIF view, reused from PhotoCraft (exposure, aperture,
//! ISO, focal length, camera and lens names). Input is the raw TIFF-structured EXIF payload
//! (`II*\0` / `MM\0*`, as stored in `Document.metadata.exif`); malformed data yields `None`
//! fields, never a panic. Tag numbers are from the EXIF 2.32 specification (CIPA DC-008).

// Semantics originate in PhotoCraft, including Brandon Thomas and Fabio Lussi.
// Unlike read_exif this view does not normalize dates or derive missing fields.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

/// Camera settings read from EXIF.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// Seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<f64>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    /// 35 mm-equivalent focal length in millimetres.
    pub focal_length_35mm: Option<f64>,
    /// DateTimeOriginal as stored (`YYYY:MM:DD HH:MM:SS`).
    pub date_taken: Option<String>,
}

impl CameraInfo {
    /// Linear exposure factor `t · ISO / N²` (relative brightness of the image for one scene).
    pub fn exposure_factor(&self) -> Option<f64> {
        let t = self.exposure_time?;
        let n = self.f_number.unwrap_or(1.0).max(0.1);
        Some(t * self.iso.unwrap_or(100.0) / 100.0 / (n * n))
    }
}

/// Text bytes decoded per EXIF read (real Make/Model/Lens/Date strings are a few dozen bytes).
const MAX_TEXT_BYTES: usize = 1 << 20;

struct Tiff<'a> {
    b: &'a [u8],
    order: craft_tiff::ByteOrder,
    /// Text bytes this read may still decode: duplicate tags can share one large blank value.
    text_left: std::cell::Cell<usize>,
}

impl Tiff<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        self.order.read_u16(self.b, o)
    }
    fn u32(&self, o: usize) -> Option<u32> {
        self.order.read_u32(self.b, o)
    }
    /// Entries of the IFD at `off`: (tag, type, count, value offset field position).
    fn ifd(&self, off: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(off) else { return Vec::new() };
        (0..n as usize)
            .filter_map(|k| {
                // Checked: offsets come from the file, and usize is 32 bits on wasm.
                let e = off.checked_add(2)?.checked_add(k * 12)?;
                Some((self.u16(e)?, self.u16(e.checked_add(2)?)?, self.u32(e.checked_add(4)?)?, e.checked_add(8)?))
            })
            .collect()
    }
    fn data_at(&self, typ: u16, _count: u32, field: usize) -> Option<usize> {
        // Keep this caller's historical classic-TIFF type policy; shared spans also
        // support IFD/BigTIFF types which CameraInfo never accepted.
        if !(1..=12).contains(&typ) {
            return None;
        }
        let span = craft_tiff::entry_span(self.b, field.checked_sub(8)?, self.order, false, 0)?;
        usize::try_from(span.start).ok()
    }
    fn number(&self, typ: u16, count: u32, field: usize) -> Option<f64> {
        let at = self.data_at(typ, count, field)?;
        match typ {
            3 => self.u16(at).map(f64::from),
            4 => self.u32(at).map(f64::from),
            5 => {
                let (n, d) = (self.u32(at)?, self.u32(at.checked_add(4)?)?);
                (d != 0).then(|| n as f64 / d as f64)
            }
            10 => {
                let (n, d) = (self.u32(at)? as i32, self.u32(at.checked_add(4)?)? as i32);
                (d != 0).then(|| n as f64 / d as f64)
            }
            _ => None,
        }
    }
    fn text(&self, typ: u16, count: u32, field: usize) -> Option<String> {
        #[cfg(test)]
        TEXT_DECODES.with(|c| c.set(c.get() + 1));
        if typ != 2 {
            return None;
        }
        let at = self.data_at(typ, count, field)?;
        let n = usize::try_from(count).ok()?;
        self.text_left.set(self.text_left.get().checked_sub(n)?);
        let s = self.b.get(at..at.checked_add(n)?)?;
        let s = String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(test)]
thread_local!(static TEXT_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// Reads [`CameraInfo`] from a TIFF-structured EXIF payload (an `Exif\0\0` prefix is skipped).
pub fn read(exif: &[u8]) -> CameraInfo {
    let b = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let mut info = CameraInfo::default();
    let le = match b.get(..4) {
        Some([b'I', b'I', 42, 0]) => true,
        Some([b'M', b'M', 0, 42]) => false,
        _ => return info,
    };
    let t = Tiff {
        b,
        order: if le { craft_tiff::ByteOrder::Little } else { craft_tiff::ByteOrder::Big },
        text_left: std::cell::Cell::new(MAX_TEXT_BYTES),
    };
    let Some(ifd0) = t.u32(4) else { return info };
    let mut queue = vec![ifd0 as usize];
    let mut seen = Vec::new();
    while let Some(off) = queue.pop() {
        if seen.contains(&off) || seen.len() > 8 {
            continue;
        }
        seen.push(off);
        for (tag, typ, count, field) in t.ifd(off) {
            match tag {
                // Duplicate TIFF text tags can point at the same large payload. Keep the first
                // recognized value and do not repeatedly decode/copy later duplicates.
                0x010F if info.make.is_none() => info.make = t.text(typ, count, field),
                0x0110 if info.model.is_none() => info.model = t.text(typ, count, field),
                0x829A => info.exposure_time = t.number(typ, count, field),
                0x829D => info.f_number = t.number(typ, count, field),
                0x8827 => info.iso = t.number(typ, count, field),
                0x920A => info.focal_length = t.number(typ, count, field),
                0xA405 => info.focal_length_35mm = t.number(typ, count, field).filter(|v| *v > 0.0),
                0xA434 if info.lens.is_none() => info.lens = t.text(typ, count, field),
                0x9003 if info.date_taken.is_none() => info.date_taken = t.text(typ, count, field),
                0x8769 => {
                    if let Some(p) = t.u32(field) {
                        queue.push(p as usize);
                    }
                }
                _ => {}
            }
        }
    }
    info
}

/// Build a classic TIFF EXIF block using the shared TIFF writer.
/// Raw date strings and numeric rounding match the camera-field view. Text is
/// bounded to the reader's 1 MiB aggregate budget; oversized inputs return an error.
/// Physical directory placement is not part of this API's contract.
pub fn try_build(info: &CameraInfo) -> craft_tiff::Result<Vec<u8>> {
    use craft_tiff::{ByteOrder, IfdBuilder, TiffError, TiffWriter, Value};
    let total =
        [&info.make, &info.model, &info.lens, &info.date_taken].into_iter().flatten().try_fold(0usize, |n, s| n.checked_add(s.len())?.checked_add(1));
    if total.is_none_or(|n| n > MAX_TEXT_BYTES) {
        return Err(TiffError::Limit("camera EXIF text"));
    }
    let mut root = IfdBuilder::new();
    let mut exif = IfdBuilder::new();
    for (tag, value) in [(0x010f, &info.make), (0x0110, &info.model)] {
        if let Some(value) = value {
            root.set(tag, Value::Ascii(value.clone()));
        }
    }
    for (tag, value) in [(0x9003, &info.date_taken), (0xa434, &info.lens)] {
        if let Some(value) = value {
            exif.set(tag, Value::Ascii(value.clone()));
        }
    }
    for (tag, value) in [(0x829a, info.exposure_time), (0x829d, info.f_number), (0x920a, info.focal_length)] {
        if let Some(value) = value {
            exif.set(tag, Value::Rational(vec![((value * 10000.0).round() as u32, 10000)]));
        }
    }
    for (tag, value) in [(0x8827, info.iso), (0xa405, info.focal_length_35mm)] {
        if let Some(value) = value {
            exif.set(tag, Value::Short(vec![value as u16]));
        }
    }
    // Empty child IFDs are valid here; the reader yields absent fields.
    root.set_child(0x8769, exif);
    TiffWriter::new(ByteOrder::Little, false).write(&[root])
}

#[cfg(test)]
fn build(info: &CameraInfo) -> Vec<u8> {
    try_build(info).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_survives_garbage() {
        let info = CameraInfo {
            make: Some("Generic".into()),
            model: Some("Camera 1".into()),
            lens: Some("24-70mm".into()),
            exposure_time: Some(0.004),
            f_number: Some(8.0),
            iso: Some(200.0),
            focal_length: Some(35.0),
            focal_length_35mm: Some(52.0),
            date_taken: Some("2024:02:17 12:40:19".into()),
        };
        let b = build(&info);
        let r = read(&b);
        assert_eq!(r, info);
        assert!((r.exposure_factor().unwrap() - 0.004 * 2.0 / 64.0).abs() < 1e-9);
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&b);
        assert_eq!(read(&prefixed), info);
        assert_eq!(read(b"nonsense"), CameraInfo::default());
        for cut in [5, 12, 30, b.len() - 3] {
            let _ = read(&b[..cut]);
        }
    }

    /// Hostile payloads: huge counts, offsets near `u32::MAX`, self-referencing IFDs and random
    /// bytes read as `None` fields instead of panicking or overflowing (usize is 32 bits on wasm).
    #[test]
    fn blank_duplicate_text_tags_share_one_decode_budget() {
        // Blank values never fill `make`, so every duplicate would decode the same 1 MiB again;
        // the per-read budget stops after the first, so the later real value is not reached.
        let blank = MAX_TEXT_BYTES;
        let entry = |count: u32, off: u32| {
            let mut e = Vec::new();
            e.extend_from_slice(&0x010Fu16.to_le_bytes());
            e.extend_from_slice(&2u16.to_le_bytes());
            e.extend_from_slice(&count.to_le_bytes());
            e.extend_from_slice(&off.to_le_bytes());
            e
        };
        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend(entry(blank as u32, 50));
        b.extend(entry(blank as u32, 50));
        b.extend(entry(2, u32::from_le_bytes(*b"M\0\0\0")));
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(b.len(), 50);
        b.extend(std::iter::repeat_n(b' ', blank));
        assert_eq!(read(&b).make, None);
    }

    #[test]
    fn duplicate_text_tags_keep_the_first_value_without_copying_large_duplicates() {
        let value = vec![b'X'; 2 * 1024 * 1024];
        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&8u32.to_le_bytes());
        // IFD0: Make="M", a duplicate Make pointing at a 2 MiB payload, and the Exif IFD link.
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(&0x010Fu16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"M\0\0\0");
        b.extend_from_slice(&0x010Fu16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&(value.len() as u32).to_le_bytes());
        b.extend_from_slice(&80u32.to_le_bytes());
        b.extend_from_slice(&0x8769u16.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&50u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        // Exif IFD at 50: DateTimeOriginal="D" and the same large duplicate payload at offset 80.
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&0x9003u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"D\0\0\0");
        b.extend_from_slice(&0x9003u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&(value.len() as u32).to_le_bytes());
        b.extend_from_slice(&80u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&value);

        TEXT_DECODES.with(|c| c.set(0));
        let info = read(&b);
        assert_eq!(info.make.as_deref(), Some("M"));
        assert_eq!(info.date_taken.as_deref(), Some("D"));
        // One decode per tag: duplicates must be skipped before `text` copies their payload.
        assert_eq!(TEXT_DECODES.with(|c| c.get()), 2);
    }

    #[test]
    fn hostile_payloads_never_panic() {
        let entry = |tag: u16, typ: u16, count: u32, value: u32| {
            let mut e = tag.to_le_bytes().to_vec();
            e.extend_from_slice(&typ.to_le_bytes());
            e.extend_from_slice(&count.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        let mut b = b"II*\0".to_vec();
        b.extend_from_slice(&8u32.to_le_bytes());
        let entries = [
            entry(0x010F, 2, u32::MAX, 8),      // Make: count far past the end
            entry(0x0110, 2, 16, u32::MAX - 1), // Model: offset at the top of the range
            entry(0x829A, 5, u32::MAX, 20),     // ExposureTime: count overflows the size
            entry(0x829D, 5, 1, u32::MAX - 3),  // FNumber: rational straddles u32::MAX
            entry(0x9003, 2, 0x4000_0000, 8),   // DateTimeOriginal: size overflow on 32 bits
            entry(0x8769, 4, 1, 8),             // Exif IFD pointing back at IFD0
            entry(0x8769, 4, 1, u32::MAX),      // ... and past the end
        ];
        b.extend_from_slice(&(entries.len() as u16 + 3).to_le_bytes()); // claims more entries than exist
        for e in entries {
            b.extend_from_slice(&e);
        }
        assert_eq!(read(&b), CameraInfo::default());
        let mut be = b"MM\0*".to_vec();
        be.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(read(&be), CameraInfo::default());
        // Deterministic noise behind a valid header, and every truncation of a real payload.
        let mut x = 0x2545_F491u32;
        for _ in 0..200 {
            let mut n = b"II*\0\x08\0\0\0".to_vec();
            for _ in 0..96 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                n.push(x as u8);
            }
            let _ = read(&n);
        }
        let good = build(&CameraInfo {
            make: Some("Make".into()),
            date_taken: Some("2024:02:17 12:40:19".into()),
            exposure_time: Some(0.004),
            ..Default::default()
        });
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
    }
}

// Keep the previous byte reader only as a test oracle during the shared-span cutover.
#[cfg(test)]
#[path = "camera_fields_legacy_tests.rs"]
mod legacy;

#[cfg(test)]
mod shared_span_parity {
    use super::*;

    fn compare(bytes: &[u8]) {
        assert_eq!(format!("{:?}", read(bytes)), format!("{:?}", legacy::read(bytes)));
    }

    #[test]
    fn shared_writer_matches_legacy_fields_and_rejects_oversized_text() {
        for value in [0.0, -1.0, 0.004, 2.8, 65535.0, f64::INFINITY, f64::NAN] {
            let camera = CameraInfo {
                exposure_time: Some(value),
                f_number: Some(value),
                iso: Some(value),
                focal_length: Some(value),
                focal_length_35mm: Some(value),
                date_taken: Some("raw date".into()),
                make: Some("M".into()),
                model: Some("Model".into()),
                lens: Some("Lens".into()),
            };
            let old = legacy::CameraInfo {
                exposure_time: camera.exposure_time,
                f_number: camera.f_number,
                iso: camera.iso,
                focal_length: camera.focal_length,
                focal_length_35mm: camera.focal_length_35mm,
                date_taken: camera.date_taken.clone(),
                make: camera.make.clone(),
                model: camera.model.clone(),
                lens: camera.lens.clone(),
            };
            let bytes = try_build(&camera).unwrap();
            assert_eq!(format!("{:?}", read(&bytes)), format!("{:?}", legacy::read(&legacy::build(&old))));
        }
        assert_eq!(read(&try_build(&CameraInfo::default()).unwrap()), CameraInfo::default());
        assert!(try_build(&CameraInfo { make: Some("x".repeat(MAX_TEXT_BYTES)), ..Default::default() }).is_err());
        assert!(try_build(&CameraInfo { make: Some("x".repeat(MAX_TEXT_BYTES - 1)), ..Default::default() }).is_ok());
    }

    #[test]
    fn preserves_camera_selection_for_valid_truncated_and_mutated_payloads() {
        let bytes = build(&CameraInfo {
            make: Some("Maker".into()),
            model: Some("Camera".into()),
            lens: Some("Lens".into()),
            exposure_time: Some(0.125),
            f_number: Some(2.8),
            iso: Some(200.0),
            focal_length: Some(50.0),
            focal_length_35mm: Some(75.0),
            date_taken: Some("not normalized: original date".into()),
        });
        compare(&bytes);
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&bytes);
        compare(&prefixed);
        for end in 0..=bytes.len() {
            compare(&bytes[..end]);
        }
        // Exercise offsets, tags, types, counts and payloads independently, not random
        // external data. The complete shared span must remain bounded for every edit.
        for index in 0..bytes.len() {
            for value in [0, 0xff, 0x80] {
                let mut changed = bytes.clone();
                changed[index] = value;
                compare(&changed);
            }
        }
    }

    #[test]
    fn preserves_partial_entry_and_zero_count_semantics_in_both_orders() {
        for little in [true, false] {
            let u16b = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
            let u32b = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
            for count in [0u32, 1, 2, u32::MAX] {
                let mut bytes = if little { b"II".to_vec() } else { b"MM".to_vec() };
                bytes.extend(u16b(42));
                bytes.extend(u32b(8));
                bytes.extend(u16b(1));
                bytes.extend(u16b(0x8827));
                bytes.extend(u16b(3));
                bytes.extend(u32b(count));
                bytes.extend(u16b(200));
                compare(&bytes); // two inline bytes, no full 12-byte entry/next pointer
                bytes.extend(u16b(0));
                compare(&bytes);
            }
        }
    }
}
