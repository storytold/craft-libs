#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! OpenDocument packages (ODF 1.3 Part 2): what every OpenDocument file has in common, whatever
//! it holds (text, a spreadsheet, a presentation, a drawing, a chart, a formula).
//!
//! - [`Package`] opens the ZIP container within [`Limits`] and reads its `mimetype`, its
//!   [`Manifest`] and its files. [`PackageWriter`] writes one: the `mimetype` first and stored,
//!   the manifest, then the files.
//! - [`crypt`] opens and saves password-protected packages (AES-256-CBC with PBKDF2, as
//!   LibreOffice writes them).
//! - [`flat`] turns a flat single-file document (`.fodt`, `.fods`, `.fodp`, `.fodg`) into a
//!   package and back.
//! - [`zip_guard`] refuses a ZIP that declares too many entries before `zip` parses them.
//!
//! What the XML inside means (paragraphs, cells, styles) is not here.
//!
//! Input is hostile: entries, sizes, key-derivation work and nesting are bounded, and nothing
//! here panics on any bytes.
//!
//! ```
//! use craft_odf::{Limits, Package, PackageWriter, mime};
//!
//! let mut w = PackageWriter::new(mime::TEXT);
//! w.file("content.xml", "text/xml", b"<office:document-content/>");
//! let bytes = w.finish()?;
//!
//! let mut p = Package::open(&bytes, &Limits::DEFAULT)?;
//! assert_eq!(p.mimetype().as_deref(), Some(mime::TEXT));
//! assert_eq!(p.read("content.xml")?.as_deref(), Some(b"<office:document-content/>".as_slice()));
//! assert_eq!(p.manifest().media_type("content.xml"), Some("text/xml"));
//! # Ok::<(), craft_odf::Error>(())
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod crypt;
pub mod flat;
mod manifest;
mod package;
#[cfg(feature = "text")]
pub mod text;
pub mod zip_guard;

pub use manifest::{Encryption, FileEntry, MANIFEST_NS, MANIFEST_PATH, Manifest};
pub use package::{Limits, Package, PackageWriter};

/// The media types of OpenDocument files (ODF 1.3 Part 3 section 3.3, and the `mimetype` file of
/// Part 2 section 3.3).
pub mod mime {
    pub const TEXT: &str = "application/vnd.oasis.opendocument.text";
    pub const TEXT_TEMPLATE: &str = "application/vnd.oasis.opendocument.text-template";
    pub const SPREADSHEET: &str = "application/vnd.oasis.opendocument.spreadsheet";
    pub const SPREADSHEET_TEMPLATE: &str = "application/vnd.oasis.opendocument.spreadsheet-template";
    pub const PRESENTATION: &str = "application/vnd.oasis.opendocument.presentation";
    pub const PRESENTATION_TEMPLATE: &str = "application/vnd.oasis.opendocument.presentation-template";
    pub const GRAPHICS: &str = "application/vnd.oasis.opendocument.graphics";
    pub const GRAPHICS_TEMPLATE: &str = "application/vnd.oasis.opendocument.graphics-template";
    pub const CHART: &str = "application/vnd.oasis.opendocument.chart";
    pub const FORMULA: &str = "application/vnd.oasis.opendocument.formula";
}

/// Why a package was not read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The ZIP declares more entries than [`Limits::max_entries`], or has too many end records.
    #[error("{0}")]
    TooManyEntries(String),
    /// A file is larger than [`Limits::max_entry`].
    #[error("the file {0} in the package is larger than {1} bytes")]
    EntryTooLarge(String, u64),
    /// The files read add up to more than [`Limits::max_total`].
    #[error("the package's files are larger than {0} bytes together")]
    TotalTooLarge(u64),
    /// The bytes are no ZIP package, or one that cannot be read or written.
    #[error("not a valid package ({0})")]
    Zip(String),
    /// A flat document that is no `office:document`, or not UTF-8.
    #[error("not a flat OpenDocument file ({0})")]
    Flat(&'static str),
    /// A manifest that [`Manifest::parse_strict`] refuses: malformed XML, a document type
    /// declaration, a root that is no `manifest:manifest`, or more entries than it reads.
    #[error("the package's manifest is not valid ({0})")]
    Manifest(String),
}

impl Error {
    /// Was a limit exceeded (as opposed to the package being wrong)?
    pub fn is_limit(&self) -> bool {
        matches!(self, Error::TooManyEntries(_) | Error::EntryTooLarge(..) | Error::TotalTooLarge(_))
    }
}

#[cfg(test)]
mod tests;
