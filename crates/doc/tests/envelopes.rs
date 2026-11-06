//! Documents whose envelope is written with the format crates' own types read through craft-doc,
//! and envelopes those crates refuse are refused here too.

mod common;

use std::collections::BTreeMap;

use common::*;
use craft_doc::{Error, Kind, Limits, RenditionKind, info, outline, renditions, sniff};

fn limits() -> Limits {
    Limits::default()
}

fn vectorcraft_document() -> serde_json::Value {
    serde_json::json!({
        "artboards": [{"id": 1, "name": "Front", "rect": {"x0": 0.0, "y0": 0.0, "x1": 612.0, "y1": 792.0}}],
        "layers": [{"id": 1, "name": "Art", "kind": {"type": "layer", "children": []}}],
    })
}

#[test]
fn a_vectorcraft_file_written_with_the_envelope_types_reads() {
    let preview = png(16, 16, 3);
    let pdf: Vec<u8> = PDF.iter().copied().cycle().take(craft_vectorcraft_format::INLINE_MAX + 1).collect();
    let mut tail = craft_vectorcraft_format::Tail::new(craft_vectorcraft_format::VERSION);
    let mut file = craft_vectorcraft_format::File::new(vectorcraft_document());
    file.generator = Some("VectorCraft 0.1.0".into());
    file.preview = Some(tail.image("image/png", &preview, false));
    file.pdf = Some(tail.image("application/pdf", &pdf, false));
    assert!(file.pdf.as_ref().unwrap().blob.is_some() && file.preview.as_ref().unwrap().blob.is_none());
    let mut bytes = serde_json::to_vec(&file).unwrap();
    tail.write_to(&mut bytes).unwrap();

    for bytes in [bytes.clone(), gzip(&bytes)] {
        assert_eq!(sniff(&bytes), Some(Kind::VectorCraft));
        let i = info(&bytes, &limits()).unwrap();
        assert_eq!((i.version, i.generator.as_deref()), (craft_vectorcraft_format::VERSION, Some("VectorCraft 0.1.0")));
        let r = renditions(&bytes, &limits()).unwrap();
        assert_eq!((r[0].kind, r[0].bytes.as_slice()), (RenditionKind::Thumbnail, preview.as_slice()));
        assert_eq!((r[1].kind, r[1].bytes.as_slice()), (RenditionKind::Pdf, pdf.as_slice()));
        let o = outline(&bytes, &limits()).unwrap();
        assert_eq!((o.pages[0].name.as_str(), o.pages[0].width, o.layers[0].name.as_str()), ("Front", 612.0, "Art"));
    }
}

#[test]
fn a_pcraft_manifest_written_with_the_envelope_type_reads() {
    let document = serde_json::json!({
        "size": {"width": 64, "height": 32}, "resolution_dpi": 72.0, "mode": "Rgb", "depth": "U8",
        "layers": [{"name": "Background", "visible": true, "blend": "Normal", "opacity": 1.0, "content": {"kind": "raster"}}],
    });
    let mut manifest = craft_pcraft_format::Manifest::new("photocraft-format 0.1.0", document);
    manifest.thumbnail = Some(craft_pcraft_format::THUMB.into());
    let json = serde_json::to_vec(&manifest).unwrap();
    let thumb = png(4, 4, 1);
    let bundle = zip(&[store(craft_pcraft_format::MANIFEST, &json), store(craft_pcraft_format::THUMB, &thumb)]);

    assert_eq!(sniff(&bundle), Some(Kind::PhotoCraft));
    assert_eq!(info(&bundle, &limits()).unwrap().generator.as_deref(), Some("photocraft-format 0.1.0"));
    let r = renditions(&bundle, &limits()).unwrap();
    assert_eq!((r.len(), r[0].kind, r[0].bytes.as_slice()), (1, RenditionKind::Thumbnail, thumb.as_slice()));
    let o = outline(&bundle, &limits()).unwrap();
    assert_eq!((o.canvas.unwrap().width, o.layers[0].name.as_str()), (64, "Background"));
}

#[test]
fn an_ecproj_file_written_with_the_envelope_type_reads() {
    let view = BTreeMap::from([("activeComp", 1)]);
    let project = serde_json::json!({
        "schema": craft_ecproj_format::SCHEMA_VERSION,
        "items": {"1": {"id": 1, "name": "Main", "kind": {"type": "Comp", "width": 1280, "height": 720, "layers": []}}},
    });
    let text = craft_ecproj_format::File::new("0.12.0", &view, &project).to_json().unwrap();

    assert_eq!(sniff(text.as_bytes()), Some(Kind::EffectCraft));
    let i = info(text.as_bytes(), &limits()).unwrap();
    assert_eq!((i.version, i.generator.as_deref()), (craft_ecproj_format::SCHEMA_VERSION, Some("0.12.0")));
    let o = outline(text.as_bytes(), &limits()).unwrap();
    assert_eq!((o.pages[0].name.as_str(), o.pages[0].width), ("Main", 1280.0));
}

