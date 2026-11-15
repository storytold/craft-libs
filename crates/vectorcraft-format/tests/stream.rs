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
