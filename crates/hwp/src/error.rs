//! The crate's error type.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

/// Why an HWP document could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HwpError {
    /// Not an HWP 5.0 document, or damaged beyond reading.
    #[error("not a readable HWP document: {0}")]
    Format(String),
    /// A limit that protects against hostile files was reached.
    #[error("HWP document too large: {0}")]
    TooLarge(&'static str),
    /// Password-protected or distribution-only (Hancom: 배포용) documents.
    #[error("the HWP document is password-protected or distribution-only")]
    Protected,
}

pub(crate) type Result<T> = std::result::Result<T, HwpError>;

impl HwpError {
    pub(crate) fn format(m: impl std::fmt::Display) -> Self {
        HwpError::Format(m.to_string())
    }
}

impl From<craft_hwpx::HwpxError> for HwpError {
    fn from(e: craft_hwpx::HwpxError) -> Self {
        match e {
            craft_hwpx::HwpxError::Format(m) => HwpError::Format(m),
            craft_hwpx::HwpxError::TooLarge(what) => HwpError::TooLarge(what),
            craft_hwpx::HwpxError::Protected => HwpError::Protected,
        }
    }
}
