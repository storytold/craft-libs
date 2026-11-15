#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The neutral JSON header and version contract for FigCraft's native project format.
//! App-specific document fields remain a generic flattened payload.
pub mod library;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const EXTENSION: &str = "figcraft";
pub const FORMAT: &str = "figcraft";
pub const VERSION: u32 = 1;

fn is_false(value: &bool) -> bool {
    !value
}

/// One native version-history index entry. Invalid optional values decode to the same defaults as
/// the historical lenient reader; the app applies document-specific validity and issue reporting.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct VersionEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub time: u64,
    #[serde(skip_serializing_if = "is_false")]
    pub auto: bool,
    pub images: Vec<String>,
}

impl<'de> Deserialize<'de> for VersionEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let string = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let images = value.get("images").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
        Ok(Self {
            id: string("id"),
            name: string("name"),
            description: string("description"),
            time: value.get("time").and_then(Value::as_u64).unwrap_or(0),
            auto: value.get("auto").and_then(Value::as_bool).unwrap_or(false),
            images,
        })
    }
}

/// Native version-history index, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct VersionIndex {
    pub versions: Vec<VersionEntry>,
}

/// Encode the native version-history index using the FigCraft wire schema.
pub fn write_version_index(index: &VersionIndex) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(index)
}

/// Decode the native version-history index using the format's lenient optional-field defaults.
pub fn read_version_index(bytes: &[u8]) -> Result<VersionIndex, serde_json::Error> {
    serde_json::from_slice(bytes)
}

impl<'de> Deserialize<'de> for VersionIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let versions = value
            .get("versions")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().cloned().map(serde_json::from_value).collect::<Result<Vec<_>, _>>())
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        Ok(Self { versions })
    }
}

/// State stored on a native branch entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchState {
    #[default]
    Active,
    Merged,
    Archived,
}

/// One native branch index entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    pub id: String,
    pub name: String,
    pub time: u64,
    pub client: u32,
    pub state: BranchState,
    pub base_images: Vec<String>,
    pub head_images: Vec<String>,
}

impl<'de> Deserialize<'de> for BranchEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let string = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let images =
            |key: &str| value.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
        let state = value.get("state").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let client = value.get("client").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
        Ok(Self {
            id: string("id"),
            name: string("name"),
            time: value.get("time").and_then(Value::as_u64).unwrap_or(0),
            client,
            state,
            base_images: images("baseImages"),
            head_images: images("headImages"),
        })
    }
}

/// Native branch index.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BranchIndex {
    pub branches: Vec<BranchEntry>,
}

/// Encode the native branch index using the FigCraft wire schema.
pub fn write_branch_index(index: &BranchIndex) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(index)
}

/// Decode the native branch index using the format's lenient optional-field defaults.
pub fn read_branch_index(bytes: &[u8]) -> Result<BranchIndex, serde_json::Error> {
    serde_json::from_slice(bytes)
}

impl<'de> Deserialize<'de> for BranchIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let branches = value
            .get("branches")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().cloned().map(serde_json::from_value).collect::<Result<Vec<_>, _>>())
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        Ok(Self { branches })
    }
}

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

/// Failure while reading the native document envelope or its app-supplied payload type.
#[derive(Debug, thiserror::Error)]
pub enum ReadDocumentError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    NewerVersion(#[from] NewerVersion),
}

/// Read a document payload after checking the native header and version first.
///
/// The payload type is supplied by the consumer, keeping app model conversion outside this crate.
pub fn read_document_json<D: serde::de::DeserializeOwned>(text: &str) -> Result<(D, Header), ReadDocumentError> {
    let header = read_header(text)?;
    check_version(&header)?;
    let document = serde_json::from_str(text)?;
    Ok((document, header))
}

/// The native document fields needed to render a recent-file preview.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PreviewDocument {
    #[serde(default)]
    pub pages: Vec<u64>,
    #[serde(default)]
    pub nodes: std::collections::BTreeMap<String, PreviewNode>,
    #[serde(default)]
    pub kind: PreviewKind,
}

/// Preview-relevant fields from one native document node.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PreviewNode {
    #[serde(default)]
    pub background: Option<Value>,
}

/// The supported native document kinds as stored on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewKind {
    #[default]
    Design,
    Board,
}

/// Decode the preview-relevant subset of the native FigCraft document schema.
pub fn read_preview_document(bytes: &[u8]) -> Result<PreviewDocument, serde_json::Error> {
    serde_json::from_slice(bytes)
}

