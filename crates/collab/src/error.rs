//! One error type for the crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The caller's hub failed to send or connect.
    #[error("hub failed: {0}")]
    Hub(String),

    /// A payload did not parse as a protocol message.
    #[error("malformed {direction} message: {detail}")]
    Malformed { direction: &'static str, detail: String },
}

impl Error {
    pub(crate) fn malformed(direction: &'static str, detail: impl Into<String>) -> Error {
        Error::Malformed { direction, detail: detail.into() }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
