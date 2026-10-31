#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The envelope of PhotoCraft's `.pcraft` bundle, shared by PhotoCraft and by readers in other
//! apps.
//!
//! ```text
//! manifest.json            versioned manifest (see [`Manifest`])
//! tiles/<blake3>.zst       content-addressed 256×256 tiles (zstd, little-endian samples)
//! blobs/<blake3>.zst       content-addressed binary data
//! thumb.png                optional thumbnail
//! composite/preview.png    optional flattened preview
//! save.lock                directory bundles only: held while a save runs (empty)
//! ```
//!
//! A bundle is a ZIP archive or a directory with this layout. [`Manifest`] holds the format
//! version, the writer and the preview paths, with the document as the type parameter `D`, so
//! the document model stays in the app. [`TileRef`] is a tile's place in a surface. The manifest
//! JSON is checked against [`Limits`] and parsed with [`parse_manifest`], and [`migrate`] upgrades
//! it to the version the caller reads with the caller's steps.
//!
//! The crate does not read ZIP archives, open files or decode tiles. It builds for
//! `wasm32-unknown-unknown`.
//!
//! ```
//! use craft_pcraft_format::{FORMAT_VERSION, Limits, Manifest};
//!
//! let raw = br#"{"format_version":1,"generator":"photocraft-format 0.1.0","document":{"name":"Untitled"}}"#;
//! let mut value = craft_pcraft_format::parse_manifest(raw, &Limits::default()).unwrap();
//! let steps: &[craft_pcraft_format::Step] = &[];
//! craft_pcraft_format::migrate(&mut value, steps, FORMAT_VERSION).unwrap();
//! let manifest: Manifest<serde_json::Value> = serde_json::from_value(value).unwrap();
//! assert_eq!(manifest.document["name"], "Untitled");
//! assert_eq!(manifest.thumbnail, None);
//! ```

mod limits;
mod migrate;

use serde::{Deserialize, Serialize};

pub use limits::{Limits, MAX_DEPTH, MAX_GROUP_DEPTH, MAX_MANIFEST_ITEMS, check_manifest, manifest_depth, parse_manifest};
pub use migrate::{Step, migrate};

/// The manifest version PhotoCraft writes.
pub const FORMAT_VERSION: u32 = 1;

/// File extension of the bundle, without the dot.
pub const EXTENSION: &str = "pcraft";

/// The manifest entry.
pub const MANIFEST: &str = "manifest.json";
/// The thumbnail entry, a PNG.
pub const THUMB: &str = "thumb.png";
/// The flattened preview entry, a PNG.
pub const COMPOSITE: &str = "composite/preview.png";
/// The file a directory save holds an exclusive lock on while it runs. ZIP bundles do not have it.
pub const SAVE_LOCK: &str = "save.lock";

/// Hex-encoded BLAKE3 hash (64 lowercase hex digits) of an object's uncompressed bytes.
pub type Hash = String;

/// The entry of the tile with hash `hash`: `tiles/<hash>.zst`.
pub fn tile_path(hash: &str) -> String {
    format!("tiles/{hash}.zst")
}

/// The entry of the blob with hash `hash`: `blobs/<hash>.zst`.
pub fn blob_path(hash: &str) -> String {
    format!("blobs/{hash}.zst")
}

/// `true` when `hash` is 64 lowercase hex digits, the only form [`tile_path`] and [`blob_path`]
/// are given in a valid manifest. Check a hash from a file with it before building an entry name.
pub fn is_valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `manifest.json`: the envelope around the app's document `D`.
///
/// The fields serialize in this order, and `thumbnail` and `composite` serialize as `null` when
/// absent. A missing `thumbnail` or `composite` reads as `None`, and members the type does not
/// name are skipped on read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest<D> {
    pub format_version: u32,
    /// Free-form writer identification, for example `photocraft-format 0.1.0`.
    pub generator: String,
    pub document: D,
    /// The thumbnail entry, normally [`THUMB`].
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// The flattened preview entry, normally [`COMPOSITE`].
    #[serde(default)]
    pub composite: Option<String>,
}

/// One tile of a surface: its column `tx`, its row `ty` and the hash of its samples, stored at
/// [`tile_path`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileRef {
    pub tx: i32,
    pub ty: i32,
    pub hash: Hash,
}

/// Why a manifest could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The manifest breaks the format.
    #[error("corrupt .pcraft bundle: {0}")]
    Corrupt(String),
    /// The manifest needs a migration step the caller did not pass.
    #[error("unsupported .pcraft feature: {0}")]
    Unsupported(String),
    /// The manifest's `format_version` is newer than the version the caller reads.
    #[error("bundle written by a newer version (format {found}, this build reads up to {supported})")]
    TooNew { found: u32, supported: u32 },
    /// The manifest passes one of the caller's [`Limits`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// The manifest is not valid JSON.
    #[error("manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}
