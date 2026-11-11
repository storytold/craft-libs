//! Colour for Craft apps: the colour picker, wells, swatch tables, ramps and the hex field.
//!
//! The widgets are behind the `ui` feature; the colour model and conversions are in `craft-theme`.
//! Without `ui`, the crate has the helpers that need no egui: [`AreaMode`], [`GrayModel`],
//! [`CmykModel`], web-safe colours ([`web_safe`], [`not_web_safe`]), swatch files ([`palette`]), the display transform ([`display`]) and colour
//! harmonies ([`harmony`]).
//! [`AreaMode`] (and `ui::RampKind`) serialize as their [`key`](AreaMode::key); the swatch and harmony settings types serialize too.

mod cmyk;
pub mod display;
mod gray;
pub mod harmony;
mod mode;
pub mod palette;
#[cfg(feature = "ui")]
pub mod ui;
mod web;

pub use cmyk::CmykModel;
pub use gray::GrayModel;
pub use mode::AreaMode;
pub use web::{not_web_safe, web_safe};

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_color;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
