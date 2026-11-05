//! One error type for every e-sign contract.
//!
//! Cloud failures are wrapped so a host can surface sign-in and reachability
//! the same way it already does for files. The e-sign-only cases are the ones
//! a UI acts on differently: declined, voided, expired, identity check.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Something the cloud contracts already classify.
    #[error(transparent)]
    Cloud(#[from] craft_cloud::Error),

    /// A recipient refused to sign.
    #[error("declined: {0}")]
    Declined(String),

    /// The sender voided the envelope.
    #[error("voided: {0}")]
    Voided(String),

    /// The envelope passed its deadline without completing.
    #[error("expired: {0}")]
    Expired(String),

    /// The provider rejected the signer's identity check.
    #[error("identity check failed: {0}")]
    IdentityCheck(String),

    /// The chosen provider refuses this request under its policy.
    #[error("provider policy: {0}")]
    ProviderPolicy(String),

    /// The service does not offer this at all.
    #[error("this signing service does not support {0}")]
    Unsupported(&'static str),

    /// The answer did not match the protocol.
    #[error("malformed {what}: {detail}")]
    Malformed { what: &'static str, detail: String },

    /// A field the caller supplied could not be used safely.
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
}

impl Error {
    pub fn malformed(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Malformed { what, detail: detail.into() }
    }

    pub fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Invalid { what, detail: detail.into() }
    }

    /// Whether trying again could succeed without the user doing anything.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Cloud(e) if e.is_transient())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
