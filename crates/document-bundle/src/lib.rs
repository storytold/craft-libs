#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! A JSON document bundle envelope over ZIP.
//!
//! Applications keep their document models. This crate owns the reusable container mechanics:
//! ordered mimetype, metadata, JSON model, previews, embedded asset paths, compression policy,
//! bounded reads and extraction of legacy base64 asset maps. Entry names and MIME type are
//! supplied by the format using the bundle.

use std::io::{Cursor, Read, Seek, Write};

use craft_zip::{Archive, Limits};
use serde_json::Value;

const MAX_ENTRIES: u64 = 100_000;
const JSON_LIMIT: u64 = 256 << 20;
const ASSET_LIMIT: u64 = 1 << 30;
const EXPANSION_LIMIT: u64 = 4 << 30;
/// Largest proxy preview read from a bundle; larger entries are skipped.
pub const PROXY_LIMIT: u64 = 1 << 20;

/// Per-entry and total-expansion limits for an untrusted document bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// Maximum expanded size of metadata and document JSON.
    pub json_bytes: u64,
    /// Maximum expanded size of one compressed asset. Stored assets are bounded by archive size.
    pub compressed_asset_bytes: u64,
    /// Maximum expansion beyond the bytes in the source archive.
    pub expansion_bytes: u64,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self { json_bytes: JSON_LIMIT, compressed_asset_bytes: ASSET_LIMIT, expansion_bytes: EXPANSION_LIMIT }
    }
}

/// Names and identity used by one JSON document bundle format.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// Stored first entry and its payload.
    pub mimetype_entry: &'a str,
    pub mimetype: &'a str,
    /// Metadata and document JSON member names.
    pub metadata_entry: &'a str,
    pub document_entry: &'a str,
    /// Optional preview entry name.
    pub preview_entry: &'a str,
    /// Directory prefix for embedded assets, without a trailing slash.
    pub assets_directory: &'a str,
    /// Directory prefix for proxy previews of linked assets, without a trailing slash.
    pub proxies_directory: &'a str,
    /// Metadata value written in the `format` field.
    pub format_name: &'a str,
    /// Old single-file JSON field containing base64 assets keyed by decimal id.
    pub legacy_assets_field: Option<&'a str>,
}

/// An embedded asset passed to [`write`].
#[derive(Clone, Copy, Debug)]
pub struct Asset<'a> {
    /// Decimal asset id used in the archive entry path.
    pub id: &'a str,
    /// MIME type used to select the extension and compression policy.
    pub mime: &'a str,
    /// File bytes retained byte-for-byte.
    pub data: &'a [u8],
}

/// A document and embedded payloads returned by [`read`].
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    /// Parsed model JSON, after extracting legacy inline assets.
    pub document: Value,
    /// Format version declared by metadata, or 1 when absent.
    pub version: u64,
    /// Embedded asset bytes keyed by decimal asset id.
    pub assets: Vec<(String, Vec<u8>)>,
    /// Proxy preview bytes (JPEG or PNG) keyed by decimal asset id, for ids the document lists.
    pub proxies: Vec<(String, Vec<u8>)>,
}

/// Why a document bundle could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a bundle or legacy JSON document.
    #[error("not a document bundle: {0}")]
    NotBundle(String),
    /// The bundle requires a newer reader.
    #[error("document format {0} is newer than this reader supports")]
    TooNew(u64),
    /// A member could not be written.
    #[error("could not write document bundle: {0}")]
    Write(String),
    /// A member exceeded a read limit or could not be read.
    #[error("could not read document bundle: {0}")]
    Read(String),
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/tiff" => "tif",
        "application/pdf" => "pdf",
        "application/postscript" => "eps",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// Whether `bytes` are a ZIP bundle that contains the configured document member.
pub fn has_document(bytes: &[u8], spec: Spec<'_>) -> bool {
    let Ok(archive) = Archive::open(bytes, Limits { max_entries: MAX_ENTRIES, max_total_bytes: 0 }) else { return false };
    archive.contains(spec.document_entry)
}

/// Whether `prefix` starts with the configured stored mimetype entry.
pub fn starts_with_mimetype(prefix: &[u8], spec: Spec<'_>) -> bool {
    craft_zip::stored_mimetype(prefix).is_some_and(|mime| mime == spec.mimetype.as_bytes())
}

/// Refuses every write and seek after the first failure of the wrapped writer. Dropping a
/// `ZipWriter` after an error finalizes the archive, which must not touch a cancelled target.
struct Latch<W> {
    inner: W,
    failed: bool,
}

impl<W> Latch<W> {
    fn latched() -> std::io::Error {
        std::io::Error::other("writer already failed")
    }
}

impl<W: Write> Write for Latch<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.failed {
            return Err(Self::latched());
        }
        self.inner.write(buf).inspect_err(|_| self.failed = true)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.failed {
            return Err(Self::latched());
        }
        self.inner.flush().inspect_err(|_| self.failed = true)
    }
}

