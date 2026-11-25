use crate::container::bytes::OutOfBounds;
use crate::container::qds::QdsError;
use std::fmt;

/// Why a project record could not be decoded. `record` names the record type, e.g. "resource image".
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    Truncated { record: &'static str, source: OutOfBounds },
    Qds { record: &'static str, source: QdsError },
    BadMagic { record: &'static str },
    MissingField { record: &'static str, field: &'static str },
    Invalid { record: &'static str, reason: String },
}

impl DecodeError {
    pub fn truncated(record: &'static str) -> impl FnOnce(OutOfBounds) -> DecodeError {
        move |source| DecodeError::Truncated { record, source }
    }

    pub fn qds(record: &'static str) -> impl FnOnce(QdsError) -> DecodeError {
        move |source| DecodeError::Qds { record, source }
    }

    pub fn invalid(record: &'static str, reason: impl Into<String>) -> DecodeError {
        DecodeError::Invalid { record, reason: reason.into() }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated { record, source } => write!(f, "{record}: {source}"),
            DecodeError::Qds { record, source } => write!(f, "{record}: {source}"),
            DecodeError::BadMagic { record } => write!(f, "{record}: unexpected file signature"),
            DecodeError::MissingField { record, field } => write!(f, "{record}: missing field '{field}'"),
            DecodeError::Invalid { record, reason } => write!(f, "{record}: {reason}"),
        }
    }
}

impl std::error::Error for DecodeError {}
