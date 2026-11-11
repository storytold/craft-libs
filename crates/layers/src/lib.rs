//! The Layers panel for Craft apps, over the app's own layer tree.
//!
//! The app implements `ui::tree::LayerTree`, a read-only view of its layers, and shows it with
//! `ui::panel::LayersPanel`, which reports what the user did as events in the order they
//! happened. The app applies them with its own commands and undo.

pub mod ui;

/// The stories and trials this crate registers.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}
