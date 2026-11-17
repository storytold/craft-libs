#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Bounded interchange for MaterialCraft portable materials (`.mcmat`).
//!
//! The manifest and its version belong to the format, while interpreting parameter values,
//! cooking maps and opening the source package belong to the caller. Unknown fields survive
//! deserialization and serialization, including fields added by later versions.

use std::collections::BTreeMap;

use craft_zip::{Archive, Compression, Writer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT: &str = "craft-material";
pub const MANIFEST_ENTRY: &str = "material.json";

/// A portable manifest; typed parameter values remain JSON so applications can adapt their own model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub graph: String,
    pub size: [u32; 2],
    pub maps: Vec<MapEntry>,
    #[serde(default)]
    pub parameters: Vec<Value>,
    #[serde(default)]
    pub inputs: Vec<Value>,
    #[serde(default)]
    pub presets: Vec<Value>,
    pub source: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    pub usage: String,
    pub file: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// All limits are supplied by the consumer, including the versions it understands.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min_version: u32,
    pub max_version: u32,
    pub max_entries: u64,
    pub max_maps: usize,
    pub max_manifest_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid portable material: {0}")]
    Invalid(String),
    #[error("unsupported portable material version {0}")]
    Version(u32),
    #[error("portable material limit: {0}")]
    Limit(String),
    #[error("portable material archive: {0}")]
    Archive(#[from] craft_zip::Error),
}

/// Whether a bounded ZIP index contains a material manifest. This does not validate the manifest.
pub fn is_material(bytes: &[u8], max_entries: u64) -> bool {
    Archive::open(bytes, craft_zip::Limits { max_entries, max_total_bytes: 0 })
        .is_ok_and(|archive| !archive.has_duplicate_names() && archive.contains(MANIFEST_ENTRY))
}

/// Manifest and original archive, retaining one shared expanded-byte budget across reads.
pub struct Material<'a> {
    pub manifest: Manifest,
    archive: Archive<'a>,
    max_entry_bytes: u64,
}

impl<'a> Material<'a> {
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Self, Error> {
        let mut archive = Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: limits.max_total_bytes })?;
        if archive.has_duplicate_names() {
            return Err(Error::Invalid("duplicate archive entries".into()));
        }
        let json = archive.read(MANIFEST_ENTRY, limits.max_manifest_bytes)?.ok_or_else(|| Error::Invalid("material.json is missing".into()))?;
        let manifest: Manifest = serde_json::from_slice(&json).map_err(|e| Error::Invalid(format!("material.json: {e}")))?;
        if manifest.format != FORMAT {
            return Err(Error::Invalid(format!("format `{}`", manifest.format)));
        }
        if !(limits.min_version..=limits.max_version).contains(&manifest.version) {
            return Err(Error::Version(manifest.version));
        }
        if manifest.maps.len() > limits.max_maps {
            return Err(Error::Limit(format!("{} maps exceeds {}", manifest.maps.len(), limits.max_maps)));
        }
        Ok(Self { manifest, archive, max_entry_bytes: limits.max_entry_bytes })
    }

    /// Read one named member, charging it to the same budget as previous reads.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, Error> {
        self.archive.read(name, self.max_entry_bytes)?.ok_or_else(|| Error::Invalid(format!("{name} is missing")))
    }

    pub fn read_source(&mut self) -> Result<Vec<u8>, Error> {
        self.read(&self.manifest.source.clone())
    }
}