#[test]
fn a_pcraft_manifest_without_a_generator_is_malformed() {
    let manifest = pcraft_manifest(1, "[]", "").replacen(r#""generator":"photocraft-format 0.1.0","#, "", 1);
    let bundle = zip(&[store("manifest.json", manifest.as_bytes())]);
    assert_eq!(info(&bundle, &limits()).unwrap().generator, None);
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
    assert!(matches!(outline(&bundle, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn a_pcraft_preview_naming_another_entry_is_malformed() {
    let manifest = pcraft_manifest(1, "[]", r#","thumbnail":"../thumb.png""#);
    let bundle = zip(&[store("manifest.json", manifest.as_bytes()), store("../thumb.png", &png(1, 1, 0))]);
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
    assert!(matches!(outline(&bundle, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn an_ecproj_nested_to_max_depth_reads() {
    let deep = |levels: usize| format!(r#"{{"savedBy":"0.12.0","schema":2,"items":{{}},"x":{}{}}}"#, "[".repeat(levels), "]".repeat(levels));
    let levels = craft_doc::MAX_DEPTH as usize - 1;
    let objects = format!(r#"{{"savedBy":"0.12.0","schema":2,"items":{{}},"x":{}1{}}}"#, r#"{"a":"#.repeat(levels), "}".repeat(levels));
    for ok in [deep(levels), objects] {
        assert_eq!(info(ok.as_bytes(), &limits()).unwrap().version, 2);
        assert_eq!(renditions(ok.as_bytes(), &limits()).unwrap(), vec![]);
        assert_eq!(outline(ok.as_bytes(), &limits()).unwrap().pages.len(), 0);
    }
}

#[test]
fn a_pcraft_manifest_with_a_repeated_member_or_a_late_newer_version_is_refused() {
    let repeated = pcraft_manifest(1, "[]", r#","thumbnail":"thumb.png","thumbnail":"thumb.png""#);
    let bundle = zip(&[store("manifest.json", repeated.as_bytes())]);
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
    let late = r#"{"generator":"photocraft-format 9","document":"elsewhere","thumbnail":7,"format_version":2}"#;
    let bundle = zip(&[store("manifest.json", late.as_bytes())]);
    let refused = Error::TooNew { kind: Kind::PhotoCraft, version: 2, supported: Kind::PhotoCraft.supported_version() };
    assert_eq!(renditions(&bundle, &limits()), Err(refused.clone()));
    assert_eq!(outline(&bundle, &limits()), Err(refused));
}

#[test]
fn a_pcraft_manifest_that_is_a_json_array_is_malformed() {
    // The members of a manifest in field order; read_manifest reads only a JSON object.
    let array = r#"[1,"photocraft-format 0.1.0",{"size":{"width":1,"height":1},"resolution_dpi":72.0,"mode":"Rgb","depth":"U8","layers":[]},"thumb.png",null]"#;
    let format_limits = craft_pcraft_format::Limits::default();
    assert!(craft_pcraft_format::read_manifest::<serde_json::Value, craft_pcraft_format::Error>(array.as_bytes(), &format_limits, &[], 1).is_err());
    let bundle = zip(&[store("manifest.json", array.as_bytes()), store("thumb.png", &png(1, 1, 0))]);
    assert!(matches!(info(&bundle, &limits()), Err(Error::Malformed(_))));
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
    assert!(matches!(outline(&bundle, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn a_newer_file_with_another_format_s_version_member_is_too_new() {
    // `version` and `schema` are not PhotoCraft's version member, nor `schema` VectorCraft's.
    let manifest = r#"{"format_version":2,"version":1,"schema":1,"generator":"photocraft-format 9","document":"elsewhere"}"#;
    let pc = zip(&[store("manifest.json", manifest.as_bytes())]);
    // A repeated `format` makes the stamp unreadable; the version still decides.
    let vc = vectorcraft_json(5, r#""schema":1,"format":"vectorcraft""#);
    for (bytes, kind, version) in [(pc, Kind::PhotoCraft, 2), (vc.into_bytes(), Kind::VectorCraft, 5)] {
        let refused = Error::TooNew { kind, version, supported: kind.supported_version() };
        assert_eq!(renditions(&bytes, &limits()), Err(refused.clone()), "{kind:?}");
        assert_eq!(outline(&bytes, &limits()), Err(refused), "{kind:?}");
    }
    // Bytes after the JSON make a newer file malformed, not too new.
    let trailing = format!("{}x", vectorcraft_json(5, ""));
    assert!(matches!(renditions(trailing.as_bytes(), &limits()), Err(Error::Malformed(_))));
}

#[test]
fn a_generator_that_is_not_a_string_reads_as_none() {
    let vc = vectorcraft_json(4, "").replacen(r#""generator":"VectorCraft 0.1.0""#, r#""generator":{"name":"VectorCraft"}"#, 1);
    assert_eq!(info(vc.as_bytes(), &limits()).unwrap().generator, None);
    assert_eq!(outline(vc.as_bytes(), &limits()).unwrap().pages.len(), 2);
    let file = craft_vectorcraft_format::File::<serde_json::Value>::read(vc.as_bytes()).unwrap();
    assert_eq!(file.generator, None);
    // read_manifest needs a string, so the other calls refuse it; migrate reads it as none.
    let manifest = pcraft_manifest(1, "[]", "").replacen(r#""generator":"photocraft-format 0.1.0""#, r#""generator":7"#, 1);
    let bundle = zip(&[store("manifest.json", manifest.as_bytes())]);
    assert_eq!(info(&bundle, &limits()).unwrap().generator, None);
    assert!(matches!(renditions(&bundle, &limits()), Err(Error::Malformed(_))));
}

#[test]
fn a_repeated_vectorcraft_member_reads_as_its_last_value() {
    let (first, last) = (png(1, 1, 0), png(2, 2, 9));
    let members = format!(r#""generator":"VectorCraft 0.2.0","preview":{},"preview":{}"#, inline("image/png", &first), inline("image/png", &last));
    let vc = vectorcraft_json(4, &members);
    let file = craft_vectorcraft_format::File::<serde_json::Value>::read(vc.as_bytes()).unwrap();
    assert_eq!(file.preview.unwrap().bytes(&[]).unwrap(), last);
    let r = renditions(vc.as_bytes(), &limits()).unwrap();
    assert_eq!((r.len(), &r[0].bytes), (1, &last));
    assert_eq!(info(vc.as_bytes(), &limits()).unwrap().generator.as_deref(), Some("VectorCraft 0.2.0"));
    // A repeated version is refused, as File::read refuses it.
    let twice = vectorcraft_json(4, r#""version":4"#);
    assert!(craft_vectorcraft_format::File::<serde_json::Value>::read(twice.as_bytes()).is_err());
    assert!(matches!(info(twice.as_bytes(), &limits()), Err(Error::Malformed(_))));
    // A repeated version is malformed, not too new, when the second one is newer.
    let newer = vectorcraft_json(4, r#""version":9"#);
    assert!(craft_vectorcraft_format::File::<serde_json::Value>::read(newer.as_bytes()).is_err());
    assert!(matches!(renditions(newer.as_bytes(), &limits()), Err(Error::Malformed(_))));
    assert!(matches!(outline(newer.as_bytes(), &limits()), Err(Error::Malformed(_))));
}

#[test]
fn a_vectorcraft_file_without_one_document_gives_no_renditions() {
    let preview = vectorcraft_json(4, &format!(r#""preview":{}"#, inline("image/png", &png(1, 1, 0))));
    let missing = preview.replacen(r#""document":"#, r#""elsewhere":"#, 1);
    let repeated = preview.replacen(r#""images":{}"#, r#""document":{}"#, 1);
    for file in [missing, repeated] {
        assert!(craft_vectorcraft_format::File::<serde_json::Value>::read(file.as_bytes()).is_err());
        assert!(matches!(renditions(file.as_bytes(), &limits()), Err(Error::Malformed(_))), "{file}");
    }
    assert_eq!(renditions(preview.as_bytes(), &limits()).unwrap().len(), 1);
}

#[test]
fn a_vectorcraft_member_without_a_mime_type_is_skipped() {
    let file = vectorcraft_json(4, &format!(r#""preview":{{"data":"{}"}}"#, base64(&png(1, 1, 0))));
    assert!(renditions(file.as_bytes(), &limits()).unwrap().is_empty());
    let empty = vectorcraft_json(4, &format!(r#""preview":{},"pdf":{}"#, inline("", &png(1, 1, 0)), inline("", PDF)));
    assert!(renditions(empty.as_bytes(), &limits()).unwrap().is_empty());
}

#[test]
fn a_format_member_inside_another_member_is_not_vectorcraft() {
    let nested = br#"{"meta":{"format":"vectorcraft"},"version":4}"#;
    assert_eq!(sniff(nested), None);
    let second = br#"{"version":4,"format":"vectorcraft","document":{}}"#;
    assert_eq!(sniff(second), Some(Kind::VectorCraft));
}
