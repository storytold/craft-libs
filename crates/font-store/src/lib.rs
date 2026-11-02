#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The suite's font store: the font indexes of every source (Google Fonts, Noto, indexes the user
//! adds), exact lookup of a missing font, and verified installs into the one folder every app's
//! font catalog reads.
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
//! The index and its lookups, [`verify_font`] and [`IndexPin`] build on every target. The store
//! and the HTTP client are native only.

mod index;
mod pin;
mod source;
mod sources;
mod verify;

#[cfg(not(target_arch = "wasm32"))]
mod http;
#[cfg(not(target_arch = "wasm32"))]
mod store;

pub use index::{
    Axis, Face, Family, FontFile, FontSource, GOOGLE_FONTS, Index, IndexError, Instance, LICENSES, Language, LicenseFile, MAJOR_VERSION,
    MAX_FILE_BYTES, MAX_INDEX_BYTES, MAX_LICENSE_BYTES, Sample, Script, SourceProblem, Unavailable, parse_style,
};
pub use pin::{IndexPin, PINNED_INDEX};
pub use sources::{SourceEntry, SourcesFile};
pub use verify::{VerifyError, is_font, sha256_hex, verify_font};

#[cfg(not(target_arch = "wasm32"))]
pub use http::{Cancel, Client, FetchError, Timeouts, USER_AGENT};
#[cfg(not(target_arch = "wasm32"))]
pub use store::{
    Caller, Error, FontStore, InstalledFamily, InstalledFile, LegalText, Network, PREVIEW_SERVICE_ENV, PreviewServiceSource, Problem, Progress,
    UserSource, check_preview_service,
};

#[cfg(test)]
mod tests;
