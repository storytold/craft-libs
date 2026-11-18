//! Each format's documents, built from scratch: what `sniff`, `info`, `renditions` and `outline`
//! return for them.

mod common;

use common::*;
use craft_doc::{Error, Kind, Limits, RenditionKind, info, outline, rendition, renditions, sniff};

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

#[test]
fn vectorcraft_sniff_does_not_unwrap_nested_gzip() {
    let json = vectorcraft_json(4, "");
    assert_eq!(sniff(&gzip(&gzip(json.as_bytes()))), None);
}

#[test]
fn selected_rendition_skips_missing_zip_payloads_and_enforces_its_own_limits() {
    let manifest = pcraft_manifest(1, "[]", r#", "thumbnail":"thumb.png", "composite":"composite/preview.png""#);
    let image = png(2, 2, 1);
    let bytes = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &image)]);
    assert!(renditions(&bytes, &limits()).is_err());
    assert_eq!(rendition(&bytes, RenditionKind::Composite, &limits()).unwrap().unwrap().bytes, image);
    assert!(rendition(&bytes, RenditionKind::Thumbnail, &limits()).is_err());
    assert_eq!(rendition(&bytes, RenditionKind::Pdf, &limits()).unwrap(), None);
    let mut small = limits();
    small.max_entry = PDF.len() as u64 - 1;
    let vc = vectorcraft_json(3, &format!(r#""pdf":{{"mime":"application/pdf","data":"{}"}}"#, base64(PDF)));
    assert!(matches!(rendition(vc.as_bytes(), RenditionKind::Pdf, &small), Err(Error::LimitExceeded(_))));
}

#[test]
fn selected_rendition_preserves_format_and_version_checks() {
    assert_eq!(rendition(b"not a document", RenditionKind::Pdf, &limits()), Err(Error::Unrecognized));
    assert!(matches!(rendition(vectorcraft_json(99, "").as_bytes(), RenditionKind::Pdf, &limits()), Err(Error::TooNew { .. })));
    for bytes in [vectorcraft_json(4, "").into_bytes(), ecproj(2).into_bytes()] {
        assert_eq!(rendition(&bytes, RenditionKind::Pdf, &limits()).unwrap(), None);
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
fn sniff_tells_the_three_formats() {
    let vc = vectorcraft_json(4, "");
    assert_eq!(sniff(vc.as_bytes()), Some(Kind::VectorCraft));
    assert_eq!(sniff(&gzip(vc.as_bytes())), Some(Kind::VectorCraft));
    let legacy = vc.replacen("\"vectorcraft\"", "\"drawcraft\"", 1);
    assert_eq!(sniff(legacy.as_bytes()), Some(Kind::VectorCraft));
    // A gzip stream cut short still shows its start.
    let gz = gzip(vc.as_bytes());
    assert_eq!(sniff(gz.get(..gz.len() - 8).unwrap()), Some(Kind::VectorCraft));

    let pc = pcraft(&png(4, 4, 9), &png(8, 8, 9));
    assert_eq!(sniff(&pc), Some(Kind::PhotoCraft), "the manifest last, found through the central directory");
    let first = zip(&[store("manifest.json", b"{}"), store("thumb.png", &png(1, 1, 0))]);
    assert_eq!(sniff(first.get(..60).unwrap()), Some(Kind::PhotoCraft), "the manifest first, in a short head");
    let second = zip(&[store("tiles/a.zst", b"x"), store("manifest.json", b"{}")]);
    assert_eq!(sniff(second.get(..100).unwrap()), Some(Kind::PhotoCraft), "a later local header in the head");

    assert_eq!(sniff(ecproj(2).as_bytes()), Some(Kind::EffectCraft));
    let old = r#"{"schema":1,"settings":{},"items":{},"next_id":1}"#;
    assert_eq!(sniff(old.as_bytes()), Some(Kind::EffectCraft), "saved before savedBy");
}

#[test]
fn sniff_refuses_other_files() {
    let plain_zip = zip(&[store("word/document.xml", b"<w/>"), deflated("readme.txt", b"hello hello hello")]);
    let behind = zip(&[store("tiles/a.zst", &[7; 4096]), store("manifest.json", b"{}")]);
    for (what, bytes) in [
        ("empty", &b""[..]),
        ("a PDF", PDF),
        ("a PNG", &png(2, 2, 1)),
        ("a ZIP without a manifest", &plain_zip),
        ("a manifest past the head", behind.get(..100).unwrap()),
        ("other JSON", br#"{"name":"x","items":[],"schema":"y"}"#),
        ("JSON naming the format in a value", br#"{"title":"vectorcraft"}"#),
        ("another format", br#"{"format":"vectorcraftx","version":4}"#),
        ("a JSON array", br#"["vectorcraft"]"#),
        ("savedBy without schema", br#"{"savedBy":"1.0","items":{}}"#),
        ("savedBy with schema only in a nested object", br#"{"savedBy":"1.0","metadata":{"schema":1}}"#),
        ("savedBy with schema only in a value", br#"{"savedBy":"1.0","note":"\"schema\""}"#),
        ("schema with items only in a nested object", br#"{"schema":1,"meta":{"items":[]}}"#),
        ("gzip of other JSON", &gzip(br#"{"format":"svg"}"#)),
        ("PK alone", b"PK\x03\x04"),
    ] {
        assert_eq!(sniff(bytes), None, "{what}");
    }
}

#[test]
fn sniff_reads_the_names_in_the_central_directory_of_a_whole_archive() {
    let mut pc = zip(&[store("manifest.json", pcraft_manifest(1, "[]", "").as_bytes())]);
    // The local header still names manifest.json; the central directory names Xanifest.json.
    let c = pc.windows(4).rposition(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
    pc[c + 46] = b'X';
    assert_eq!(sniff(&pc), None);
    assert_eq!(info(&pc, &limits()), Err(Error::Unrecognized));
    // A head too short to reach the central directory is read through its local headers.
    assert_eq!(sniff(pc.get(..60).unwrap()), Some(Kind::PhotoCraft));
}

#[test]
fn a_zip_whose_manifest_has_no_format_version_is_not_photocraft() {
    // A browser extension: a ZIP archive with a manifest.json of its own.
    let ext = zip(&[store("manifest.json", br#"{"manifest_version":3,"name":"Example","version":"1.0"}"#)]);
    assert_eq!(sniff(&ext), Some(Kind::PhotoCraft));
    assert_eq!(info(&ext, &limits()), Err(Error::Unrecognized));
    assert_eq!(renditions(&ext, &limits()), Err(Error::Unrecognized));
    assert_eq!(outline(&ext, &limits()), Err(Error::Unrecognized));
    // A format_version that is present and not a number is a damaged manifest.
    for version in ["null", r#""1""#] {
        let damaged = zip(&[store("manifest.json", pcraft_manifest(1, "[]", "").replacen("1", version, 1).as_bytes())]);
        assert!(matches!(info(&damaged, &limits()), Err(Error::Malformed(_))), "{version}");
        assert!(matches!(outline(&damaged, &limits()), Err(Error::Malformed(_))), "{version}");
    }
}

#[test]
fn info_reads_version_and_writer() {
    let i = info(vectorcraft_json(3, "").as_bytes(), &limits()).unwrap();
    assert_eq!((i.kind, i.version, i.generator.as_deref(), i.supported), (Kind::VectorCraft, 3, Some("VectorCraft 0.1.0"), true));
    let i = info(&gzip(vectorcraft_json(4, "").as_bytes()), &limits()).unwrap();
    assert_eq!((i.version, i.supported), (4, true));

    let i = info(&pcraft(&png(1, 1, 0), &png(1, 1, 0)), &limits()).unwrap();
    assert_eq!((i.kind, i.version, i.generator.as_deref(), i.supported), (Kind::PhotoCraft, 1, Some("photocraft-format 0.1.0"), true));

    let i = info(ecproj(2).as_bytes(), &limits()).unwrap();
    assert_eq!((i.kind, i.version, i.generator.as_deref(), i.supported), (Kind::EffectCraft, 2, Some("0.12.0"), true));
    let i = info(br#"{"schema":1,"settings":{},"items":{},"next_id":1}"#, &limits()).unwrap();
    assert_eq!((i.version, i.generator), (1, None));

    assert_eq!(info(PDF, &limits()), Err(Error::Unrecognized));
}

#[test]
fn too_new_files_are_reported_and_refused() {
    // Newer versions may change everything after the version: a member of a new shape.
    let vc = vectorcraft_json(5, r#""pdf":"elsewhere""#);
    let manifest = r#"{"format_version":2,"generator":"photocraft-format 9","document":"elsewhere","thumbnail":7}"#;
    let pc = zip(&[store("manifest.json", manifest.as_bytes())]);
    let ec = r#"{"savedBy":"9.0.0","schema":3,"items":[]}"#.to_string();
    for (bytes, kind, version) in [(vc.into_bytes(), Kind::VectorCraft, 5), (pc, Kind::PhotoCraft, 2), (ec.into_bytes(), Kind::EffectCraft, 3)] {
        let i = info(&bytes, &limits()).unwrap();
        assert_eq!((i.kind, i.version, i.supported), (kind, version, false));
        let refused = Error::TooNew { kind, version, supported: kind.supported_version() };
        assert_eq!(renditions(&bytes, &limits()), Err(refused.clone()), "{kind:?}");
        assert_eq!(outline(&bytes, &limits()), Err(refused), "{kind:?}");
    }
}

#[test]
fn a_pcraft_format_version_of_zero_is_malformed() {
    let pc = zip(&[store("manifest.json", pcraft_manifest(0, "[]", "").as_bytes())]);
    assert!(matches!(info(&pc, &limits()), Err(Error::Malformed(_))));
    assert!(matches!(outline(&pc, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn vectorcraft_members_inline_as_base64() {
    let preview = png(16, 8, 200);
    let members = format!(r#""preview":{},"pdf":{}"#, inline("image/png", &preview), inline("application/pdf", PDF));
    let json = vectorcraft_json(3, &members);
    for bytes in [json.clone().into_bytes(), gzip(json.as_bytes())] {
        let r = renditions(&bytes, &limits()).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].kind, r[0].mime.as_str(), r[0].bytes.as_slice()), (RenditionKind::Thumbnail, "image/png", preview.as_slice()));
        assert_eq!((r[1].kind, r[1].mime.as_str(), r[1].bytes.as_slice()), (RenditionKind::Pdf, "application/pdf", PDF));
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
fn compressed_members_after_the_json_read_their_own_ranges() {
    // 100,000 bytes the document uses, then the preview and the PDF; then the same bytes named by
    // a PDF member that overlaps the preview.
    let image = vec![0xCD; 100_000];
    let preview = png(5, 5, 2);
    let mut tail = image.clone();
    tail.extend(&preview);
    tail.extend(PDF);
    let at = image.len();
    for (pdf_at, pdf_len, pdf) in [(at + preview.len(), PDF.len(), PDF.to_vec()), (at, preview.len() + PDF.len(), [&preview[..], PDF].concat())] {
        let members = format!(r#""preview":{},"pdf":{}"#, blob("image/png", at, preview.len()), blob("application/pdf", pdf_at, pdf_len));
        let mut file = vectorcraft_json(4, &members).into_bytes();
        file.extend(BLOB_MAGIC);
        file.extend(&tail);
        for bytes in [file.clone(), gzip(&file)] {
            let r = renditions(&bytes, &limits()).unwrap();
            assert_eq!((&r[0].bytes, &r[1].bytes), (&preview, &pdf));
        }
    }
}

#[test]
fn the_marker_follows_the_json_directly() {
    // A space between the JSON and the marker: the format crate does not split the file there.
    let mut file = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 0, PDF.len()))).into_bytes();
    file.push(b' ');
    file.extend(BLOB_MAGIC);
    file.extend(PDF);
    assert!(craft_vectorcraft_format::split_tail(&file).1.is_empty());
    for bytes in [file.clone(), gzip(&file)] {
        assert!(matches!(info(&bytes, &limits()), Err(Error::Malformed(_))));
        assert!(matches!(renditions(&bytes, &limits()), Err(Error::Malformed(_))));
        assert!(matches!(outline(&bytes, &limits()), Err(Error::Malformed(_))));
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

#[test]
fn a_padded_base64_member_fits_a_limit_of_its_own_size() {
    for len in 1..=3 {
        let data = vec![7u8; len];
        let bytes = vectorcraft_json(3, &format!(r#""preview":{}"#, inline("image/png", &data))).into_bytes();
        let mut l = limits();
        l.max_entry = len as u64;
        assert_eq!(renditions(&bytes, &l).unwrap()[0].bytes, data);
    }
}

#[test]
fn vectorcraft_without_members_has_no_renditions() {
    let json = vectorcraft_json(4, "");
    assert_eq!(renditions(json.as_bytes(), &limits()).unwrap(), vec![]);
    assert_eq!(renditions(&gzip(json.as_bytes()), &limits()).unwrap(), vec![]);
}

#[test]
fn vectorcraft_outline_has_artboards_and_top_level_layers() {
    for bytes in [vectorcraft_json(4, "").into_bytes(), gzip(vectorcraft_json(4, "").as_bytes())] {
        let o = outline(&bytes, &limits()).unwrap();
        assert_eq!(o.canvas, None);
        let pages: Vec<_> = o.pages.iter().map(|p| (p.name.as_str(), p.x, p.y, p.width, p.height)).collect();
        assert_eq!(pages, [("Front", 0.0, 0.0, 612.0, 792.0), ("Back", 650.0, 700.0, 50.0, 100.0)]);
        assert!(o.pages.iter().all(|p| p.timing.is_none() && p.layers.is_empty()));
        let layers: Vec<_> = o.layers.iter().map(|l| (l.name.as_str(), l.kind.as_str(), l.visible, l.opacity, l.blend.as_deref())).collect();
        assert_eq!(layers, [("Art", "layer", true, Some(1.0), Some("Normal")), ("Notes", "layer", false, Some(0.5), Some("Multiply"))]);
        assert!(o.layers.iter().all(|l| l.children.is_empty()));
    }
}

/// Thumbnail and legacy composite previews never claim a full-size source.
#[test]
fn pcraft_previews_stored_and_deflated_with_the_manifest_last() {
    let (thumb, composite) = (png(4, 3, 10), png(40, 30, 20));
    let r = renditions(&pcraft(&thumb, &composite), &limits()).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].kind, r[0].mime.as_str(), &r[0].bytes), (RenditionKind::Thumbnail, "image/png", &thumb));
    assert_eq!((r[1].kind, r[1].mime.as_str(), &r[1].bytes), (RenditionKind::Composite, "image/png", &composite));
    assert!(r.iter().all(|rendition| !rendition.is_full_composite));
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

/// The source flag distinguishes identical payloads selected from different manifest members.
#[test]
fn pcraft_identical_images_keep_their_composite_source() {
    let image = png(4, 3, 30);
    let mut variants = Vec::new();
    for declared in [false, true] {
        let fields = if declared {
            r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#
        } else {
            r#", "composite":"composite/preview.png""#
        };
        let manifest = pcraft_manifest(1, "[]", fields);
        let bundle = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/full.png", &image), store("composite/preview.png", &image)]);
        let rendition = renditions(&bundle, &limits()).unwrap().pop().unwrap();
        assert_eq!(rendition.is_full_composite, declared);
        variants.push(rendition);
    }
    assert_eq!((variants[0].kind, &variants[0].mime, &variants[0].bytes), (variants[1].kind, &variants[1].mime, &variants[1].bytes));
    assert_ne!(variants[0], variants[1], "equality includes the selected source");
}

/// Selected extraction keeps full-composite precedence and provenance without reading other images.
#[test]
fn selected_full_composite_skips_missing_thumbnail_and_damaged_preview() {
    let full = png(4, 3, 30);
    let manifest =
        pcraft_manifest(1, "[]", r#", "thumbnail":"thumb.png", "composite":"composite/preview.png", "full_composite":"composite/full.png""#);
    let pc =
        zip(&[store("manifest.json", manifest.as_bytes()), store("composite/full.png", &full), store("composite/preview.png", b"unused preview")]);
    let selected = rendition(&pc, RenditionKind::Composite, &limits()).unwrap().unwrap();
    assert_eq!(selected.bytes, full);
    assert!(selected.is_full_composite);
    assert!(rendition(&pc, RenditionKind::Thumbnail, &limits()).is_err());
    let without_full = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &full)]);
    assert!(rendition(&without_full, RenditionKind::Composite, &limits()).is_err(), "a declared full image cannot silently fall back");
}

/// Missing and null full declarations ignore an undeclared full entry; thumbnails stay false.
#[test]
fn pcraft_legacy_and_thumbnail_sources_are_not_full_composites() {
    let image = png(4, 3, 30);
    for declaration in ["", r#", "full_composite":null"#, r#", "full_composite":"composite/full.png""#] {
        let manifest = pcraft_manifest(1, "[]", &format!(r#", "thumbnail":"thumb.png", "composite":"composite/preview.png"{declaration}"#));
        let bundle = zip(&[
            store("manifest.json", manifest.as_bytes()),
            store("thumb.png", &image),
            store("composite/preview.png", &image),
            store("composite/full.png", &image),
        ]);
        let renditions = renditions(&bundle, &limits()).unwrap();
        assert_eq!(renditions.len(), 2);
        assert_eq!(renditions[0].kind, RenditionKind::Thumbnail);
        assert!(!renditions[0].is_full_composite);
        assert_eq!(renditions[1].kind, RenditionKind::Composite);
        assert_eq!(renditions[1].is_full_composite, declaration.contains("composite/full.png"));
    }
}

/// VectorCraft thumbnails and PDFs retain false provenance for plain and gzip documents.
#[test]
fn vectorcraft_renditions_are_not_full_composites() {
    let document = vectorcraft_json(3, &format!(r#""preview":{}, "pdf":{}"#, inline("image/png", &png(4, 3, 30)), inline("application/pdf", PDF)));
    for bytes in [document.as_bytes().to_vec(), gzip(document.as_bytes())] {
        let renditions = renditions(&bytes, &limits()).unwrap();
        assert_eq!(renditions.iter().map(|rendition| rendition.kind).collect::<Vec<_>>(), [RenditionKind::Thumbnail, RenditionKind::Pdf]);
        assert!(renditions.iter().all(|rendition| !rendition.is_full_composite));
    }
}

/// A checksum failure in the declared full entry remains an error despite a valid preview.
#[test]
fn a_damaged_declared_full_composite_does_not_fall_back() {
    let image = png(4, 3, 30);
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#);
    let mut bundle = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &image), store("composite/full.png", &image)]);
    let central = bundle
        .windows(4)
        .enumerate()
        .find(|(at, magic)| *magic == b"PK\x01\x02" && bundle.get(at + 46..at + 46 + b"composite/full.png".len()) == Some(b"composite/full.png"))
        .unwrap()
        .0;
    bundle[central + 16] ^= 1;
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
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

#[test]
fn pcraft_without_previews_has_none() {
    let pc = zip(&[store("manifest.json", pcraft_manifest(1, "[]", "").as_bytes())]);
    assert_eq!(renditions(&pc, &limits()).unwrap(), vec![]);
    let missing = zip(&[store("manifest.json", pcraft_manifest(1, "[]", r#","thumbnail":"thumb.png""#).as_bytes())]);
    assert!(matches!(renditions(&missing, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn pcraft_outline_has_the_canvas_and_layer_tree() {
    let o = outline(&pcraft(&png(1, 1, 0), &png(1, 1, 0)), &limits()).unwrap();
    let c = o.canvas.unwrap();
    assert_eq!((c.width, c.height, c.resolution_dpi, c.mode.as_str(), c.depth.as_str()), (640, 480, 300.0, "Rgb", "U16"));
    assert!(o.pages.is_empty());
    let layers: Vec<_> = o.layers.iter().map(|l| (l.name.as_str(), l.kind.as_str(), l.visible, l.opacity, l.blend.as_deref())).collect();
    assert_eq!(layers, [("Background", "raster", true, Some(1.0), Some("Normal")), ("Group", "group", false, Some(0.25), Some("Screen"))]);
    let inner = &o.layers[1].children;
    assert_eq!(inner.len(), 1);
    assert_eq!((inner[0].name.as_str(), inner[0].blend.as_deref(), inner[0].opacity), ("Inner", Some("Multiply"), Some(0.5)));
}

#[test]
fn pcraft_groups_nest_to_photocraft_s_depth_limit() {
    // 3 levels to the layer list, 3 per group and 4 for the leaf: 367 levels for 120 groups,
    // PhotoCraft's limit (`manifest_depth(MAX_GROUP_DEPTH)`).
    let nested = |groups: usize| {
        let mut layer = pcraft_layer("Leaf", true, "Normal", 1.0, raster());
        for i in 0..groups {
            layer = pcraft_layer(&format!("G{i}"), true, "Normal", 1.0, &format!(r#"{{"kind":"group","expanded":false,"children":[{layer}]}}"#));
        }
        zip(&[deflated("manifest.json", pcraft_manifest(1, &format!("[{layer}]"), "").as_bytes())])
    };
    let o = outline(&nested(120), &limits()).unwrap();
    let mut depth = 0;
    let mut l = &o.layers[0];
    while let Some(c) = l.children.first() {
        depth += 1;
        l = c;
    }
    assert_eq!((depth, l.name.as_str()), (120, "Leaf"));
    // One more group is past the limit, though within MAX_DEPTH.
    let deeper = nested(121);
    assert!(matches!(outline(&deeper, &limits()), Err(Error::LimitExceeded(_))));
    assert!(matches!(renditions(&deeper, &limits()), Err(Error::LimitExceeded(_))));
    assert_eq!(info(&deeper, &limits()).unwrap().version, 1);
}

#[test]
fn ecproj_has_no_renditions_and_lists_its_comps() {
    let p = ecproj(2);
    assert_eq!(renditions(p.as_bytes(), &limits()).unwrap(), vec![]);
    let o = outline(p.as_bytes(), &limits()).unwrap();
    assert!(o.canvas.is_none() && o.layers.is_empty());
    // By item id (3 before 10), the folder and the footage left out.
    let pages: Vec<_> = o.pages.iter().map(|p| (p.name.as_str(), p.width, p.height)).collect();
    assert_eq!(pages, [("Intro", 640.0, 360.0), ("Main", 1920.0, 1080.0)]);
    let t = o.pages[1].timing.unwrap();
    assert_eq!((t.frame_rate_num, t.frame_rate_den, t.duration_seconds), (24000, 1001, 5.0));
    assert_eq!(o.pages[0].timing.unwrap().duration_seconds, 1.0);
    // Bottom first: the file lists the top layer first.
    let layers: Vec<_> = o.pages[1].layers.iter().map(|l| (l.name.as_str(), l.kind.as_str(), l.visible, l.opacity, l.blend.as_deref())).collect();
    assert_eq!(layers, [("Plate", "Footage", false, None, Some("Normal")), ("Title", "Text", true, None, Some("Add"))]);
}

#[test]
fn an_ecproj_comp_without_a_duration_has_no_timing() {
    let p = ecproj(2).replacen(r#""duration": 254016000000, "#, "", 1);
    let o = outline(p.as_bytes(), &limits()).unwrap();
    assert_eq!((o.pages[0].name.as_str(), o.pages[0].timing), ("Intro", None));
    assert!(o.pages[1].timing.is_some());
}

#[test]
fn kinds_name_their_extension_and_version() {
    assert_eq!(
        [Kind::PhotoCraft, Kind::VectorCraft, Kind::EffectCraft].map(|k| (k.extension(), k.supported_version())),
        [("pcraft", 1), ("vectorcraft", 4), ("ecproj", 2)]
    );
}
