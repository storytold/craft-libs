#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The Craft suite's theme files, its machine-local theme library and the shared suite
//! selection, in the folder `ArtCraft/themes`:
//!
//! ```text
//! selection.json                     suite selection and revisions
//! selection.lock                     lock file, never deleted
//! library/<id>.json                  one user theme per file
//! selection.broken-<unix-ts>.json    a damaged selection, moved aside on the next write
//! ```
//!
//! Writes follow a lock/revision protocol so that several apps can change the selection and the
//! library at once without losing a change: see [`Store`]. Without a folder (tests, isolated
//! runs, web builds) the same store keeps everything in memory. No egui; the theme contract is
//! `craft-theme`.
//!
//! ```
//! use craft_theme::{ThemeId, AppDefaults, ModeChoice, Size, ThemePair};
//! use craft_theme_store::{SelectionChange, Store, StoreOptions, SuiteDir};
//!
//! let mut store = Store::open(&SuiteDir::memory(), StoreOptions::new("doc 1.0"));
//! store.load_selection().unwrap();
//! assert!(store.accepted().suite.is_empty()); // no app has chosen anything yet
//!
//! // The user picks Graphite, which has both variants: both theme slots are written; the mode
//! // stays each app's own default.
//! let base = store.accepted().clone();
//! let graphite = store.theme(&ThemeId::parse("craft.graphite").unwrap()).unwrap();
//! let accepted = store.update(&base, SelectionChange::pick_theme(&graphite)).unwrap();
//! assert_eq!((accepted.suite.theme_dark.as_ref(), accepted.suite.mode, accepted.revision), (Some(&graphite.id), None, 1));
//!
//! let artcraft = ThemeId::parse("craft.artcraft").unwrap();
//! let defaults = AppDefaults { theme: ThemePair::same(artcraft), mode: ModeChoice::Dark, size: Size::Compact, sizes: Default::default() };
//! let selection = accepted.suite.with_defaults(&defaults);
//! assert_eq!((selection.theme.light, selection.mode), (graphite.id, ModeChoice::Dark));
//! ```

mod backend;
mod client;
mod copy;
mod dir;
#[cfg(not(target_arch = "wasm32"))]
mod disk;
mod selection;
mod store;
pub mod testing;
#[cfg(feature = "ui")]
pub mod ui;
mod watch;

pub use backend::{FileStamp, LibraryFile};
pub use client::SuiteClient;
/// Theme files: [`format::parse`], [`format::write`] and their errors, which
/// [`StoreError::Format`] carries.
pub use craft_theme::format;
pub use craft_theme::format::{FormatError, FormatErrors, MAX_THEME_BYTES};
pub use dir::{DirSource, SUITE_DIR_ENV, SuiteDir};
pub use selection::{Accepted, MAX_SELECTION_BYTES, SELECTION_SCHEMA_VERSION, SelectionChange, SelectionError};
pub use store::{Imported, LOCK_TIMEOUT, Library, ParseCounts, Problem, Store, StoreError, StoreOptions, ThemeEntry, ThemeRecord};
pub use watch::{Op, OpOutput, RESULTS_KEPT, STAT_INTERVAL, Snapshot, SuiteWatch, VERIFY_INTERVAL};

#[cfg(test)]
mod tests;

#[cfg(test)]
extern crate self as craft_theme_store;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;
#[cfg(test)]
#[path = "../tests/two_processes.rs"]
mod two_processes;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
