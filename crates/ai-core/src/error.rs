//! One error type for everything a provider or the model layer can report.
//!
//! The `kind` lets an app choose behaviour (retry, ask the user to sign in, show "this model
//! cannot take a mask"); the `message` is for people and is safe to show. Build provider errors
//! with [`AiError::redacted`] so an echoed key never reaches a log.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::secret::{SecretString, redact};

/// What went wrong, in terms an app can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The model or provider cannot do this (no mask input, no streaming, too many references).
    Unsupported,
    /// The request itself is wrong (missing input, bad size).
    InvalidRequest,
    /// Missing, expired or rejected credentials.
    Auth,
    /// Too many requests; see `retry_after_secs`.
    RateLimited,
    /// Out of credits, allowance or budget.
    Quota,
    /// Could not reach the provider.
    Network,
    /// The provider failed or returned something unusable.
    Provider,
    /// The user or app cancelled the job.
    Cancelled,
    /// The request would send data to a remote connection the user has not agreed to yet. Show
    /// the disclosure, record the agreement, and try again.
    ConsentRequired,
    /// A bug or impossible state inside the library. Reported, never panicked.
    Internal,
}

impl ErrorKind {
    /// A short lowercase name for messages and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::InvalidRequest => "invalid request",
            Self::Auth => "authentication",
            Self::RateLimited => "rate limited",
            Self::Quota => "quota",
            Self::Network => "network",
            Self::Provider => "provider",
            Self::Cancelled => "cancelled",
            Self::ConsentRequired => "consent required",
            Self::Internal => "internal",
        }
    }
}

/// The error type of the whole crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiError {
    /// What went wrong, for the app to act on.
    pub kind: ErrorKind,
    /// Human-readable detail. Secrets are scrubbed when built with [`AiError::redacted`].
    pub message: String,
    /// How long to wait before retrying, when the provider said so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

impl AiError {
    /// Builds an error of the given kind.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), retry_after_secs: None }
    }

    /// Builds an [`ErrorKind::Unsupported`] error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// Builds an [`ErrorKind::InvalidRequest`] error.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidRequest, message)
    }

    /// Builds an [`ErrorKind::Cancelled`] error.
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }

    /// Builds an error whose message has every given secret scrubbed out.
    pub fn redacted(kind: ErrorKind, message: &str, secrets: &[&SecretString]) -> Self {
        Self::new(kind, redact(message, secrets))
    }

    /// Sets the wait, in seconds, before a retry.
    pub fn with_retry_after(mut self, secs: u64) -> Self {
        self.retry_after_secs = Some(secs);
        self
    }

    /// Worth trying again later without changing anything.
    pub fn is_retryable(&self) -> bool {
        matches!(self.kind, ErrorKind::RateLimited | ErrorKind::Network)
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for AiError {}
