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
//! use craft_theme::{AppDefaults, Mode, ThemeId};
//! use craft_theme_store::{SelectionChange, Store, StoreOptions, SuiteDir};
//!
//! let mut store = Store::open(&SuiteDir::memory(), StoreOptions::new("doc 1.0"));
//! store.load_selection().unwrap();
//! assert!(store.accepted().suite.is_empty()); // no app has chosen anything yet
//!
//! // The user picks Graphite: only the theme is written; the mode stays each app's own default.
//! let base = store.accepted().clone();
//! let graphite = ThemeId::parse("craft.graphite").unwrap();
//! let accepted = store.update(&base, SelectionChange::theme(graphite.clone())).unwrap();
//! assert_eq!((accepted.suite.theme.as_ref(), accepted.suite.mode, accepted.revision), (Some(&graphite), None, 1));
//!
//! let defaults = AppDefaults { theme: ThemeId::parse("craft.artcraft").unwrap(), mode: Mode::Dark };
//! assert_eq!(accepted.suite.with_defaults(&defaults).mode, Mode::Dark);
//! ```

mod backend;
mod dir;
#[cfg(not(target_arch = "wasm32"))]
mod disk;
mod format;
mod selection;
mod store;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod watch;

pub use backend::{FileStamp, LibraryFile};
pub use dir::{DirSource, SUITE_DIR_ENV, SuiteDir};
pub use format::{FormatError, MAX_THEME_BYTES, THEME_SCHEMA, THEME_SCHEMA_VERSION, ThemeRecord, parse_theme, theme_to_json};
pub use selection::{Accepted, MAX_SELECTION_BYTES, SELECTION_SCHEMA_VERSION, SelectionChange, SelectionError, SuiteChoice};
pub use store::{Imported, LOCK_TIMEOUT, Library, ParseCounts, Problem, Store, StoreError, StoreOptions, ThemeEntry};
pub use watch::{Op, OpOutput, RESULTS_KEPT, STAT_INTERVAL, Snapshot, SuiteWatch, VERIFY_INTERVAL};

#[cfg(test)]
mod tests;
