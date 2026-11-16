//! The automation widgets. Every target.
//!
//! - [`actions`]: the Actions panel.
//! - [`agents`]: the Agents page of Preferences (desktop, feature `agents`).

pub mod actions;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub mod agents;
