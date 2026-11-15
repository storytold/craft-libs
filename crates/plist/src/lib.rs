//! Bounded binary Apple property lists, independent of operating-system preferences.
//! XML and writing are intentionally not implemented in this first extraction.
//! File discovery, locale selection, bundle validation and application policy stay in callers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
mod binary;
mod bytes;
pub use binary::{DateValue, Value, first_string_in_array, parse_binary, parse_binary_with_limits};

/// Caller-selected resource limits. Zero values refuse the corresponding resource.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_objects: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_expanded_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { max_input_bytes: 16 << 20, max_objects: 100_000, max_nodes: 100_000, max_depth: 64, max_expanded_bytes: 64 << 20 }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("malformed property list: {0}")]
    Malformed(String),
    #[error("truncated property list: {0}")]
    Truncated(String),
    #[error("unsupported property list: {0}")]
    Unsupported(String),
    #[error("property list resource limit: {0}")]
    TooLarge(String),
}
