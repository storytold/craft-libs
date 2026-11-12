//! Resolution in Exif and XMP blocks, rewritten in place, and the Exif thumbnail dropped on
//! export. Every block here is built by hand.

use std::borrow::Cow;

use craft_metadata::edit::*;
use proptest::prelude::{any, proptest};

const X: u16 = 282;

const Y: u16 = 283;

const UNIT: u16 = 296;

const SHORT: u16 = 3;

const LONG: u16 = 4;

const RATIONAL: u16 = 5;

const ASCII: u16 = 2;

/// One IFD0 entry: tag, type, count and the value bytes (in the block's byte order).
struct E(u16, u16, u32, Vec<u8>);

struct Exif {
    big: bool,
}

impl Exif {
    fn u16(&self, v: u16) -> [u8; 2] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }
    fn u32(&self, v: u32) -> [u8; 4] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }
    fn rational(&self, n: u32, d: u32) -> Vec<u8> {
        [self.u32(n), self.u32(d)].concat()
    }
    fn short(&self, v: u16) -> Vec<u8> {
        self.u16(v).to_vec()
    }
    fn x(&self, n: u32, d: u32) -> E {
        E(X, RATIONAL, 1, self.rational(n, d))
    }
    fn y(&self, n: u32, d: u32) -> E {
        E(Y, RATIONAL, 1, self.rational(n, d))
    }
    fn unit(&self, u: u16) -> E {
        E(UNIT, SHORT, 1, self.short(u))
    }
    fn make(&self) -> E {
        E(0x010F, ASCII, 6, b"NIKON\0".to_vec())
    }
    fn orientation(&self, o: u16) -> E {
        E(0x0112, SHORT, 1, self.short(o))
    }

    /// A TIFF-structured EXIF block: header, IFD0 at 8, then the out-of-line values.
    fn build(&self, entries: &[E]) -> Vec<u8> {
        let mut v = if self.big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
        v.extend_from_slice(&self.u32(8));
        v.extend_from_slice(&self.u16(entries.len() as u16));
        let mut data_at = 8 + 2 + entries.len() * 12 + 4;
        let mut data = Vec::new();
        for E(tag, ty, count, value) in entries {
            v.extend_from_slice(&self.u16(*tag));
            v.extend_from_slice(&self.u16(*ty));
            v.extend_from_slice(&self.u32(*count));
            if value.len() <= 4 {
                let mut inline = value.clone();
                inline.resize(4, 0);
                v.extend_from_slice(&inline);
            } else {
                v.extend_from_slice(&self.u32(data_at as u32));
                data.extend_from_slice(value);
                data_at += value.len();
            }
        }
        v.extend_from_slice(&self.u32(0));
        v.extend_from_slice(&data);
        v
    }
}

const LE: Exif = Exif { big: false };

const BE: Exif = Exif { big: true };

/// A D3200-style block: Make, XResolution 300/1, YResolution 300/1, ResolutionUnit 2.
fn camera_exif(e: &Exif) -> Vec<u8> {
    e.build(&[e.make(), e.x(300, 1), e.y(300, 1), e.unit(2)])
}

fn near(got: Option<(f32, f32)>, want: f32) {
    let (x, y) = got.unwrap_or_else(|| panic!("no resolution, wanted {want}"));
    assert!((x - want).abs() < 0.01 && (y - want).abs() < 0.01, "got {x}×{y}, wanted {want}");
}

proptest! {
    #[test]
    fn hostile_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..256), ppi in 0.0f32..2e6) {
        let _ = exif_resolution(&bytes);
        let _ = exif_with_resolution(&bytes, (ppi, ppi));
        let _ = photoshop_resolution(&bytes);
        let s = String::from_utf8_lossy(&bytes);
        let _ = xmp_resolution(&s);
        let _ = xmp_with_resolution(&s, (ppi, ppi));
    }

    #[test]
    fn mutated_exif_never_panics(i in 0usize..80, v in any::<u8>(), ppi in 1.0f32..10_000.0) {
        let mut exif = camera_exif(&LE);
        if let Some(b) = exif.get_mut(i) { *b = v; }
        let _ = exif_resolution(&exif);
        let fixed = exif_with_resolution(&exif, (ppi, ppi));
        let _ = exif_resolution(&fixed);
        let _ = exif_orientation(&fixed);
    }
}

