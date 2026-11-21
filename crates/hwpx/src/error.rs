//! The crate's error type.

/// Why an HWPX document could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HwpxError {
    /// Not an HWPX document, or damaged beyond reading.
    #[error("not a readable HWPX document: {0}")]
    Format(String),
    /// A limit that protects against hostile files was reached.
    #[error("HWPX document too large: {0}")]
    TooLarge(&'static str),
    /// Password-protected or distribution-only (Hancom: 배포용) documents.
    #[error("the HWPX document is password-protected or distribution-only")]
    Protected,
}

pub(crate) type Result<T> = std::result::Result<T, HwpxError>;

impl HwpxError {
    pub(crate) fn format(m: impl std::fmt::Display) -> Self {
        HwpxError::Format(m.to_string())
    }
}
