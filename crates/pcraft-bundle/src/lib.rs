#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Container and content-addressed object storage for `.pcraft` bundles.
//!
//! Document schemas stay with the application. This crate owns the wire-level paths, ZIP
//! envelope, zstd object frames and content hashes. A caller supplies serialized manifest bytes
//! and opaque tile/blob bytes; it receives their stable paths to put in its manifest.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use craft_zip::{Archive, Compression, Writer};

/// Default bounds for opening an untrusted `.pcraft` ZIP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of ZIP entries.
    pub max_entries: u64,
    /// Maximum total uncompressed ZIP bytes read.
    pub max_total_bytes: u64,
    /// Maximum manifest size.
    pub max_manifest_bytes: u64,
    /// Maximum compressed object bytes.
    pub max_object_bytes: u64,
    /// Maximum decoded object bytes.
    pub max_decoded_object_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        let limits = craft_pcraft_format::Limits::default();
        Self {
            max_entries: limits.max_items as u64 + 4,
            max_total_bytes: limits.max_total_bytes,
            max_manifest_bytes: limits.max_manifest_bytes as u64,
            max_object_bytes: limits.max_blob_bytes as u64,
            max_decoded_object_bytes: limits.max_blob_bytes as u64,
        }
    }
}

/// Why a `.pcraft` container or object could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A path is not one of the bundle's canonical entry names.
    #[error("invalid .pcraft entry path: {0}")]
    InvalidPath(String),
    /// An entry is missing, duplicated or violates a bundle invariant.
    #[error("invalid .pcraft bundle: {0}")]
    InvalidBundle(String),
    /// An entry exceeds its caller-supplied bound.
    #[error("{entry} exceeds the {limit}-byte limit")]
    LimitExceeded { entry: String, limit: u64 },
    /// The object's content hash does not match its entry path.
    #[error("content hash mismatch for {0}")]
    HashMismatch(String),
    /// ZIP container error.
    #[error(transparent)]
    Zip(#[from] craft_zip::Error),
    /// I/O or zstd decoding error.
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Whether a ZIP archive declares a `manifest.json` entry.
pub fn is_pcraft_zip(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let limits = Limits { max_entries: 1_000_000, ..Limits::default() };
    Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: 0 })
        .is_ok_and(|archive| !archive.has_duplicate_names() && archive.contains(craft_pcraft_format::MANIFEST))
}

/// Whether an entry path is a canonical manifest, preview, tile or blob path.
pub fn valid_entry_path(path: &str) -> bool {
    if matches!(
        path,
        craft_pcraft_format::MANIFEST | craft_pcraft_format::THUMB | craft_pcraft_format::COMPOSITE | craft_pcraft_format::FULL_COMPOSITE
    ) {
        return true;
    }
    let Some((kind, name)) = path.split_once('/') else { return false };
    let Some(hash) = name.strip_suffix(craft_pcraft_format::OBJECT_SUFFIX) else { return false };
    craft_pcraft_format::is_valid_hash(hash) && (kind == craft_pcraft_format::TILES_DIR || kind == craft_pcraft_format::BLOBS_DIR)
}

/// Content-addressed, compressed object bytes and their canonical bundle path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    /// `tiles/<hash>.zst` or `blobs/<hash>.zst`.
    pub path: String,
    /// BLAKE3 of the uncompressed bytes.
    pub hash: String,
    /// Zstandard frame containing the uncompressed bytes.
    pub compressed: Vec<u8>,
}

/// The kind of content-addressed object being stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    /// Pixel tile samples.
    Tile,
    /// ICC, metadata, masks, or other opaque document data.
    Blob,
}

/// Hashes and compresses an object, returning the path to record in the manifest.
pub fn encode_object(kind: ObjectKind, bytes: &[u8]) -> StoredObject {
    let hash = blake3::hash(bytes).to_hex().to_string();
    let path = match kind {
        ObjectKind::Tile => craft_pcraft_format::tile_path(&hash),
        ObjectKind::Blob => craft_pcraft_format::blob_path(&hash),
    }
    .unwrap_or_default();
    let compressed = ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest);
    StoredObject { path, hash, compressed }
}

