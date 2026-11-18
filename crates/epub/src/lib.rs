#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! EPUB packages, written and read.
//!
//! An EPUB is an OCF ZIP package: an uncompressed `mimetype` entry first, `META-INF/container.xml`
//! naming the package document (OPF), which lists every resource in its manifest and the reading
//! order in its spine, and (EPUB 3) a navigation document.
//!
//! - [`write`] turns a [`Book`] (metadata, resources, spine, navigation) into an EPUB 3 package,
//!   reflowable or fixed layout. The content documents are the caller's: this crate packages
//!   them, it does not make XHTML out of a document model ([`xhtml`] is only the shell).
//! - [`Epub::open`] reads an EPUB 2 or 3 package: container, metadata, manifest and spine, with
//!   every entry read inside byte budgets that don't trust the sizes the archive claims.
//!
//! Written from the public EPUB 3.3, OCF 3.3 and OPF 2.0.1 specifications.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod read;
mod text;
mod toc;
mod write;

pub use read::{Epub, Item, Limits, Package, SpineItem, dir_of, parse_container, parse_package, resolve};
pub use text::{contents_title, urn_uuid, valid_language_tag, w3cdtf};
pub use toc::{Encrypted, MAX_TOC_ENTRIES, TocEntry, parse_encryption, parse_nav, parse_ncx};
pub use write::{Book, Layout, MAX_RESOURCES, MAX_TITLE_CHARS, Metadata, Nav, NavItem, NavKind, Resource, write, xhtml};

/// The content of the first, stored `mimetype` entry.
pub const MIMETYPE: &str = "application/epub+zip";

/// Why a package was not read or not written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a ZIP archive, or the archive lacks what makes it an EPUB.
    #[error("not an EPUB package: {0}")]
    NotEpub(String),
    /// A required part is truncated or malformed.
    #[error("EPUB part {part}: {msg}")]
    Part { part: String, msg: String },
    /// A part is larger than [`Limits::max_entry_bytes`], or the parts read so far have used up
    /// [`Limits::max_total_bytes`].
    #[error("EPUB part {part} is larger than the {limit}-byte limit left for it")]
    TooLarge { part: String, limit: u64 },
    /// The archive has more entries than [`Limits::max_entries`].
    #[error("the EPUB package has {found} entries (at most {limit})")]
    TooManyEntries { found: usize, limit: usize },
    /// The book given to [`write`] cannot be a valid package.
    #[error("invalid EPUB book: {0}")]
    Invalid(String),
}

/// Does `bytes` look like an EPUB (a ZIP whose first, stored entry is the EPUB `mimetype`)?
///
/// Packages made by careless tools put `mimetype` elsewhere or compress it; [`Epub::open`] reads
/// those too, so this is a sniff for telling formats apart, not a validity check.
pub fn is_epub(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") && bytes.get(30..38) == Some(b"mimetype") && {
        let extra = bytes.get(28..30).map_or(0, |e| usize::from(u16::from_le_bytes([e[0], e[1]])));
        bytes.get(38 + extra..).is_some_and(|rest| rest.starts_with(MIMETYPE.as_bytes()))
    }
}
