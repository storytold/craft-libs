#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! SHP source compilation to SHX containers, extracted from CADCraft.
//! Font discovery, glyph execution and text layout remain consumer responsibilities.
mod compile;
mod error;
pub mod parse;
pub mod program;
pub use compile::compile;
pub use error::ShxError;
pub use parse::{ParsedFont, Record, ShxKind};
