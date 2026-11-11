//! Raster image editing for Craft apps: warps, adjustments, layer styles, selection tools and masks, the full-window editors, channels and scopes.
//!
//! [`scopes`], [`selection`], [`trace`] and [`layer_style`] work without egui and build on every target. The widgets are behind the `ui` feature.

pub mod layer_style;
pub mod scopes;
pub mod selection;
pub mod trace;
#[cfg(feature = "ui")]
pub mod ui;

/// The registered stories and suite cases of the widgets, for `tests/it`.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_imaging;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
