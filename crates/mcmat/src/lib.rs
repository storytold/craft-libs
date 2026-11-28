#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! The clean-room MCMAT v3 source archive. Applications interpret the SBS entry graph.

use craft_zip::{Archive, Compression, Writer};
use serde::Deserializer;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: u32 = 3;
pub const MANIFEST_ENTRY: &str = "material.json";
pub const SOURCE_ENTRY: &str = "source.sbs";
const MARKER: &[u8] = br#"{"version":3}"#;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_entries: u64,
    pub max_manifest_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_entries: 100_000, max_manifest_bytes: 64 << 20, max_entry_bytes: 256 << 20, max_total_bytes: 1 << 30 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid portable material: {0}")]
    Invalid(String),
    #[error("unsupported .mcmat version {0} (expected 3). Republish from the original authoring project")]
    Version(u32),
    #[error("portable material archive: {0}")]
    Archive(#[from] craft_zip::Error),
}

/// File sniffing also recognizes old manifests so the reader can give republishing guidance.
pub fn is_material(bytes: &[u8], max_entries: u64) -> bool {
    Archive::open(bytes, craft_zip::Limits { max_entries, max_total_bytes: 0 }).is_ok_and(|archive| archive.contains(MANIFEST_ENTRY))
}

fn valid_resource(name: &str) -> bool {
    name.len() <= 4096
        && name.starts_with("resources/")
        && !name.contains(['\\', ':'])
        && !name.chars().any(char::is_control)
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

#[derive(Debug)]
pub struct Material {
    pub source: Vec<u8>,
    /// Keys are paths relative to source.sbs, including the resources/ prefix.
    pub resources: BTreeMap<String, Vec<u8>>,
}

impl Material {
    pub fn open(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        let mut archive = Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: limits.max_total_bytes })?;
        if archive.has_duplicate_names() || archive.has_overlapping_files()? {
            return Err(Error::Invalid("duplicate or overlapping archive entries".into()));
        }
        let json = archive.read(MANIFEST_ENTRY, limits.max_manifest_bytes)?.ok_or_else(|| Error::Invalid("missing material.json".into()))?;
        struct Marker;
        impl<'de> Visitor<'de> for Marker {
            type Value = (u32, bool);
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a version object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut version = None;
                let mut extra = false;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "version" {
                        if version.is_some() {
                            return Err(serde::de::Error::duplicate_field("version"));
                        }
                        version = Some(map.next_value::<u32>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                        extra = true;
                    }
                }
                Ok((version.ok_or_else(|| serde::de::Error::missing_field("version"))?, extra))
            }
        }
        let mut deserializer = serde_json::Deserializer::from_slice(&json);
        let (version, extra) = deserializer.deserialize_map(Marker).map_err(|err| Error::Invalid(format!("material.json: {err}")))?;
        deserializer.end().map_err(|err| Error::Invalid(format!("material.json: {err}")))?;
        if version != VERSION {
            return Err(Error::Version(version));
        }
        if extra {
            return Err(Error::Invalid("material.json must contain only the version".into()));
        }
        let source = archive.read(SOURCE_ENTRY, limits.max_entry_bytes)?.ok_or_else(|| Error::Invalid("missing source.sbs".into()))?;
        let mut resources = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for index in 0..archive.len() {
            let name = archive.entry_name(index)?.ok_or_else(|| Error::Invalid("invalid entry name".into()))?;
            if name == SOURCE_ENTRY || name == MANIFEST_ENTRY {
                continue;
            }
            if name.strip_suffix('/').is_some_and(|dir| dir == "resources" || valid_resource(dir)) {
                if archive.entry_sizes(index)? != Some((0, 0)) {
                    return Err(Error::Invalid("directory entries must have no payload".into()));
                }
                continue;
            }
            if !valid_resource(&name) || !seen.insert(name.to_ascii_lowercase()) {
                return Err(Error::Invalid(format!("invalid or ambiguous resource path `{name}`")));
            }
            let data = archive.read_index(index, limits.max_entry_bytes)?.ok_or_else(|| Error::Invalid(format!("missing `{name}`")))?;
            resources.insert(name, data);
        }
        Ok(Self { source, resources })
    }
}

