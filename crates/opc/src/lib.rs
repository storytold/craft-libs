#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Open Packaging Conventions (ECMA-376 Part 2): the ZIP container that DOCX, XLSX and PPTX
//! files share.
//!
//! - [`Package`] inflates every part up front, within [`Limits`]. [`LazyPackage`] indexes the
//!   archive and inflates a part when it is asked for, against a running total.
//! - [`Rels`] and [`ContentTypes`] read `_rels/*.rels` and `[Content_Types].xml`.
//! - [`resolve`], [`relative`], [`rels_path`] and the other part-name functions.
//! - [`PackageOut`], [`RelsOut`] and [`ZipOut`] write a package.
//! - [`check_entry_count`] refuses an archive that declares too many entries before the `zip`
//!   crate indexes them. Both readers call it.
//!
//! The limits have no defaults: each format keeps its own numbers.
//!
//! ```
//! use craft_opc::{Limits, Package, PackageOut, RelsOut};
//!
//! let mut out = PackageOut::default();
//! out.xml_part("word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", b"<w:document/>".to_vec());
//! let mut rels = RelsOut::default();
//! rels.add("officeDocument", "word/document.xml", false);
//! out.rels("", &rels);
//! let bytes = out.finish()?;
//!
//! let pkg = Package::open(&bytes, &Limits::new(1000, 1 << 20, 1 << 22))?;
//! let main = pkg.rels("").first("officeDocument").map(|r| r.target.clone());
//! assert_eq!(main.as_deref(), Some("word/document.xml"));
//! assert!(pkg.content_types().of("word/document.xml").is_some_and(|t| t.ends_with("main+xml")));
//! # Ok::<(), craft_opc::Error>(())
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod name;
mod read;
mod rels;
mod write;

#[cfg(any(test, feature = "test-support"))]
pub use craft_zip::zip64_entry_count_sample;

/// Refuse `bytes` when the end record the `zip` crate would use declares more than
/// `max_entries` entries, before anything is indexed ([`craft_zip::check_entry_count`]).
pub fn check_entry_count(bytes: &[u8], max_entries: u64) -> Result<(), Error> {
    craft_zip::check_entry_count(bytes, max_entries).map_err(zip_error)
}

fn zip_error(e: craft_zip::Error) -> Error {
    match e {
        craft_zip::Error::TooManyEntries { declared, limit } => Error::TooManyEntries { declared, limit },
        craft_zip::Error::TooManyEndRecords(n) => Error::TooManyEndRecords(n),
        craft_zip::Error::Write(m) => Error::Write(m),
        craft_zip::Error::NotZip(m) => Error::Zip(m),
        e => Error::Zip(e.to_string()),
    }
}
pub use name::{dir_of, file_of, normalize, percent_decode, relative, rels_path, resolve, split_dir};
pub use read::{LazyPackage, Package, declared_oversized};
pub use rels::{ContentTypes, Rel, Rels, type_is};
pub use write::{Compression, PackageOut, RelOut, RelsOut, ZipOut, compression_for, zip_entries};

/// The relationship type namespace of ECMA-376 transitional packages: a type is this and a
/// last segment (`image`, `styles`…).
pub const RT_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
/// The namespace of a relationships part.
pub const NS_RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// The namespace of `[Content_Types].xml`.
pub const NS_CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// The content type of a relationships part.
pub const CT_RELATIONSHIPS: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// What a reader refuses to exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most ZIP entries. Checked before the `zip` crate indexes them.
    pub max_entries: usize,
    /// Most bytes one part inflates to.
    pub max_part: u64,
    /// Most bytes kept from the whole package.
    pub max_total: u64,
    /// Most bytes inflated for the whole package, counting parts that are then left out (too
    /// large, or failing part-way). Never less than useful below `max_total`.
    pub max_work: u64,
    /// What [`Package::open`] does with a part over a limit.
    pub oversized: Oversized,
}

/// What [`Package::open`] does with a part over a limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oversized {
    /// The package is not opened.
    Refuse,
    /// The part is left out with a logged warning; reading stops when `max_work` is spent.
    Skip,
}

impl Limits {
    /// `max_work` is `max_total` (every inflated byte counts, kept or not) and a part over a
    /// limit refuses the package.
    pub const fn new(max_entries: usize, max_part: u64, max_total: u64) -> Limits {
        Limits { max_entries, max_part, max_total, max_work: max_total, oversized: Oversized::Refuse }
    }
}

/// Why a package was not read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a ZIP archive the `zip` crate can read, or an entry is damaged.
    #[error("zip: {0}")]
    Zip(String),
    /// The archive declares more entries than [`Limits::max_entries`].
    #[error("the package declares {declared} entries, more than {limit}")]
    TooManyEntries { declared: u64, limit: u64 },
    /// More end of central directory records than a package can have.
    #[error("more than {0} end of central directory records")]
    TooManyEndRecords(usize),
    /// A part declares or inflates to more than `limit` bytes.
    #[error("part {part} is larger than {limit} bytes")]
    PartTooLarge { part: String, limit: u64 },
    /// The parts together inflate to more than `limit` bytes.
    #[error("the package expands to more than {limit} bytes")]
    PackageTooLarge { limit: u64 },
    /// An XML part could not be read.
    #[error("{part}: {error}")]
    Xml { part: String, error: craft_xml::Error },
    /// The archive could not be written.
    #[error("could not write the package: {0}")]
    Write(String),
}

impl Error {
    /// Was a limit exceeded (as opposed to the archive being damaged)?
    pub fn is_limit(&self) -> bool {
        match self {
            Error::TooManyEntries { .. } | Error::TooManyEndRecords(_) | Error::PartTooLarge { .. } | Error::PackageTooLarge { .. } => true,
            Error::Xml { error, .. } => error.is_limit(),
            Error::Zip(_) | Error::Write(_) => false,
        }
    }
}
