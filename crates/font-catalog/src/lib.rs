#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The installed fonts, for every Craft app: discovery on every platform, lookup by family and
//! style or by any other name, and loading shared between threads.
//!
//! A [`Catalog`] scans the platform's font folders ([`font_dirs`]), the suite's font store
//! ([`font_store_dir`]), the app's own folders ([`Catalog::set_app_dirs`]) and, with the `system`
//! feature, the files the platform's font service knows ([`system`]): CoreText on macOS,
//! DirectWrite on Windows, fontconfig on Linux and FreeBSD. On Windows it also reads the
//! registry's font list. The scan reads each file's table directory and its `name`, `OS/2` and
//! `fvar` tables, never whole files, and a rescan reads only files that changed.
//!
//! ```no_run
//! use craft_font_catalog::{Catalog, Match};
//!
//! let catalog = Catalog::global();
//! catalog.scan_in_background();
//! if let Some((face, Match::Exact)) = catalog.resolve("Source Serif 4", "Bold Italic") {
//!     let data = catalog.load(&face);
//! }
//! ```

mod catalog;
mod env;
mod fontconfig;
mod registry;
mod scan;
mod suitcase;
pub mod system;

pub use catalog::{Catalog, FontData, FontFileError, Limits, Lister, LoadedFace, Match, Scope, Watch};
pub use env::{FontEnv, Os, font_dirs, font_store_dir, office_font_dirs};
pub use fontconfig::fontconfig_dirs;
pub use scan::{FaceInfo, LocalizedName, Source, is_font_file, style_italic, style_weight, style_width};

#[cfg(test)]
mod tests;
