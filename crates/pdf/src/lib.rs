//! Shared optional PDF reading, rendering and writing for Craft applications.
//!
//! Applications retain document models, layout, shaping, font policy and scheduling.
//! Advanced backend re-exports are coupled to the pinned backend, not a universal document model.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "cos")]
pub mod cos;
#[cfg(feature = "crypt")]
pub mod crypt;
#[cfg(feature = "filters")]
pub mod filters;
#[cfg(feature = "interpret")]
pub use hayro_interpret as interpret;
mod link;
pub mod settings;
#[cfg(feature = "read")]
pub use hayro_syntax as syntax;
pub use link::safe_url;

#[cfg(feature = "read")]
mod read;
#[cfg(feature = "read")]
pub use read::*;
#[cfg(feature = "paths")]
pub mod paths;
#[cfg(feature = "raster")]
pub mod raster;
#[cfg(all(feature = "cos", any(test, feature = "test-support")))]
pub mod testing;
#[cfg(feature = "write")]
pub mod write;
#[cfg(feature = "write-raster")]
#[path = "write/raster.rs"]
pub mod write_raster;

/// Low-level, direct-to-output framing of new classic-xref PDF files.
pub mod classic;
/// PDF single-byte encodings (ISO 32000-2 Annex D), WinAnsi/Unicode CMap helpers, and Standard-14 metrics.
#[cfg(feature = "fonts")]
pub mod fonts;

#[cfg(test)]
extern crate self as craft_pdf;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
