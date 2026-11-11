#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The components of the craft UI library: buttons, choices, display, forms, lists, menus, notices, text fields, value
//! drags, pickers, tabs, tables, graphs and bars (layer 5).
//!
//! The root re-exports craft-ui-contract's modules, so components name them as `crate::runtime::…` and derive output
//! as `craft_ui::…`.

// The derives' output names the crate as `craft_ui`, here as in every other crate.
extern crate self as craft_ui;

#[cfg(feature = "testing")]
pub use craft_ui_contract::testing_registered;
#[doc(hidden)]
pub use craft_ui_contract::{__private, __register_command_set, __register_contract, __register_pattern};
pub use craft_ui_contract::{InstallError, InstallSpec, contract, fonts, install, layout, primitives, runtime, style, words};

pub mod bars;
pub mod buttons;
pub mod choice;
pub mod controls;
pub mod display;
pub mod edit;
pub mod form;
pub mod graph;
pub mod layout_components;
pub mod list;
pub mod menu;
pub mod notices;
pub mod pickers;
pub mod popover;
#[cfg(feature = "testing")]
#[doc(hidden)]
pub mod story_kit;
pub mod table;
pub mod tabs;
pub mod text;
pub mod tile_grid;
pub mod value_drag;

/// The fixture, the suite runner and the story helpers, with the helpers that show components: `props` and
/// `story_props`.
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::story_kit::{props, story_props};
    pub use craft_ui_contract::testing::*;

    crate::testing_registered!();
}