/// Write one SBS document with its referenced source assets.
pub fn write<'a>(source: &[u8], resources: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<Vec<u8>, Error> {
    let limits = Limits::default();
    let mut writer = Writer::new();
    let mut seen = BTreeSet::new();
    let mut total = (source.len() as u64).saturating_add(MARKER.len() as u64);
    if source.len() as u64 > limits.max_entry_bytes {
        return Err(Error::Invalid("source.sbs exceeds the entry limit".into()));
    }
    writer.add(MANIFEST_ENTRY, MARKER, Compression::Deflated)?;
    writer.add(SOURCE_ENTRY, source, Compression::Deflated)?;
    for (name, bytes) in resources {
        if !valid_resource(name) || !seen.insert(name.to_ascii_lowercase()) {
            return Err(Error::Invalid(format!("invalid or ambiguous resource path `{name}`")));
        }
        total = total.saturating_add(bytes.len() as u64);
        if bytes.len() as u64 > limits.max_entry_bytes || total > limits.max_total_bytes || seen.len() as u64 + 2 > limits.max_entries {
            return Err(Error::Invalid(format!("archive limits exceeded at `{name}`")));
        }
        let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
        let compression = match extension.as_deref() {
            Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "exr" | "woff" | "woff2") => Compression::Stored,
            _ => Compression::Deflated,
        };
        writer.add(name, bytes, compression)?;
    }
    Ok(writer.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_source_and_assets_round_trip_under_one_budget() {
        let bytes = write(b"<package/>", [("resources/oak.png", b"image".as_slice())]).unwrap();
        let pkg = Material::open(&bytes, Limits::default()).unwrap();
        assert_eq!(pkg.source, b"<package/>");
        assert_eq!(pkg.resources["resources/oak.png"], b"image");
        let mut archive = Archive::open(&bytes, craft_zip::Limits { max_entries: 3, max_total_bytes: 128 }).unwrap();
        assert_eq!(archive.read(MANIFEST_ENTRY, 32).unwrap().unwrap(), MARKER);
        assert_eq!(archive.is_stored(SOURCE_ENTRY), Some(false));
        assert_eq!(archive.is_stored("resources/oak.png"), Some(true));
        for limits in [
            Limits { max_entries: 2, ..Default::default() },
            Limits { max_entry_bytes: 2, ..Default::default() },
            Limits { max_total_bytes: 24, ..Default::default() },
        ] {
            assert!(Material::open(&bytes, limits).is_err());
        }
    }

    #[test]
    fn marker_requires_an_object_and_complete_input() {
        for marker in [b"[3]".as_slice(), b"[2]", b"null", b"3", br#"{"version":3} {}"#, br#"{"version":3,"version":3}"#] {
            let mut zip = Writer::new();
            zip.add(MANIFEST_ENTRY, marker, Compression::Stored).unwrap();
            zip.add(SOURCE_ENTRY, b"source", Compression::Stored).unwrap();
            assert!(matches!(Material::open(&zip.finish().unwrap(), Limits::default()), Err(Error::Invalid(_))));
        }
    }

    #[test]
    fn directory_members_must_be_empty() {
        for payload in [b"".as_slice(), b"hidden"] {
            let mut zip = Writer::new();
            zip.add(MANIFEST_ENTRY, MARKER, Compression::Stored).unwrap();
            zip.add(SOURCE_ENTRY, b"source", Compression::Stored).unwrap();
            zip.add("resources/", payload, Compression::Stored).unwrap();
            assert_eq!(Material::open(&zip.finish().unwrap(), Limits::default()).is_ok(), payload.is_empty());
        }
    }

    #[test]
    fn legacy_encoded_resource_names_read_by_index_under_one_budget() {
        let mut zip = Writer::new();
        zip.add(MANIFEST_ENTRY, MARKER, Compression::Stored).unwrap();
        zip.add(SOURCE_ENTRY, b"s", Compression::Stored).unwrap();
        zip.add("resources/a.bin", b"data", Compression::Stored).unwrap();
        let mut bytes = zip.finish().unwrap();
        // Same-length CP437 name in both headers: no UTF-8 flag, 0x82 decodes to é.
        let offsets: Vec<_> =
            bytes.windows(b"resources/a.bin".len()).enumerate().filter_map(|(i, n)| (n == b"resources/a.bin").then_some(i + 10)).collect();
        assert_eq!(offsets.len(), 2);
        for offset in offsets {
            bytes[offset] = 0x82;
        }
        let total = MARKER.len() as u64 + 1 + 4;
        let limits = Limits { max_total_bytes: total, ..Default::default() };
        let material = Material::open(&bytes, limits).unwrap();
        assert_eq!(material.resources["resources/é.bin"], b"data");
        assert!(Material::open(&bytes, Limits { max_total_bytes: total - 1, ..limits }).is_err());
    }

    #[test]
    fn compresses_unencoded_source_assets() {
        let bytes = write(b"<package/>", [("resources/shape.svg", b"<svg/>".as_slice()), ("resources/font.ttf", b"font".as_slice())]).unwrap();
        let mut archive = Archive::open(&bytes, craft_zip::Limits { max_entries: 4, max_total_bytes: 128 }).unwrap();
        assert_eq!(archive.is_stored("resources/shape.svg"), Some(false));
        assert_eq!(archive.is_stored("resources/font.ttf"), Some(false));
    }

    #[test]
    fn rejects_previous_versions_before_requiring_the_v3_layout() {
        for version in [0, 1, 2, 4] {
            let mut zip = Writer::new();
            zip.add(MANIFEST_ENTRY, format!(r#"{{"format":"craft-material","version":{version}}}"#).as_bytes(), Compression::Deflated).unwrap();
            let bytes = zip.finish().unwrap();
            assert!(is_material(&bytes, 2));
            assert!(matches!(Material::open(&bytes, Limits::default()), Err(Error::Version(v)) if v == version));
        }
    }

    #[test]
    fn v3_requires_source_and_a_version_only_marker() {
        for (marker, source) in [(MARKER, None), (br#"{"version":3,"graph":"Other"}"#.as_slice(), Some(b"<package/>".as_slice()))] {
            let mut zip = Writer::new();
            zip.add(MANIFEST_ENTRY, marker, Compression::Deflated).unwrap();
            if let Some(source) = source {
                zip.add(SOURCE_ENTRY, source, Compression::Deflated).unwrap();
            }
            assert!(Material::open(&zip.finish().unwrap(), Limits::default()).is_err());
        }
    }

    #[test]
    fn rejects_ambiguous_or_nonlocal_resource_members() {
        for name in ["resources/../../x", "resources/a\\x", "/x", "maps/color.png", "resources/C:/x", "preview.png"] {
            assert!(write(b"x", [(name, &b"x"[..])]).is_err());
            let placeholder = "x".repeat(name.len());
            let bytes = {
                let mut zip = Writer::new();
                zip.add(MANIFEST_ENTRY, MARKER, Compression::Stored).unwrap();
                zip.add(SOURCE_ENTRY, b"x", Compression::Deflated).unwrap();
                match zip.add(name, b"x", Compression::Stored) {
                    Ok(()) => zip.finish().unwrap(),
                    Err(_) => {
                        let mut zip = Writer::new();
                        zip.add(MANIFEST_ENTRY, MARKER, Compression::Stored).unwrap();
                        zip.add(SOURCE_ENTRY, b"x", Compression::Deflated).unwrap();
                        zip.add(&placeholder, b"x", Compression::Stored).unwrap();
                        let mut b = zip.finish().unwrap();
                        let p = placeholder.as_bytes();
                        let n = name.as_bytes();
                        for i in 0..b.len().saturating_sub(p.len()) {
                            if &b[i..i + p.len()] == p {
                                b[i..i + p.len()].copy_from_slice(n);
                            }
                        }
                        b
                    }
                }
            };
            assert!(Material::open(&bytes, Limits::default()).is_err());
        }
        assert!(write(b"x", [("resources/A", &b"a"[..]), ("resources/a", &b"b"[..])]).is_err());
    }
}
