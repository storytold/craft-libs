//! JPEG XL container metadata (ISO/IEC 18181-2): the `Exif` and `xml ` boxes.
//!
//! A JPEG XL file is either a bare codestream (`FF 0A`, no metadata) or an ISO base media file
//! that starts with the 12-byte `JXL ` signature box. Its metadata boxes may be stored as they
//! are, or wrapped in a `brob` box: the real box type followed by a Brotli stream of the payload.
//! An `Exif` payload starts with a four-byte big-endian `tiff_header_offset`: the TIFF header
//! begins that many bytes after the prefix.

use crate::container::Embedded;
use crate::cr3::{MAX_BOXES, be32, boxes};

/// The signature box every JPEG XL container starts with.
const SIGNATURE: &[u8; 12] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// A decompressed `brob` payload larger than this is left out, as too large for metadata.
const MAX_BROB: usize = 32 << 20;

/// Whether `bytes` are a JPEG XL container (a bare codestream is not: it carries no metadata).
pub fn is_jxl_container(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE)
}

/// The Exif and XMP blocks of a JPEG XL container: the first `Exif` and the first `xml ` box,
/// plain or inside `brob`. Anything malformed is left out rather than reported.
pub fn metadata(bytes: &[u8]) -> Embedded {
    let mut out = Embedded::default();
    if !is_jxl_container(bytes) {
        return out;
    }
    let mut budget = MAX_BOXES;
    for b in boxes(bytes, SIGNATURE.len(), bytes.len(), &mut budget) {
        if out.exif.is_some() && out.xmp.is_some() {
            break;
        }
        let Some(payload) = bytes.get(b.body..b.end) else { break };
        let (kind, payload) = match &b.kind {
            b"brob" => {
                let (Some(kind), Some(stream)) = (payload.get(..4), payload.get(4..)) else { continue };
                let mut k = [0u8; 4];
                k.copy_from_slice(kind);
                if (&k == b"Exif" && out.exif.is_none()) || (&k == b"xml " && out.xmp.is_none()) {
                    let Some(data) = unbrotli(stream) else { continue };
                    (k, std::borrow::Cow::Owned(data))
                } else {
                    continue;
                }
            }
            k => (*k, std::borrow::Cow::Borrowed(payload)),
        };
        match &kind {
            b"Exif" if out.exif.is_none() => out.exif = exif_tiff(&payload).map(<[u8]>::to_vec),
            b"xml " if out.xmp.is_none() => out.xmp = Some(String::from_utf8_lossy(&payload).trim_end_matches('\0').to_string()),
            _ => {}
        }
    }
    out
}

/// The TIFF stream of an `Exif` box payload: after the four-byte prefix and the offset it holds.
/// `None` when the offset points past the payload, or at nothing.
pub(crate) fn exif_tiff(payload: &[u8]) -> Option<&[u8]> {
    let skip = be32(payload, 0).and_then(|v| usize::try_from(v).ok())?.checked_add(4)?;
    payload.get(skip..).filter(|b| !b.is_empty())
}