/// Largest `thumbnail.png` returned for a recent-file preview.
pub const MAX_PREVIEW_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;
/// Largest `document.json` read to build a recent-file preview.
pub const MAX_PREVIEW_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Native `.figcraft` members needed by the recent-file preview.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewArchive {
    /// Whether the archive has a `document.json` entry, even if it is damaged or oversized.
    /// Consumers use this to distinguish a malformed native archive from other ZIP formats.
    pub has_document: bool,
    /// The thumbnail bytes, absent when missing, damaged, or above [`MAX_PREVIEW_THUMBNAIL_BYTES`].
    pub thumbnail_png: Option<Vec<u8>>,
    /// The first-page fields, absent when `document.json` is missing, damaged, or oversized.
    pub document: Option<PreviewDocument>,
}

/// Read only the thumbnail member for a native archive-list thumbnail.
///
/// This avoids unpacking the document when a caller needs only the image. Missing, damaged,
/// and oversized entries return None; an unreadable archive is an error.
pub fn read_preview_thumbnail<R: std::io::Read + std::io::Seek>(reader: R) -> Result<Option<Vec<u8>>, FormatError> {
    let mut archive = SeekArchive::open(reader, u64::MAX).map_err(|error| FormatError::Corrupt(error.to_string()))?;
    Ok(archive.read("thumbnail.png", MAX_PREVIEW_THUMBNAIL_BYTES).ok().flatten())
}

/// Read the bounded native preview members of a `.figcraft` archive.
///
/// An unreadable archive is an error; a missing, damaged, or oversized member is represented by
/// `None`, so the caller can still show the other part of the preview. The thumbnail and document
/// retain the separate limits used by the app's recent-file preview.
pub fn read_preview_archive<R: std::io::Read + std::io::Seek>(reader: R) -> Result<PreviewArchive, FormatError> {
    let mut archive = SeekArchive::open(reader, u64::MAX).map_err(|error| FormatError::Corrupt(error.to_string()))?;
    Ok(read_preview_archive_from(&mut archive))
}

