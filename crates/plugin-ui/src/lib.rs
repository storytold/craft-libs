#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Host-drawn plugin panels for craft-abi 1 plugins.
//!
//! - [`tree`] holds the widget tree a plugin returns from a `panel` invocation, and the validation
//!   that bounds it (node count, depth, unique ids, finite numbers).
//! - [`render`] draws a validated tree with egui and returns the [`UiEvent`]s the user caused.
//! - [`grants`] is the permission prompt model plus its egui view.
//!
//! - `manager` (feature `host`, native only) holds the plug-in manager window, the permission prompt
//!   window and panel windows. The app owns the state and carries out the returned acts.
//!
//! Plugin code never touches egui. The host owns styling and fonts.

pub mod grants;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub mod manager;
pub mod render;
pub mod tree;

pub use grants::{
    Decision, GrantAction, GrantError, GrantOutcome, GrantPrompt, GrantRow, GrantsReview, Permission, PromptAction, PromptLevel, describe, level_tag,
    parse_permission, show_prompt, show_review,
};
pub use render::{UiEvent, show};
pub use tree::{ParamDef, ParamType, TreeError, Widget, WidgetTree};
