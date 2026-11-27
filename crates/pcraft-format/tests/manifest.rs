//! Integration tests of craft-pcraft-format: manifest serialization and tile references.
use craft_pcraft_format::{COMPOSITE, FORMAT_VERSION, FULL_COMPOSITE, Limits, Manifest, Step, THUMB, TileRef, blob_path, tile_path};

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
fn written_manifests_read_back_equal() {
    for (text, previews) in [(WITH_PREVIEWS, true), (WITHOUT_PREVIEWS, false)] {
        let read: Manifest<Doc> = read(text).unwrap();
        assert_eq!(read, manifest(previews));
    }
}

#[test]
fn an_optional_full_composite_round_trips_without_changing_older_manifests() {
    let mut manifest = manifest(true);
    assert_eq!(manifest.full_composite, None);
    manifest.full_composite = Some(FULL_COMPOSITE.into());
    let text = serde_json::to_string_pretty(&manifest).unwrap();
    assert!(text.ends_with("\"full_composite\": \"composite/full.png\"\n}"));
    assert_eq!(read::<Doc>(&text).unwrap(), manifest);
    manifest.full_composite = None;
    assert_eq!(serde_json::to_string_pretty(&manifest).unwrap(), WITH_PREVIEWS);
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