/// Write a manifest, its named maps, and the source. Duplicate member names are rejected.
pub fn write(manifest: &Manifest, maps: &[(String, Vec<u8>)], source: &[u8]) -> Result<Vec<u8>, Error> {
    // Flattened fields must not serialize a second copy of a known key.
    const KNOWN: &[&str] = &["format", "version", "name", "graph", "size", "maps", "parameters", "inputs", "presets", "source"];
    if manifest.extra.keys().any(|key| KNOWN.contains(&key.as_str()))
        || manifest.maps.iter().any(|map| map.extra.contains_key("usage") || map.extra.contains_key("file"))
    {
        return Err(Error::Invalid("an extra field duplicates a known field".into()));
    }
    let json = serde_json::to_vec_pretty(manifest).map_err(|e| Error::Invalid(format!("material.json: {e}")))?;
    let mut writer = Writer::new();
    writer.add(MANIFEST_ENTRY, &json, Compression::Deflated)?;
    for (name, bytes) in maps {
        writer.add(name, bytes, Compression::Stored)?;
    }
    writer.add(&manifest.source, source, Compression::Stored)?;
    Ok(writer.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            min_version: 1,
            max_version: 2,
            max_entries: 100,
            max_maps: 4,
            max_manifest_bytes: 4096,
            max_entry_bytes: 100,
            max_total_bytes: 8192,
        }
    }

    fn sample(version: u32) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "format": "craft-material", "version": version, "name": "Stone", "graph": "Stone", "size": [64, 64],
            "maps": [{"usage": "baseColor", "file": "maps/base.png", "colorSpace": "srgb", "futureMap": true}],
            "parameters": [{"id": "rough", "value": {"t": "float", "v": 0.25}, "futureParam": 42}],
            "inputs": [{"identifier": "curvature", "kind": "gray"}], "presets": [],
            "source": if version == 1 { "source.mcraft" } else { "source.sbs" }, "futureField": {"keep": true}
        }))
        .unwrap()
    }

    #[test]
    fn both_protocol_versions_keep_unknown_fields_and_payloads() {
        for version in [1, 2] {
            let manifest = sample(version);
            let bytes = write(&manifest, &[("maps/base.png".into(), vec![1, 2, 3])], b"source").unwrap();
            assert!(is_material(&bytes, 100));
            assert!(!is_material(&bytes, 1));
            let mut material = Material::open(&bytes, limits()).unwrap();
            assert_eq!(material.manifest, manifest);
            assert_eq!(material.read("maps/base.png").unwrap(), [1, 2, 3]);
            assert_eq!(material.read_source().unwrap(), b"source");
            let json = serde_json::to_value(&material.manifest).unwrap();
            assert_eq!(json["futureField"]["keep"], true);
            assert_eq!(json["maps"][0]["futureMap"], true);
            assert_eq!(json["parameters"][0]["futureParam"], 42);
        }
    }

    #[test]
    fn limits_apply_to_the_manifest_maps_and_aggregate_reads() {
        let manifest = sample(2);
        let bytes = write(&manifest, &[("maps/base.png".into(), vec![1; 8])], &[2; 8]).unwrap();
        let mut unsupported = limits();
        unsupported.max_version = 1;
        assert!(matches!(Material::open(&bytes, unsupported), Err(Error::Version(2))));
        let mut no_maps = limits();
        no_maps.max_maps = 0;
        assert!(matches!(Material::open(&bytes, no_maps), Err(Error::Limit(_))));
        let mut tiny_entry = limits();
        tiny_entry.max_entry_bytes = 7;
        assert!(Material::open(&bytes, tiny_entry).unwrap().read("maps/base.png").is_err());
        let json_len = serde_json::to_vec_pretty(&manifest).unwrap().len() as u64;
        let mut aggregate = limits();
        aggregate.max_total_bytes = json_len + 15;
        let mut material = Material::open(&bytes, aggregate).unwrap();
        assert_eq!(material.read("maps/base.png").unwrap().len(), 8);
        assert!(material.read_source().is_err());
    }

    #[test]
    fn refuses_extra_fields_that_duplicate_known_keys() {
        let mut manifest = sample(2);
        manifest.extra.insert("version".into(), 3.into());
        assert!(write(&manifest, &[], b"source").is_err());
        manifest.extra.remove("version");
        manifest.maps[0].extra.insert("file".into(), "other".into());
        assert!(write(&manifest, &[], b"source").is_err());
    }

    #[test]
    fn refuses_duplicate_names_on_write() {
        let manifest = sample(2);
        let duplicate = [("maps/base.png".to_string(), vec![1]), ("maps/base.png".to_string(), vec![2])];
        assert!(write(&manifest, &duplicate, b"source").is_err());
    }
}
