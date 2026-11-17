use craft_vectorcraft_format::stream::{Envelope, Indent, KeyScan};
use std::io::{self, Write};

#[test]
fn envelope_streams_nested_json_and_escapes_keys() {
    for pretty in [false, true] {
        let mut bytes = Vec::new();
        let mut env = Envelope::new(pretty);
        env.field(&mut bytes, "quoted\"key", &vec![1, 2]).unwrap();
        env.key(&mut bytes, "document").unwrap();
        let mut output = Indent { inner: &mut bytes, on: pretty };
        if pretty {
            serde_json::to_writer_pretty(&mut output, &serde_json::json!({"nested": [3,4]})).unwrap();
        } else {
            serde_json::to_writer(&mut output, &serde_json::json!({"nested": [3,4]})).unwrap();
        }
        env.finish(&mut bytes).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value, serde_json::json!({"quoted\"key": [1, 2], "document": {"nested": [3,4]}}));
        let mut expected = Vec::new();
        let empty = Envelope::new(pretty);
        empty.finish(&mut expected).unwrap();
        assert_eq!(expected, b"{}");
    }
}

#[test]
fn scan_survives_every_chunk_boundary_and_forwards_data() {
    let json = br#"{"key":"one","nested":[{"key":"two \\\"quoted\\\""},{"key":42}],"other":"x"}"#;
    for chunk in 1..=json.len() {
        let mut scan = KeyScan::new(Vec::new());
        for part in json.chunks(chunk) {
            scan.write_all(part).unwrap();
        }
        assert_eq!(scan.inner, json);
        assert!(scan.keys.contains("one"));
        assert_eq!(scan.keys.len(), 2);
    }
}

struct Broken;
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("failure"))
    }
}
#[test]
fn forwards_writer_errors() {
    assert!(Envelope::new(false).field(&mut Broken, "x", &1).is_err());
    assert!(KeyScan::new(Broken).write_all(b"{}").is_err());
    assert!(Indent { inner: Broken, on: true }.write_all(b"x\ny").is_err());
}

