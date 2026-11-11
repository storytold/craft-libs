#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The canvas of the craft UI library: `Canvas` with its tool host, tools, rulers and navigator, the canvas
//! controls, and the full-window dialogs that show a canvas (layers 5 and 6).
//!
//! The root re-exports craft-ui-panels' modules and the ones it re-exports; `dialog` adds the full-window family to
//! panels' dialogs.

// The derives' output names the crate as `craft_ui`, here as in every other crate.
extern crate self as craft_ui;

#[doc(hidden)]
pub use craft_ui_panels::canvas_data;
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use craft_ui_panels::story_kit;
#[cfg(feature = "testing")]
pub use craft_ui_panels::testing_registered;
#[doc(hidden)]
pub use craft_ui_panels::{__private, __register_command_set, __register_contract, __register_pattern};
pub use craft_ui_panels::{InstallError, InstallSpec, contract, fonts, install, layout, primitives, runtime, style, words};
pub use craft_ui_panels::{
    bars, buttons, choice, controls, display, edit, form, graph, layout_components, list, menu, notices, pickers, popover, table, tabs, text,
    tile_grid, value_drag,
};
pub use craft_ui_panels::{docking, rails, sections};

pub mod canvas;
pub mod canvas_controls;

/// Dialog shells and prompts, with the full-window dialog family that shows a canvas.
pub mod dialog {
    pub use crate::canvas::full_window::{FullWindowDialog, FullWindowDialogStyle};
    pub use crate::canvas::full_window_parts::{
        FullWindowCanvas, FullWindowCanvasOutput, FullWindowSide, FullWindowSideOutput, FullWindowSideStyle, FullWindowStatus,
        FullWindowStatusOutput, FullWindowTitle, FullWindowTitleOutput, FullWindowView, FullWindowZoomBar, FullWindowZoomBarOutput, RasterOverlay,
        SideFooter,
    };
    pub use craft_ui_panels::dialog::*;
}

/// The fixture, the suite runner and the story helpers, with the canvas helpers: `ToolFixture`, `tool_story` and
/// `canvas_story`.
#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::canvas::testing::{canvas_story, tool_fixture, tool_fixture::ToolFixture, tool_story::tool_story};
    pub use craft_ui_panels::testing::*;

    crate::testing_registered!();
}
