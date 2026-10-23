use std::io;

/// Errors produced by the demuxer and muxer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A box or structure ended before all required fields were read.
    #[error("truncated data while reading {0}")]
    Truncated(&'static str),
    /// Structurally invalid data.
    #[error("invalid data: {0}")]
    Invalid(String),
    /// Valid but unsupported feature (e.g. compressed `cmov` movie header).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A count or size exceeds sanity limits relative to the file size.
    #[error("size limit exceeded: {0}")]
    TooLarge(&'static str),
    /// No `moov` box was found.
    #[error("no moov box found")]
    NoMoov,
    /// A track index was out of range.
    #[error("track index {0} out of range")]
    NoSuchTrack(usize),
    /// A sample index was out of range.
    #[error("sample index {0} out of range")]
    NoSuchSample(usize),
    /// Muxer usage error (e.g. invalid track config, bad sample).
    #[error("muxer: {0}")]
    Mux(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}
