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
mod embedding;
mod env;
mod face_data;
mod fallback;
mod fontconfig;
mod ligature;
mod line_break;
mod outline;
mod plan_cache;
mod registry;
mod scan;
mod segments;
/// Bounded SFNT face metadata discovery over caller-provided random-access readers.
pub mod sfnt;
mod shape;
mod suitcase;
pub mod system;
mod visual_order;

pub use catalog::{Catalog, FontData, FontFileError, Limits, Lister, LoadedFace, Match, Watch};
pub use embedding::{SubsetEmbedding, subset_embedding};
pub use env::{FontEnv, Os, font_dirs, font_store_dir, office_font_dirs};
pub use face_data::FaceData;
pub use fallback::{cluster_face, drawable_characters};
pub use fontconfig::fontconfig_dirs;
pub use ligature::ligature_caret_stops;
pub use line_break::settle_line_break;
pub use outline::{FontOutline, GlyphPen, OutlineError};
pub use plan_cache::ShapePlanCache;
pub use scan::{FaceInfo, LocalizedName, Source, is_font_file, style_italic, style_weight, style_width};
pub use segments::SegmentBoundary;
pub use shape::{RunGlyph, RunInput, ShapeRunError, shape_run};
pub use visual_order::{VisualGlyph, visual_order};

#[cfg(test)]
mod tests;