/// Decompress a `brob` stream, giving up on a malformed stream or one past [`MAX_BROB`].
fn unbrotli(stream: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    let limit = u64::try_from(MAX_BROB).ok()?.checked_add(1)?;
    brotli_decompressor::Decompressor::new(stream, 4096).take(limit).read_to_end(&mut out).ok()?;
    (out.len() <= MAX_BROB).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    fn brob(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut body = kind.to_vec();
        let mut w = brotli::CompressorWriter::new(&mut body, 4096, 5, 22);
        w.write_all(payload).unwrap();
        drop(w);
        bx(b"brob", &body)
    }

    /// A container whose codestream is a stand-in: the metadata reader never parses it.
    fn container(boxes: &[Vec<u8>]) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend(bx(b"ftyp", b"jxl \0\0\0\0jxl "));
        v.extend(bx(b"jxlc", &[0xff, 0x0a, 1, 2, 3]));
        for b in boxes {
            v.extend_from_slice(b);
        }
        v
    }

    /// A little-endian TIFF with one IFD0 entry: `Make` = `m`.
    fn tiff(make: &str) -> Vec<u8> {
        let m = format!("{make}\0");
        let mut v = b"II*\0\x08\0\0\0\x01\0\x0f\x01\x02\0".to_vec();
        v.extend_from_slice(&(m.len() as u32).to_le_bytes());
        v.extend_from_slice(&26u32.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(m.as_bytes());
        v
    }

    fn exif_box(tiff: &[u8], offset: u32) -> Vec<u8> {
        let mut p = offset.to_be_bytes().to_vec();
        p.extend(std::iter::repeat_n(0u8, offset as usize));
        p.extend_from_slice(tiff);
        p
    }

    const XMP: &str = "<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:title><rdf:Alt><rdf:li xml:lang='x-default'>Golden horizon</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>";

    #[test]
    fn plain_exif_and_xml_boxes_are_read() {
        let t = tiff("NIKON CORPORATION");
        let f = container(&[bx(b"Exif", &exif_box(&t, 0)), bx(b"xml ", XMP.as_bytes())]);
        let e = embedded(&f);
        assert_eq!(e.exif.as_deref(), Some(t.as_slice()));
        assert_eq!(e.xmp.as_deref(), Some(XMP));
        let m = crate::extract(&f);
        assert_eq!(m.make.as_deref(), Some("NIKON CORPORATION"));
        assert_eq!(m.title.as_deref(), Some("Golden horizon"));
    }

    #[test]
    fn brotli_wrapped_boxes_are_read() {
        let t = tiff("SONY");
        let f = container(&[brob(b"Exif", &exif_box(&t, 0)), brob(b"xml ", XMP.as_bytes())]);
        let e = embedded(&f);
        assert_eq!(e.exif.as_deref(), Some(t.as_slice()));
        assert_eq!(e.xmp.as_deref(), Some(XMP));
    }

    /// The `Exif` prefix is an offset to the TIFF header, not always 0.
    #[test]
    fn the_tiff_header_offset_is_honoured() {
        let t = tiff("Canon");
        let f = container(&[bx(b"Exif", &exif_box(&t, 6))]);
        assert_eq!(embedded(&f).exif.as_deref(), Some(t.as_slice()));
        assert_eq!(crate::extract(&f).make.as_deref(), Some("Canon"));
        // an offset past the payload yields nothing
        let f = container(&[bx(b"Exif", &[0, 0, 0, 100, 1, 2, 3, 4])]);
        assert_eq!(embedded(&f).exif, None);
        assert_eq!(exif_tiff(&[0, 0, 1, 0, 1, 2, 3]), None);
        assert_eq!(exif_tiff(&[0, 0]), None);
    }

    #[test]
    fn the_first_of_each_box_wins_and_the_bare_codestream_has_none() {
        let (a, b) = (tiff("A"), tiff("B"));
        let f = container(&[bx(b"Exif", &exif_box(&a, 0)), bx(b"Exif", &exif_box(&b, 0)), bx(b"xml ", b"<one/>"), bx(b"xml ", b"<two/>")]);
        let e = embedded(&f);
        assert_eq!(e.exif.as_deref(), Some(a.as_slice()));
        assert_eq!(e.xmp.as_deref(), Some("<one/>"));
        assert_eq!(embedded(&[0xff, 0x0a, 1, 2, 3, 4]), Embedded::default());
        assert!(!is_jxl_container(b"\0\0\0\x0cJXL \r\n\x87"));
    }

    #[test]
    fn broken_files_yield_nothing_without_panicking() {
        let t = tiff("X");
        let good = container(&[brob(b"Exif", &exif_box(&t, 0)), bx(b"xml ", XMP.as_bytes())]);
        for n in 0..good.len() {
            let _ = embedded(&good[..n]);
        }
        // a box that claims more than the file holds, a zero size, a 64-bit size and junk Brotli
        let mut huge = bx(b"Exif", &exif_box(&t, 0));
        huge[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(embedded(&container(&[huge])), Embedded::default());
        let mut zero = bx(b"xml ", b"<z/>");
        zero[..4].copy_from_slice(&[0; 4]);
        assert_eq!(embedded(&container(&[zero])).xmp.as_deref(), Some("<z/>"));
        let mut large = vec![0, 0, 0, 1];
        large.extend_from_slice(b"xml ");
        large.extend_from_slice(&(8u64 + 8 + 4).to_be_bytes());
        large.extend_from_slice(b"<l/>");
        assert_eq!(embedded(&container(&[large])).xmp.as_deref(), Some("<l/>"));
        assert_eq!(embedded(&container(&[bx(b"brob", b"Exif\x01\x02\x03\x04\x05"), bx(b"brob", b"xm")])), Embedded::default());
        // a brob that inflates past the cap is left out
        let big = vec![0u8; MAX_BROB + 1];
        assert_eq!(embedded(&container(&[brob(b"xml ", &big)])), Embedded::default());
    }
}
