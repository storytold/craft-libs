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

/// A native ZIP reader for seekable sources that should not be copied into memory first.
pub struct SeekArchive<R: std::io::Read + std::io::Seek> {
    inner: zip::ZipArchive<R>,
}

impl<R: std::io::Read + std::io::Seek> SeekArchive<R> {
    pub fn open(reader: R, max_entries: u64) -> Result<Self, craft_zip::Error> {
        let inner = zip::ZipArchive::new(reader).map_err(|e| craft_zip::Error::NotZip(e.to_string()))?;
        let count = inner.len() as u64;
        if count > max_entries {
            return Err(craft_zip::Error::TooManyEntries { declared: count, limit: max_entries });
        }
        Ok(Self { inner })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.inner.index_for_name(name).is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect()
    }

    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        self.inner.index_for_name(name).and_then(|i| self.inner.by_index_raw(i).ok().map(|f| f.size()))
    }

    pub fn read(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, craft_zip::Error> {
        let Some(index) = self.inner.index_for_name(name) else { return Ok(None) };
        let mut file = self.inner.by_index(index).map_err(|e| craft_zip::Error::Entry { name: name.to_owned(), msg: e.to_string() })?;
        if file.size() > max_bytes {
            return Err(craft_zip::Error::EntryTooLarge { name: name.to_owned(), limit: max_bytes });
        }
        let mut bytes = Vec::new();
        let mut limited = std::io::Read::take(&mut file, max_bytes.saturating_add(1));
        let read = std::io::Read::read_to_end(&mut limited, &mut bytes);
        if let Err(error) = read {
            return Err(craft_zip::Error::Entry { name: name.to_owned(), msg: error.to_string() });
        }
        if bytes.len() as u64 > max_bytes {
            return Err(craft_zip::Error::EntryTooLarge { name: name.to_owned(), limit: max_bytes });
        }
        Ok(Some(bytes))
    }
}

/// Shared access contract implemented by in-memory and seekable native archive readers.
pub trait NativeArchive {
    fn contains(&self, name: &str) -> bool;
    fn names(&self) -> Vec<String>;
    fn declared_size(&mut self, name: &str) -> Option<u64>;
    fn compressed_size(&mut self, name: &str) -> Option<u64>;
    fn read_member(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, craft_zip::Error>;
}

impl NativeArchive for Archive<'_> {
    fn contains(&self, name: &str) -> bool {
        Archive::contains(self, name)
    }
    fn names(&self) -> Vec<String> {
        Archive::names(self).map(|n| n.into_owned()).collect()
    }
    fn declared_size(&mut self, name: &str) -> Option<u64> {
        Archive::declared_size(self, name)
    }
    fn compressed_size(&mut self, name: &str) -> Option<u64> {
        Archive::compressed_size(self, name)
    }
    fn read_member(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, craft_zip::Error> {
        Archive::read(self, name, max_bytes)
    }
}

impl<R: std::io::Read + std::io::Seek> NativeArchive for SeekArchive<R> {
    fn contains(&self, name: &str) -> bool {
        SeekArchive::contains(self, name)
    }
    fn names(&self) -> Vec<String> {
        SeekArchive::names(self)
    }
    fn declared_size(&mut self, name: &str) -> Option<u64> {
        SeekArchive::declared_size(self, name)
    }
    fn compressed_size(&mut self, name: &str) -> Option<u64> {
        self.inner.index_for_name(name).and_then(|i| self.inner.by_index_raw(i).ok().map(|f| f.compressed_size()))
    }
    fn read_member(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, craft_zip::Error> {
        SeekArchive::read(self, name, max_bytes)
    }
}

#[cfg(test)]
mod seek_archive_tests {
    use super::*;

    #[test]
    fn seekable_reader_matches_bounded_in_memory_reader() {
        let bytes = write_archive([Member { name: "document.json", bytes: b"payload", compression: Compression::Stored }]).unwrap();
        let mut reader = SeekArchive::open(std::io::Cursor::new(bytes), 4).unwrap();
        assert!(reader.contains("document.json"));
        assert_eq!(reader.read_member("document.json", 16).unwrap(), Some(b"payload".to_vec()));
        assert_eq!(reader.declared_size("document.json"), Some(7));
        assert!(reader.read_member("document.json", 3).is_err());
    }
}

/// Largest individual ZIP member read by the native format.
pub const MAX_MEMBER: u64 = 512 * 1024 * 1024;
/// Most version snapshots kept on save and read on open.
pub const MAX_VERSIONS: usize = 100;
/// Maximum version and branch name lengths, in Unicode scalar values.
pub const MAX_VERSION_NAME: usize = 200;
pub const MAX_VERSION_DESCRIPTION: usize = 2000;
/// Maximum bytes of an index and all version snapshots read.
pub const MAX_VERSION_INDEX: u64 = 16 * 1024 * 1024;
pub const MAX_VERSION_BYTES: u64 = 1024 * 1024 * 1024;
/// Maximum images read and maximum combined image bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageBudget {
    pub images: usize,
    pub bytes: u64,
}
pub const IMAGE_BUDGET: ImageBudget = ImageBudget { images: 20_000, bytes: 4 * 1024 * 1024 * 1024 };
/// Maximum branch histories kept.
pub const MAX_BRANCHES: usize = 100;
/// The branch index member.
pub const BRANCH_INDEX: &str = "branches/index.json";
/// Maximum uncompressed bytes read from one native archive in total.
#[cfg(not(target_pointer_width = "32"))]
pub const MAX_UNPACKED: u64 = 6 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
pub const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;

/// Native-format errors shared with the consumer's document mapping.
#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    Io(String),
    Corrupt(String),
    Newer(u32),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Corrupt(e) => write!(f, "not a valid FigCraft file: {e}"),
            Self::Newer(v) => write!(f, "this file was saved by a newer FigCraft (format version {v})"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Whether `id` can name a version or branch: 1–64 ASCII letters, digits, `-` and `_`.
pub fn is_valid_version_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Clip text to at most `max` Unicode scalar values.
pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
