//! Integration tests of craft-pcraft-format: reading manifests from container files.
use craft_pcraft_format::{COMPOSITE, Error, FORMAT_VERSION, Limits, Manifest, Step, THUMB, TileRef, read_manifest};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Doc {
    name: String,
    #[serde(default)]
    tiles: Vec<TileRef>,
}

const NONE: &[Step] = &[];

fn read(json: &[u8]) -> Result<Manifest<Doc>, Error> {
    read_manifest(json, &Limits::default(), NONE, FORMAT_VERSION)
}

#[test]
fn a_newer_manifest_is_too_new_and_names_its_writer() {
    let json = br#"{"format_version":2,"generator":"photocraft-format 9.0.0","document":{"name":"a"}}"#;
    let Err(Error::TooNew { found, supported, generator, .. }) = read(json) else { panic!("not TooNew") };
    assert_eq!((found, supported), (2, 1));
    assert_eq!(generator.as_deref(), Some("photocraft-format 9.0.0"));
    let message = read(json).err().map(|e| e.to_string());
    assert_eq!(message.as_deref(), Some("bundle written by a newer version (format 2, this build reads up to 1)"));
}

#[test]
fn the_steps_run_before_the_document_is_read() {
    fn rename(v: &mut Value) -> Result<(), Error> {
        v["document"]["name"] = v["document"]["title"].take();
        Ok(())
    }
    let steps: &[Step] = &[rename];
    let json = br#"{"format_version":1,"generator":"g","document":{"title":"old"}}"#;
    let m: Manifest<Doc> = read_manifest(json, &Limits::default(), steps, 2).unwrap();
    assert_eq!(m.format_version, 2);
    assert_eq!(m.document.name, "old");
}

#[test]
fn a_preview_other_than_its_entry_is_corrupt() {
    for (member, entry) in [
        ("thumbnail", "../x"),
        ("thumbnail", "/etc/passwd"),
        ("composite", "../x"),
        ("composite", "C:\\x.png"),
        ("thumbnail", "composite/preview.png"),
        ("composite", "thumb.png"),
    ] {
        let json = json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, member: entry});
        let e = read(json.to_string().as_bytes()).err();
        assert!(matches!(&e, Some(Error::Corrupt(_))), "{json}: {e:?}");
    }
    let json =
        json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, "thumbnail": "thumb.png", "composite": "composite/preview.png"});
    let m = read(json.to_string().as_bytes()).unwrap();
    assert_eq!((m.thumbnail.as_deref(), m.composite.as_deref()), (Some(THUMB), Some(COMPOSITE)));
}