/// Read bounded native preview members from an already-open shared archive reader.
///
/// This lets callers preserve fallback handling for other ZIP formats without implementing ZIP
/// member access themselves.
pub fn read_preview_archive_from(archive: &mut impl NativeArchive) -> PreviewArchive {
    let has_document = archive.contains("document.json");
    let thumbnail_png = archive.read_member("thumbnail.png", MAX_PREVIEW_THUMBNAIL_BYTES).ok().flatten();
    let document = if has_document {
        archive.read_member("document.json", MAX_PREVIEW_DOCUMENT_BYTES).ok().flatten().and_then(|bytes| read_preview_document(&bytes).ok())
    } else {
        None
    };
    PreviewArchive { has_document, thumbnail_png, document }
}

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
    fn preview_archive_reads_native_members_and_preserves_document_presence() {
        let document = br#"{"pages":[7],"nodes":{"7":{"background":{"r":0.25}}},"kind":"board"}"#;
        let bytes = write_archive([
            Member { name: "thumbnail.png", bytes: b"png bytes", compression: Compression::Stored },
            Member { name: "document.json", bytes: document, compression: Compression::Deflated },
        ])
        .unwrap();
        assert_eq!(read_preview_thumbnail(std::io::Cursor::new(bytes.clone())).unwrap().as_deref(), Some(b"png bytes".as_slice()));
        let mut archive = SeekArchive::open(std::io::Cursor::new(bytes.clone()), u64::MAX).unwrap();
        assert!(read_preview_archive_from(&mut archive).has_document);
        let preview = read_preview_archive(std::io::Cursor::new(bytes)).unwrap();
        assert!(preview.has_document);
        assert_eq!(preview.thumbnail_png.as_deref(), Some(b"png bytes".as_slice()));
        let document = preview.document.unwrap();
        assert_eq!(document.pages, [7]);
        assert_eq!(document.kind, PreviewKind::Board);
        assert_eq!(document.nodes["7"].background.as_ref().unwrap()["r"], 0.25);

        // A malformed native document still identifies this as a FigCraft archive, allowing the
        // app to retain its existing fallback instead of interpreting it as another ZIP format.
        let bytes = write_archive([Member { name: "document.json", bytes: b"{", compression: Compression::Stored }]).unwrap();
        let preview = read_preview_archive(std::io::Cursor::new(bytes)).unwrap();
        assert!(preview.has_document);
        assert!(preview.document.is_none());
        assert!(preview.thumbnail_png.is_none());
        assert!(read_preview_archive(std::io::Cursor::new(b"not a zip".as_slice())).is_err());
    }

    #[test]
    fn preview_archive_keeps_thumbnail_and_document_size_limits_independent() {
        let too_large_thumbnail = vec![0; MAX_PREVIEW_THUMBNAIL_BYTES as usize + 1];
        let bytes = write_archive([
            Member { name: "thumbnail.png", bytes: &too_large_thumbnail, compression: Compression::Deflated },
            Member { name: "document.json", bytes: br#"{"pages":[3]}"#, compression: Compression::Stored },
        ])
        .unwrap();
        let preview = read_preview_archive(std::io::Cursor::new(bytes)).unwrap();
        assert!(preview.thumbnail_png.is_none());
        assert_eq!(preview.document.unwrap().pages, [3]);

        let too_large_document = vec![b' '; MAX_PREVIEW_DOCUMENT_BYTES as usize + 1];
        let bytes = write_archive([Member { name: "document.json", bytes: &too_large_document, compression: Compression::Deflated }]).unwrap();
        let preview = read_preview_archive(std::io::Cursor::new(bytes)).unwrap();
        assert!(preview.has_document);
        assert!(preview.document.is_none());
    }

    #[test]
    fn header_skips_app_fields() {
        let h = read_header(r#"{"version":1,"imageIndex":[{"hash":"abc"}],"nested":{"unused":true}}"#).unwrap();
        assert_eq!(h.version, Some(1));
        assert_eq!(h.images, Some(vec!["abc".into()]));
    }

    #[test]
    fn version_index_preserves_lenient_defaults_and_writes_compact_auto_flag() {
        let index = read_version_index(br#"{"extra":true,"versions":[{"id":"v1","time":7,"auto":false,"images":["a",3]},{}]}"#).unwrap();
        assert_eq!(index.versions.len(), 2);
        assert_eq!(index.versions[0].images, vec!["a"]);
        assert_eq!(index.versions[1], VersionEntry::default());
        let encoded: Value = serde_json::from_slice(&write_version_index(&index).unwrap()).unwrap();
        assert!(encoded["versions"][0].get("auto").is_none());
        assert_eq!(encoded["versions"][0]["time"], 7);
    }

    #[test]
    fn branch_index_uses_native_camel_case_and_tolerates_invalid_optional_fields() {
        let index =
            read_branch_index(br#"{"branches":[{"id":"b1","client":4294967296,"state":"unknown","baseImages":["a",false],"headImages":["b"]},{}]}"#)
                .unwrap();
        let first = &index.branches[0];
        assert_eq!(first.client, 0);
        assert_eq!(first.state, BranchState::Active);
        assert_eq!(first.base_images, vec!["a"]);
        assert_eq!(first.head_images, vec!["b"]);
        let encoded = serde_json::to_value(&index).unwrap();
        assert_eq!(encoded["branches"][0]["baseImages"][0], "a");
        assert!(encoded["branches"][0].get("base_images").is_none());
    }

    #[test]
    fn document_limit_keeps_compressible_files_and_caps_expansion() {
        const MB: u64 = 1024 * 1024;
        assert_eq!(document_limit(0, 0), MAX_MEMBER);
        assert_eq!(document_limit(MB, 2 * MB), MAX_MEMBER);
        assert_eq!(document_limit(u64::MAX, MB), MAX_MEMBER);
        assert_eq!(document_limit(MAX_DOCUMENT, MAX_DOCUMENT), MAX_DOCUMENT);
        assert_eq!(document_limit(244_362_923, 260 * MB), 244_362_923 * DOCUMENT_RATIO);
    }

    #[test]
    fn read_budget_enforces_member_and_archive_totals() {
        let bytes = write_archive([
            Member { name: "one", bytes: b"12345678", compression: Compression::Stored },
            Member { name: "two", bytes: b"abcdefgh", compression: Compression::Stored },
        ])
        .unwrap();
        let mut archive = Archive::open(&bytes, ArchiveLimits { max_entries: 4, max_total_bytes: 128 }).unwrap();
        let mut budget = ReadBudget::new(10);
        assert_eq!(budget.read(&mut archive, "one", 16).unwrap(), b"12345678");
        assert!(budget.exceeds_remaining(&mut archive, "two"));
        assert!(budget.exhausted_message().contains("contents together pass 10 bytes"));
        assert!(budget.read(&mut archive, "two", 16).unwrap_err().to_string().contains("contents together pass 10 bytes"));

        let mut member_budget = ReadBudget::new(100);
        assert!(member_budget.read(&mut archive, "two", 4).unwrap_err().to_string().contains("two is too large"));
        assert_eq!(member_budget.read(&mut archive, "two", 16).unwrap(), b"abcdefgh");
    }
    #[test]
    fn newer_version_is_reported_before_document_decode() {
        assert_eq!(check_version(&Header { version: Some(2), images: None }), Err(NewerVersion(2)));
        assert_eq!(check_version(&Header { version: Some(u64::MAX), images: None }), Err(NewerVersion(u32::MAX)));

        #[derive(Deserialize)]
        struct Payload {
            document: u32,
        }
        let text = r#"{"version":2,"document":{"wrong":"type"}}"#;
        assert!(matches!(read_document_json::<Payload>(text), Err(ReadDocumentError::NewerVersion(NewerVersion(2)))));
        let (payload, header) = read_document_json::<Payload>(r#"{"version":1,"imageIndex":[{"hash":"a"}],"document":5}"#).unwrap();
        assert_eq!(payload.document, 5);
        assert_eq!(header.images, Some(vec!["a".into()]));
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

    #[test]
    fn preview_document_reads_native_page_background_and_kind() {
        let doc = read_preview_document(br#"{"pages":[7],"nodes":{"7":{"background":{"r":0.1}}},"kind":"board","unrelated":true}"#).unwrap();
        assert_eq!(doc.pages, vec![7]);
        assert_eq!(doc.nodes["7"].background.as_ref().and_then(|v| v.get("r")).and_then(Value::as_f64), Some(0.1));
        assert_eq!(doc.kind, PreviewKind::Board);
        assert_eq!(read_preview_document(b"{}").unwrap(), PreviewDocument::default());
        assert!(read_preview_document(br#"{"kind":"unknown"}"#).is_err());
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
/// The document expansion ratio that its compressed size may claim.
pub const DOCUMENT_RATIO: u64 = 16;
#[cfg(not(target_pointer_width = "32"))]
pub const MAX_DOCUMENT: u64 = 4 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
pub const MAX_DOCUMENT: u64 = 1024 * 1024 * 1024;

/// Upper bound for the expanded document member based on both the archive and member policy.
/// The floor preserves support for large but highly compressible documents; the ceiling prevents
/// a tiny archive from claiming an unbounded expansion.
pub fn document_limit(packed: u64, archive_bytes: u64) -> u64 {
    packed.min(archive_bytes).saturating_mul(DOCUMENT_RATIO).clamp(MAX_MEMBER, MAX_DOCUMENT)
}
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

/// Tracks the total bytes unpacked from one native archive across its document, images, library,
/// history, and branch members.
#[derive(Clone, Debug)]
pub struct ReadBudget {
    left: u64,
    total: u64,
}

impl ReadBudget {
    pub fn new(total: u64) -> Self {
        Self { left: total, total }
    }

    fn passed(&self) -> String {
        match self.total / (1024 * 1024) {
            0 => format!("too large: the file's contents together pass {} bytes", self.total),
            mb => format!("too large: the file's contents together pass {mb} MB"),
        }
    }

    /// Whether the member's declared uncompressed size exceeds what remains in this budget.
    pub fn exceeds_remaining(&self, archive: &mut impl NativeArchive, name: &str) -> bool {
        archive.declared_size(name).is_some_and(|size| size > self.left)
    }

    /// Human-readable reason for leaving out a member after the archive-wide limit is spent.
    pub fn exhausted_message(&self) -> String {
        self.passed()
    }

    /// Read one native member within its own size limit and the remaining archive-wide budget.
    /// A failed read that consumed a member is charged at its declared size.
    pub fn read(&mut self, archive: &mut impl NativeArchive, name: &str, limit: u64) -> Result<Vec<u8>, FormatError> {
        let declared = archive.declared_size(name);
        if declared.is_some_and(|size| size > self.left && size <= limit) {
            return Err(FormatError::Corrupt(format!("{name}: {}", self.passed())));
        }
        let cap = limit.min(self.left);
        let size = declared.unwrap_or(0);
        let result = match archive.read_member(name, cap) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(FormatError::Corrupt(format!("{name}: not found"))),
            Err(error) if error.is_limit() => Err(FormatError::Corrupt(format!("{name} is too large"))),
            Err(error) => Err(FormatError::Corrupt(format!("{name}: {error}"))),
        };
        if size <= cap {
            self.left = self.left.saturating_sub(result.as_ref().map_or(size, |bytes| bytes.len() as u64));
        }
        result
    }
}

/// Whether `id` can name a version or branch: 1–64 ASCII letters, digits, `-` and `_`.
pub fn is_valid_version_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Clip text to at most `max` Unicode scalar values.
pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
