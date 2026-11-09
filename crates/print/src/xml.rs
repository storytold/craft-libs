//! A bounded parse of the XML the driver hands back (PrintTicket, PrintCapabilities). roxmltree's
//! work grows with the square of an element's attributes and namespace declarations, so those,
//! the nesting depth, the node count and the size are checked before it runs. The limits sit far
//! above what a driver writes.

use std::fmt;

/// The largest document read, in bytes. A PrintCapabilities document is rarely over 1 MiB.
pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
/// The most nodes (elements, attributes' owners, text, comments) one document may hold.
pub(crate) const MAX_NODES: u32 = 1_000_000;
/// The most attributes on one element, namespace declarations included.
pub(crate) const MAX_ATTRIBUTES: usize = 256;
/// The most namespace declarations on one element.
pub(crate) const MAX_NAMESPACES: usize = 64;
/// The deepest nesting of elements.
pub(crate) const MAX_DEPTH: usize = 256;

/// Why a document was not parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum XmlError {
    /// A limit was passed: which one, and the limit.
    Limit(&'static str, usize),
    /// The document is not well-formed XML.
    Parse(String),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(what, limit) => write!(f, "printer XML has more than {limit} {what}"),
            Self::Parse(e) => write!(f, "printer XML: {e}"),
        }
    }
}

/// `text` parsed, once it is within every limit above.
pub(crate) fn parse(text: &str) -> Result<roxmltree::Document<'_>, XmlError> {
    if text.len() > MAX_BYTES {
        return Err(XmlError::Limit("bytes", MAX_BYTES));
    }
    scan(text.as_bytes())?;
    let options = roxmltree::ParsingOptions { nodes_limit: MAX_NODES, ..roxmltree::ParsingOptions::default() };
    roxmltree::Document::parse_with_options(text, options).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => XmlError::Limit("nodes", MAX_NODES as usize),
        e => XmlError::Parse(e.to_string()),
    })
}

/// Checks the depth and each start tag's attributes, skipping comments, CDATA, processing
/// instructions and declarations. Malformed input is left for roxmltree to refuse.
fn scan(bytes: &[u8]) -> Result<(), XmlError> {
    let mut depth = 0usize;
    let mut at = 0usize;
    while let Some(offset) = bytes.get(at..).and_then(|rest| rest.iter().position(|&b| b == b'<')) {
        let start = at + offset;
        let rest = bytes.get(start..).unwrap_or_default();
        at = if rest.starts_with(b"<!--") {
            skip_past(bytes, start + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            skip_past(bytes, start + 9, b"]]>")
        } else if rest.starts_with(b"<?") {
            skip_past(bytes, start + 2, b"?>")
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            skip_past(bytes, start, b">")
        } else if rest.starts_with(b"<!") {
            skip_past(bytes, start, b">")
        } else {
            let (end, empty) = start_tag(bytes, start)?;
            if !empty {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(XmlError::Limit("levels of nesting", MAX_DEPTH));
                }
            }
            end
        };
    }
    Ok(())
}

/// The position just past `end` at or after `start`, or the end of the input.
fn skip_past(bytes: &[u8], start: usize, end: &[u8]) -> usize {
    bytes.get(start..).and_then(|rest| rest.windows(end.len()).position(|w| w == end)).map_or(bytes.len(), |p| start + p + end.len())
}

/// Counts the attributes and namespace declarations of the start tag at `start`. Returns the
/// position past its `>` and whether it is an empty-element tag (`/>`).
fn start_tag(bytes: &[u8], start: usize) -> Result<(usize, bool), XmlError> {
    let (mut attributes, mut namespaces) = (0usize, 0usize);
    let mut quote = None;
    let mut previous = b'<';
    for (i, &b) in bytes.iter().enumerate().skip(start + 1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Ok((i + 1, previous == b'/')),
            None if b == b'=' => {
                attributes += 1;
                if attributes > MAX_ATTRIBUTES {
                    return Err(XmlError::Limit("attributes on an element", MAX_ATTRIBUTES));
                }
                if attribute_name(bytes.get(start..i).unwrap_or_default()).starts_with(b"xmlns") {
                    namespaces += 1;
                    if namespaces > MAX_NAMESPACES {
                        return Err(XmlError::Limit("namespace declarations on an element", MAX_NAMESPACES));
                    }
                }
            }
            None => {}
        }
        previous = b;
    }
    Ok((bytes.len(), false))
}

