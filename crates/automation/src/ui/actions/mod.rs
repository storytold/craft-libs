//! The Actions panel (F13.06): sets, actions and steps as a tree, the include and dialog cells,
//! rename, drag to move, Button Mode, the footer and the menus. It runs nothing; it reports
//! events. Builds on every target.
//!
//! | Part | Names |
//! |---|---|
//! | The panel | [`ActionsPanel`], [`ActionsPanelOutput`], [`ActionsEvent`], [`ActionMove`], [`ActionPlace`], [`ActionSelection`], [`ShortcutChoice`] |
//! | The app's side | [`ActionsHost`], [`ActionsPanelCommands`], [`ActionsMenuFacts`] and [`panel_rows`] (the panel menu), [`ActionsMenuAction`], [`LibraryPrompt`] (Clear All, Reset, Replace) |
//! | State | [`ActionsPanelState`], [`ActionsSettings`] (persisted) |
//! | Columns and words | [`INCLUDE_COLUMN`], [`DIALOG_COLUMN`], [`ActionsWords`] |

mod body;
mod footer;
mod menu;
#[cfg(feature = "screen-testing")]
pub mod mock;
pub(crate) mod panel;
mod prompt;
mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use body::{ActionMove, ActionPlace};
pub use menu::{ActionsMenuAction, ActionsMenuFacts, ActionsPanelCommands, panel_rows};
pub use panel::{
    ActionSelection, ActionsEvent, ActionsHost, ActionsPanel, ActionsPanelOutput, ActionsPanelState, ActionsSettings, ShortcutChoice, StepCells,
};
pub use prompt::LibraryPrompt;
pub use rows::{DIALOG_COLUMN, INCLUDE_COLUMN};
pub use words::ActionsWords;
