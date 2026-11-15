//! The Paths panel ([`PathsPanel`]): saved paths, the work path, shape paths and vector masks as a list with a
//! footer.

mod footer;
mod menu;
mod model;
mod panel;
mod rows;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

pub use footer::PathsFooterButton;
pub use menu::PathsCommands;
pub use model::{PathKind, PathRow, PathsEvent, PathsOutput};
pub use panel::PathsPanel;
pub use words::PathsWords;
