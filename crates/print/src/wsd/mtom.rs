//! MTOM/XOP packaging for SendDocument, and a bounded reader for multipart replies.
//!
//! The packaging and the multipart reader are the WSD core's; this module adds the print
//! `DocumentData` element around them.
//!
//! `DocumentData` is a base64 element in the schema; the service description's sample sends it as
//! a binary part referenced by `xop:Include` (`multipart/related`, first part the envelope). Both
//! forms are valid, so the caller picks with [`Transfer`].

use super::adapter::{base64, xop_include_ref};

/// How the document bytes travel inside SendDocument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Transfer {
    /// A `multipart/related` request with the bytes as a binary part (no 33 % base64 growth).
    #[default]
    Mtom,
    /// Plain `application/soap+xml` with the bytes as base64 text.
    InlineBase64,
}

/// The `DocumentData` element for an XOP reference to the part with this content id.
pub fn xop_include(cid: &str) -> String {
    format!("<wprt:DocumentData>{}</wprt:DocumentData>", xop_include_ref(cid))
}

/// The `DocumentData` element holding the bytes as base64.
pub fn inline_data(bytes: &[u8]) -> String {
    format!("<wprt:DocumentData>{}</wprt:DocumentData>", base64(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_data_element_holds_an_include_or_base64() {
        let inc = xop_include("doc@x");
        assert!(inc.starts_with("<wprt:DocumentData><xop:Include") && inc.contains("href=\"cid:doc@x\""), "{inc}");
        assert_eq!(inline_data(b"foobar"), "<wprt:DocumentData>Zm9vYmFy</wprt:DocumentData>");
        assert_eq!(Transfer::default(), Transfer::Mtom);
    }
}