/// The name before an `=` that ends `tag`: the last word, past any whitespace before the `=`.
fn attribute_name(tag: &[u8]) -> &[u8] {
    let end = tag.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(0, |p| p + 1);
    let head = tag.get(..end).unwrap_or_default();
    let start = head.iter().rposition(u8::is_ascii_whitespace).map_or(0, |p| p + 1);
    head.get(start..).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_attributes(count: usize, name: &str) -> String {
        let attributes: String = (0..count).map(|i| format!(r#" {name}{i}="v""#)).collect();
        format!("<root{attributes}/>")
    }

    fn nested(depth: usize) -> String {
        format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth))
    }

    #[test]
    fn a_document_at_each_limit_parses() {
        assert!(parse(&with_attributes(MAX_ATTRIBUTES, "a")).is_ok());
        assert!(parse(&with_attributes(MAX_NAMESPACES, "xmlns:p")).is_ok());
        assert!(parse(&nested(MAX_DEPTH)).is_ok());
    }

    #[test]
    fn a_document_just_past_each_limit_is_refused() {
        let limit = |text: &str| match parse(text) {
            Err(XmlError::Limit(what, _)) => what,
            other => panic!("expected a limit, got {other:?}"),
        };
        assert_eq!(limit(&with_attributes(MAX_ATTRIBUTES + 1, "a")), "attributes on an element");
        assert_eq!(limit(&with_attributes(MAX_NAMESPACES + 1, "xmlns:p")), "namespace declarations on an element");
        let default = format!("<root xmlns='u'{}/>", (1..=MAX_NAMESPACES).map(|i| format!(" xmlns:p{i}='u{i}'")).collect::<String>());
        assert_eq!(limit(&default), "namespace declarations on an element", "the default namespace counts");
        let spaced: String = (0..=MAX_NAMESPACES).map(|i| format!(" xmlns:p{i} \t= 'u{i}'")).collect();
        assert_eq!(limit(&format!("<root{spaced}/>")), "namespace declarations on an element");
        assert_eq!(limit(&nested(MAX_DEPTH + 1)), "levels of nesting");
        let nodes = format!("<r>{}</r>", "<a/>".repeat(MAX_NODES as usize));
        assert_eq!(limit(&nodes), "nodes");
        let big = format!("<r>{}</r>", " ".repeat(MAX_BYTES));
        assert_eq!(limit(&big), "bytes");
    }

    #[test]
    fn markup_inside_comments_cdata_and_values_is_not_counted() {
        let tag = with_attributes(MAX_ATTRIBUTES + 1, "a");
        let deep = "<a>".repeat(MAX_DEPTH + 1);
        let hidden = format!("<r><!-- {tag} {deep} --><![CDATA[{tag}{deep}]]><?pi {tag}?><e v='{}'/></r>", "a=b ".repeat(MAX_ATTRIBUTES + 1));
        assert!(parse(&hidden).is_ok());
        // A comment that opens with what looks like its own end still runs on.
        assert!(parse(&format!("<r><!--> {tag} --></r>")).is_ok());
        let quoted = format!("<root{}/>", (0..MAX_ATTRIBUTES).map(|i| format!(" a{i}='x>y'")).collect::<String>());
        assert!(parse(&quoted).is_ok(), "a > inside a value");
    }

    #[test]
    fn empty_and_closed_elements_do_not_deepen_the_nesting() {
        let flat = format!("<r>{}</r>", "<a/><b></b>".repeat(MAX_DEPTH * 2));
        assert!(parse(&flat).is_ok());
    }

    #[test]
    fn malformed_xml_is_a_parse_error() {
        assert!(matches!(parse("<a><b></a>"), Err(XmlError::Parse(_))));
    }
}
