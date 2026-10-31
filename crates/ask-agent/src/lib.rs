#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Ask Agent for the Craft apps: the person selects something, types an instruction, and the agent
//! they already run (over MCP or the app's control channel) does it in the document.
//!
//! This crate holds what every app shares:
//! - [`hub`]: the request queue, the agents listening, the one rule for who may edit, what is held,
//!   and the two control methods `agent.listen` / `agent.reply` ([`AgentHub::handle`]);
//! - [`ui`] (feature `egui`): the chip beside the selection and the bubble with Keep, Undo and a
//!   follow-up field.
//!
//! Each app keeps its own part: the selection as JSON, its undo grouping, authorship ("Agent (name)")
//! and what "held" means for its documents. See the README.

pub mod hub;
#[cfg(feature = "egui")]
pub mod ui;

pub use hub::{AgentHub, HubError, Request, Snapshot, Status, Turn, colour};
