use craft_media::bitstream::BitError;

/// Errors produced by the decoder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bitstream ended early or contained an invalid Exp-Golomb code.
    #[error("bitstream error: {0}")]
    Bitstream(#[from] BitError),
    /// A syntax element had a value outside its legal range.
    #[error("invalid bitstream: {0}")]
    Invalid(String),
    /// A valid feature this decoder does not implement (yet).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A slice referenced a parameter set that has not been received.
    #[error("missing parameter set: {0}")]
    MissingParameterSet(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

pub(crate) fn unsupported<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Unsupported(msg.into()))
}

/// `ensure!(cond, "message")` returns `Error::Invalid` when `cond` is false.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::h264::decode::error::Error::Invalid(format!($($arg)+)));
        }
    };
}
pub(crate) use ensure;
