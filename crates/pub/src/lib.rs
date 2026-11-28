//! Reader for Microsoft Publisher 2002 and later files (`.pub`).
//!
//! The file is a compound file ([`craft_cfb`]) holding a `Contents` stream (pages, masters,
//! objects, palette), a Quill text store and Escher drawing records. This crate decodes those into
//! plain data. It knows nothing about any document model: mapping to pages, frames and styles
//! belongs to the application. Every size and count read from the file is checked.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod contents;
pub mod escher;
pub mod quill;
pub mod read;
pub mod tables;

/// Why a file could not be read. [`Error::code`] is stable; the text is for people.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a Microsoft Publisher file: {0}")]
    NotPub(String),
    #[error("unsupported Publisher file: {0}")]
    Unsupported(String),
    #[error("damaged Publisher file: data ends early")]
    Truncated,
    #[error("damaged Publisher file: {0}")]
    Malformed(String),
}

/// A stable, machine-readable reason for an [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotPublisher,
    Unsupported,
    Truncated,
    Malformed,
}

impl Error {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotPub(_) => ErrorCode::NotPublisher,
            Self::Unsupported(_) => ErrorCode::Unsupported,
            Self::Truncated => ErrorCode::Truncated,
            Self::Malformed(_) => ErrorCode::Malformed,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
pub(crate) use Error as PubError;

/// Capitalization a character run asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capitalization {
    SmallCaps,
    AllCaps,
}

/// Vertical position of a character run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Superscript,
}
