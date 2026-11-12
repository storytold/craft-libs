//! A caller-defined PhotoCraft document head, without reading image payloads.

mod common;

use common::*;
use craft_doc::{Error, Kind, Limits, outline, pcraft_document};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Head {
    mode: String,
    layers: Vec<Layer>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Layer {
    name: String,
    clipped: bool,
    fill_opacity: f32,
}

/// A document head whose optional previews and raster payload deliberately do not exist.
fn manifest() -> String {
    pcraft_manifest(
        1,
        r#"[{"name":"Paint","visible":true,"blend":"Normal","opacity":0.75,"clipped":true,"fill_opacity":0.5,"content":{"kind":"raster","surface":{"tiles":[{"hash":"missing"}]}}}]"#,
        r#", "thumbnail":"thumb.png", "composite":"composite/preview.png""#,
    )
}

/// Exercise both supported ZIP storage methods with the same manifest bytes.
fn bundle(json: &str, compressed: bool) -> Vec<u8> {
    zip(&[if compressed { deflated("manifest.json", json.as_bytes()) } else { store("manifest.json", json.as_bytes()) }])
}

#[test]
/// Caller-selected metadata must agree with the shared format reader without requiring pixels.
fn reads_the_callers_document_head_without_requiring_declared_payloads() {
    let json = manifest();
    for compressed in [false, true] {
        let bytes = bundle(&json, compressed);
        let head: Head = pcraft_document(&bytes, &Limits::default()).unwrap();
        assert_eq!(head, Head { mode: "Rgb".into(), layers: vec![Layer { name: "Paint".into(), clipped: true, fill_opacity: 0.5 }] });
        let existing = outline(&bytes, &Limits::default()).unwrap();
        assert_eq!(head.layers[0].name, existing.layers[0].name);
        assert_eq!(head.mode, existing.canvas.unwrap().mode);
        // The shared JSON reader uses the same caller-owned document schema.
        let shared: craft_pcraft_format::Manifest<Head> =
            craft_pcraft_format::read_manifest::<_, craft_pcraft_format::Error>(json.as_bytes(), &Default::default(), &[], 1).unwrap();
        assert_eq!(head, shared.document);
    }
}

#[test]
/// Unrequested image entries must not consume the manifest-only reader's byte budget.
fn skips_oversized_and_unreadable_unrequested_entries() {
    let json = manifest();
    let mut bytes =
        zip(&[store("tiles/missing.zst", b"not zstd"), store("composite/preview.png", b"not png"), store("manifest.json", json.as_bytes())]);
    let entries: Vec<_> = bytes.windows(4).enumerate().filter_map(|(i, w)| (w == b"PK\x01\x02").then_some(i)).take(2).collect();
    for at in entries {
        bytes[at + 10..at + 12].copy_from_slice(&99u16.to_le_bytes());
        bytes[at + 24..at + 28].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
    }
    let mut limits = Limits::default();
    limits.max_entry = json.len() as u64;
    limits.max_total = json.len() as u64;
    assert_eq!(pcraft_document::<Head>(&bytes, &limits).unwrap().layers.len(), 1);
    assert!(matches!(craft_doc::renditions(&bytes, &limits), Err(Error::Malformed(_))), "the declared thumbnail is absent");
}

#[test]
/// Envelope validity, supported versions and the caller's schema are separate requirements.
fn checks_the_manifest_and_the_callers_json_types() {
    let json = manifest();
    for broken in [
        "[]".into(),
        "{".into(),
        json.replacen("\"format_version\":1", "\"format_version\":0", 1),
        json.replacen("\"generator\":", "\"generator\":null,\"generator\":", 1),
        json.replace("thumb.png", "elsewhere.png"),
        json.replace("\"clipped\":true", "\"clipped\":42"),
        json.replace("\"fill_opacity\":0.5,", ""),
    ] {
        assert!(matches!(pcraft_document::<Head>(&bundle(&broken, false), &Limits::default()), Err(Error::Malformed(_))), "{broken}");
    }
    let newer = json.replacen("\"format_version\":1", "\"format_version\":2", 1);
    assert!(matches!(pcraft_document::<Head>(&bundle(&newer, false), &Limits::default()), Err(Error::TooNew { kind: Kind::PhotoCraft, .. })));
    assert_eq!(pcraft_document::<Head>(&bundle("{}", false), &Limits::default()), Err(Error::Unrecognized));
    assert_eq!(pcraft_document::<Head>(&zip(&[store("readme", b"none")]), &Limits::default()), Err(Error::Unrecognized));
    assert!(matches!(pcraft_document::<Head>(vectorcraft_json(4, "").as_bytes(), &Limits::default()), Err(Error::Unsupported(_))));
    assert_eq!(pcraft_document::<Head>(b"not a document", &Limits::default()), Err(Error::Unrecognized));
}

#[test]
/// Limits must reject the manifest before invoking potentially allocating caller code.
fn checks_byte_depth_and_item_limits_before_the_document_is_deserialized() {
    #[derive(Debug)]
    struct Never;
    impl<'de> Deserialize<'de> for Never {
        /// Fail if a limit check lets control reach the caller's deserializer.
        fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
            panic!("document deserialization must follow limit checks")
        }
    }
    let json = manifest();
    for compressed in [false, true] {
        let bytes = bundle(&json, compressed);
        for cap in ["JSON", "entry", "total", "depth", "items"] {
            let mut limits = Limits::default();
            match cap {
                "JSON" => limits.max_json = json.len() as u64 - 1,
                "entry" => limits.max_entry = json.len() as u64 - 1,
                "total" => limits.max_total = json.len() as u64 - 1,
                "depth" => limits.max_depth = 2,
                "items" => limits.max_items = 1,
                _ => unreachable!(),
            }
            assert!(matches!(pcraft_document::<Never>(&bytes, &limits), Err(Error::LimitExceeded(_))), "{limits:?}");
        }
    }
}

#[test]
/// Ignoring a field must not let an oversized manifest bypass the structural item cap.
fn the_item_budget_also_bounds_a_callers_vec_and_ignored_fields() {
    #[derive(Debug, Deserialize)]
    struct Small {
        #[serde(rename = "values")]
        _values: Vec<u32>,
    }
    #[derive(Debug, Deserialize)]
    struct Empty {}
    for field in ["values", "ignored"] {
        let json = format!(r#"{{"format_version":1,"generator":"test","document":{{"{field}":[0,1,2,3,4,5,6,7,8,9]}}}}"#);
        let bytes = bundle(&json, true);
        let mut limits = Limits::default();
        limits.max_items = 8;
        let result = match field {
            "values" => pcraft_document::<Small>(&bytes, &limits).map(|_| ()),
            _ => pcraft_document::<Empty>(&bytes, &limits).map(|_| ()),
        };
        assert!(matches!(result, Err(Error::LimitExceeded(_))), "{field}: {result:?}");
        assert!(pcraft_document::<Empty>(&bytes, &Limits::default()).is_ok());
    }
}
