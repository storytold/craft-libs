//! Each format's documents, built from scratch: what `sniff`, `info`, `renditions` and `outline`
//! return for them.

use super::common;

use common::*;
use craft_doc::{Error, Limits, RenditionKind, info, outline, rendition, renditions};

fn limits() -> Limits {
    Limits::default()
}

/// A PDF-only importer need not decode a broken preview, for inline or blob payloads.
#[test]
fn selected_rendition_ignores_other_payloads() {
    let inline = vectorcraft_json(
        3,
        &format!(r#""preview":{{"mime":"image/png","data":"!bad!"}}, "pdf":{{"mime":"application/pdf","data":"{}"}}"#, base64(PDF)),
    );
    let mut blob = vectorcraft_json(
        4,
        &format!(r#""preview":{{"mime":"image/png","blob":[{},100]}}, "pdf":{{"mime":"application/pdf","blob":[0,{}]}}"#, PDF.len() + 100, PDF.len()),
    )
    .into_bytes();
    blob.extend(BLOB_MAGIC);
    blob.extend(PDF);
    for bytes in [inline.into_bytes(), blob] {
        for bytes in [bytes.clone(), gzip(&bytes)] {
            assert!(renditions(&bytes, &limits()).is_err());
            let selected = rendition(&bytes, RenditionKind::Pdf, &limits()).unwrap().unwrap();
            assert_eq!(selected.bytes, PDF);
            assert!(rendition(&bytes, RenditionKind::Thumbnail, &limits()).is_err());
        }
    }
}

/// Stored DEFLATE blocks contain the intact prefix; the next block has reserved BTYPE=3.
fn gzip_with_broken_tail(prefix: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    for chunk in prefix.chunks(u16::MAX as usize) {
        let length = chunk.len() as u16;
        bytes.push(0); // Non-final stored block.
        bytes.extend(length.to_le_bytes());
        bytes.extend((!length).to_le_bytes());
        bytes.extend(chunk);
    }
    bytes.push(7); // Final block with an invalid block type.
    bytes.extend([0; 8]);
    bytes
}

#[test]
fn selected_inline_pdf_ignores_gzip_damage_after_its_json_marker() {
    for padding in [0, 128 * 1024] {
        let json = vectorcraft_json(
            4,
            &format!(r#""padding":"{}","preview":{},"pdf":{}"#, "x".repeat(padding), blob("image/png", 0, 100), inline("application/pdf", PDF)),
        );
        let mut prefix = json.as_bytes().to_vec();
        prefix.extend(BLOB_MAGIC);
        let bytes = gzip_with_broken_tail(&prefix);
        let mut exact = limits();
        exact.max_json = json.len() as u64;
        exact.max_total = (prefix.len() + PDF.len()) as u64;
        exact.max_entry = PDF.len() as u64;
        assert_eq!(rendition(&bytes, RenditionKind::Pdf, &exact).unwrap().unwrap().bytes, PDF);
        assert!(matches!(rendition(&bytes, RenditionKind::Thumbnail, &limits()), Err(Error::Malformed(_))));
        assert!(matches!(renditions(&bytes, &limits()), Err(Error::Malformed(_))));
        exact.max_total -= 1;
        assert!(matches!(rendition(&bytes, RenditionKind::Pdf, &exact), Err(Error::LimitExceeded(_))));
        exact.max_total += 1;
        exact.max_json -= 1;
        assert!(matches!(rendition(&bytes, RenditionKind::Pdf, &exact), Err(Error::LimitExceeded(_))));
        // A broken stream without a complete marker still fails, even after a complete JSON.
        assert!(matches!(rendition(&gzip_with_broken_tail(json.as_bytes()), RenditionKind::Pdf, &limits()), Err(Error::Malformed(_))));
    }
}

/// Ignored output is not charged, but gzip still has to traverse bytes before the PDF.
#[test]
fn selected_rendition_counts_its_payload_and_the_compressed_prefix() {
    let preview = vec![0; 128 * 1024];
    for before in [false, true] {
        let (preview_at, pdf_at) = if before { (0, preview.len()) } else { (PDF.len(), 0) };
        let json = vectorcraft_json(
            4,
            &format!(
                r#""preview":{{"mime":"image/png","blob":[{preview_at},{}]}},"pdf":{{"mime":"application/pdf","blob":[{pdf_at},{}]}}"#,
                preview.len(),
                PDF.len()
            ),
        );
        let mut bytes = json.as_bytes().to_vec();
        bytes.extend(BLOB_MAGIC);
        if before {
            bytes.extend(&preview);
            bytes.extend(PDF);
        } else {
            bytes.extend(PDF);
            bytes.extend(&preview);
        }
        let mut small = limits();
        small.max_entry = PDF.len() as u64;
        small.max_total = (json.len() + BLOB_MAGIC.len() + PDF.len()) as u64;
        assert!(matches!(renditions(&bytes, &small), Err(Error::LimitExceeded(_))));
        assert_eq!(rendition(&bytes, RenditionKind::Pdf, &small).unwrap().unwrap().bytes, PDF);
        let compressed = rendition(&gzip(&bytes), RenditionKind::Pdf, &small);
        if before {
            assert!(matches!(compressed, Err(Error::LimitExceeded(_))));
        } else {
            assert_eq!(compressed.unwrap().unwrap().bytes, PDF);
        }
    }
}

#[test]
fn vectorcraft_members_after_the_json() {
    // Version 4: the PDF after the marker, behind an image the document uses; the preview inline.
    let image = vec![0xAB; 9000];
    let preview = png(3, 3, 1);
    let members = format!(r#""preview":{},"pdf":{}"#, inline("image/png", &preview), blob("application/pdf", image.len(), PDF.len()));
    let mut file = vectorcraft_json(4, &members).into_bytes();
    file.extend(BLOB_MAGIC);
    file.extend(&image);
    file.extend(PDF);
    for bytes in [file.clone(), gzip(&file)] {
        let r = renditions(&bytes, &limits()).unwrap();
        assert_eq!(r.iter().map(|r| r.kind).collect::<Vec<_>>(), [RenditionKind::Thumbnail, RenditionKind::Pdf]);
        assert_eq!(r[0].bytes, preview);
        assert_eq!(r[1].bytes, PDF);
        // The JSON alone answers info and outline.
        assert_eq!(info(&bytes, &limits()).unwrap().version, 4);
        assert_eq!(outline(&bytes, &limits()).unwrap().pages.len(), 2);
    }
}

#[test]
fn a_compressed_marker_across_the_reader_s_64_kib_chunks() {
    // The marker starts at every offset around the first 64 KiB boundary of the unpacked stream.
    let base = vectorcraft_json(4, &format!(r#""pdf":{},"pad":"""#, blob("application/pdf", 0, PDF.len())));
    for json_len in (64 << 10) - 12..(64 << 10) + 2 {
        let json = base.replacen(r#""pad":"""#, &format!(r#""pad":"{}""#, "x".repeat(json_len - base.len())), 1);
        assert_eq!(json.len(), json_len);
        let mut file = json.into_bytes();
        file.extend(BLOB_MAGIC);
        file.extend(PDF);
        let gz = gzip(&file);
        assert_eq!(info(&gz, &limits()).unwrap().version, 4, "{json_len}");
        assert_eq!(outline(&gz, &limits()).unwrap().pages.len(), 2, "{json_len}");
        assert_eq!(renditions(&gz, &limits()).unwrap()[0].bytes, PDF, "{json_len}");
    }
}

/// Full-member identity is retained even when its dimensions differ from the document canvas.
#[test]
fn pcraft_prefers_the_full_size_composite_without_reading_the_preview() {
    let full = png(80, 60, 30);
    let small_preview = png(4, 3, 20);
    for preview in [false, true] {
        let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#);
        let mut entries = vec![store("manifest.json", manifest.as_bytes()), deflated("composite/full.png", &full)];
        if preview {
            entries.push(store("composite/preview.png", &small_preview));
        }
        let r = renditions(&zip(&entries), &limits()).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].kind, r[0].mime.as_str(), &r[0].bytes), (RenditionKind::Composite, "image/png", &full));
        assert!(r[0].is_full_composite);
        let canvas = outline(&zip(&entries), &limits()).unwrap().canvas.unwrap();
        assert_eq!((canvas.width, canvas.height), (640, 480), "the flag does not validate the 80 by 60 PNG against this canvas");
    }
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#);
    let mut tight = limits();
    tight.max_entry = manifest.len().max(full.len()) as u64;
    let oversized = vec![0; tight.max_entry as usize + 1];
    for preview in [store("composite/preview.png", &oversized), deflated("composite/preview.png", &oversized)] {
        let pc = zip(&[store("manifest.json", manifest.as_bytes()), deflated("composite/full.png", &full), preview]);
        assert_eq!(renditions(&pc, &tight).unwrap()[0].bytes, full);
    }
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png""#);
    let pc = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/full.png", &full)]);
    let selected = renditions(&pc, &limits()).unwrap().pop().unwrap();
    assert_eq!(selected.bytes, full);
    assert!(selected.is_full_composite);
    let mut small = limits();
    small.max_entry = manifest.len() as u64;
    // Keep the manifest under the limit while bounding the image independently.
    let large = vec![0; manifest.len() + full.len()];
    let pc = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/full.png", &large)]);
    assert!(matches!(renditions(&pc, &small), Err(Error::LimitExceeded(_))));
}

/// Invalid declarations and missing full entries never silently fall back to the preview.
#[test]
fn pcraft_refuses_invalid_or_missing_full_size_composites() {
    for field in [r#""../full.png""#, r#""composite/preview.png""#, "12", r#""composite/full.png", "full_composite":null"#] {
        let manifest = pcraft_manifest(1, "[]", &format!(r#", "full_composite":{field}"#));
        let pc = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &png(4, 3, 20))]);
        assert!(matches!(renditions(&pc, &limits()), Err(Error::Malformed(_))), "{field}");
    }
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#);
    let pc = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &png(4, 3, 20))]);
    assert!(matches!(renditions(&pc, &limits()), Err(Error::Malformed(_))));
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":null, "composite":"composite/preview.png""#);
    let preview = png(4, 3, 20);
    let pc = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &preview)]);
    let selected = renditions(&pc, &limits()).unwrap().pop().unwrap();
    assert_eq!(selected.bytes, preview);
    assert!(!selected.is_full_composite);
}
