//! Decode and validate SVG/SVGZ text before a caller hands it to a renderer.
//!
//! This boundary has no dependency on an SVG renderer. It decodes the file's text encoding and
//! bounded gzip container, then checks entity expansion, attribute work, XML structure, node
//! count and nesting depth.

use std::borrow::Cow;

/// Limits for decoding and validating one SVG document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of source text, also the largest unpacked SVGZ.
    pub bytes: usize,
    /// Deepest XML element nesting (the root alone is 1).
    pub depth: usize,
    /// Total bytes entities may expand to.
    pub entity_bytes: usize,
    /// Refuse a doctype entity containing markup.
    pub refuse_entity_markup: bool,
    /// Attributes on one element.
    pub attributes: usize,
    /// Sum of squared attribute counts across the document.
    pub attribute_work: usize,
    /// Maximum XML nodes after entity expansion.
    pub xml_nodes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            bytes: 64 << 20,
            depth: 256,
            entity_bytes: crate::guard::MAX_ENTITY_EXPANSION,
            refuse_entity_markup: false,
            attributes: crate::guard::MAX_ATTRIBUTES,
            attribute_work: crate::guard::MAX_ATTRIBUTE_WORK,
            xml_nodes: 4_000_000,
        }
    }
}

/// Why an SVG's text or XML could not be read within `Limits`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are too large, corrupt SVGZ, nested SVGZ, or invalid text encoding.
    #[error(transparent)]
    Text(#[from] crate::svgz::Error),
    /// Entity references expand past the configured limit.
    #[error("the SVG's entities expand to too much text")]
    Entities,
    /// A doctype entity contains markup and is refused by the caller's policy.
    #[error("the SVG's document type declares an entity holding markup, which isn't supported")]
    EntityMarkup,
    /// XML element nesting exceeds the configured limit.
    #[error("the SVG's elements are nested more than {0} levels deep")]
    TooDeep(usize),
    /// Attribute count or total attribute work exceeds the configured limit.
    #[error("an element of the SVG has more than {0} attributes, or its elements have too many in all")]
    TooWide(usize),
    /// The XML is malformed or exceeds the node limit.
    #[error("invalid SVG XML: {0}")]
    Xml(String),
}

/// Decodes SVG or SVGZ text and checks the XML against `limits`.
pub fn text<'a>(bytes: &'a [u8], limits: &Limits) -> Result<Cow<'a, str>, Error> {
    let text = crate::svgz::text(bytes, limits.bytes)?;
    parse_text(&text, limits)?;
    Ok(text)
}

/// Parses already-decoded SVG text and checks it against `limits`.
pub fn parse_text<'a>(svg: &'a str, limits: &Limits) -> Result<roxmltree::Document<'a>, Error> {
    if svg.len() > limits.bytes {
        return Err(Error::Text(crate::svgz::Error::TooLarge(limits.bytes)));
    }
    if limits.refuse_entity_markup && crate::guard::entity_holds_markup(svg) {
        return Err(Error::EntityMarkup);
    }
    if crate::guard::entities_expand_past(svg, limits.entity_bytes) {
        return Err(Error::Entities);
    }
    if crate::guard::too_wide(svg, limits.attributes, limits.attribute_work) {
        return Err(Error::TooWide(limits.attributes));
    }
    if crate::guard::deeper_than(svg, limits.depth) {
        return Err(Error::TooDeep(limits.depth));
    }
    let options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: limits.xml_nodes, ..Default::default() };
    roxmltree::Document::parse_with_options(svg, options).map_err(|e| Error::Xml(e.to_string()))
}
