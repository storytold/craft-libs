//! Menu presets with `MenuItem` rows, menu-bar keys and mnemonics.

pub mod bar;
pub(crate) mod edit;
pub(crate) mod item;
#[doc(hidden)]
pub mod level;
pub(crate) mod panel;
pub(crate) mod popup;
pub mod search;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use bar::{MenuBar, MenuBarOutput, MenuBarStyle, MenuHelp, MenuTitle};
pub(crate) use edit::text_menu;
pub use edit::{EditMenu, EditMenuOutput};
pub use item::{MenuItem, MenuItemOutput, MenuItemStyle, MenuSamplePaint, MenuShortcut};
pub use panel::{GroupItem, PanelMenu, PanelMenuOutput, PanelRow};
#[doc(hidden)]
pub use popup::entry_rows;
#[doc(hidden)]
pub use popup::layer;
pub(crate) use popup::pick_values_from;
pub use popup::{Menu, MenuOutput, MenuPick, MenuStep, MenuStyle};
pub use search::HelpEntry;
pub use words::MenuWords;
