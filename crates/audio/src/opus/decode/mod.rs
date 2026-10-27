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
pub use packet::{Bandwidth, Mode, Packet, Toc};

/// Errors from header parsing and decoding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidArgument(&'static str),
    InvalidPacket(&'static str),
    InvalidHeader(&'static str),
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

pub type Result<T> = std::result::Result<T, Error>;
