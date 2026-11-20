#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The share dialog, the cloud Open and Save pages, the request-signatures dialog, the account
//! menu and the presence bar that every Craft app shares.
//!
//! This is storytold main's cloud and share UI as main wrote it in craft-ui: raw egui, outside the
//! craft UI library's component contract, themes and tokens. It is to be ported onto the
//! framework's components. Until then the `craft-ui` facade re-exports `account`, `cloud`, `esign`,
//! `share` and `presence` from here, with the same `share`, `cloud` and `esign` features, so
//! `craft_ui::…` paths stay as on main.

#[cfg(feature = "share")]
pub mod account;
mod buttons;
#[cfg(feature = "cloud")]
pub mod cloud;
#[cfg(feature = "esign")]
pub mod esign;
pub mod menu;
pub mod presence;
#[cfg(feature = "share")]
pub mod share;