#[test]
fn rewriting_keeps_every_other_byte() {
    for e in [&LE, &BE] {
        let src = e.build(&[e.make(), e.orientation(6), e.x(300, 1), e.y(300, 1), e.unit(3)]);
        let out = exif_with_resolution(&src, (240.0, 240.0)).into_owned();
        assert_eq!(out.len(), src.len());
        near(exif_resolution(&out), 240.0);
        let want = e.build(&[e.make(), e.orientation(6), e.x(240, 1), e.y(240, 1), e.unit(2)]);
        assert_eq!(out, want);
        // Fractions keep three decimals.
        near(exif_resolution(&exif_with_resolution(&src, (299.5, 72.25))).map(|(x, _)| (x, x)), 299.5);
        // Nothing to change: borrowed.
        assert!(matches!(exif_with_resolution(&camera_exif(e), (300.0, 300.0)), Cow::Borrowed(_)));
        let no_res = e.build(&[e.make()]);
        assert!(matches!(exif_with_resolution(&no_res, (240.0, 240.0)), Cow::Borrowed(_)));
        // With the JPEG prefix.
        let prefixed = [b"Exif\0\0".as_slice(), &src].concat();
        assert_eq!(exif_with_resolution(&prefixed, (240.0, 240.0)).as_ref(), [b"Exif\0\0".as_slice(), &want].concat());
    }
}

#[test]
fn entries_that_cannot_be_rewritten_are_removed() {
    for e in [&LE, &BE] {
        // XResolution as a SHORT and a LONG unit: both go, the rest stays readable.
        let src = e.build(&[e.make(), E(X, SHORT, 1, e.short(300)), e.y(300, 1), E(UNIT, LONG, 1, e.u32(2).to_vec()), e.orientation(6)]);
        let out = exif_with_resolution(&src, (240.0, 240.0)).into_owned();
        assert_eq!(out.len(), src.len());
        let count = if e.big { u16::from_be_bytes([out[8], out[9]]) } else { u16::from_le_bytes([out[8], out[9]]) };
        assert_eq!(count, 3);
        assert_eq!(exif_orientation(&out), 6);
        // Only YResolution is left (240): no X, so no resolution at all, and no stale 300.
        assert_eq!(exif_resolution(&out), None);
        let tags: Vec<u16> = (0..3)
            .map(|k| {
                let at = 10 + k * 12;
                if e.big { u16::from_be_bytes([out[at], out[at + 1]]) } else { u16::from_le_bytes([out[at], out[at + 1]]) }
            })
            .collect();
        assert_eq!(tags, vec![0x010F, Y, 0x0112]);
        // The next-IFD pointer followed the table, the freed entries are zero.
        assert_eq!(&out[10 + 36..10 + 40], &[0, 0, 0, 0]);
        assert!(out[10 + 40..10 + 64].iter().all(|&b| b == 0));
        // The Make string (out of line) did not move.
        assert!(out.windows(6).any(|w| w == b"NIKON\0"));
    }
}

