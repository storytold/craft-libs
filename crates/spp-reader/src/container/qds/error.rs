use crate::container::bytes::OutOfBounds;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum QdsError {
    Truncated(OutOfBounds),
    /// A QVariant type this reader does not know; its size is unknown, so reading cannot continue.
    UnsupportedType {
        type_id: u32,
        at: usize,
    },
    InvalidUtf16 {
        at: usize,
    },
    /// Containers nested deeper than the reader allows (protects the stack against corrupt data).
    TooDeep {
        at: usize,
    },
}

impl fmt::Display for QdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QdsError::Truncated(e) => write!(f, "truncated QDataStream: {e}"),
            QdsError::UnsupportedType { type_id, at } => {
                write!(f, "unsupported QVariant type {type_id} at offset {at}")
            }
            QdsError::InvalidUtf16 { at } => write!(f, "invalid UTF-16 string at offset {at}"),
            QdsError::TooDeep { at } => write!(f, "QVariant containers nested too deeply at offset {at}"),
        }
    }
}

impl std::error::Error for QdsError {}

impl From<OutOfBounds> for QdsError {
    fn from(e: OutOfBounds) -> Self {
        QdsError::Truncated(e)
    }
}
