//! XFA XML infrastructure, extracted from PDFCraft. Layout and application policy stay local.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
/// Compatibility depth limit for XFA XML documents.
pub const MAX_XML_DEPTH: usize = 1024;
pub mod xml;
