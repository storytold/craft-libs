use craft_media::bitstream::BitError;

/// Errors produced by the decoder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The uncompressed header ended early.
    #[error("bitstream error: {0}")]
    Bitstream(#[from] BitError),
    /// A syntax element had a value outside its legal range, or a size did not fit the data.
    #[error("invalid bitstream: {0}")]
    Invalid(String),
    /// A valid feature this decoder does not implement.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// An inter frame referenced a reference slot that was never filled (e.g. decoding started
    /// at a non-key frame).
    #[error("missing reference frame: {0}")]
    MissingReference(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// `ensure!(cond, "message")` returns `Error::Invalid` when `cond` is false.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::vp9::decode::error::Error::Invalid(format!($($arg)+)));
        }
    };
}
pub(crate) use ensure;
