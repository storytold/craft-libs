#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The panels of the craft UI library: sections, dialogs, docking and rails (layer 5), and the canvas data that
//! patterns read without the canvas.
//!
//! The root re-exports craft-ui-widgets' modules and the ones it re-exports.

// The derives' output names the crate as `craft_ui`, here as in every other crate.
extern crate self as craft_ui;

#[cfg(feature = "testing")]
#[doc(hidden)]
pub use craft_ui_widgets::story_kit;
#[cfg(feature = "testing")]
pub use craft_ui_widgets::testing_registered;
#[doc(hidden)]
pub use craft_ui_widgets::{__private, __register_command_set, __register_contract, __register_pattern};
pub use craft_ui_widgets::{InstallError, InstallSpec, contract, fonts, install, layout, primitives, runtime, style, words};
pub use craft_ui_widgets::{
    bars, buttons, choice, controls, display, edit, form, graph, layout_components, list, menu, notices, pickers, popover, table, tabs, text,
    tile_grid, value_drag,
};

#[doc(hidden)]
pub mod canvas_data;
pub mod dialog;
pub mod docking;
pub mod rails;
pub mod sections;
#[cfg(feature = "testing")]
mod testing_internals;

/// The fixture, the suite runner and the story helpers, with `internals`: crate-private logic the trials check
/// without a frame.
#[cfg(feature = "testing")]
pub mod testing {
    pub use craft_ui_widgets::testing::*;

    pub mod internals {
        //! Crate-private logic the trials check without a frame.
        pub use crate::testing_internals::*;
    }

    crate::testing_registered!();
}