/// Decompresses a stored object within `max_decoded_bytes` and checks its content hash.
pub fn decode_object(path: &str, compressed: &[u8], max_decoded_bytes: u64) -> Result<Vec<u8>> {
    if !valid_object_path(path) {
        return Err(Error::InvalidPath(path.to_owned()));
    }
    let max = usize::try_from(max_decoded_bytes).unwrap_or(usize::MAX);
    let decoder = ruzstd::decoding::StreamingDecoder::new(compressed).map_err(|e| Error::Io(e.to_string()))?;
    let mut bytes = Vec::new();
    decoder.take(max_decoded_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| Error::Io(e.to_string()))?;
    if bytes.len() > max {
        return Err(Error::LimitExceeded { entry: path.to_owned(), limit: max_decoded_bytes });
    }
    let expected = path.rsplit('/').next().unwrap_or_default().trim_end_matches(craft_pcraft_format::OBJECT_SUFFIX);
    if blake3::hash(&bytes).to_hex().as_str() != expected {
        return Err(Error::HashMismatch(path.to_owned()));
    }
    Ok(bytes)
}

fn valid_object_path(path: &str) -> bool {
    let Some((kind, _)) = path.split_once('/') else { return false };
    (kind == craft_pcraft_format::TILES_DIR || kind == craft_pcraft_format::BLOBS_DIR) && valid_entry_path(path)
}

/// An in-memory `.pcraft` ZIP reader with bounded entry and total reads.
pub struct ZipBundle<'a> {
    archive: Archive<'a>,
    limits: Limits,
}

impl<'a> ZipBundle<'a> {
    /// Opens a bundle, rejecting duplicate entry names and archives without a manifest.
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        let archive = Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: limits.max_total_bytes })?;
        if archive.has_duplicate_names() || !archive.contains(craft_pcraft_format::MANIFEST) {
            return Err(Error::InvalidBundle("missing manifest or duplicate entry names".into()));
        }
        Ok(Self { archive, limits })
    }

    /// Reads the serialized document manifest.
    pub fn manifest(&mut self) -> Result<Vec<u8>> {
        self.read(craft_pcraft_format::MANIFEST, self.limits.max_manifest_bytes)
    }

    /// Reads an optional preview entry.
    pub fn preview(&mut self, path: &str) -> Result<Option<Vec<u8>>> {
        if !matches!(path, craft_pcraft_format::THUMB | craft_pcraft_format::COMPOSITE | craft_pcraft_format::FULL_COMPOSITE) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
        self.archive.read(path, self.limits.max_object_bytes).map_err(Into::into)
    }

    /// Reads and verifies a compressed content-addressed object.
    pub fn object(&mut self, path: &str) -> Result<Vec<u8>> {
        let compressed = self.read(path, self.limits.max_object_bytes)?;
        decode_object(path, &compressed, self.limits.max_decoded_object_bytes)
    }

    fn read(&mut self, path: &str, limit: u64) -> Result<Vec<u8>> {
        if !valid_entry_path(path) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
        self.archive.read(path, limit)?.ok_or_else(|| Error::InvalidBundle(format!("missing {path}")))
    }
}

/// Builds a deterministic STORE ZIP bundle from format-neutral entries.
#[derive(Default)]
pub struct ZipBundleWriter {
    entries: BTreeMap<String, Vec<u8>>,
}

impl ZipBundleWriter {
    /// Adds or replaces the manifest bytes.
    pub fn set_manifest(&mut self, manifest: Vec<u8>) -> Result<()> {
        if manifest.is_empty() {
            return Err(Error::InvalidBundle("manifest is empty".into()));
        }
        self.entries.insert(craft_pcraft_format::MANIFEST.into(), manifest);
        Ok(())
    }

