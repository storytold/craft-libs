#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Bounded XML for hostile input.
//!
//! - [`parse`] and [`Options::parse`] read bytes into an owned tree of [`El`] and [`Node`], with
//!   limits on input size, nesting, node count and attributes per element ([`Limits`]).
//! - [`Options::parse_streaming`] hands repeated elements to a callback, so a large part never
//!   becomes a whole tree. [`Options::events`] is the pull reader underneath.
//! - [`Names`] chooses how names are read: as written, by local name, or rewritten to the
//!   caller's canonical prefixes by namespace URI.
//! - [`esc`], [`esc_attr`], [`El::to_xml`], [`El::to_xml_pretty`] and [`Writer`] write XML.
//!
//! No document type definition is processed, and no entity is expanded except the five
//! predefined ones (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;`) and numeric character references.
//! The text the reader produces is therefore never more than three times its input.
//!
//! ```
//! let root = craft_xml::parse(b"<a x='1'><b>one &amp; two</b><b/></a>")?;
//! assert_eq!(root.attr_u32("x"), Some(1));
//! assert_eq!(root.child("b").map(|b| b.text().into_owned()).as_deref(), Some("one & two"));
//! assert_eq!(root.to_xml(), r#"<a x="1"><b>one &amp; two</b><b/></a>"#);
//! # Ok::<(), craft_xml::Error>(())
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod read;
mod tree;
pub mod value;
mod write;

pub use read::{
    AttrValueHook, Canonical, Doctype, ElementHook, Event, Events, Limits, Names, NsName, Options, Scope, UnknownEntity, XML_NS, entity, parse,
    to_utf8, unescape,
};
pub use tree::{Descendants, El, Local, Node, Pat};
pub use write::{DECL, DECL_STANDALONE, Tag, Writer, esc, esc_attr, is_name, is_xml_char, push_esc, push_esc_attr};

/// Why a document was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The input is longer than [`Limits::max_input`].
    #[error("the XML is larger than {0} bytes")]
    TooLarge(usize),
    /// Elements nest deeper than [`Limits::max_depth`].
    #[error("XML elements nest deeper than {0} levels")]
    TooDeep(usize),
    /// The tree would hold more than [`Limits::max_nodes`] nodes.
    #[error("the XML holds more than {0} nodes")]
    TooManyNodes(usize),
    /// One start tag has more than [`Limits::max_attrs`] attributes.
    #[error("an XML element has more than {0} attributes")]
    TooManyAttrs(usize),
    /// The tree would hold more than [`Limits::max_attrs_total`] attributes.
    #[error("the XML holds more than {0} attributes")]
    TooManyAttrsTotal(usize),
    /// The document has a document type declaration that [`Options::doctype`] refuses.
    #[error("the XML has a document type declaration, which is not accepted")]
    Doctype,
    /// A reference to an entity that is neither predefined nor numeric, with
    /// [`UnknownEntity::Error`].
    #[error("the entity `&{name};` at byte {at} is not defined")]
    UnknownEntity { at: usize, name: String },
    /// Text that is not UTF-8, in a strict parse.
    #[error("the XML is not valid UTF-8 at byte {0}")]
    Encoding(usize),
    /// Markup that cannot be read. `at` is a byte offset into the UTF-8 text ([`to_utf8`]).
    #[error("malformed XML at byte {at}: {msg}")]
    Syntax { at: usize, msg: String },
    /// There is no root element.
    #[error("the XML has no root element")]
    NoRoot,
}

impl Error {
    /// Was a limit exceeded (as opposed to the markup being wrong)?
    pub fn is_limit(&self) -> bool {
        matches!(self, Error::TooLarge(_) | Error::TooDeep(_) | Error::TooManyNodes(_) | Error::TooManyAttrs(_) | Error::TooManyAttrsTotal(_))
    }
}
