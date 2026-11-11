#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The base of the craft UI library: runtime, style, fonts, primitives, the component contract, layout and the test
//! kit (layers 2 to 4).
//!
//! The crates above it re-export these modules at their roots, so `crate::runtime::…` and `craft_ui::contract::…`
//! name the same items in every crate of the library.

// The derives' output names the crate as `craft_ui`, here as in every other crate.
extern crate self as craft_ui;

pub use craft_ui_macros::words;

pub mod contract;
pub mod fonts;
pub mod layout;
pub mod primitives;
pub mod runtime;
pub mod style;
#[cfg(feature = "testing")]
pub mod testing;

pub use contract::install;
pub use runtime::{InstallError, InstallSpec};

#[doc(hidden)]
pub mod __private {
    pub use egui;
    pub use inventory;
}
