use std::fmt;
use std::io;

/// Errors produced by the demuxer.
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O failure.
    Io(io::Error),
    /// Structurally invalid data that could not be recovered from.
    Invalid(String),
    /// Valid but unsupported feature (e.g. zlib-compressed or encrypted tracks when reading data).
    Unsupported(String),
    /// Not an EBML file, or an EBML file whose DocType is not `matroska`/`webm`.
    NotMatroska(String),
    /// A track index was out of range.
    NoSuchTrack(usize),
    /// A sample index was out of range.
    NoSuchSample(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Invalid(s) => write!(f, "invalid data: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported: {s}"),
            Error::NotMatroska(s) => write!(f, "not a Matroska/WebM file: {s}"),
            Error::NoSuchTrack(i) => write!(f, "track index {i} out of range"),
            Error::NoSuchSample(i) => write!(f, "sample index {i} out of range"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}
