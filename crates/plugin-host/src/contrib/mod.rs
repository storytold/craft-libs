//! Contribution kinds the host registers with the app. One module per kind.
//!
//! Registration builds closures that must be `Send + Sync`, and a guest instance is not `Send` on
//! `wasm32` (browser engines hold JS handles). These modules therefore build on native targets
//! only. A web host reaches plugins through a worker proxy engine, which is a separate task.

pub mod command;
pub mod format;
pub mod hook;
pub mod surface;
