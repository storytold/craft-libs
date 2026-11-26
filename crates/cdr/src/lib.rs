//! Reader for `.cdr` drawings of versions 1 to 27 (`.cdr`, and `.cdt` templates): the `WL` files
//! of versions 1 and 2, the RIFF chunk tree of versions 3 on with its compressed lists, the ZIP
//! packages of versions 14 on with their data streams, then pages, layers, guidelines, groups,
//! curves, rectangles, ellipses, arcs and polygons with uniform fills and outlines. It also reads
//! `.cmx` exchange files with 32-bit coordinates ([`read_cmx`]): their pages, layers, groups and
//! curves with uniform fills and outlines. [`preview()`] gives the picture a `.cdr` file keeps of its
//! drawing, without reading the drawing.
//!
//! Every record layout the reader relies on is listed in `PROVENANCE.md` with its public source
//! and status; code cites the row ids (R1 ...). Reading is bounded by [`Limits`], never recurses and does not
//! panic.
//!
//! [`write`] writes a [`Drawing`] as a `.cdr` file of version 15, laying out each record as the
//! reader reads it. Nothing here writes `.cmx` files.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

mod build;
mod bytes;
mod cmx;
mod geometry;
mod inline;
mod layout;
mod limits;
pub mod model;
mod package;
mod preview;
mod records;
mod riff;
mod styles;
#[cfg(any(test, feature = "synth"))]
pub mod synth;
#[cfg(any(test, feature = "synth"))]
pub mod synth_cmx;
mod wl;
mod write;
mod zip;

pub use limits::Limits;
pub use model::{Drawing, Warning};
pub use preview::{Preview, PreviewFormat, preview};
pub use write::{WRITTEN_VERSION, WriteError, write};

/// Does `bytes` look like a `.cdr` drawing: a RIFF file, a ZIP package holding one, or a `WL` file
/// of version 1 or 2 whose header offsets fit the file?
pub fn is_cdr(bytes: &[u8]) -> bool {
    riff::is_cdr_riff(bytes) || package::is_package(bytes) || wl::is_wl(bytes)
}

/// Does `bytes` start as a `.cmx` exchange file: a RIFF form (or its big-endian RIFX form) of type
/// `CMX1`?
pub fn is_cmx(bytes: &[u8]) -> bool {
    cmx::is_cmx(bytes)
}

/// Why a file can't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a `.cdr` drawing.
    NotCdr,
    /// Not a `.cmx` file.
    NotCmx,
    /// A drawing of a version this reader doesn't read (hundreds: 2800 is version 28).
    Version(u16),
    /// A container structure that doesn't parse.
    Damaged(&'static str),
    /// Past one of the [`Limits`].
    Limit(&'static str),
    /// A file this reader doesn't open: an encrypted, ZIP64 or split package, or a RIFF file that
    /// names a version before 3; a big-endian `.cmx` file or one with 16-bit coordinates; a
    /// `DISP` preview that is not an uncompressed bitmap with a 40-byte header; a version 2 file's
    /// picture.
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotCdr => write!(f, "not a CDR drawing"),
            Error::NotCmx => write!(f, "not a CMX file"),
            Error::Version(v) => write!(f, "version {} drawings are not read", v / 100),
            Error::Damaged(what) => write!(f, "damaged {what}"),
            Error::Limit(what) => write!(f, "past the {what} limit"),
            Error::Unsupported(what) => write!(f, "{what} are not read"),
        }
    }
}

impl std::error::Error for Error {}

/// Read a drawing within `limits`.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Drawing, Error> {
    if bytes.len() > limits.input {
        return Err(Error::Limit("input size"));
    }
    let mut budget = limits::Budget::new(limits);
    // Versions 1 and 2 are not RIFF files (R40).
    if wl::is_wl(bytes) {
        return wl::drawing(bytes, &mut budget);
    }
    let mut warnings = Vec::new();
    // Versions 14 on save a ZIP package around the RIFF stream (R24, R25, R26).
    let (stream, streams) = if zip::is_zip(bytes) { package::open(bytes, &mut budget)? } else { (std::borrow::Cow::Borrowed(bytes), None) };
    let ix = riff::Index::build(&stream, streams, &mut budget, &mut warnings)?;
    let mut drawing = build::drawing(&ix, &mut budget, &mut warnings)?;
    ix.stream_warnings(&mut drawing.warnings);
    Ok(drawing)
}

/// Read a `.cmx` exchange file within `limits`. Files in big-endian byte order and files with
/// 16-bit coordinates are refused with [`Error::Unsupported`]. Its [`Drawing::version`] is 0.
pub fn read_cmx(bytes: &[u8], limits: Limits) -> Result<Drawing, Error> {
    if bytes.len() > limits.input {
        return Err(Error::Limit("input size"));
    }
    let mut budget = limits::Budget::new(limits);
    cmx::drawing(bytes, &mut budget)
}
