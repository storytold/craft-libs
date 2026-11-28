//! The preferences model: the rows the host builds each frame, the trait the host implements, the search and the
//! settings document that adapts an app's settings struct to them.
//!
//! - [`PageSpec`], [`GroupSpec`], [`RowSpec`], [`RowKind`], [`PrefValue`]: the row model the host builds each frame
//!   from its settings and its draft. The library keeps no copy of the values.
//! - [`PreferencesModel`]: what the app implements: pages, edits, reset, custom rows and nested dialogs.
//!   [`PreferencesDraft`] adds the draft's apply, cancel and differs for a window with OK, Cancel and Apply.
//! - [`PreferencesPage`]: the search, page list and shown page, applying each edit to the model at once.
//! - [`PreferencesWindow`]: the page in a modal window with OK, Cancel and Apply over a [`PreferencesDraft`].
//! - [`PreferencesWords`]: every caption of the window.
//! - [`PathSetting`]: a path setting for a custom row or page, typed or picked through the host's file service.
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
pub use model::{Closed, GroupSpec, PageSpec, PrefValue, PreferencesDraft, PreferencesModel, PreferencesOutput, RowKind, RowSpec};
pub use search::{Listed, list_pages, matches, row_matches, row_shown, shown_position};
pub use words::{PreferencesWords, SettingsDocumentWords};

// The page, the window around it and their rows.
pub(crate) mod page;
pub(crate) mod path_setting;
mod rows;
pub(crate) mod window;

pub use page::{PreferencesLayout, PreferencesPage, PreferencesPageOutput};

pub use path_setting::{PathSetting, PathSettingOutput};

pub use window::{DEFAULT_SIZE, MIN_SIZE, PreferencesMode, PreferencesState, PreferencesWindow};
