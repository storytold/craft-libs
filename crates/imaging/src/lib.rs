//! Raster image editing for Craft apps: warps, adjustments, layer styles, selection tools and masks, the full-window editors, channels and scopes.
//!
//! [`scopes`], [`selection`], [`trace`], [`layer_style`], [`raw`] (the RAW Develop settings) and [`fold_order`] work without egui and build on every target. The widgets are behind the `ui` feature.
#![warn(missing_docs)]

pub mod fold_order;
pub mod layer_style;
pub mod raw;
pub mod scopes;
pub mod selection;
pub mod trace;
#[cfg(feature = "ui")]
pub mod ui;
pub mod wide_angle;

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
