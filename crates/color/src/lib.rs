//! Colour for Craft apps: the colour picker, wells, swatch tables, ramps and the hex field, and the
//! colour tags of compositor and display frames.
//!
//! The widgets are behind the `ui` feature; the colour model and conversions are in `craft-theme`.
//! Without `ui`, the crate has the helpers that need no egui: [`AreaMode`], [`GrayModel`],
//! [`CmykModel`], web-safe colours ([`web_safe`], [`not_web_safe`]), swatch files ([`palette`]), the display transform ([`display`]) and colour
//! harmonies ([`harmony`]) and gradient stops ([`gradient`]). [`tags`] has the colour tags of compositor and
//! display frames (matrix, transfer, primaries, range, HDR metadata) and their conversions, re-exported here.
//! [`AreaMode`] (and `ui::RampKind`) serialize as their [`key`](AreaMode::key); the swatch and harmony settings types serialize too.
#![warn(missing_docs)]

mod cmyk;
pub mod display;
pub mod gradient;
mod gray;
pub mod harmony;
mod mode;
pub mod palette;
pub mod tags;
#[cfg(feature = "ui")]
pub mod ui;
mod web;

pub use cmyk::CmykModel;
pub use gray::GrayModel;
pub use mode::AreaMode;
pub use tags::{
    ColorInfo, HdrMetadata, InvalidBitDepth, Matrix, Primaries, REFERENCE_WHITE_NITS, Range, Transfer, from_linear, hlg_inverse_oetf, hlg_oetf,
    hsl_to_rgb, linear_to_srgb, linear_to_srgb_u8, linear_to_srgb_u8_table, luma709, normalize_c, normalize_y, parse_hex, pq_eotf, pq_inverse_eotf,
    rgb_to_hsl, rgb_to_ycbcr, srgb_to_linear, srgb_u8_to_linear_table, to_hex, to_linear, ycbcr_to_rgb,
};
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
