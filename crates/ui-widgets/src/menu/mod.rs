//! Menu presets with `MenuItem` rows, menu-bar keys and mnemonics.

pub mod bar;
mod edit;
pub(crate) mod item;
#[doc(hidden)]
pub mod level;
mod panel;
mod popup;
pub mod search;
#[cfg(feature = "testing")]
mod stories;
mod words;

pub use bar::{MenuBar, MenuBarOutput, MenuBarStyle, MenuHelp, MenuTitle};
pub use edit::{EditMenu, EditMenuOutput};
pub use item::{MenuItem, MenuItemOutput, MenuItemStyle};
pub use panel::{GroupItem, PanelMenu, PanelMenuOutput, PanelRow};
#[doc(hidden)]
pub use popup::entry_rows;
#[doc(hidden)]
pub use popup::layer;
pub(crate) use popup::pick_values_from;
pub use popup::{Menu, MenuOutput, MenuPick, MenuStep, MenuStyle};
pub use search::HelpEntry;
pub use words::MenuWords;
