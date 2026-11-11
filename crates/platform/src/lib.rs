#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Shared lifecycle mechanics; application state, theme choices and file policy stay in hosts.
#[cfg(feature = "appearance")]
pub mod appearance;
pub mod autosave;
#[cfg(test)]
mod autosave_tests;
#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub mod browser;
#[cfg(feature = "clipboard")]
pub mod clipboard;
#[cfg(all(test, feature = "clipboard"))]
mod clipboard_tests;
pub mod clock;
#[cfg(not(target_arch = "wasm32"))]
pub mod default_app;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod default_app_tests;
pub mod dialog_outcome;
#[cfg(not(target_arch = "wasm32"))]
pub mod dialogs;
pub mod logs;
mod message;
#[cfg(test)]
mod message_tests;
pub mod open;
#[cfg(test)]
mod open_tests;
pub mod pacing;
#[cfg(feature = "pointer")]
pub mod pointer;
#[cfg(all(feature = "appearance", unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod process;
#[cfg(feature = "screen")]
pub mod screen;
pub mod session;
#[cfg(test)]
mod session_tests;
#[cfg(not(target_arch = "wasm32"))]
pub mod temp;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod temp_tests;
#[cfg(feature = "usage")]
pub mod usage;
pub mod watch;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod watch_tests;

pub use message::{PlatformText, texts};

#[cfg(test)]
extern crate self as craft_platform;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
