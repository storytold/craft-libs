#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The neutral JSON header and version contract for FigCraft's native project format.
//! App-specific document fields remain a generic flattened payload.
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EXTENSION: &str = "figcraft";
pub const FORMAT: &str = "figcraft";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub version: Option<u64>,
    pub images: Option<Vec<String>>,
}

pub fn read_header(text: &str) -> Result<Header, serde_json::Error> {
    serde_json::from_str(text)
}

impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Header;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a FigCraft document object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Header, A::Error> {
                let mut h = Header::default();
                while let Some(k) = m.next_key::<String>()? {
                    match k.as_str() {
                        "version" => h.version = m.next_value::<Value>()?.as_u64(),
                        "imageIndex" => {
                            h.images = m
                                .next_value::<Value>()?
                                .as_array()
                                .map(|a| a.iter().filter_map(|v| v.get("hash").and_then(Value::as_str).map(str::to_owned)).collect())
                        }
                        _ => {
                            m.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(h)
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("this file was saved by a newer FigCraft (format version {0})")]
pub struct NewerVersion(pub u32);

pub fn check_version(h: &Header) -> Result<(), NewerVersion> {
    if let Some(v) = h.version
        && v > u64::from(VERSION)
    {
        return Err(NewerVersion(u32::try_from(v).unwrap_or(u32::MAX)));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImageIndexEntry {
    pub hash: String,
    pub width: u32,
    pub height: u32,
}

pub fn write_document_json<D: Serialize, W: std::io::Write>(doc: &D, images: &[ImageIndexEntry], w: W) -> Result<(), serde_json::Error> {
    #[derive(Serialize)]
    struct Flat<'a, D: ?Sized> {
        format: &'static str,
        version: u32,
        #[serde(rename = "imageIndex")]
        images: &'a [ImageIndexEntry],
        #[serde(flatten)]
        document: &'a D,
    }
    Flat { format: FORMAT, version: VERSION, images, document: doc }.serialize(&mut serde_json::Serializer::new(w))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn header_skips_app_fields() {
        let h = read_header(r#"{"version":1,"imageIndex":[{"hash":"abc"}],"nested":{"unused":true}}"#).unwrap();
        assert_eq!(h.version, Some(1));
        assert_eq!(h.images, Some(vec!["abc".into()]));
    }
    #[test]
    fn newer_version_is_reported_before_document_decode() {
        assert_eq!(check_version(&Header { version: Some(2), images: None }), Err(NewerVersion(2)));
        assert_eq!(check_version(&Header { version: Some(u64::MAX), images: None }), Err(NewerVersion(u32::MAX)));
    }
    #[test]
    fn writer_flattens_document_fields() {
        let doc = serde_json::json!({"name":"example"});
        let mut bytes = Vec::new();
        write_document_json(&doc, &[], &mut bytes).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["format"], FORMAT);
        assert_eq!(v["version"], VERSION);
        assert_eq!(v["name"], "example");
    }
}

/// Limits for the native ZIP package reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    pub max_entries: u64,
    pub max_total_bytes: u64,
}

/// A bounded reader for a native `.figcraft` ZIP package.
pub struct Archive<'a> {
    inner: craft_zip::Archive<'a>,
}

impl<'a> Archive<'a> {
    pub fn open(bytes: &'a [u8], limits: ArchiveLimits) -> Result<Self, craft_zip::Error> {
        let inner = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: limits.max_total_bytes })?;
        Ok(Self { inner })
    }

    pub fn names(&self) -> impl Iterator<Item = std::borrow::Cow<'_, str>> {
        self.inner.names()
    }

    pub fn has_duplicate_names(&self) -> bool {
        self.inner.has_duplicate_names()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.inner.contains(name)
    }

    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        self.inner.declared_size(name)
    }

    pub fn compressed_size(&mut self, name: &str) -> Option<u64> {
        self.inner.compressed_size(name)
    }

    pub fn read(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, craft_zip::Error> {
        self.inner.read(name, max_bytes)
    }

    pub fn budget_used(&self) -> u64 {
        self.inner.budget().used()
    }
}

/// One member to write into a native ZIP package.
#[derive(Clone, Copy, Debug)]
pub struct Member<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub compression: craft_zip::Compression,
}

/// Write a deterministic native ZIP package using the shared ZIP writer.
pub fn write_archive<'a>(members: impl IntoIterator<Item = Member<'a>>) -> Result<Vec<u8>, craft_zip::Error> {
    let mut writer = craft_zip::Writer::new();
    for member in members {
        writer.add(member.name, member.bytes, member.compression)?;
    }
    writer.finish()
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn bounded_archive_reader_and_writer_round_trip_members() {
        let bytes =
            write_archive([Member { name: "document.json", bytes: br#"{"version":1}"#, compression: craft_zip::Compression::Deflated }]).unwrap();
        let mut archive = Archive::open(&bytes, ArchiveLimits { max_entries: 4, max_total_bytes: 128 }).unwrap();
        assert!(archive.contains("document.json"));
        assert_eq!(archive.read("document.json", 32).unwrap(), Some(br#"{"version":1}"#.to_vec()));
        assert_eq!(archive.budget_used(), 13);
    }

    #[test]
    fn archive_reader_enforces_entry_and_total_limits() {
        let bytes = write_archive([Member { name: "document.json", bytes: b"123456789", compression: craft_zip::Compression::Deflated }]).unwrap();
        let mut small_entry = Archive::open(&bytes, ArchiveLimits { max_entries: 4, max_total_bytes: 100 }).unwrap();
        assert!(small_entry.read("document.json", 8).is_err());
        let mut small_total = Archive::open(&bytes, ArchiveLimits { max_entries: 4, max_total_bytes: 8 }).unwrap();
        assert!(small_total.read("document.json", 100).is_err());
    }
}

/// A native archive writer that keeps the streaming document serializer out of the consumer.
pub struct ArchiveWriter {
    inner: craft_zip::Writer,
}

impl ArchiveWriter {
    pub fn new() -> Self {
        Self { inner: craft_zip::Writer::new() }
    }

    pub fn add(&mut self, name: &str, bytes: &[u8], compression: craft_zip::Compression) -> Result<(), craft_zip::Error> {
        self.inner.add(name, bytes, compression)
    }

    pub fn add_document_json<D: Serialize>(&mut self, name: &str, document: &D, images: &[ImageIndexEntry]) -> Result<(), ArchiveWriteError> {
        self.inner
            .add_with(name, craft_zip::Compression::Level(3), |w| write_document_json(document, images, w).map_err(std::io::Error::other))
            .map_err(ArchiveWriteError::Archive)
    }

    pub fn finish(self) -> Result<Vec<u8>, craft_zip::Error> {
        self.inner.finish()
    }
}

impl Default for ArchiveWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveWriteError {
    #[error("could not serialize native document JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not write native archive: {0}")]
    Archive(#[from] craft_zip::Error),
}

#[cfg(test)]
mod streaming_writer_tests {
    use super::*;

    #[test]
    fn native_writer_streams_flat_document_into_archive() {
        let document = serde_json::json!({"name":"streamed"});
        let mut writer = ArchiveWriter::new();
        writer.add_document_json("document.json", &document, &[]).unwrap();
        let bytes = writer.finish().unwrap();
        let mut archive = Archive::open(&bytes, ArchiveLimits { max_entries: 4, max_total_bytes: 4096 }).unwrap();
        let json = archive.read("document.json", 1024).unwrap().unwrap();
        let value: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["name"], "streamed");
        assert_eq!(value["format"], FORMAT);
    }
}

/// ZIP compression modes for native archive members.
pub use craft_zip::Compression;