#[test]
fn xmp_resolution_is_rewritten() {
    let attr = r#"<rdf:Description tiff:XResolution="300/1" tiff:YResolution='300/1' tiff:ResolutionUnit="3" tiff:Make="X"/>"#;
    assert_eq!(
        xmp_with_resolution(attr, (240.0, 240.0)),
        r#"<rdf:Description tiff:XResolution="240/1" tiff:YResolution='240/1' tiff:ResolutionUnit="2" tiff:Make="X"/>"#
    );
    let elem = "<tiff:XResolution>300/1</tiff:XResolution><tiff:YResolution>300/1</tiff:YResolution>";
    assert_eq!(
        xmp_with_resolution(elem, (72.5, 72.5)),
        "<tiff:XResolution>72500/1000</tiff:XResolution><tiff:YResolution>72500/1000</tiff:YResolution>"
    );
    for same in [r#"tiff:XResolution="240" tiff:YResolution="240/1""#, "<x/>"] {
        assert!(matches!(xmp_with_resolution(same, (240.0, 240.0)), Cow::Borrowed(_)), "{same}");
    }
}

/// `camera_exif` plus IFD1, the embedded thumbnail a camera (or the last editor) wrote:
/// JPEGInterchangeFormat / JPEGInterchangeFormatLength naming a fake JPEG after the IFD.
fn exif_with_thumbnail(e: &Exif) -> Vec<u8> {
    let mut v = camera_exif(e);
    let ifd1 = v.len() as u32;
    // IFD0 (4 entries) ends with its next-IFD pointer.
    v[8 + 2 + 4 * 12..8 + 2 + 4 * 12 + 4].copy_from_slice(&e.u32(ifd1));
    let thumb = b"\xFF\xD8stale thumbnail\xFF\xD9";
    let thumb_at = ifd1 + 2 + 2 * 12 + 4;
    v.extend_from_slice(&e.u16(2));
    for (tag, value) in [(0x0201u16, thumb_at), (0x0202, thumb.len() as u32)] {
        v.extend_from_slice(&e.u16(tag));
        v.extend_from_slice(&e.u16(LONG));
        v.extend_from_slice(&e.u32(1));
        v.extend_from_slice(&e.u32(value));
    }
    v.extend_from_slice(&e.u32(0));
    v.extend_from_slice(thumb);
    v
}

fn next_ifd(e: &Exif, exif: &[u8]) -> u32 {
    let at = 8 + 2 + 4 * 12;
    let b: [u8; 4] = exif[at..at + 4].try_into().unwrap();
    if e.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
}

/// #2147 / #2373: an export must not carry the source's EXIF thumbnail (IFD1). Finder, Android
/// and upload pickers show it instead of the image, so an edited photo kept its old thumbnail.
#[test]
fn exports_drop_the_source_exif_thumbnail() {
    for e in [&LE, &BE] {
        let exif = exif_with_thumbnail(e);
        assert_ne!(next_ifd(e, &exif), 0);
        for ppi in [None, Some((300.0, 300.0)), Some((240.0, 240.0))] {
            let out = export_exif(&exif, ppi);
            assert_eq!(next_ifd(e, &out), 0, "IFD1 is unlinked ({ppi:?})");
            assert_eq!(out.len(), exif.len(), "nothing moves");
            assert!(out.windows(6).any(|w| w == b"NIKON\0"), "the rest of the EXIF is kept");
            near(exif_resolution(&out), ppi.map_or(300.0, |p| p.0));
        }
        // Only the pointer and the thumbnail's own bytes (the JPEG at the end) change.
        let out = export_exif(&exif, None);
        let thumb_len = b"\xFF\xD8stale thumbnail\xFF\xD9".len();
        let differing: Vec<usize> = (0..exif.len()).filter(|&i| exif[i] != out[i]).collect();
        assert!(differing.iter().all(|&i| (8 + 2 + 4 * 12..8 + 2 + 4 * 12 + 4).contains(&i) || i >= exif.len() - thumb_len), "{differing:?}");
        assert!(!out.windows(5).any(|w| w == b"stale"), "the thumbnail bytes are blanked, not just unlinked");
        // With a JPEG-style prefix too.
        let prefixed = [b"Exif\0\0".as_slice(), &exif].concat();
        assert_eq!(next_ifd(e, &export_exif(&prefixed, None)[6..]), 0);
        // An EXIF without a thumbnail comes through untouched.
        assert!(matches!(export_exif(&camera_exif(e), None), Cow::Borrowed(_)));
    }
}