#[test]
fn tail_calls_hook_in_order_and_propagates_errors() {
    use craft_vectorcraft_format::{BLOB_MAGIC, INLINE_MAX, Tail};
    let first = vec![1; INLINE_MAX];
    let second = vec![2; INLINE_MAX];
    let mut tail = Tail::new(4);
    tail.image("image/png", &first, false);
    tail.image("application/pdf", &second, false);
    let mut out = Vec::new();
    let mut seen = Vec::new();
    tail.write_to_with(&mut out, |_, mime| {
        seen.push(mime.to_owned());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, ["image/png", "application/pdf"]);
    assert_eq!(out, [BLOB_MAGIC, &first, &second].concat());
    assert!(tail.write_to_with(Vec::new(), |_, _| Err(io::Error::other("failure"))).is_err());
    let mut called = false;
    Tail::new(4)
        .write_to_with(Vec::new(), |_, _| {
            called = true;
            Ok(())
        })
        .unwrap();
    assert!(!called);
}

#[test]
fn native_schema_order_and_versions() {
    use craft_vectorcraft_format::stream::{FileHeader, write_file_json};
    let profiles = Default::default();
    for (version, name) in [(1, "drawcraft"), (4, "vectorcraft")] {
        for pretty in [false, true] {
            let header = FileHeader { version, generator: "test", pdf_compatible: Some(false), preview: None, profiles: &profiles, pdf: None };
            let mut out = Vec::new();
            write_file_json(&mut out, &header, pretty, |w| {
                w.write_all(if pretty { b"{\n  \"key\": \"image\"\n}" } else { b"{\"key\":\"image\"}" })?;
                Ok(Default::default())
            })
            .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(value["format"], name);
            assert_eq!(value["pdfCompatible"], false);
            let text = std::str::from_utf8(&out).unwrap();
            assert!(text.find("\"format\"").unwrap() < text.find("\"version\"").unwrap());
            assert!(text.find("\"document\"").unwrap() < text.find("\"images\"").unwrap());
        }
    }
    let header = FileHeader { version: 0, generator: "test", pdf_compatible: None, preview: None, profiles: &profiles, pdf: None };
    assert!(write_file_json(Vec::new(), &header, false, |_| Ok(Default::default())).is_err());
}

// Oracle for the previous app writer: each member was first serialized to a Vec, then
// indented and appended. Compare decoded file bytes, not backend-dependent gzip bytes.
#[test]
fn streamed_native_file_matches_previous_framing() {
    use craft_vectorcraft_format::{
        Tail,
        stream::{FileHeader, write_file_json},
    };
    use flate2::{Compression, read::GzDecoder, write::GzEncoder};
    use std::collections::BTreeMap;
    use std::io::Read;

    fn old_field(out: &mut Vec<u8>, first: &mut bool, pretty: bool, key: &str, value: &impl serde::Serialize) {
        out.extend_from_slice(if std::mem::take(first) { b"{" } else { b"," });
        if pretty {
            out.extend_from_slice(b"\n  ");
        }
        out.extend_from_slice(format!("\"{key}\"{}", if pretty { ": " } else { ":" }).as_bytes());
        let json = if pretty { serde_json::to_vec_pretty(value) } else { serde_json::to_vec(value) }.unwrap();
        let mut indent = Indent { inner: out, on: pretty };
        indent.write_all(&json).unwrap();
    }

    let payload = vec![42; craft_vectorcraft_format::INLINE_MAX];
    for version in [1, 3, 4] {
        for pretty in [false, true] {
            let mut tail = Tail::new(version);
            let image = tail.image("image/png", &payload, false);
            let profile = tail.image("application/vnd.iccprofile", &payload, false);
            let pdf = tail.image("application/pdf", &payload, false);
            let profiles = BTreeMap::from([("profile", profile)]);
            let images = BTreeMap::from([("image", image)]);
            let document = serde_json::json!({"layers": [{"key": "image", "name": "A"}]});
            let header = FileHeader {
                version,
                generator: "VectorCraft test",
                pdf_compatible: Some(true),
                preview: None,
                profiles: &profiles,
                pdf: Some(&pdf),
            };
            let mut actual = Vec::new();
            write_file_json(&mut actual, &header, pretty, |w| {
                if pretty { serde_json::to_writer_pretty(w, &document) } else { serde_json::to_writer(w, &document) }.map_err(io::Error::other)?;
                Ok(images.clone())
            })
            .unwrap();
            tail.write_to(&mut actual).unwrap();

            let mut old = Vec::new();
            let mut first = true;
            let format = if version < 3 { "drawcraft" } else { "vectorcraft" };
            old_field(&mut old, &mut first, pretty, "format", &format);
            old_field(&mut old, &mut first, pretty, "version", &version);
            old_field(&mut old, &mut first, pretty, "generator", &"VectorCraft test");
            old_field(&mut old, &mut first, pretty, "pdfCompatible", &true);
            old_field(&mut old, &mut first, pretty, "profiles", &profiles);
            old_field(&mut old, &mut first, pretty, "pdf", &pdf);
            old_field(&mut old, &mut first, pretty, "document", &document);
            old_field(&mut old, &mut first, pretty, "images", &images);
            old.extend_from_slice(if pretty { b"\n}" } else { b"}" });
            tail.write_to(&mut old).unwrap();
            assert_eq!(actual, old, "version {version}, pretty {pretty}");
            for bytes in [&old, &actual] {
                let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
                gzip.write_all(bytes).unwrap();
                let mut decoded = Vec::new();
                GzDecoder::new(gzip.finish().unwrap().as_slice()).read_to_end(&mut decoded).unwrap();
                assert_eq!(decoded, old);
            }
        }
    }
}
