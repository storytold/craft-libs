#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The `craft` developer CLI, as a library so its parts are testable.
//!
//! - [`new`]: `craft new`, plugin packages from templates.
//! - [`build`]: `craft build`, cargo for wasm32 plus the policy gate.
//! - [`gate`]: the install gate, as `craft check` and `craft build` run it.
//! - [`inspect`]: `craft inspect`, manifest, ops, permissions, gate report.
//! - [`run`]: `craft run --headless`, the real plugin host with no app behind it.
//! - [`watch`]: `craft watch`, rebuild on source change.
//! - [`devfolder`]: developer mode, reload a plugin when its `.wasm` changes.
//! - [`bindgen`]: `craft bindgen`, a typed client from an app's command list.

pub mod bindgen;
pub mod build;
pub mod cli;
pub mod devfolder;
pub mod error;
pub mod gate;
pub mod inspect;
pub mod new;
pub mod run;
pub mod watch;
