#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Container and content-addressed object storage for `.pcraft` bundles.
//!
//! Document schemas stay with the application. This crate owns the wire-level paths, ZIP
//! envelope, zstd object frames and content hashes. A caller supplies serialized manifest bytes
//! and opaque tile/blob bytes; it receives their stable paths to put in its manifest.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

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
    let hash = hash_content(bytes);
    let path = match kind {
        ObjectKind::Tile => craft_pcraft_format::tile_path(&hash),
        ObjectKind::Blob => craft_pcraft_format::blob_path(&hash),
    }
    .unwrap_or_default();
    let compressed = encode_frame(bytes);
    StoredObject { path, hash, compressed }
}

/// BLAKE3 hash of object bytes, in the canonical lowercase hexadecimal form used by entry paths.
pub fn hash_content(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Encodes one bounded zstd frame at the format's fixed fastest compression level.
pub fn encode_frame(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

/// Decodes one zstd frame under a byte limit and optional exact output length.
pub fn decode_frame(data: &[u8], max_bytes: usize, exact_bytes: Option<usize>, name: &str) -> Result<Vec<u8>> {
    let window = u64::try_from(exact_bytes.unwrap_or(max_bytes)).unwrap_or(u64::MAX).max(8 << 20);
    let mut decoder = match ruzstd::decoding::StreamingDecoder::new_with_max_window_size(data, window) {
        Ok(decoder) => decoder,
        Err(ruzstd::decoding::errors::FrameDecoderError::WindowSizeTooBig { requested, .. }) => {
            return Err(Error::LimitExceeded { entry: format!("{name} zstd window ({requested})"), limit: window });
        }
        Err(error) => return Err(Error::Io(format!("{name}: zstd: {error}"))),
    };
    let mut out = Vec::new();
    let read_limit = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
    (&mut decoder).take(read_limit).read_to_end(&mut out).map_err(|error| Error::Io(format!("{name}: zstd: {error}")))?;
    if out.len() > max_bytes {
        return Err(Error::LimitExceeded { entry: name.to_owned(), limit: max_bytes as u64 });
    }
    if let Some(expected) = exact_bytes
        && out.len() != expected
    {
        return Err(Error::InvalidBundle(format!("{name}: expected {expected} bytes, got {}", out.len())));
    }
    Ok(out)
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
    if hash_content(&bytes) != expected {
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

    /// Reads any canonical bundle entry, including the stored zstd bytes of an object.
    pub fn entry(&mut self, path: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        if !valid_entry_path(path) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
        self.archive.read(path, max_bytes).map_err(Into::into)
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
        self.stage_compressed_object(path, compressed)
    }

    /// Stages an already-compressed object for a bundle write. Its path is checked now and its
    /// frame/hash are verified by [`Self::finish`] or [`DirectoryBundleWriter::save_directory`].
    pub fn stage_compressed_object(&mut self, path: &str, compressed: Vec<u8>) -> Result<()> {
        if !valid_object_path(path) {
            return Err(Error::InvalidPath(path.to_owned()));
        }
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
            if is_object_path(&path) {
                let _ = decode_object(&path, &bytes, 1 << 30)?;
            }
            writer.add(&path, &bytes, Compression::Stored)?;
        }
        writer.finish().map_err(Into::into)
    }

    /// Saves the same entries as a directory bundle. Object files are published before the
    /// manifest, the bundle lock serializes writers, and unreferenced canonical objects are
    /// collected after the new manifest is durable.
    pub fn save_directory(self, root: &Path) -> Result<DirectoryStats> {
        DirectoryBundleWriter::default().save_directory(root, self)
    }
}

/// Work performed by a directory bundle save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryStats {
    /// Entries atomically written, including `manifest.json`.
    pub written: usize,
    /// Stale content-addressed objects collected after manifest publication.
    pub removed: usize,
    /// Existing, verified content-addressed objects reused without writing.
    pub reused: usize,
    /// Canonical object paths written during this save.
    pub written_objects: Vec<String>,
    /// Canonical object paths reused after verification or a matching cached signature.
    pub reused_objects: Vec<String>,
}

/// Incremental directory-bundle persistence with bounded rolling object verification.
pub struct DirectoryBundleWriter {
    root: Option<PathBuf>,
    generation: u64,
    reverify_budget: u64,
    lock_wait: Duration,
    verified: HashMap<String, CachedFile>,
}

impl Default for DirectoryBundleWriter {
    fn default() -> Self {
        Self { root: None, generation: 0, reverify_budget: 16 << 20, lock_wait: Duration::from_secs(10), verified: HashMap::new() }
    }
}

#[derive(Clone, Copy)]
struct CachedFile {
    signature: FileSignature,
    checked_at: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    unix: (u64, u64, i64, i64),
}

impl DirectoryBundleWriter {
    /// Compressed bytes of already-verified objects to re-read on each save.
    /// `0` trusts unchanged signatures; `u64::MAX` rechecks all referenced objects.
    pub fn set_reverify_budget(&mut self, bytes: u64) {
        self.reverify_budget = bytes;
    }

    /// How long to wait for another writer to release the bundle lock.
    pub fn set_lock_wait(&mut self, wait: Duration) {
        self.lock_wait = wait;
    }

    /// Saves an entry set incrementally. Existing objects are reused only after a hash check;
    /// unchanged file signatures may skip checks until they enter the rolling verification
    /// window. The manifest is atomically published last, then stale previews and objects are
    /// collected while the bundle lock is held.
    pub fn save_directory(&mut self, root: &Path, bundle: ZipBundleWriter) -> Result<DirectoryStats> {
        if !bundle.entries.contains_key(craft_pcraft_format::MANIFEST) {
            return Err(Error::InvalidBundle("manifest not set".into()));
        }
        for path in bundle.entries.keys() {
            if !valid_entry_path(path) {
                return Err(Error::InvalidPath(path.clone()));
            }
        }
        fs::create_dir_all(root).map_err(io_error)?;
        let root = fs::canonicalize(root).map_err(io_error)?;
        if self.root.as_ref() != Some(&root) {
            self.root = Some(root.clone());
            self.verified.clear();
            self.generation = 0;
        }
        let lock_path = root.join(craft_pcraft_format::SAVE_LOCK);
        refuse_symlink(&lock_path)?;
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path).map_err(io_error)?;
        let start = Instant::now();
        let _lock = loop {
            match lock.try_lock() {
                Ok(()) => break Some(lock),
                Err(std::fs::TryLockError::WouldBlock) if start.elapsed() < self.lock_wait => std::thread::sleep(Duration::from_millis(20)),
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(Error::Io("another save of this bundle is in progress".into()));
                }
                Err(std::fs::TryLockError::Error(_)) => break None,
            }
        };
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let recheck = self.rolling_recheck(&bundle.entries);
        let mut stats = DirectoryStats::default();

        for (path, bytes) in bundle.entries.iter().filter(|(path, _)| is_object_path(path)) {
            let target = safe_target(&root, path)?;
            if let Some(parent) = target.parent() {
                ensure_directory(parent)?;
            }
            refuse_symlink(&target)?;
            if let Some(signature) = file_signature(&target)? {
                let cached = self.verified.get(path).is_some_and(|entry| entry.signature == signature);
                if cached && !recheck.contains(path) {
                    stats.reused = stats.reused.saturating_add(1);
                    stats.reused_objects.push(path.clone());
                    continue;
                }
                let existing = read_bounded(&target, 1 << 30)?;
                if decode_object(path, &existing, 1 << 30).is_ok() {
                    if let Some(signature) = file_signature(&target)? {
                        self.verified.insert(path.clone(), CachedFile { signature, checked_at: generation });
                    }
                    stats.reused = stats.reused.saturating_add(1);
                    stats.reused_objects.push(path.clone());
                    continue;
                }
            }
            let _ = decode_object(path, bytes, 1 << 30)?;
            craft_atomic::atomic_write(&target, bytes).map_err(io_error)?;
            if let Some(signature) = file_signature(&target)? {
                self.verified.insert(path.clone(), CachedFile { signature, checked_at: generation });
            }
            stats.written = stats.written.saturating_add(1);
            stats.written_objects.push(path.clone());
        }

        for (path, bytes) in bundle.entries.iter().filter(|(path, _)| !is_object_path(path) && path.as_str() != craft_pcraft_format::MANIFEST) {
            let target = safe_target(&root, path)?;
            if let Some(parent) = target.parent() {
                ensure_directory(parent)?;
            }
            refuse_symlink(&target)?;
            craft_atomic::atomic_write(&target, bytes).map_err(io_error)?;
            stats.written = stats.written.saturating_add(1);
        }

        let manifest = bundle.entries.get(craft_pcraft_format::MANIFEST).ok_or_else(|| Error::InvalidBundle("manifest not set".into()))?;
        let manifest_path = root.join(craft_pcraft_format::MANIFEST);
        refuse_symlink(&manifest_path)?;
        craft_atomic::atomic_write(&manifest_path, manifest).map_err(io_error)?;
        stats.written = stats.written.saturating_add(1);

        let mut removed_paths = Vec::new();
        for kind in [craft_pcraft_format::TILES_DIR, craft_pcraft_format::BLOBS_DIR] {
            let dir = root.join(kind);
            if !dir.exists() {
                continue;
            }
            refuse_symlink(&dir)?;
            for entry in fs::read_dir(&dir).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                let name = entry.file_name();
                let path = format!("{kind}/{}", name.to_string_lossy());
                if is_object_path(&path) && !bundle.entries.contains_key(&path) {
                    let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(Error::InvalidBundle(format!("refusing to collect non-file object {path}")));
                    }
                    fs::remove_file(entry.path()).map_err(io_error)?;
                    removed_paths.push(path);
                    stats.removed = stats.removed.saturating_add(1);
                }
            }
        }
        for path in [craft_pcraft_format::THUMB, craft_pcraft_format::COMPOSITE, craft_pcraft_format::FULL_COMPOSITE] {
            if !bundle.entries.contains_key(path) {
                let target = root.join(path);
                refuse_symlink(&target)?;
                if fs::symlink_metadata(&target).is_ok() {
                    fs::remove_file(&target).map_err(io_error)?;
                }
            }
        }
        self.verified.retain(|path, _| bundle.entries.contains_key(path) && !removed_paths.contains(path));
        Ok(stats)
    }

    fn rolling_recheck(&self, entries: &BTreeMap<String, Vec<u8>>) -> HashSet<String> {
        if self.reverify_budget == 0 {
            return HashSet::new();
        }
        let mut cached: Vec<(&String, &CachedFile)> = self.verified.iter().filter(|(path, _)| entries.contains_key(*path)).collect();
        cached.sort_by(|a, b| a.1.checked_at.cmp(&b.1.checked_at).then_with(|| a.0.cmp(b.0)));
        let mut out = HashSet::new();
        let mut spent = 0u64;
        for (path, value) in cached {
            let next = spent.saturating_add(value.signature.len);
            if !out.is_empty() && next > self.reverify_budget {
                break;
            }
            spent = next;
            out.insert(path.clone());
        }
        out
    }
}

fn is_object_path(path: &str) -> bool {
    path.starts_with("tiles/") || path.starts_with("blobs/")
}

fn file_signature(path: &Path) -> Result<Option<FileSignature>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Err(Error::InvalidBundle(format!("{} is not a regular file", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    #[cfg(unix)]
    let unix = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino(), metadata.ctime(), metadata.ctime_nsec())
    };
    Ok(Some(FileSignature {
        len: metadata.len(),
        modified,
        #[cfg(unix)]
        unix,
    }))
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if metadata.len() > max {
        return Err(Error::LimitExceeded { entry: path.display().to_string(), limit: max });
    }
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.len() as u64 > max {
        return Err(Error::LimitExceeded { entry: path.display().to_string(), limit: max });
    }
    Ok(bytes)
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

    /// Reads any canonical bundle entry.
    pub fn entry(&self, path: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        self.read_optional(path, max_bytes)
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
