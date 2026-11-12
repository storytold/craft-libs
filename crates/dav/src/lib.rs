#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! WebDAV multistatus documents (RFC 4918 §13), what a server answers a `PROPFIND` or a CalDAV or
//! CardDAV `REPORT` with.
//!
//! [`parse_multistatus`] reads the bytes into a [`craft_xml::El`] tree whose names carry fixed
//! prefixes whatever the document used: `d:` for `DAV:` ([`DAV`]), `c:` for CalDAV ([`CALDAV`]),
//! `card:` for CardDAV ([`CARDDAV`]), `cs:` for CalendarServer ([`CALENDARSERVER`]), `oc:` and
//! `nc:` for ownCloud and Nextcloud ([`OWNCLOUD`], [`NEXTCLOUD`]); a name in any other namespace
//! starts with `?:` and matches none of them. [`responses`], [`href`] and [`properties`] walk it.
//!
//! Nothing here does I/O. The reader fails closed: a document type declaration, an entity that
//! is not predefined, a character XML cannot carry, malformed or unbalanced markup, a second root,
//! text outside the root, a root that is not `d:multistatus` and anything past the limits make
//! the whole document an error; a partial tree is never returned.

use craft_xml::{Canonical, Doctype, El, Limits, Options};

/// The WebDAV namespace.
pub const DAV: &str = "DAV:";
/// RFC 4791.
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
/// RFC 6352.
pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
/// Apple's calendar server extensions (`getctag`).
pub const CALENDARSERVER: &str = "http://calendarserver.org/ns/";
pub const OWNCLOUD: &str = "http://owncloud.org/ns";
pub const NEXTCLOUD: &str = "http://nextcloud.org/ns";

/// Most bytes of a document.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Most nodes (elements and stretches of text) in a document.
pub const MAX_NODES: usize = 100_000;
/// Deepest nesting of elements, the root at 1.
pub const MAX_DEPTH: usize = 32;
/// Most attributes on one element, and in the whole document.
pub const MAX_ATTRS: usize = 64;
pub const MAX_ATTRS_TOTAL: usize = 200_000;

const NAMESPACES: [(&str, &str); 6] = [("d", DAV), ("c", CALDAV), ("card", CARDDAV), ("cs", CALENDARSERVER), ("oc", OWNCLOUD), ("nc", NEXTCLOUD)];
const CANONICAL: Canonical<'static> = Canonical::new(&NAMESPACES);

/// Why a document was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not well-formed XML within the limits, or refused (see the crate's documentation).
    Xml(craft_xml::Error),
    /// Text that is not UTF-8, or a character XML cannot carry.
    Characters,
    /// Well-formed, but its root is not `DAV:multistatus`.
    NotMultistatus,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Xml(e) => write!(f, "invalid or oversized DAV XML: {e}"),
            Error::Characters => f.write_str("DAV XML with text that is not UTF-8 XML characters"),
            Error::NotMultistatus => f.write_str("DAV XML that is not a multistatus"),
        }
    }
}

impl std::error::Error for Error {}

/// Read one UTF-8 multistatus document (see the crate's documentation). The root is named
/// `d:multistatus`; namespace declarations are not kept as attributes.
pub fn parse_multistatus(bytes: &[u8]) -> Result<El, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Xml(craft_xml::Error::TooLarge(MAX_BYTES)));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Characters)?;
    if !text.chars().all(craft_xml::is_xml_char) {
        return Err(Error::Characters);
    }
    let limits = Limits { max_input: MAX_BYTES, max_depth: MAX_DEPTH, max_nodes: MAX_NODES, max_attrs: MAX_ATTRS, max_attrs_total: MAX_ATTRS_TOTAL };
    let options = Options { doctype: Doctype::Reject, ..Options::canonical(&CANONICAL).strict().with_limits(limits) };
    let root = options.parse(bytes).map_err(Error::Xml)?;
    if root.name != "d:multistatus" {
        return Err(Error::NotMultistatus);
    }
    Ok(root)
}

/// The `d:response` elements of a multistatus.
pub fn responses(multistatus: &El) -> impl Iterator<Item = &El> {
    multistatus.children("d:response")
}

/// The `d:href` of a response, trimmed (still percent-encoded, as the server wrote it).
pub fn href(response: &El) -> Option<String> {
    response.child_text("d:href").map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

/// Whether a `d:status` line (`HTTP/1.1 200 OK`) says 200.
pub fn is_ok_status(status: &str) -> bool {
    let mut words = status.split_whitespace();
    words.next().is_some_and(|v| v.starts_with("HTTP/")) && words.next() == Some("200")
}

/// The properties a response returned in a successful (`200`) propstat, in document order. A
/// property in a failed propstat (404, 403) is not one: a property missing here says nothing
/// about whether the resource still exists.
pub fn properties(response: &El) -> impl Iterator<Item = &El> {
    response
        .children("d:propstat")
        .filter(|p| p.child_text("d:status").is_some_and(|s| is_ok_status(&s)))
        .filter_map(|p| p.child("d:prop"))
        .flat_map(El::elements)
}

#[cfg(test)]
mod tests;
