#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The suite's font store: the font indexes of every source (Google Fonts, Noto, indexes the user
//! adds), exact lookup of a missing font, verified installs into the one folder every app's font
//! catalog reads, and, per family at the user's request, copies in the OS user font folder for
//! every other program ([`FontStore::make_available`]).
//!
//! ```no_run
//! # #[cfg(not(target_arch = "wasm32"))] {
//! use craft_font_store::{Cancel, FontStore, Index, PINNED_INDEX};
//! use craft_font_catalog::{Catalog, FontEnv};
//!
//! let store = FontStore::for_user(&FontEnv::from_process()).unwrap().with_catalog(Catalog::global().clone());
//! let index = store.cached_indexes(&PINNED_INDEX).unwrap();
//! if let Ok(face) = index.find_face("Lato", "Bold Italic") {
//!     store.enable_network().unwrap(); // the user clicked Install
//!     store.install_face(&index, face, &Cancel::new(), &mut |p| println!("{}/{}", p.done, p.total)).unwrap();
//! }
//! # }
//! ```
//!
//! The index and its lookups, [`verify_font`] and [`IndexPin`] build on every target. The store,
//! the HTTP client, the preview service's client ([`preview`]) and Language packs (the store's
//! second item kind, `language-pack`: see [`FontStore::install_language_pack`]) are native only.

mod index;
mod pin;
mod service;
mod source;
mod sources;
mod verify;

#[cfg(not(target_arch = "wasm32"))]
mod http;
#[cfg(not(target_arch = "wasm32"))]
mod language_packs;
#[cfg(not(target_arch = "wasm32"))]
pub mod preview;
#[cfg(not(target_arch = "wasm32"))]
mod store;

pub use index::{
    Axis, Face, Family, FontFile, FontSource, GOOGLE_FONTS, Index, IndexError, Instance, LICENSES, Language, LicenseFile, MAJOR_VERSION,
    MAX_FILE_BYTES, MAX_INDEX_BYTES, MAX_LICENSE_BYTES, Sample, Script, SourceProblem, Unavailable, parse_style,
};
pub use pin::{IndexPin, PINNED_INDEX};
pub use service::{Conflict, LegalText, PREVIEW_SERVICE_ENV, PreviewServiceSource, PreviewStore};
pub use sources::{SourceEntry, SourcesFile};
pub use verify::{VerifyError, is_font, sha256_hex, verify_font};

/// A source the user added: an index file at an https URL, never curated. Plain data, so it builds on every target
/// for the interfaces that list the sources.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UserSource {
    /// The index file (format 1.x, JSON or gzipped JSON).
    pub url: String,
    /// When it was added, in seconds since 1970.
    pub added_at: u64,
    /// The sha256 of the file first fetched from it. A different file later shows as
    /// [`FontSource::changed`].
    #[serde(default)]
    pub last_sha256: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub use http::{Cancel, Client, FetchError, SERVICE_BACKOFF, Timeouts, USER_AGENT};
#[cfg(not(target_arch = "wasm32"))]
pub use language_packs::{
    InstalledLanguagePack, LANGUAGE_PACK_INDEX_ENV, LANGUAGE_PACKS, MAX_PACK_ARCHIVE_BYTES, MAX_PACK_ENTRIES, MAX_PACK_FILE_BYTES,
    MAX_PACK_INDEX_BYTES, MAX_PACK_UNPACKED_BYTES, PackEntry, PackIndex, PackIndexPin, language_pack_index_overridden,
};
#[cfg(not(target_arch = "wasm32"))]
pub use store::{
    Caller, Error, FontStore, InstalledFamily, InstalledFile, Network, OsFontFolder, OsFontLayout, Problem, Progress, check_preview_service,
};

#[cfg(test)]
mod tests;
