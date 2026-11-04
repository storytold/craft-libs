//! One error type for every contract.
//!
//! The cases are the ones a caller acts on differently. A UI shows a different
//! thing for "you may not" than for "the server is unreachable", and a save
//! retries on a conflict where it would not on a refusal.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The provider could not be reached. Worth retrying.
    #[error("the provider could not be reached: {0}")]
    Unreachable(String),

    /// The credential was rejected. The user has to sign in again.
    #[error("the provider rejected the credential")]
    Unauthenticated,

    /// Authenticated, and not allowed to do this. Signing in again will not
    /// help, so a UI must not offer it.
    #[error("not permitted: {0}")]
    Forbidden(String),

    /// No such document, folder or share.
    #[error("{what} not found: {id}")]
    NotFound { what: &'static str, id: String },

    /// The document changed since the caller last read it. A save that would
    /// have overwritten somebody else's work lands here.
    #[error("{0} changed since you opened it")]
    Conflict(String),

    /// The provider does not offer this at all.
    ///
    /// Distinct from [`Error::Forbidden`]: there is nothing to grant. A UI
    /// hides the feature rather than explaining it.
    #[error("this provider does not support {0}")]
    Unsupported(&'static str),

    /// The answer did not match the protocol.
    #[error("malformed {what}: {detail}")]
    Malformed { what: &'static str, detail: String },

    /// A name or path could not be used safely. Carried, never panicked.
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
}

impl Error {
    pub fn not_found(what: &'static str, id: impl Into<String>) -> Error {
        Error::NotFound { what, id: id.into() }
    }

    pub fn malformed(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Malformed { what, detail: detail.into() }
    }

    pub fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Invalid { what, detail: detail.into() }
    }

    /// Whether trying again could succeed without the user doing anything.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Unreachable(_))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
