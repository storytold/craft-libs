//! The crate's error type.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

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