/// A thumbnail range that covers IFD0, or tags that are not one SHORT/LONG value, are not blanked.
#[test]
fn thumbnail_blanking_keeps_ifd0_and_ignores_malformed_tags() {
    for e in [&LE, &BE] {
        let exif = exif_with_thumbnail(e);
        let ifd1 = camera_exif(e).len();
        let table = 8..8 + 2 + 4 * 12;
        // Offset 8, length 60: the range covers IFD0's table.
        let mut over = exif.clone();
        over[ifd1 + 2 + 8..ifd1 + 2 + 12].copy_from_slice(&e.u32(8));
        over[ifd1 + 2 + 12 + 8..ifd1 + 2 + 12 + 12].copy_from_slice(&e.u32(60));
        let out = export_exif(&over, None);
        assert_eq!(next_ifd(e, &out), 0, "IFD1 is still unlinked");
        assert_eq!(out[table.start..table.end], over[table.start..table.end], "IFD0's table is untouched");
        // Offset 0, length 8: the range covers the TIFF header, which must stay parseable.
        let mut header = exif.clone();
        header[ifd1 + 2 + 8..ifd1 + 2 + 12].copy_from_slice(&e.u32(0));
        header[ifd1 + 2 + 12 + 8..ifd1 + 2 + 12 + 12].copy_from_slice(&e.u32(8));
        let out = export_exif(&header, None);
        assert_eq!(out[..8], header[..8], "the TIFF header is untouched");
        // IFD0's last entry (Unit) turned into an ExifIFD pointer at IFD1's table: that range is not a thumbnail.
        let mut sub = exif.clone();
        let entry = 8 + 2 + 3 * 12;
        sub[entry..entry + 2].copy_from_slice(&e.u16(0x8769));
        sub[entry + 2..entry + 4].copy_from_slice(&e.u16(LONG));
        sub[entry + 4..entry + 8].copy_from_slice(&e.u32(1));
        sub[entry + 8..entry + 12].copy_from_slice(&e.u32(ifd1 as u32));
        sub[ifd1 + 2 + 8..ifd1 + 2 + 12].copy_from_slice(&e.u32(ifd1 as u32 + 2));
        sub[ifd1 + 2 + 12 + 8..ifd1 + 2 + 12 + 12].copy_from_slice(&e.u32(4));
        let out = export_exif(&sub, None);
        assert_eq!(out[ifd1 + 2..ifd1 + 6], sub[ifd1 + 2..ifd1 + 6], "the sub-IFD's table is untouched");
        // A count of 2 on the offset tag: the thumbnail bytes are not blanked.
        let mut counted = exif.clone();
        counted[ifd1 + 2 + 4..ifd1 + 2 + 8].copy_from_slice(&e.u32(2));
        assert!(export_exif(&counted, None).windows(5).any(|w| w == b"stale"));
    }
}

#[test]
fn thumbnail_removal_never_panics_on_truncated_exif() {
    for e in [&LE, &BE] {
        let exif = exif_with_thumbnail(e);
        for n in 0..exif.len() {
            let _ = export_exif(&exif[..n], None);
            let _ = export_exif(&exif[..n], Some((300.0, 300.0)));
        }
    }
}

