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
//! composite/full.png       optional full-canvas flattened image
//! save.lock                directory bundles only: held while a save runs (empty)
//! ```
//!
//! A bundle is a ZIP archive or a directory with this layout. [`Manifest`] holds the format
//! version, the writer and the preview paths, with the document as the type parameter `D`, so
//! the document model stays in the app. [`TileRef`] is a tile's place in a surface.
//! [`read_manifest`] reads manifest JSON: it checks the JSON against [`Limits`], refuses a version
//! newer than the one the caller reads, upgrades older versions with the caller's steps, and reads
//! the envelope and the document.
//!
//! The crate does not read ZIP archives, open files or decode tiles. It builds for
//! `wasm32-unknown-unknown`.
//!
//! ```
//! use craft_pcraft_format::{FORMAT_VERSION, Limits, Manifest, Step};
//!
//! let raw = br#"{"format_version":1,"generator":"photocraft-format 0.1.0","document":{"name":"Untitled"}}"#;
//! let steps: &[Step] = &[];
//! let manifest: Manifest<serde_json::Value> = craft_pcraft_format::read_manifest(raw, &Limits::default(), steps, FORMAT_VERSION).unwrap();
//! assert_eq!(manifest.document["name"], "Untitled");
//! assert_eq!(manifest.thumbnail, None);
//! ```

mod limits;
mod migrate;
mod read;

use serde::{Deserialize, Serialize};

pub use limits::{
    Limits, MAX_DEPTH, MAX_GROUP_DEPTH, MAX_MANIFEST_ITEMS, check_manifest, check_manifest_with_cancel, manifest_depth, parse_manifest,
};
pub use migrate::{Step, migrate};
pub use read::{read_manifest, read_manifest_with};

/// The manifest version PhotoCraft writes.
pub const FORMAT_VERSION: u32 = 1;

/// File extension of the bundle, without the dot.
pub const EXTENSION: &str = "pcraft";

/// The directory of the tiles.
pub const TILES_DIR: &str = "tiles";
/// The directory of the blobs.
pub const BLOBS_DIR: &str = "blobs";
/// The directory of the flattened preview, [`COMPOSITE`].
pub const COMPOSITE_DIR: &str = "composite";
/// The suffix of every tile and blob entry, after the hash.
pub const OBJECT_SUFFIX: &str = ".zst";

/// The manifest entry.
pub const MANIFEST: &str = "manifest.json";
/// The thumbnail entry, a PNG.
pub const THUMB: &str = "thumb.png";
/// The flattened preview entry, a PNG, in [`COMPOSITE_DIR`].
pub const COMPOSITE: &str = "composite/preview.png";
/// The optional full-canvas flattened image, a PNG, in [`COMPOSITE_DIR`].
pub const FULL_COMPOSITE: &str = "composite/full.png";
/// The file a directory save holds an exclusive lock on while it runs. ZIP bundles do not have it.
pub const SAVE_LOCK: &str = "save.lock";

/// Hex-encoded BLAKE3 hash (64 lowercase hex digits) of an object's uncompressed bytes.
pub type Hash = String;

/// The entry of the tile with hash `hash`: `tiles/<hash>.zst`. `None` when [`is_valid_hash`]
/// refuses `hash`.
pub fn tile_path(hash: &str) -> Option<String> {
    is_valid_hash(hash).then(|| format!("{TILES_DIR}/{hash}{OBJECT_SUFFIX}"))
}

/// The entry of the blob with hash `hash`: `blobs/<hash>.zst`. `None` when [`is_valid_hash`]
/// refuses `hash`.
pub fn blob_path(hash: &str) -> Option<String> {
    is_valid_hash(hash).then(|| format!("{BLOBS_DIR}/{hash}{OBJECT_SUFFIX}"))
}

