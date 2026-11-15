//! Cinepak decoder errors.

/// An error encountered while decoding a Cinepak video stream.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bitstream data was truncated or shorter than expected.
    #[error("truncated Cinepak bitstream")]
    Truncated,

    /// The frame header was malformed or declared invalid dimensions.
    #[error("invalid Cinepak frame header: {0}")]
    InvalidHeader(&'static str),

    /// A strip header was malformed or declared coordinates outside the frame bounds.
    #[error("invalid Cinepak strip header: {0}")]
    InvalidStrip(&'static str),

    /// A subchunk within a strip had an invalid size or format.
    #[error("invalid Cinepak chunk: {0}")]
    InvalidChunk(&'static str),

    /// An interframe was encountered before any keyframe was decoded.
    #[error("inter frame decoded without a reference frame")]
    MissingReference,

    /// Frame dimensions exceeded the decoding memory budget.
    #[error("frame size ({width}x{height}) exceeds memory budget")]
    MemoryBudget {
        /// Declared frame width.
        width: u32,
        /// Declared frame height.
        height: u32,
    },
}

/// The result of a Cinepak decoding operation.
pub type Result<T> = std::result::Result<T, Error>;
