//! Shared optional PDF reading, rendering and writing for Craft applications.
//!
//! Applications retain document models, layout, shaping, font policy and scheduling.
//! Advanced backend re-exports are coupled to the pinned backend, not a universal document model.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "interpret")]
pub use hayro_interpret as interpret;
#[cfg(feature = "read")]
pub use hayro_syntax as syntax;

#[cfg(feature = "read")]
mod read;
#[cfg(feature = "read")]
pub use read::*;
#[cfg(feature = "raster")]
pub mod raster;
#[cfg(feature = "write")]
pub mod write;