    /// Adds an optional preview.
    pub fn set_preview(&mut self, path: &str, bytes: Vec<u8>) -> Result<()> {
        if !matches!(path, craft_pcraft_format::THUMB | craft_pcraft_format::COMPOSITE | craft_pcraft_format::FULL_COMPOSITE) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
        self.entries.insert(path.to_owned(), bytes);
        Ok(())
    }

    /// Adds a compressed content-addressed object, returning its manifest path.
    pub fn add_object(&mut self, kind: ObjectKind, bytes: &[u8]) -> Result<String> {
        let object = encode_object(kind, bytes);
        if object.path.is_empty() {
            return Err(Error::InvalidBundle("could not construct object path".into()));
        }
        self.entries.entry(object.path.clone()).or_insert(object.compressed);
        Ok(object.path)
    }

    /// Adds an already-compressed object after checking its path/hash.
    pub fn add_stored_object(&mut self, path: &str, compressed: Vec<u8>, max_decoded_bytes: u64) -> Result<()> {
        let _ = decode_object(path, &compressed, max_decoded_bytes)?;
        self.entries.insert(path.to_owned(), compressed);
        Ok(())
    }

    /// Writes entries in canonical path order, with every ZIP member stored without another layer
    /// of compression (object payloads already use zstd).
    pub fn finish(self) -> Result<Vec<u8>> {
        if !self.entries.contains_key(craft_pcraft_format::MANIFEST) {
            return Err(Error::InvalidBundle("manifest not set".into()));
        }
        let mut writer = Writer::new();
        let mut entries = self.entries;
        let manifest = entries.remove(craft_pcraft_format::MANIFEST).ok_or_else(|| Error::InvalidBundle("manifest not set".into()))?;
        writer.add(craft_pcraft_format::MANIFEST, &manifest, Compression::Stored)?;
        for (path, bytes) in entries {
            if !valid_entry_path(&path) {
                return Err(Error::InvalidPath(path));
            }
            writer.add(&path, &bytes, Compression::Stored)?;
        }
        writer.finish().map_err(Into::into)
    }

    /// Saves the same entries as a directory bundle. Object files are published before the
    /// manifest, the bundle lock serializes writers, and unreferenced canonical objects are
    /// collected after the new manifest is durable.
    pub fn save_directory(self, root: &Path) -> Result<DirectoryStats> {
        if !self.entries.contains_key(craft_pcraft_format::MANIFEST) {
            return Err(Error::InvalidBundle("manifest not set".into()));
        }
        for path in self.entries.keys() {
            if !valid_entry_path(path) {
                return Err(Error::InvalidPath(path.clone()));
            }
        }
        fs::create_dir_all(root).map_err(io_error)?;
        let root = fs::canonicalize(root).map_err(io_error)?;
        let lock_path = root.join(craft_pcraft_format::SAVE_LOCK);
        refuse_symlink(&lock_path)?;
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path).map_err(io_error)?;
        lock.lock().map_err(io_error)?;

        let mut written = 0usize;
        for (path, bytes) in self.entries.iter().filter(|(path, _)| path.as_str() != craft_pcraft_format::MANIFEST) {
            let target = safe_target(&root, path)?;
            if let Some(parent) = target.parent() {
                ensure_directory(parent)?;
            }
            refuse_symlink(&target)?;
            craft_atomic::atomic_write(&target, bytes).map_err(io_error)?;
            written = written.saturating_add(1);
        }
        let manifest = self.entries.get(craft_pcraft_format::MANIFEST).ok_or_else(|| Error::InvalidBundle("manifest not set".into()))?;
        let manifest_path = root.join(craft_pcraft_format::MANIFEST);
        refuse_symlink(&manifest_path)?;
        craft_atomic::atomic_write(&manifest_path, manifest).map_err(io_error)?;
        written = written.saturating_add(1);

