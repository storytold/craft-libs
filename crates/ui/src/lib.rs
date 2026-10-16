#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Small, reusable egui components for the Craft frontends.
//!
//! Applications own their assets, themes, layouts, commands and document semantics. Components
//! accept focused style values and return egui responses; no application engine is required.

#[cfg(feature = "share")]
pub mod account;
pub mod buttons;
#[cfg(feature = "cloud")]
pub mod cloud;
pub mod docking;
#[cfg(feature = "esign")]
pub mod esign;
pub mod icons;
pub mod layout;
pub mod menu;
pub mod presence;
pub mod printing;
#[cfg(feature = "share")]
pub mod share;
pub mod tabs;
pub mod theme;
