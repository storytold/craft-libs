//! Styles and data styles read from element trees. Every document here is written in this file
//! in the shape office suites write; none comes from another application.

mod number;
mod style;

use craft_xml::{Canonical, El, Options};

/// Declarations for the prefixes the documents use; some tests declare others on purpose.
pub const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#;

/// `xml` as a tree with the canonical prefixes.
pub fn tree(xml: &str) -> El {
    let canonical = Canonical::new(craft_odf::NAMESPACES);
    Options::canonical(&canonical).strict().parse(xml.as_bytes()).unwrap()
}

/// An `office:styles` holding `inner`.
pub fn styles_list(inner: &str) -> El {
    tree(&format!("<office:styles {NS}>{inner}</office:styles>"))
}