        let mut removed = 0usize;
        for kind in [craft_pcraft_format::TILES_DIR, craft_pcraft_format::BLOBS_DIR] {
            let dir = root.join(kind);
            if !dir.exists() {
                continue;
            }
            refuse_symlink(&dir)?;
            for entry in fs::read_dir(&dir).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let path = format!("{kind}/{name}");
                if valid_entry_path(&path) && !self.entries.contains_key(&path) {
                    let meta = fs::symlink_metadata(entry.path()).map_err(io_error)?;
                    if meta.file_type().is_symlink() || !meta.is_file() {
                        return Err(Error::InvalidBundle(format!("refusing to collect non-file object {path}")));
                    }
                    fs::remove_file(entry.path()).map_err(io_error)?;
                    removed = removed.saturating_add(1);
                }
            }
        }
        Ok(DirectoryStats { written, removed })
    }
}

/// Work performed by a directory bundle save.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectoryStats {
    /// Entries atomically written, including `manifest.json`.
    pub written: usize,
    /// Stale content-addressed objects collected after manifest publication.
    pub removed: usize,
}

fn io_error(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

fn refuse_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::InvalidBundle(format!("refusing symlink {}", path.display()))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

fn ensure_directory(path: &Path) -> Result<()> {
    refuse_symlink(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(Error::InvalidBundle(format!("{} is not a directory", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(io_error)?;
            Ok(())
        }
        Err(error) => Err(io_error(error)),
    }
}

fn safe_target(root: &Path, relative: &str) -> Result<PathBuf> {
    if !valid_entry_path(relative) {
        return Err(Error::InvalidPath(relative.to_owned()));
    }
    let mut target = root.to_path_buf();
    let components = relative.split('/').collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        target.push(component);
        if index + 1 < components.len() {
            ensure_directory(&target)?;
        }
    }
    Ok(target)
}

/// Opens a directory bundle and reads canonical entries with caller-provided limits.
pub struct DirectoryBundle {
    root: PathBuf,
    limits: Limits,
}

impl DirectoryBundle {
    /// Opens a bundle directory containing a regular `manifest.json`.
    pub fn open(root: &Path, limits: Limits) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(io_error)?;
        let manifest = root.join(craft_pcraft_format::MANIFEST);
        refuse_symlink(&manifest)?;
        let metadata = fs::metadata(&manifest).map_err(io_error)?;
        if !metadata.is_file() || metadata.len() > limits.max_manifest_bytes {
            return Err(Error::LimitExceeded { entry: craft_pcraft_format::MANIFEST.into(), limit: limits.max_manifest_bytes });
        }
        Ok(Self { root, limits })
    }

    /// Reads the serialized document manifest.
    pub fn manifest(&self) -> Result<Vec<u8>> {
        self.read(craft_pcraft_format::MANIFEST, self.limits.max_manifest_bytes)
    }

    /// Reads an optional preview entry.
    pub fn preview(&self, path: &str) -> Result<Option<Vec<u8>>> {
        if !matches!(path, craft_pcraft_format::THUMB | craft_pcraft_format::COMPOSITE | craft_pcraft_format::FULL_COMPOSITE) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
        self.read_optional(path, self.limits.max_object_bytes)
    }

    /// Reads and verifies a compressed content-addressed object.
    pub fn object(&self, path: &str) -> Result<Vec<u8>> {
        let compressed = self.read(path, self.limits.max_object_bytes)?;
        decode_object(path, &compressed, self.limits.max_decoded_object_bytes)
    }

    fn read(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        self.read_optional(path, limit)?.ok_or_else(|| Error::InvalidBundle(format!("missing {path}")))
    }

    fn read_optional(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>> {
        let target = safe_target(&self.root, path)?;
        refuse_symlink(&target)?;
        let metadata = match fs::metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        if !metadata.is_file() || metadata.len() > limit {
            return Err(Error::LimitExceeded { entry: path.to_owned(), limit });
        }
        let bytes = fs::read(target).map_err(io_error)?;
        if bytes.len() as u64 > limit {
            return Err(Error::LimitExceeded { entry: path.to_owned(), limit });
        }
        Ok(Some(bytes))
    }
}
