#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The craft UI library: runtime, style, fonts, primitives, the component contract, layout, components, patterns and
//! the shell (layers 2 to 7).
//!
//! The library is built as crates along its layers: craft-ui-contract (runtime to contract, and the test kit),
//! craft-ui-widgets, craft-ui-panels, then craft-ui-canvas, craft-ui-patterns and craft-ui-shell side by side. This
//! crate re-exports all of them under one root, so every `craft_ui::…` path names the same item as in the crate that
//! defines it. It also re-exports storytold main's cloud, share and e-sign UI (`account`, `cloud`, `esign`, `share`,
//! `presence`) from craft-cloud-ui, which is still raw egui.
#![warn(missing_docs)]

extern crate self as craft_ui;

#[cfg(feature = "cloud")]
pub use craft_cloud_ui::cloud;
#[cfg(feature = "esign")]
pub use craft_cloud_ui::esign;
pub use craft_cloud_ui::presence;
#[cfg(feature = "share")]
pub use craft_cloud_ui::{account, share};
pub use craft_ui_canvas::{canvas, canvas_controls, dialog};
#[cfg(feature = "testing")]
pub use craft_ui_contract::registry;
#[doc(hidden)]
pub use craft_ui_contract::{__private, __register_command_set, __register_contract, __register_pattern};
pub use craft_ui_contract::{InstallError, InstallSpec, contract, fonts, install, layout, primitives, runtime, style, words};
pub use craft_ui_panels::{docking, rails, sections};
pub use craft_ui_shell::shell;
pub use craft_ui_widgets::{
    bars, buttons, choice, controls, display, edit, form, graph, layout_components, list, menu, notices, pickers, popover, table, tabs, text,
    tile_grid, value_drag,
};

/// `Inspector`, `CommandPalette`, `PreferencesWindow`, `DockArea`, `DockPanels`, `PanelSpec`,
/// `NavigatorPanel` and the other patterns.
pub mod patterns {
    pub use craft_ui_patterns::patterns::*;

    /// Canvas panel patterns: `NavigatorPanel`.
    pub mod canvas {
        pub use craft_ui_canvas::canvas::navigator_panel::{NavigatorPanel, NavigatorPanelOutput};
    }
}

/// The fixture, the suite runner and the story helpers of every crate of the library.
#[cfg(feature = "testing")]
pub mod testing {
    pub use craft_ui_canvas::testing::*;

    #[cfg(feature = "suite")]
    crate::registry!();

    /// The framework crates whose suite items form the library.
    #[cfg(feature = "suite")]
    const LIBRARY: [fn(&mut Registry); 7] = [
        register,
        craft_ui_contract::testing::register,
        craft_ui_widgets::testing::register,
        craft_ui_panels::testing::register,
        craft_ui_canvas::testing::register,
        craft_ui_patterns::testing::register,
        craft_ui_shell::testing::register,
    ];

    /// Adds the framework library crates to a registry.
    #[cfg(feature = "suite")]
    pub fn register_library(registry: &mut Registry) {
        for register in LIBRARY {
            register(registry);
        }
    }

    /// The suite items declared by the framework library crates.
    #[cfg(feature = "suite")]
    pub fn library_registrations() -> Registry {
        Registry::of(&LIBRARY)
    }
}
