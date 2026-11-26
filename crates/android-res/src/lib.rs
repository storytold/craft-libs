//! Bounded readers for compiled Android XML and resource tables.
pub mod arsc;
pub mod axml;
mod bytes;
#[cfg(test)]
mod fixtures;

/// Failure to decode the Android wire format.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A required record did not fit within the available bytes.
    #[error("truncated: {0}")]
    Truncated(String),
    /// The available bytes do not form the required structure.
    #[error("malformed: {0}")]
    Malformed(String),
}