impl<W: Seek> Seek for Latch<W> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        if self.failed {
            return Err(Self::latched());
        }
        self.inner.seek(pos).inspect_err(|_| self.failed = true)
    }
}

fn write_err(e: impl std::fmt::Display) -> Error {
    Error::Write(e.to_string())
}

/// Write a ZIP document bundle containing the serialized document, metadata, preview, assets and
/// proxy previews.
pub fn write(
    document: &[u8],
    version: u32,
    generator: &str,
    preview: Option<&[u8]>,
    assets: &[Asset<'_>],
    proxies: &[Asset<'_>],
    spec: Spec<'_>,
) -> Result<Vec<u8>, Error> {
    let mut out = Cursor::new(Vec::new());
    write_to(&mut out, document, version, generator, preview, assets, proxies, spec)?;
    Ok(out.into_inner())
}

/// Like [`write`], streaming the archive into `out`. A writer that fails (for example to cancel
/// a long save) stops the write at the next member or buffer and the error is returned.
#[allow(clippy::too_many_arguments)]
pub fn write_to<W: Write + Seek>(
    out: W,
    document: &[u8],
    version: u32,
    generator: &str,
    preview: Option<&[u8]>,
    assets: &[Asset<'_>],
    proxies: &[Asset<'_>],
    spec: Spec<'_>,
) -> Result<(), Error> {
    use zip::CompressionMethod::{Deflated, Stored};
    let mut zip = zip::ZipWriter::new(Latch { inner: out, failed: false });
    let mut add = |name: &str, data: &[u8], method: zip::CompressionMethod| -> Result<(), Error> {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(if method == Deflated { Some(6) } else { None })
            .last_modified_time(zip::DateTime::default())
            .large_file(u64::try_from(data.len()).unwrap_or(u64::MAX) >= u64::from(u32::MAX) / 2);
        zip.start_file(name, options).map_err(|e| Error::Write(format!("{name}: {e}")))?;
        zip.write_all(data).map_err(|e| Error::Write(format!("{name}: {e}")))
    };
    add(spec.mimetype_entry, spec.mimetype.as_bytes(), Stored)?;
    let metadata = serde_json::json!({"format":spec.format_name, "version":version, "generator":generator});
    add(spec.metadata_entry, &serde_json::to_vec(&metadata).map_err(write_err)?, Deflated)?;
    if let Some(preview) = preview {
        add(spec.preview_entry, preview, Stored)?;
    }
    add(spec.document_entry, document, Deflated)?;
    for asset in assets.iter().filter(|asset| !asset.data.is_empty()) {
        let path = format!("{}/{}.{}", spec.assets_directory, asset.id, extension(asset.mime));
        let method = if matches!(asset.mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp") { Stored } else { Deflated };
        add(&path, asset.data, method)?;
    }
    for proxy in proxies.iter().filter(|proxy| !proxy.data.is_empty()) {
        let ext = if proxy.mime == "image/jpeg" { "jpg" } else { "png" };
        add(&format!("{}/{}.{ext}", spec.proxies_directory, proxy.id), proxy.data, Stored)?;
    }
    zip.finish().map(drop).map_err(write_err)
}

/// Read a ZIP bundle or legacy JSON document within bounded member sizes.
///
/// Members under the configured assets directory whose id does not occur in the document's
/// `assets` object are never decompressed. Unreferenced data cannot consume the caller's memory.
pub fn read(bytes: &[u8], supported_version: u32, spec: Spec<'_>) -> Result<Package, Error> {
    read_with_limits(bytes, supported_version, spec, ReadLimits::default())
}

/// Like [`read`], with caller-selected bounded-read limits.
pub fn read_with_limits(bytes: &[u8], supported_version: u32, spec: Spec<'_>, limits: ReadLimits) -> Result<Package, Error> {
    if !bytes.starts_with(b"PK") {
        return read_legacy(bytes, spec);
    }
    let source_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut archive = Archive::open(bytes, Limits { max_entries: MAX_ENTRIES, max_total_bytes: source_size.saturating_add(limits.expansion_bytes) })
        .map_err(|e| Error::NotBundle(e.to_string()))?;
    if archive.has_duplicate_names() {
        return Err(Error::NotBundle("the archive has duplicate member names".into()));
    }
    let version = archive
        .read(spec.metadata_entry, limits.json_bytes)
        .map_err(|e| Error::Read(e.to_string()))?
        .and_then(|metadata| serde_json::from_slice::<Value>(&metadata).ok())
        .and_then(|metadata| metadata.get("version").and_then(Value::as_u64))
        .unwrap_or(1);
    if version > u64::from(supported_version) {
        return Err(Error::TooNew(version));
    }
    let document_bytes = archive
        .read(spec.document_entry, limits.json_bytes)
        .map_err(|e| Error::Read(e.to_string()))?
        .ok_or_else(|| Error::NotBundle(format!("missing {}", spec.document_entry)))?;
    let document: Value = serde_json::from_slice(&document_bytes).map_err(|e| Error::NotBundle(e.to_string()))?;
    let wanted: std::collections::HashSet<String> =
        document.get("assets").and_then(Value::as_object).into_iter().flat_map(|assets| assets.keys().cloned()).collect();
    let prefix = format!("{}/", spec.assets_directory);
    let mut embedded = Vec::new();
    for path in archive.names().map(|name| name.into_owned()).collect::<Vec<_>>() {
        let Some(member) = path.strip_prefix(&prefix) else { continue };
        let Some(id) = member.split('.').next() else { continue };
        if !wanted.contains(id) || id.parse::<u64>().is_err() {
            continue;
        }
        let stored = archive.is_stored(&path).unwrap_or(false);
        let limit = if stored { source_size } else { limits.compressed_asset_bytes };
        let data = archive.read(&path, limit).map_err(|e| Error::Read(e.to_string()))?;
        if let Some(data) = data {
            embedded.push((id.to_owned(), data));
        }
    }
    let proxies = read_proxies(&mut archive, &wanted, spec);
    Ok(Package { document, version, assets: embedded, proxies })
}

/// Proxy previews for known asset ids. Only `<proxies>/<id>.jpg|png` is accepted; entries over
/// [`PROXY_LIMIT`] or that fail to read are skipped.
fn read_proxies(archive: &mut Archive<'_>, wanted: &std::collections::HashSet<String>, spec: Spec<'_>) -> Vec<(String, Vec<u8>)> {
    let prefix = format!("{}/", spec.proxies_directory);
    let mut proxies = Vec::new();
    for path in archive.names().map(|name| name.into_owned()).collect::<Vec<_>>() {
        let Some(member) = path.strip_prefix(&prefix) else { continue };
        let Some((id, ext)) = member.split_once('.') else { continue };
        if !matches!(ext, "jpg" | "png") || !wanted.contains(id) || id.parse::<u64>().is_err() {
            continue;
        }
        if archive.declared_size(&path).is_none_or(|size| size > PROXY_LIMIT) {
            continue;
        }
        let Some(size) = archive.declared_size(&path) else { continue };
        if archive.budget().remaining() < size {
            break;
        }
        match archive.read(&path, PROXY_LIMIT) {
            Ok(Some(data)) => proxies.push((id.to_owned(), data)),
            Ok(None) => {}
            // A failed read may have decompressed up to `size` bytes without being charged.
            Err(_) => {
                if archive.budget_mut().charge(size).is_err() {
                    break;
                }
            }
        }
    }
    proxies
}

/// Read a preview with a fixed maximum size without decompressing the full bundle.
pub fn read_preview(bytes: &[u8], preview_entry: &str) -> Option<Vec<u8>> {
    const PREVIEW_LIMIT: u64 = 4 << 20;
    let mut archive = Archive::open(bytes, Limits { max_entries: MAX_ENTRIES, max_total_bytes: PREVIEW_LIMIT }).ok()?;
    archive.read(preview_entry, PREVIEW_LIMIT).ok().flatten()
}

/// Read a bounded preview from a seekable file without loading the full bundle into memory.
pub fn read_preview_from<R: Read + Seek>(reader: R, preview_entry: &str) -> Option<Vec<u8>> {
    const PREVIEW_LIMIT: u64 = 4 << 20;
    let mut zip = zip::ZipArchive::new(reader).ok()?;
    let entry = zip.by_name(preview_entry).ok()?;
    if entry.size() > PREVIEW_LIMIT {
        return None;
    }
    let mut preview = Vec::new();
    entry.take(PREVIEW_LIMIT.saturating_add(1)).read_to_end(&mut preview).ok()?;
    (u64::try_from(preview.len()).ok()? <= PREVIEW_LIMIT).then_some(preview)
}

fn read_legacy(bytes: &[u8], spec: Spec<'_>) -> Result<Package, Error> {
    let mut document: Value = serde_json::from_slice(bytes).map_err(|e| Error::NotBundle(e.to_string()))?;
    let mut assets = Vec::new();
    let mut total = 0u64;
    if let Some(field) = spec.legacy_assets_field {
        if let Some(inline) = document.get(field).and_then(Value::as_object) {
            for (id, value) in inline {
                if id.parse::<u64>().is_ok()
                    && let Some(encoded) = value.as_str()
                {
                    let decoded_size = u64::try_from(encoded.len()).unwrap_or(u64::MAX).saturating_mul(3) / 4;
                    if decoded_size > ASSET_LIMIT {
                        return Err(Error::Read(format!("legacy asset {id} exceeds the {ASSET_LIMIT}-byte limit")));
                    }
                    total = total.saturating_add(decoded_size);
                    if total > EXPANSION_LIMIT {
                        return Err(Error::Read(format!("legacy assets exceed the {EXPANSION_LIMIT}-byte total limit")));
                    }
                    assets.push((id.clone(), decode_base64(encoded)));
                }
            }
        }
        if let Some(object) = document.as_object_mut() {
            object.remove(field);
        }
    }
    Ok(Package { document, version: 1, assets, proxies: Vec::new() })
}

fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec<'static> = Spec {
        mimetype_entry: "mimetype",
        mimetype: "application/vnd.test+zip",
        metadata_entry: "meta.json",
        document_entry: "document.json",
        preview_entry: "preview.png",
        assets_directory: "assets",
        proxies_directory: "proxies",
        format_name: "test",
        legacy_assets_field: Some("assetData"),
    };

    #[test]
    fn writes_and_reads_bundle_members() {
        let payload = [1u8, 2, 3];
        let bytes = write(br#"{"assets":{"7":{}}}"#, 4, "Test 0.1", Some(b"png"), &[Asset { id: "7", mime: "image/png", data: &payload }], &[], SPEC)
            .unwrap();
        assert!(has_document(&bytes, SPEC));
        assert!(starts_with_mimetype(&bytes, SPEC));
        let loaded = read(&bytes, 4, SPEC).unwrap();
        assert_eq!(loaded.document, serde_json::json!({"assets":{"7":{}}}));
        assert_eq!(loaded.version, 4);
        assert_eq!(loaded.assets, [("7".to_owned(), payload.to_vec())]);
        assert_eq!(read_preview(&bytes, SPEC.preview_entry).as_deref(), Some(&b"png"[..]));
    }

    #[test]
    fn extracts_legacy_inline_assets() {
        let loaded = read(br#"{"assetData":{"7":"AQID"},"name":"x"}"#, 4, SPEC).unwrap();
        assert_eq!(loaded.document, serde_json::json!({"name":"x"}));
        assert_eq!(loaded.assets, [("7".to_owned(), vec![1, 2, 3])]);
    }

    #[test]
    fn refuses_newer_versions() {
        let bytes = write(b"{}", 9, "future", None, &[], &[], SPEC).unwrap();
        assert_eq!(read(&bytes, 4, SPEC), Err(Error::TooNew(9)));
    }

    #[test]
    fn stored_assets_use_source_size_and_deflated_assets_use_their_limit() {
        let bytes = vec![7u8; 128];
        let document = br#"{"assets":{"7":{}}}"#;
        let limits = ReadLimits { json_bytes: 1 << 20, compressed_asset_bytes: 8, expansion_bytes: 1 << 20 };
        let stored = write(document, 4, "test", None, &[Asset { id: "7", mime: "image/png", data: &bytes }], &[], SPEC).unwrap();
        let package = read_with_limits(&stored, 4, SPEC, limits).unwrap();
        assert_eq!(package.assets, [("7".to_owned(), bytes.clone())]);

        let compressed = write(document, 4, "test", None, &[Asset { id: "7", mime: "application/octet-stream", data: &bytes }], &[], SPEC).unwrap();
        assert!(read_with_limits(&compressed, 4, SPEC, limits).is_err());
    }

    #[test]
    fn failed_proxy_reads_are_charged_and_stop_reading_when_the_budget_is_spent() {
        let pad = " ".repeat(40_000);
        let document = format!(r#"{{"assets":{{"1":{{}},"2":{{}},"3":{{}},"4":{{}}}},"pad":"{pad}"}}"#);
        let blobs: Vec<Vec<u8>> = (1u8..=4).map(|n| vec![n.wrapping_mul(50); 1000]).collect();
        let ids = ["1", "2", "3", "4"];
        let proxies: Vec<Asset<'_>> = ids.iter().zip(&blobs).map(|(id, data)| Asset { id, mime: "image/png", data }).collect();
        let mut bytes = write(document.as_bytes(), 4, "t", None, &[], &proxies, SPEC).unwrap();
        // Corrupt proxies 1 to 3 so their reads fail their checksum.
        for blob in &blobs[..3] {
            let at = bytes.windows(blob.len()).position(|w| w == blob.as_slice()).unwrap();
            bytes[at + 500] ^= 0xFF;
        }
        let source = bytes.len() as u64;
        let all = read(&bytes, 4, SPEC).unwrap();
        assert_eq!(all.proxies.len(), 1);
        let tight = ReadLimits { json_bytes: 1 << 20, compressed_asset_bytes: 1 << 20, expansion_bytes: 40_100 + 2_500 - source };
        assert!(read_with_limits(&bytes, 4, SPEC, tight).unwrap().proxies.is_empty());
    }
}
