//! Reads SPP project containers, records, and layer models as data an application can translate into its own document.
//! The decoder also reads embedded compiled material graphs and translates them to graph text.
//!
//! The crate is layered; each layer only uses the ones above it in this list:
//! - [`container`]: generic file formats (HDF5, Qt QDataStream, byte cursors). Knows nothing about the reference app.
//! - [`decode`]: the reference app's individual records (settings, resource images, texture cache, mesh scene, document).
//! - [`model`]: the reference app's own concepts (texture sets, layers, effects, sources, strokes) built from `document.bin`.
//! - [`project`]: one opened project file, giving typed access to its parts.
//! - [`inspect`]: human-readable reports about one project or a whole folder of them.
//! - [`blockers`]: the shared vocabulary for what an importer can't evaluate yet (`graph:<graph>`,
//!   `brush-alpha:<name>`, …), so importers, reports and corpus censuses can name the same things the same way.
//!
//! Everything is read from the user's own files at run time; the crate contains no code, data or resources of the reference app.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blockers;
pub mod container;
pub mod decode;
pub mod decompile;
pub mod format_names;
pub mod inspect;
pub mod model;
pub mod project;

mod inflate;
mod png_out;
#[cfg(test)]
mod robustness_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_corpus;