/// Many Exif pointers to one huge sub-IFD must not cost a full scan each: past the scan budget the
/// thumbnail bytes are left alone (the IFD1 pointer is still cleared).
#[test]
fn thumbnail_overlap_scan_is_bounded_by_a_budget() {
    let e = &LE;
    let (pointers, sub_entries) = (100usize, 65_535usize);
    let sub = 8 + 2 + pointers * 12 + 4;
    let ifd1 = sub + 2 + sub_entries * 12 + 4;
    let thumb = b"\xFF\xD8stale thumbnail\xFF\xD9";
    let mut v = b"II*\0".to_vec();
    v.extend_from_slice(&e.u32(8));
    v.extend_from_slice(&e.u16(pointers as u16));
    for _ in 0..pointers {
        v.extend_from_slice(&[e.u16(0x8769), e.u16(LONG)].concat());
        v.extend_from_slice(&[e.u32(1), e.u32(sub as u32)].concat());
    }
    v.extend_from_slice(&e.u32(ifd1 as u32));
    v.extend_from_slice(&e.u16(sub_entries as u16));
    v.resize(ifd1, 0); // the sub-IFD's entries are type 0: nothing to hit
    v.extend_from_slice(&e.u16(2));
    for (tag, value) in [(0x0201u16, (ifd1 + 2 + 2 * 12 + 4) as u32), (0x0202, thumb.len() as u32)] {
        v.extend_from_slice(&[e.u16(tag), e.u16(LONG)].concat());
        v.extend_from_slice(&[e.u32(1), e.u32(value)].concat());
    }
    v.extend_from_slice(&e.u32(0));
    v.extend_from_slice(thumb);
    let out = export_exif(&v, None);
    assert_eq!(next_ifd_at(e, &out, 8 + 2 + pointers * 12), 0, "IFD1 is unlinked");
    assert!(out.windows(5).any(|w| w == b"stale"), "over the scan budget the thumbnail bytes stay");
}

fn next_ifd_at(e: &Exif, exif: &[u8], at: usize) -> u32 {
    let b: [u8; 4] = exif[at..at + 4].try_into().unwrap();
    if e.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
}

/// A thumbnail range that covers the Interoperability IFD (an Exif sub-IFD's 0xA005 target) is not
/// blanked: its pointer would stay behind, aimed at zeroes.
#[test]
fn thumbnail_blanking_keeps_the_interoperability_ifd() {
    for e in [&LE, &BE] {
        let (ifd0, sub, interop, ifd1) = (8usize, 8 + 2 + 12 + 4, 8 + 2 + 12 + 4 + 2 + 12 + 4, 8 + 2 + 12 + 4 + 2 + 12 + 4 + 2 + 12 + 4);
        let mut v = if e.big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
        v.extend_from_slice(&e.u32(ifd0 as u32));
        // IFD0: one ExifIFD pointer, then IFD1.
        v.extend_from_slice(&e.u16(1));
        v.extend_from_slice(&[e.u16(0x8769), e.u16(LONG)].concat());
        v.extend_from_slice(&[e.u32(1), e.u32(sub as u32)].concat());
        v.extend_from_slice(&e.u32(ifd1 as u32));
        // The Exif IFD: one Interoperability pointer.
        v.extend_from_slice(&e.u16(1));
        v.extend_from_slice(&[e.u16(0xA005), e.u16(LONG)].concat());
        v.extend_from_slice(&[e.u32(1), e.u32(interop as u32)].concat());
        v.extend_from_slice(&e.u32(0));
        // The Interoperability IFD: one inline ASCII entry (InteroperabilityIndex).
        v.extend_from_slice(&e.u16(1));
        v.extend_from_slice(&[e.u16(1), e.u16(ASCII)].concat());
        v.extend_from_slice(&[e.u32(4), b"R98\0".to_vec().try_into().unwrap_or([0; 4])].concat());
        v.extend_from_slice(&e.u32(0));
        assert_eq!(v.len(), ifd1);
        // IFD1: a thumbnail whose range is the Interoperability IFD.
        v.extend_from_slice(&e.u16(2));
        for (tag, value) in [(0x0201u16, interop as u32), (0x0202, 18)] {
            v.extend_from_slice(&[e.u16(tag), e.u16(LONG)].concat());
            v.extend_from_slice(&[e.u32(1), e.u32(value)].concat());
        }
        v.extend_from_slice(&e.u32(0));
        let out = export_exif(&v, None);
        assert_eq!(out[interop..interop + 18], v[interop..interop + 18], "the Interoperability IFD is untouched");
    }
}
