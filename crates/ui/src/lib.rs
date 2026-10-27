#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Small, reusable egui components for the Craft frontends.
//!
//! Applications own their assets, themes, layouts, commands and document semantics. Components
//! accept focused style values and return egui responses; no application engine is required.

#[cfg(feature = "share")]
pub mod account;
pub mod buttons;
pub mod capture;
#[cfg(feature = "cloud")]
pub mod cloud;
pub mod direction;
pub mod docking;
#[cfg(feature = "esign")]
pub mod esign;
mod focus;
pub mod fonts;
pub mod icons;
pub mod ime;
pub mod layout;
pub mod localized;
pub mod menu;
pub mod panes;
pub mod presence;
pub mod printing;
#[cfg(feature = "share")]
pub mod share;
pub mod tabs;
#[cfg(all(feature = "screen-testing", not(target_arch = "wasm32")))]
pub mod testing;
pub mod text;
pub mod theme;
