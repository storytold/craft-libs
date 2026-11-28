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
//! What every type shares inside the XML, read from element trees named with [`NAMESPACES`]:
//!
//! - [`style`]: font declarations, default, common and automatic styles with their parents
//!   resolved, page layouts and master pages;
//! - [`number`]: data styles as spreadsheet format codes;
//! - [`units`]: lengths, angles, percentages, colours and borders.
//!
//! What the content means (paragraphs, cells, shapes) is not here, except for text documents
//! (feature `text`).
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
pub mod number;
mod package;
pub mod style;
#[cfg(feature = "text")]
pub mod text;
pub mod units;
pub mod zip_guard;

pub use manifest::{Encryption, FileEntry, MANIFEST_NS, MANIFEST_PATH, Manifest};
pub use package::{Limits, Package, PackageWriter};

/// Canonical prefixes for reading OpenDocument XML with [`craft_xml::Canonical`]: ODF 1.3's
/// namespaces (Part 3, section 1.5) and the extension namespaces office suites write. Trees read
/// with them name everything as [`style`] and [`number`] expect (`style:style`, `fo:color`,
/// `loext:…`), whatever prefixes the file declared.
pub const NAMESPACES: &[(&str, &str)] = &[
    ("office", "urn:oasis:names:tc:opendocument:xmlns:office:1.0"),
    ("style", "urn:oasis:names:tc:opendocument:xmlns:style:1.0"),
    ("text", "urn:oasis:names:tc:opendocument:xmlns:text:1.0"),
    ("table", "urn:oasis:names:tc:opendocument:xmlns:table:1.0"),
    ("draw", "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"),
    ("fo", "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"),
    ("xlink", "http://www.w3.org/1999/xlink"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("meta", "urn:oasis:names:tc:opendocument:xmlns:meta:1.0"),
    ("number", "urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"),
    ("svg", "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"),
    ("chart", "urn:oasis:names:tc:opendocument:xmlns:chart:1.0"),
    ("dr3d", "urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0"),
    ("math", "http://www.w3.org/1998/Math/MathML"),
    ("form", "urn:oasis:names:tc:opendocument:xmlns:form:1.0"),
    ("script", "urn:oasis:names:tc:opendocument:xmlns:script:1.0"),
    ("config", "urn:oasis:names:tc:opendocument:xmlns:config:1.0"),
    ("presentation", "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"),
    ("smil", "urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0"),
    ("anim", "urn:oasis:names:tc:opendocument:xmlns:animation:1.0"),
    ("db", "urn:oasis:names:tc:opendocument:xmlns:database:1.0"),
    ("of", "urn:oasis:names:tc:opendocument:xmlns:of:1.2"),
    ("xforms", "http://www.w3.org/2002/xforms"),
    ("xhtml", "http://www.w3.org/1999/xhtml"),
    ("grddl", "http://www.w3.org/2003/g/data-view#"),
    ("dom", "http://www.w3.org/2001/xml-events"),
    ("xsd", "http://www.w3.org/2001/XMLSchema"),
    ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
    ("ooo", "http://openoffice.org/2004/office"),
    ("ooow", "http://openoffice.org/2004/writer"),
    ("oooc", "http://openoffice.org/2004/calc"),
    ("rpt", "http://openoffice.org/2005/report"),
    ("tableooo", "http://openoffice.org/2009/table"),
    ("drawooo", "http://openoffice.org/2010/draw"),
    ("chartooo", "http://openoffice.org/2010/chart"),
    ("calcext", "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0"),
    ("loext", "urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"),
    ("field", "urn:openoffice:names:experimental:ooo-ms-interop:xmlns:field:1.0"),
    ("formx", "urn:openoffice:names:experimental:ooxml-odf-interop:xmlns:form:1.0"),
    ("css3t", "http://www.w3.org/TR/css3-text/"),
];

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
