#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
//! AAF and OMF: the two mob-based editorial interchange formats, read and written through one
//! composition model.
//!
//! - [`model`]: a [`model::Document`] of compositions (tracks of fillers, source clips and
//!   overlapping transitions, with clip gain and markers) and the media sources they reference,
//!   with exact times ([`craft_timeline::time::Tick`]).
//! - [`aaf`]: Advanced Authoring Format, Edit Protocol, in structured storage (`craft-cfb`).
//! - [`omf`]: OMF Interchange 2.0 in a Bento container; sound only.
//!
//! Both read into and write from the same model, so an application maps its timeline onto the
//! model once. Audio essence can be embedded: the model carries it as PCM, and the WAVE / AIFF
//! wrappers the formats use are craft-audio's.
//!
//! Clean-room: written from the public specifications only (AMWA *AAF Object Specification* v1.1,
//! *AAF Low-Level Container Specification* v1.0.1, *AAF Edit Protocol* v1.0, SMPTE RP 224 / RP 210
//! labels; Avid *OMF Interchange Specification* 2.0, Apple *Bento Specification* 1.0d5). No file
//! I/O: documents are bytes and media references are strings.
//!
//! Input is untrusted: a damaged or crafted file gives an [`Error`] or a [`Report`] entry, never
//! a panic, a hang or an allocation sized by a count in the file.

pub mod aaf;
pub mod model;
pub mod omf;
mod pcm;
#[cfg(test)]
mod roundtrip_tests;
#[cfg(test)]
mod testdoc;

pub use craft_timeline::report::Report;

/// Error from a reader or writer. Unsupported *features* never error; they go in the [`Report`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a readable document of the format.
    #[error("{format} parse error: {message}")]
    Parse { format: &'static str, message: String },
    /// The document has no composition.
    #[error("the document contains no timeline")]
    Empty,
    /// The document could not be written.
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Who a written file says made it (the AAF `Identification` object, the OMF `IDNT` object).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity<'a> {
    pub company: &'a str,
    pub product: &'a str,
    /// The product's version, as text.
    pub version: &'a str,
    pub platform: &'a str,
    /// AAF `ProductID`: an AUID naming the product.
    pub product_id: [u8; 16],
}
