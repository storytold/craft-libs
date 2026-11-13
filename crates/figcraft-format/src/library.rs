//! Wire schema and member limits for published libraries embedded in a native project archive.
use serde::{Deserialize, Serialize};

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

/// Whether `id` is a valid stable library id in the native manifest.
pub fn is_valid_id(id: &str) -> bool {
    (1..=MAX_ID_CHARS).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Clip a library display string to its native-format scalar-value limit.
pub fn clip_display(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_wire_names_defaults_and_round_trip_match_native_contract() {
        let json = r#"{"id":"lib-1","components":[{"id":12,"set":true}],"styles":[{"id":"s","type":"paint"}],"collections":[{"id":"c"}],"variables":[{"id":"v","collection":"c","type":"color"}]}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.components[0].id, 12);
        assert_eq!(manifest.styles[0].kind, "paint");
        assert_eq!(manifest.variables[0].kind, "color");
        let encoded = serde_json::to_value(manifest).unwrap();
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
}
