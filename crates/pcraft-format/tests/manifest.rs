use craft_pcraft_format::{COMPOSITE, EXTENSION, FORMAT_VERSION, MANIFEST, Manifest, SAVE_LOCK, THUMB, TileRef, blob_path, is_valid_hash, tile_path};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Doc {
    id: u64,
    name: String,
    tiles: Vec<TileRef>,
}

const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn doc() -> Doc {
    Doc { id: 7, name: "Untitled-1".into(), tiles: vec![TileRef { tx: 0, ty: -1, hash: HASH.into() }] }
}

fn manifest(previews: bool) -> Manifest<Doc> {
    Manifest {
        format_version: FORMAT_VERSION,
        generator: "photocraft-format 0.1.0".into(),
        document: doc(),
        thumbnail: previews.then(|| THUMB.to_owned()),
        composite: previews.then(|| COMPOSITE.to_owned()),
    }
}

const WITH_PREVIEWS: &str = r#"{
  "format_version": 1,
  "generator": "photocraft-format 0.1.0",
  "document": {
    "id": 7,
    "name": "Untitled-1",
    "tiles": [
      {
        "tx": 0,
        "ty": -1,
        "hash": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
      }
    ]
  },
  "thumbnail": "thumb.png",
  "composite": "composite/preview.png"
}"#;

const WITHOUT_PREVIEWS: &str = r#"{
  "format_version": 1,
  "generator": "photocraft-format 0.1.0",
  "document": {
    "id": 7,
    "name": "Untitled-1",
    "tiles": [
      {
        "tx": 0,
        "ty": -1,
        "hash": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
      }
    ]
  },
  "thumbnail": null,
  "composite": null
}"#;

#[test]
fn a_manifest_with_previews_writes_photocraft_s_pretty_json_byte_for_byte() {
    let bytes = serde_json::to_vec_pretty(&manifest(true)).unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), WITH_PREVIEWS);
}

#[test]
fn a_manifest_without_previews_writes_null_members() {
    let bytes = serde_json::to_vec_pretty(&manifest(false)).unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), WITHOUT_PREVIEWS);
}

#[test]
fn written_manifests_read_back_equal() {
    for (text, previews) in [(WITH_PREVIEWS, true), (WITHOUT_PREVIEWS, false)] {
        let read: Manifest<Doc> = serde_json::from_str(text).unwrap();
        assert_eq!(read, manifest(previews));
    }
}

#[test]
fn missing_preview_members_read_as_none_and_unknown_members_are_skipped() {
    let text =
        r#"{"format_version":1,"generator":"g","document":{"id":1,"name":"a","tiles":[]},"full_composite":"composite/full.png","later":{"x":[1,2]}}"#;
    let read: Manifest<Doc> = serde_json::from_str(text).unwrap();
    assert_eq!(read.thumbnail, None);
    assert_eq!(read.composite, None);
    assert_eq!(read.document.name, "a");
}

#[test]
fn a_manifest_without_a_document_or_generator_does_not_read() {
    assert!(serde_json::from_str::<Manifest<Doc>>(r#"{"format_version":1,"generator":"g"}"#).is_err());
    assert!(serde_json::from_str::<Manifest<Doc>>(r#"{"format_version":1,"document":{"id":1,"name":"a","tiles":[]}}"#).is_err());
}

#[test]
fn the_document_can_be_any_json_value() {
    let read: Manifest<serde_json::Value> = serde_json::from_str(WITH_PREVIEWS).unwrap();
    assert_eq!(read.document["tiles"][0]["ty"], -1);
    assert_eq!(read.composite.as_deref(), Some(COMPOSITE));
}

#[test]
fn entry_names_match_the_bundle_layout() {
    assert_eq!(EXTENSION, "pcraft");
    assert_eq!(MANIFEST, "manifest.json");
    assert_eq!(THUMB, "thumb.png");
    assert_eq!(COMPOSITE, "composite/preview.png");
    assert_eq!(SAVE_LOCK, "save.lock");
    assert_eq!(tile_path(HASH), format!("tiles/{HASH}.zst"));
    assert_eq!(blob_path(HASH), format!("blobs/{HASH}.zst"));
}

#[test]
fn only_64_lowercase_hex_digits_are_a_valid_hash() {
    assert!(is_valid_hash(HASH));
    assert!(is_valid_hash(&"0".repeat(64)));
    assert!(!is_valid_hash(&HASH.to_uppercase()));
    assert!(!is_valid_hash(&HASH[1..]));
    assert!(!is_valid_hash(&format!("{HASH}0")));
    assert!(!is_valid_hash(&format!("../{}", &HASH[3..])));
    assert!(!is_valid_hash(&format!("{}g", &HASH[1..])));
    assert!(!is_valid_hash(""));
}
