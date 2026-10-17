//! The crate's error type: everything fails with `Result`, never a panic.

use std::fmt;

/// Every failure a caller can act on: the variant says what went wrong,
/// the message says why.
#[derive(Debug)]
pub enum AiError {
    /// The HTTP layer failed (DNS, TLS, status code, body).
    Http(String),
    /// A downloaded file's SHA-256 did not match the expected digest.
    Checksum { expected: String, got: String },
    /// The local disk refused us (permissions, full, vanished mid-write).
    Io(std::io::Error),
    /// The provider returned a response we could not parse.
    BadResponse(String),
    /// The provider rejected the request (bad token, quota, model name).
    Rejected(String),
    /// No provider is configured — the UI should route to setup.
    NotConfigured,
    /// Local model inference failed (load, OOM, bad GGUF).
    Inference(String),
    /// The operation was cancelled by the user.
    Cancelled,
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(m) => write!(f, "network error: {m}"),
            Self::Checksum { expected, got } => write!(f, "checksum mismatch: expected {expected}, got {got}"),
            Self::Io(e) => write!(f, "disk error: {e}"),
            Self::BadResponse(m) => write!(f, "unreadable response: {m}"),
            Self::Rejected(m) => write!(f, "provider rejected the request: {m}"),
            Self::NotConfigured => write!(f, "no AI provider configured"),
            Self::Inference(m) => write!(f, "local model error: {m}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for AiError {}

impl From<std::io::Error> for AiError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
