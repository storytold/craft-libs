use craft_pcraft_format::{
    BLOBS_DIR, COMPOSITE, COMPOSITE_DIR, EXTENSION, FORMAT_VERSION, Limits, MANIFEST, Manifest, OBJECT_SUFFIX, SAVE_LOCK, Step, THUMB, TILES_DIR,
    TileRef, blob_path, is_valid_hash, tile_path,
};
use serde::{Deserialize, Serialize};

fn read<D: serde::de::DeserializeOwned>(text: &str) -> Result<Manifest<D>, craft_pcraft_format::Error> {
    let none: &[Step] = &[];
    craft_pcraft_format::read_manifest(text.as_bytes(), &Limits::default(), none, FORMAT_VERSION)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Doc {
    id: u64,
    name: String,
    tiles: Vec<TileRef>,
}

const HASH: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn doc() -> Doc {
    Doc { id: 7, name: "Untitled-1".into(), tiles: vec![TileRef::new(0, -1, HASH.into())] }
}

fn manifest(previews: bool) -> Manifest<Doc> {
    let mut manifest = Manifest::new("photocraft-format 0.1.0", doc());
    manifest.thumbnail = previews.then(|| THUMB.to_owned());
    manifest.composite = previews.then(|| COMPOSITE.to_owned());
    manifest
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
        let read: Manifest<Doc> = read(text).unwrap();
        assert_eq!(read, manifest(previews));
    }
}

#[test]
fn a_new_manifest_is_at_the_current_version_without_previews() {
    let m = Manifest::new(String::from("g"), 5);
    assert_eq!((m.format_version, m.generator.as_str(), m.document, m.thumbnail, m.composite), (FORMAT_VERSION, "g", 5, None, None));
}

#[test]
fn missing_preview_members_read_as_none_and_unknown_members_are_skipped() {
    let text =
        r#"{"format_version":1,"generator":"g","document":{"id":1,"name":"a","tiles":[]},"full_composite":"composite/full.png","later":{"x":[1,2]}}"#;
    let read: Manifest<Doc> = read(text).unwrap();
    assert_eq!(read.thumbnail, None);
    assert_eq!(read.composite, None);
    assert_eq!(read.document.name, "a");
}

#[test]
fn a_manifest_without_a_document_or_generator_does_not_read() {
    assert!(read::<Doc>(r#"{"format_version":1,"generator":"g"}"#).is_err());
    assert!(read::<Doc>(r#"{"format_version":1,"document":{"id":1,"name":"a","tiles":[]}}"#).is_err());
}

#[test]
fn the_document_can_be_any_json_value() {
    let read: Manifest<serde_json::Value> = read(WITH_PREVIEWS).unwrap();
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
    assert_eq!((TILES_DIR, BLOBS_DIR, COMPOSITE_DIR, OBJECT_SUFFIX), ("tiles", "blobs", "composite", ".zst"));
    assert!(COMPOSITE.starts_with(&format!("{COMPOSITE_DIR}/")));
    assert_eq!(tile_path(HASH), Some(format!("tiles/{HASH}.zst")));
    assert_eq!(blob_path(HASH), Some(format!("blobs/{HASH}.zst")));
}

#[test]
fn no_entry_name_is_built_from_an_invalid_hash() {
    for hash in ["../../etc/passwd", "../x", "", &HASH.to_uppercase(), &HASH[1..]] {
        assert_eq!(tile_path(hash), None, "{hash}");
        assert_eq!(blob_path(hash), None, "{hash}");
    }
    let tile: TileRef = serde_json::from_str(r#"{"tx":0,"ty":0,"hash":"../x"}"#).unwrap();
    assert_eq!(tile.path(), None);
    let tile = TileRef::new(1, -1, HASH.into());
    assert_eq!(tile.path(), Some(format!("tiles/{HASH}.zst")));
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
