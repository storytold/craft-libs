//! The preferences model: the rows the host builds each frame, the trait the host implements, the search and the
//! settings document that adapts an app's settings struct to them.
//!
//! - [`PageSpec`], [`GroupSpec`], [`RowSpec`], [`RowKind`], [`PrefValue`]: the row model the host builds each frame
//!   from its settings and its draft. The library keeps no copy of the values.
//! - [`PreferencesModel`]: what the app implements: pages, draft, apply, cancel, reset, custom rows and nested dialogs.
//! - [`PreferencesWords`]: every caption of the window.
//! - [`SettingsDocument`]: a [`PreferencesModel`] built from the app's settings struct, one row per setting by type,
//!   labels from the key, with [`SettingRule`]s and the [`StandardPage`]s.
//! - [`matches`](fn@matches), [`list_pages`], [`row_shown`], [`shown_position`]: the search rule on the shared matcher,
//!   public so hosts and tests share it.

mod words;

// The model, the trait, the search and the settings document.
mod document;
mod model;
mod search;

pub use document::{CustomSettings, PageInfo, RuleKind, SettingRule, SettingsDocument, SettingsDraft, StandardPage, setting_label};
pub use model::{Closed, Columns, GroupSpec, PageSpec, PrefValue, PreferencesModel, PreferencesOutput, RowKind, RowSpec};
pub use search::{Listed, list_pages, matches, row_matches, row_shown, shown_position};
pub use words::{PreferencesWords, SettingsDocumentWords};

// The window, its body and its rows.
mod body;
mod rows;
mod window;

pub use window::{PreferencesState, PreferencesWindow};
