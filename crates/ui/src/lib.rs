#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The craft UI library: runtime, style, fonts, primitives, the component contract, layout, components, patterns and
//! the shell (layers 2 to 7).
//!
//! The library is built as crates along its layers: craft-ui-contract (runtime to contract, and the test kit),
//! craft-ui-widgets, craft-ui-panels, then craft-ui-canvas, craft-ui-patterns and craft-ui-shell side by side. This
//! crate re-exports all of them under one root, so every `craft_ui::…` path names the same item as in the crate that
//! defines it.

pub use craft_ui_canvas::{canvas, canvas_controls, dialog};
#[cfg(feature = "testing")]
pub use craft_ui_contract::testing_registered;
#[doc(hidden)]
pub use craft_ui_contract::{__private, __register_command_set, __register_contract, __register_pattern};
pub use craft_ui_contract::{InstallError, InstallSpec, contract, fonts, install, layout, primitives, runtime, style, words};
pub use craft_ui_panels::{docking, rails, sections};
pub use craft_ui_shell::shell;
pub use craft_ui_widgets::{
    bars, buttons, choice, controls, display, edit, form, graph, layout_components, list, menu, notices, pickers, popover, table, tabs, text,
    tile_grid, value_drag,
};

/// `Inspector`, `CommandPalette`, `PreferencesWindow`, `DebugInspector`, `DockArea`, `DockPanels`, `PanelSpec`,
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

    crate::testing_registered!();

    /// The crates of the library, each with its `registered`.
    const LIBRARY: [(&str, fn() -> Vec<&'static Registration>); 7] = [
        ("craft-ui", registered),
        ("craft-ui-contract", craft_ui_contract::testing::registered),
        ("craft-ui-widgets", craft_ui_widgets::testing::registered),
        ("craft-ui-panels", craft_ui_panels::testing::registered),
        ("craft-ui-canvas", craft_ui_canvas::testing::registered),
        ("craft-ui-patterns", craft_ui_patterns::testing::registered),
        ("craft-ui-shell", craft_ui_shell::testing::registered),
    ];

    /// Every registration declared in the library crates of the library: this crate and the six it re-exports.
    /// Types declared in integration binaries are fixtures and are left out.
    pub fn library_registrations() -> Vec<&'static Registration> {
        // A crate's registrations run only in a binary that links the crate; naming each crate's `registered`
        // links them all.
        let library = std::hint::black_box(LIBRARY);
        crate::contract::registrations()
            .into_iter()
            .filter(|registration| {
                library.iter().any(|(name, _)| *name == registration.crate_name())
                    && registration.crate_target() == registration.crate_name().replace('-', "_")
            })
            .collect()
    }
}
