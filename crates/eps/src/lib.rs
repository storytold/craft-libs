//! Encapsulated PostScript as a container: what a file is, where its PostScript and its previews
//! are, and what its header comments say, without running its program.
//!
//! - [`sniff`]: EPS, plain PostScript or PDF, and whether it is an Illustrator file (PostScript-
//!   or PDF-based).
//! - [`sections`] / [`wrap`]: the binary header ("DOS EPS") that puts a TIFF or Windows metafile
//!   preview beside the PostScript, read and written.
//! - [`Comments`]: the DSC header comments, with the bounding box also when it is `(atend)`.
//! - [`epsi`]: the device-independent preview bitmap in `%%BeginPreview` comments.
//! - [`ascii85`]: the ASCII85 data encoding, both ways.
//!
//! Written from the PostScript Language Document Structuring Conventions 3.0 and the
//! Encapsulated PostScript File Format Specification 3.0 (Adobe Technical Notes 5001 and 5002),
//! and the PostScript Language Reference (ASCII85). Input is hostile: every offset is checked
//! against the file, scans are bounded, and nothing panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ascii85;
mod container;
mod dsc;
mod preview;

pub use container::{DOS_HEADER, DOS_MAGIC, Sections, sections, wrap};
pub use dsc::{Comments, MAX_HEADER, MAX_TRAILER};
pub use preview::{Bitmap, MAX_EPSI, epsi};

use std::fmt;

/// Why a file could not be read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The binary header points outside the file, or is cut short.
    OutsideFile,
    /// A section is too large for the binary header's 32-bit offsets (4 GiB).
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideFile => f.write_str("the EPS preview header points outside the file"),
            Self::TooLarge => f.write_str("the file is too large for an EPS preview header (4 GiB)"),
        }
    }
}

impl std::error::Error for Error {}

/// The format of a file, by its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Encapsulated PostScript: a binary preview header, or a first line `%!PS-Adobe-… EPSF-…`.
    Eps,
    /// A PostScript program that is not encapsulated (`%!`).
    PostScript,
    /// A PDF (`%PDF-`).
    Pdf,
}

/// What [`sniff`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffed {
    pub kind: Kind,
    /// An Illustrator file. PostScript-based (the legacy `.ai` format, or an Illustrator EPS): its
    /// header has `%AI…` comments or names Illustrator as its creator. PDF-based (`.ai` since
    /// version 9): the PDF names the `AIPrivateData` or `AIPDFPrivateData` of its editing copy
    /// within [`MAX_PDF_SCAN`] bytes of its start or end, which is a hint, not proof: only a PDF
    /// reader can look in the first page's `PieceInfo`, and an object stream hides it from here.
    pub illustrator: bool,
}

/// Bytes of a PDF's start and of its end scanned for the Illustrator private data keys.
pub const MAX_PDF_SCAN: usize = 1 << 20;

/// What `bytes` are, or `None` for anything but EPS, PostScript and PDF. File names never count:
/// a `.ai` file is told from a `.eps` or `.pdf` by its content.
pub fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    if bytes.starts_with(b"%PDF-") {
        let head = bytes.get(..MAX_PDF_SCAN).unwrap_or(bytes);
        let tail = bytes.get(bytes.len().saturating_sub(MAX_PDF_SCAN)..).unwrap_or(bytes);
        let private = |hay: &[u8]| [&b"/AIPrivateData"[..], b"/AIPDFPrivateData"].iter().any(|key| hay.windows(key.len()).any(|w| w == *key));
        return Some(Sniffed { kind: Kind::Pdf, illustrator: private(head) || private(tail) });
    }
    if bytes.starts_with(&DOS_MAGIC) {
        // A header that points outside the file is still an EPS file, one that can't be read.
        let illustrator = sections(bytes).is_ok_and(|s| Comments::read(s.postscript).illustrator);
        return Some(Sniffed { kind: Kind::Eps, illustrator });
    }
    if !bytes.starts_with(b"%!") {
        return None;
    }
    let first = bytes.split(|b| matches!(b, b'\n' | b'\r')).next().unwrap_or_default();
    let encapsulated = first.starts_with(b"%!PS-Adobe") && first.windows(4).any(|w| w == b"EPSF");
    Some(Sniffed { kind: if encapsulated { Kind::Eps } else { Kind::PostScript }, illustrator: Comments::read(bytes).illustrator })
}

/// An EPS file (plain, or with a binary header)?
pub fn is_eps(bytes: &[u8]) -> bool {
    sniff(bytes).is_some_and(|s| s.kind == Kind::Eps)
}

#[cfg(test)]
mod tests;