/// `true` when `hash` is 64 lowercase hex digits, the only form a valid manifest gives.
/// [`tile_path`] and [`blob_path`] build no entry name for any other string.
pub fn is_valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `manifest.json`: the envelope around the app's document `D`.
///
/// The fields serialize in this order, and `thumbnail` and `composite` serialize as `null` when
/// absent. [`read_manifest`] reads one: a missing `thumbnail` or `composite` reads as `None`,
/// members the type does not name are skipped, and a preview that names any entry other than
/// [`THUMB`] or [`COMPOSITE`] is refused.
/// `full_composite` names [`FULL_COMPOSITE`] when present and is omitted on serialization
/// when absent, preserving the bytes of manifests without it.
///
/// The struct is `#[non_exhaustive]`: build one with [`Manifest::new`] and set the previews.
///
/// ```compile_fail
/// let _ = craft_pcraft_format::Manifest { format_version: 1, generator: String::new(), document: (), thumbnail: None, composite: None };
/// ```
///
/// It has no `Deserialize`, so a manifest is read only through [`read_manifest`]:
///
/// ```compile_fail
/// let _: craft_pcraft_format::Manifest<serde_json::Value> = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Manifest<D> {
    pub format_version: u32,
    /// Free-form writer identification, for example `photocraft-format 0.1.0`.
    pub generator: String,
    pub document: D,
    /// The thumbnail entry, [`THUMB`]. [`read_manifest`] refuses any other name.
    pub thumbnail: Option<String>,
    /// The flattened preview entry, [`COMPOSITE`]. [`read_manifest`] refuses any other name.
    pub composite: Option<String>,
    /// The full-canvas flattened image, [`FULL_COMPOSITE`]. [`read_manifest`] refuses any other name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_composite: Option<String>,
}

impl<D> Manifest<D> {
    /// A manifest at [`FORMAT_VERSION`] written by `generator`, holding `document`, with no
    /// previews.
    pub fn new(generator: impl Into<String>, document: D) -> Self {
        Manifest { format_version: FORMAT_VERSION, generator: generator.into(), document, thumbnail: None, composite: None, full_composite: None }
    }
}

/// One tile of a surface: its column `tx`, its row `ty` and the hash of its samples, stored at
/// [`tile_path`]. `hash` is read as written; [`TileRef::path`] checks it.
///
/// The struct is `#[non_exhaustive]`: build one with [`TileRef::new`].
///
/// ```compile_fail
/// let _ = craft_pcraft_format::TileRef { tx: 0, ty: 0, hash: String::new() };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TileRef {
    pub tx: i32,
    pub ty: i32,
    pub hash: Hash,
}

impl TileRef {
    /// The tile at column `tx` and row `ty` whose samples hash to `hash`.
    pub fn new(tx: i32, ty: i32, hash: Hash) -> Self {
        TileRef { tx, ty, hash }
    }

    /// The tile's entry, [`tile_path`] of its `hash`: `None` when the hash is not valid.
    pub fn path(&self) -> Option<String> {
        tile_path(&self.hash)
    }
}

/// Why a manifest could not be read.
///
/// [`Error::TooNew`] is `#[non_exhaustive]`: match it with `..`.
///
/// ```compile_fail
/// fn found(e: craft_pcraft_format::Error) -> u32 {
///     match e {
///         craft_pcraft_format::Error::TooNew { found, supported: _, generator: _ } => found,
///         _ => 0,
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A cancellation-aware validation observed the caller's request.
    #[error("manifest check canceled")]
    Cancelled,
    /// The manifest breaks the format.
    #[error("corrupt .pcraft bundle: {0}")]
    Corrupt(String),
    /// The manifest needs a migration step the caller did not pass.
    #[error("unsupported .pcraft feature: {0}")]
    Unsupported(String),
    /// The manifest's `format_version` is newer than the version the caller reads.
    #[error("bundle written by a newer version (format {found}, this build reads up to {supported})")]
    #[non_exhaustive]
    TooNew {
        /// The manifest's `format_version`.
        found: u32,
        /// The version the caller reads.
        supported: u32,
        /// The manifest's `generator`, so a message can name the writer. `None` when it is
        /// missing or not a string.
        generator: Option<String>,
    },
    /// The manifest passes one of the caller's [`Limits`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// The manifest is not valid JSON, or a member does not have the type the reader expects.
    #[error("manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}
