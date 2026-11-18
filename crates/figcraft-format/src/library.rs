//! Wire schema and member limits for published libraries embedded in a native project archive.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MANIFEST: &str = "library/manifest.json";
pub const DOCUMENT: &str = "library/document.json";
pub const IMAGES_PREFIX: &str = "library/images/";
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_ID_CHARS: usize = 64;
pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Native library manifest, independent of the consumer's document model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub description: String,
    pub time: u64,
    pub components: Vec<Component>,
    pub styles: Vec<Style>,
    pub collections: Vec<Collection>,
    pub variables: Vec<Variable>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub set: bool,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub published: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u64,
    pub published: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub published: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Variable {
    pub id: String,
    pub name: String,
    pub collection: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u64,
    pub published: bool,
}

/// Decode the native published-library manifest.
pub fn read_manifest(bytes: &[u8]) -> Result<Manifest, serde_json::Error> {
    serde_json::from_slice(bytes)
}

/// Encode the native published-library manifest.
pub fn write_manifest(manifest: &Manifest) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(manifest)
}

/// Whether `id` is a valid stable library id in the native manifest.
pub fn is_valid_id(id: &str) -> bool {
    (1..=MAX_ID_CHARS).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Clip a library display string to its native-format scalar-value limit.
pub fn clip_display(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Write the complete published-library member group into a native archive. The consumer maps
/// its document to the neutral manifest and image index; path layout, compression, and shared
/// image reuse stay here.
pub fn write_members<'a, D: Serialize>(
    writer: &mut super::ArchiveWriter,
    manifest: &Manifest,
    document: &D,
    image_index: &[super::ImageIndexEntry],
    already_written: &BTreeSet<String>,
    images: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<(), super::ArchiveWriteError> {
    let manifest_bytes = write_manifest(manifest)?;
    writer.add(MANIFEST, &manifest_bytes, super::Compression::Level(3))?;
    writer.add_document_json(DOCUMENT, document, image_index)?;
    for (hash, bytes) in images {
        if already_written.contains(hash) || !valid_image_key(hash) {
            continue;
        }
        writer.add(&format!("{IMAGES_PREFIX}{hash}"), bytes, super::Compression::Stored)?;
    }
    Ok(())
}

/// Read only published-library image resources selected by the consumer's document model.
/// Missing and oversized images are omitted, leaving the consumer to render its established
/// placeholder. The shared layer handles path choice, safe member names, and byte accounting.
pub fn read_image_resources(
    archive: &mut impl super::NativeArchive,
    hashes: impl IntoIterator<Item = String>,
    already_available: &BTreeSet<String>,
    unpacked: &mut super::ReadBudget,
) -> BTreeMap<String, Vec<u8>> {
    let mut remaining = MAX_IMAGE_BYTES;
    let mut images = BTreeMap::new();
    for hash in hashes {
        if already_available.contains(&hash) || !valid_image_key(&hash) {
            continue;
        }
        let preferred = format!("{IMAGES_PREFIX}{hash}");
        let fallback = format!("images/{hash}");
        let Some(name) = [preferred, fallback].into_iter().find(|name| archive.contains(name)) else { continue };
        let Ok(bytes) = unpacked.read(archive, &name, remaining.min(super::MAX_MEMBER)) else { continue };
        remaining = remaining.saturating_sub(bytes.len() as u64);
        images.insert(hash, bytes);
    }
    images
}

fn valid_image_key(key: &str) -> bool {
    (1..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_wire_names_defaults_and_round_trip_match_native_contract() {
        let json = r#"{"id":"lib-1","components":[{"id":12,"set":true}],"styles":[{"id":"s","type":"paint"}],"collections":[{"id":"c"}],"variables":[{"id":"v","collection":"c","type":"color"}]}"#;
        let manifest = read_manifest(json.as_bytes()).unwrap();
        assert_eq!(manifest.components[0].id, 12);
        assert_eq!(manifest.styles[0].kind, "paint");
        assert_eq!(manifest.variables[0].kind, "color");
        let encoded: serde_json::Value = serde_json::from_slice(&write_manifest(&manifest).unwrap()).unwrap();
        assert_eq!(encoded["styles"][0]["type"], "paint");
        assert!(encoded["styles"][0].get("kind").is_none());
        assert_eq!(encoded["variables"][0]["type"], "color");
    }

    #[test]
    fn library_id_and_display_limits_are_format_level() {
        assert!(is_valid_id(&"a".repeat(MAX_ID_CHARS)));
        assert!(!is_valid_id(&"a".repeat(MAX_ID_CHARS + 1)));
        assert!(!is_valid_id("../bad"));
        assert_eq!(clip_display("aé🙂b", 2), "aé");
    }

    #[test]
    fn library_member_writer_and_reader_own_resource_layout_and_budgets() {
        let mut writer = super::super::ArchiveWriter::new();
        let manifest = Manifest { id: "library-1".into(), ..Manifest::default() };
        let already_written = BTreeSet::from(["existing".to_string()]);
        write_members(
            &mut writer,
            &manifest,
            &serde_json::json!({"name":"library snapshot"}),
            &[],
            &already_written,
            [("stored", &b"library bytes"[..]), ("existing", &b"duplicate"[..]), ("../bad", &b"hostile"[..])],
        )
        .unwrap();
        writer.add("images/fallback", b"fallback bytes", super::super::Compression::Stored).unwrap();
        let bytes = writer.finish().unwrap();
        let mut archive = super::super::Archive::open(&bytes, super::super::ArchiveLimits { max_entries: 8, max_total_bytes: 1024 }).unwrap();
        assert!(archive.contains(MANIFEST));
        assert!(archive.contains(DOCUMENT));
        let manifest_bytes = archive.read(MANIFEST, MAX_MANIFEST_BYTES).unwrap().unwrap();
        assert_eq!(read_manifest(&manifest_bytes).unwrap(), manifest);
        assert!(archive.contains("library/images/stored"));
        assert!(!archive.contains("library/images/existing"));
        assert!(!archive.contains("library/images/../bad"));

        let available = BTreeSet::from(["existing".to_string()]);
        let mut budget = super::super::ReadBudget::new(1024);
        let images =
            read_image_resources(&mut archive, ["stored".into(), "existing".into(), "fallback".into(), "../bad".into()], &available, &mut budget);
        assert_eq!(images["stored"], b"library bytes");
        assert_eq!(images["fallback"], b"fallback bytes");
        assert!(!images.contains_key("existing"));
        assert!(!images.contains_key("../bad"));
    }
}
