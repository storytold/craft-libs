//! The decoder half of [`crate::opus`]: FilmCraft's `filmcraft-opus` (storytold/filmcraft@8fda2f2).

// The fixed-point parts mirror the normative integer arithmetic; keep C-like shift expressions.
#![allow(clippy::precedence, clippy::int_plus_one)]

pub(crate) mod celt;
mod decoder;
mod header;
mod multistream;
pub mod packet;
pub(crate) mod range;
pub(crate) mod silk;

pub use decoder::{SAMPLE_RATES, StreamDecoder};
pub use header::OpusHead;
pub use multistream::Decoder;
pub use packet::{Mode, Packet, Toc};

/// Errors from header parsing and decoding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An argument is out of range (sampling rate, channel count, concealment length, output size).
    InvalidArgument(&'static str),
    /// A malformed packet (RFC 6716 §3.4).
    InvalidPacket(&'static str),
    /// A malformed `OpusHead` (RFC 7845 §5.1) or MP4 `dOps` box.
    InvalidHeader(&'static str),
    /// A valid stream uses a feature this decoder does not implement.
    Unsupported(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidArgument(s) => write!(f, "invalid argument: {s}"),
            Error::InvalidPacket(s) => write!(f, "invalid packet: {s}"),
            Error::InvalidHeader(s) => write!(f, "invalid header: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result with this module's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
