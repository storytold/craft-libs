//! The Actions panel (F13.06): sets, actions and steps as a tree, the include and dialog cells,
//! rename, drag to move, Button Mode, the footer and the menus. It runs nothing; it reports
//! events. Builds on every target.
//!
//! | Part | Names |
//! |---|---|
//! | The panel | [`ActionsPanel`], [`ActionsPanelOutput`], [`ActionsEvent`], [`ActionMove`], [`ActionPlace`], [`ActionSelection`], [`ShortcutChoice`] |
//! | The app's side | [`ActionsHost`], [`ActionsPanelCommands`] and [`panel_rows`] (the panel menu), [`ActionsMenuAction`] |
//! | State | [`ActionsPanelState`], [`ActionsSettings`] (persisted) |
//! | Columns and words | [`INCLUDE_COLUMN`], [`DIALOG_COLUMN`], [`ActionsWords`] |

mod body;
mod footer;
mod menu;
#[cfg(feature = "screen-testing")]
pub mod mock;
mod panel;
mod rows;
#[cfg(feature = "screen-testing")]
mod stories;
mod words;

pub use body::{ActionMove, ActionPlace};
pub use menu::{ActionsMenuAction, ActionsPanelCommands, panel_rows};
pub use panel::{ActionSelection, ActionsEvent, ActionsHost, ActionsPanel, ActionsPanelOutput, ActionsPanelState, ActionsSettings, ShortcutChoice};
pub use rows::{DIALOG_COLUMN, INCLUDE_COLUMN};
pub use words::ActionsWords;
