#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! IDML (InDesign Markup Language) packages, and InDesign Tagged Text.
//!
//! An IDML package is a ZIP whose first entry is an uncompressed `mimetype`, followed by
//! `designmap.xml` (the root `Document` element, which includes the other parts) and the
//! `Resources/`, `MasterSpreads/`, `Spreads/` and `Stories/` parts.
//!
//! This crate is the layer between the bytes and IDML's own element tree:
//!
//! - [`Package`] reads parts within byte budgets that don't trust the sizes the archive claims,
//!   and finds them however the writer spelled their names. [`Package::document`] reads
//!   `designmap.xml` and puts the parts it includes in place ([`Document`]).
//! - [`write_package`] zips parts behind the stored `mimetype`; [`part_root`], [`part_xml`] and
//!   [`designmap_xml`] make the parts.
//! - Elements are [`craft_xml::El`]. [`Props`] adds IDML's two spellings of a property: an
//!   attribute, or a child of `<Properties>`.
//! - [`value`] has the codecs of attribute values: numbers, transforms, `Self` ids, style names,
//!   link URIs, base64 `Contents`.
//! - [`icml`] reads and writes InCopy Markup: one story and what it uses, as one XML document.
//! - [`tagged`] reads and writes InDesign Tagged Text, the plain-text interchange format.
//! - [`model`] types the layout (spreads and parent spreads, pages, guides, page items and the
//!   graphics placed in them, with path geometry) and stories (paragraph and character ranges,
//!   text, tables, notes, anchored items). Styles, swatches and preferences are walked as
//!   elements for now.
//!
//! Implemented clean-room from the public IDML File Format Specification and from inspecting
//! IDML packages exported from synthetic documents.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod icml;
pub mod model;
mod package;
mod part;
pub mod tagged;
pub mod value;

pub use craft_xml::{El, Local, Node};
pub use package::{Document, Limits, Package, write_package};
pub use part::{CONTAINER_XML, Props, designmap_xml, parse_part, part_root, part_xml, prop, with_props, xml_limits};

/// The IDML package mimetype (content of the first, stored `mimetype` entry).
pub const MIMETYPE: &str = "application/vnd.adobe.indesign-idml-package";
/// The DOM version written to every part.
pub const DOM_VERSION: &str = "16.0";
/// The namespace of the `idPkg` prefix.
pub const PACKAGING_NS: &str = "http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging";

/// Why a package was not read or not written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a ZIP archive, or the archive lacks what makes it an IDML package.
    #[error("not an IDML package: {0}")]
    NotIdml(String),
    /// A part could not be decompressed, or its XML could not be read.
    #[error("IDML part {part}: {msg}")]
    Part { part: String, msg: String },
    /// A part is larger than [`Limits::max_part_bytes`], or the parts read so far have used up
    /// [`Limits::max_total_bytes`].
    #[error("IDML part {part} is larger than the {limit}-byte limit left for it")]
    TooLarge { part: String, limit: u64 },
    /// The archive has more entries than [`Limits::max_entries`].
    #[error("the IDML package has {found} entries (at most {limit})")]
    TooManyEntries { found: usize, limit: usize },
    /// The archive could not be written (two parts with one name, …).
    #[error("the IDML package could not be written: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Does `bytes` look like an IDML package (a ZIP whose first, stored entry is the IDML `mimetype`)?
pub fn is_idml(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") && bytes.get(30..38) == Some(b"mimetype") && {
        let extra = bytes.get(28..30).map_or(0, |e| usize::from(u16::from_le_bytes([e[0], e[1]])));
        bytes.get(38 + extra..).is_some_and(|rest| rest.starts_with(MIMETYPE.as_bytes()))
    }
}
