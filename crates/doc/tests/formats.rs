//! Each format's documents, built from scratch: what `sniff`, `info`, `renditions` and `outline`
//! return for them.

mod common;

use common::*;
use craft_doc::{Error, Kind, Limits, RenditionKind, info, outline, renditions, sniff};

fn limits() -> Limits {
    Limits::default()
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
        ("gzip of other JSON", &gzip(br#"{"format":"svg"}"#)),
        ("PK alone", b"PK\x03\x04"),
    ] {
        assert_eq!(sniff(bytes), None, "{what}");
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

#[test]
fn pcraft_previews_stored_and_deflated_with_the_manifest_last() {
    let (thumb, composite) = (png(4, 3, 10), png(40, 30, 20));
    let r = renditions(&pcraft(&thumb, &composite), &limits()).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].kind, r[0].mime.as_str(), &r[0].bytes), (RenditionKind::Thumbnail, "image/png", &thumb));
    assert_eq!((r[1].kind, r[1].mime.as_str(), &r[1].bytes), (RenditionKind::Composite, "image/png", &composite));
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
fn pcraft_groups_nest_to_the_depth_cap() {
    // 3 levels to the layer list, 3 per group and 4 for the leaf: 397 of the 400 levels.
    let mut layer = pcraft_layer("Leaf", true, "Normal", 1.0, raster());
    for i in 0..130 {
        layer = pcraft_layer(&format!("G{i}"), true, "Normal", 1.0, &format!(r#"{{"kind":"group","expanded":false,"children":[{layer}]}}"#));
    }
    let pc = zip(&[deflated("manifest.json", pcraft_manifest(1, &format!("[{layer}]"), "").as_bytes())]);
    let o = outline(&pc, &limits()).unwrap();
    let mut depth = 0;
    let mut l = &o.layers[0];
    while let Some(c) = l.children.first() {
        depth += 1;
        l = c;
    }
    assert_eq!((depth, l.name.as_str()), (130, "Leaf"));
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
fn kinds_name_their_extension_and_version() {
    assert_eq!(
        [Kind::PhotoCraft, Kind::VectorCraft, Kind::EffectCraft].map(|k| (k.extension(), k.supported_version())),
        [("pcraft", 1), ("vectorcraft", 4), ("ecproj", 2)]
    );
}
