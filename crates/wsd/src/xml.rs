//! Bounded XML reading and escaped writing.
//!
//! Replies come from the network, so every parse is capped three ways before the tree is built:
//! the byte size, the element nesting (a scan of the markup, since a tree walk would already have
//! paid for the depth) and the node count. DTDs, and with them entity expansion, are refused.

use roxmltree::{Document, Node, ParsingOptions};

use crate::error::{Error, Result};

/// The largest XML text parsed (an envelope, never a document).
pub const MAX_XML_BYTES: usize = 4 << 20;
/// The most nodes a parsed message may hold.
pub const MAX_NODES: u32 = 100_000;
/// The deepest element nesting accepted.
pub const MAX_DEPTH: usize = 40;
/// The longest text kept from one element.
pub const MAX_TEXT: usize = 4096;

/// Decode and check size and depth; the result is what [`parse`] takes.
pub fn checked_text(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > MAX_XML_BYTES {
        return Err(Error::TooLarge("the XML message", MAX_XML_BYTES));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Xml("not valid UTF-8".into()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    check_depth(text)?;
    Ok(text)
}

/// Parse with a node cap and no DTD.
pub fn parse(text: &str) -> Result<Document<'_>> {
    let options = ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES };
    Document::parse_with_options(text, options).map_err(|e| Error::Xml(e.to_string()))
}

/// Walk the markup and refuse nesting deeper than [`MAX_DEPTH`]. Comments, CDATA, processing
/// instructions and quoted attribute values are skipped so their contents cannot fake a tag.
pub fn check_depth(text: &str) -> Result<()> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes.get(i) != Some(&b'<') {
            i += 1;
            continue;
        }
        let rest = bytes.get(i..).unwrap_or(&[]);
        if rest.starts_with(b"<!--") {
            i = skip_to(bytes, i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(bytes, i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = skip_to(bytes, i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            // A DOCTYPE: the parser refuses it, but do not loop on it here.
            i = skip_to(bytes, i + 2, b">");
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = skip_to(bytes, i + 2, b">");
        } else {
            let end = tag_end(bytes, i + 1);
            let self_closing = end.checked_sub(2).and_then(|p| bytes.get(p)) == Some(&b'/');
            if !self_closing {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(Error::Xml(format!("elements nested deeper than {MAX_DEPTH}")));
                }
            }
            i = end;
        }
    }
    Ok(())
}

/// The index just past the next `needle` at or after `from`, or the end of `bytes` if there is none.
fn skip_to(bytes: &[u8], from: usize, needle: &[u8]) -> usize {
    let mut i = from;
    while i < bytes.len() {
        if bytes.get(i..).is_some_and(|r| r.starts_with(needle)) {
            return i + needle.len();
        }
        i += 1;
    }
    bytes.len()
}

/// Index just past the `>` that ends the tag whose name starts at `from`.
fn tag_end(bytes: &[u8], from: usize) -> usize {
    let mut quote: Option<u8> = None;
    let mut i = from;
    while let Some(&b) = bytes.get(i) {
        i += 1;
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return i,
            None => {}
        }
    }
    bytes.len()
}

/// Is `node` the element `{ns}local`?
pub fn is(node: Node<'_, '_>, ns: &str, local: &str) -> bool {
    node.is_element() && node.tag_name().name() == local && node.tag_name().namespace() == Some(ns)
}

/// The first child element `{ns}local`.
pub fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, local: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| is(*c, ns, local))
}

/// Every child element `{ns}local`.
pub fn children<'a, 'i, 'n>(node: Node<'a, 'i>, ns: &'n str, local: &'n str) -> impl Iterator<Item = Node<'a, 'i>> + 'n
where
    'a: 'n,
    'i: 'n,
{
    node.children().filter(move |c| is(*c, ns, local))
}

/// The first element below `node` (any depth) named `{ns}local`.
pub fn find<'a, 'i>(node: Node<'a, 'i>, ns: &str, local: &str) -> Option<Node<'a, 'i>> {
    node.descendants().find(|c| is(*c, ns, local))
}

/// The direct text of an element, trimmed and cut at [`MAX_TEXT`] characters.
pub fn text(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    for c in node.children().filter(|c| c.is_text()) {
        out.push_str(c.text().unwrap_or(""));
        if out.len() > MAX_TEXT * 4 {
            break;
        }
    }
    out.trim().chars().take(MAX_TEXT).collect()
}

/// Resolve a QName value (`prefix:local`, or `local` in the default namespace) in the scope of `node`.
pub fn resolve_qname(node: Node<'_, '_>, qname: &str) -> Option<(Option<String>, String)> {
    let qname = qname.trim();
    if qname.is_empty() || qname.len() > 512 {
        return None;
    }
    match qname.split_once(':') {
        Some((prefix, local)) if !prefix.is_empty() && !local.is_empty() && !local.contains(':') => {
            let uri = node.lookup_namespace_uri(Some(prefix))?;
            Some((Some(uri.to_owned()), local.to_owned()))
        }
        Some(_) => None,
        None => Some((node.lookup_namespace_uri(None).map(str::to_owned), qname.to_owned())),
    }
}

/// A qualified name: namespace URI plus local name. Prefixes never matter.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QName {
    pub ns: String,
    pub local: String,
}

impl QName {
    /// The name `{ns}local`; nothing is validated.
    pub fn new(ns: impl Into<String>, local: impl Into<String>) -> Self {
        Self { ns: ns.into(), local: local.into() }
    }

    /// Whether this is the name `{ns}local`.
    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.ns == ns && self.local == local
    }
}

impl std::fmt::Display for QName {
    /// Clark notation: `{namespace}local`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{{}}}{}", self.ns, self.local)
    }
}

/// Is `s` an XML NCName restricted to ASCII letters, digits, `_`, `-` and `.`, starting with a
/// letter or `_`? Names taken from device replies that fail this are never written back out.
pub fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The whitespace-separated QName list in `node`'s text (`Types`), resolved in its scope. Tokens
/// whose prefix is not declared, or that name no namespace, are skipped; at most `max` tokens are read.
pub fn qname_list(node: Node<'_, '_>, max: usize) -> Vec<QName> {
    text(node)
        .split_whitespace()
        .take(max)
        .filter_map(|tok| match resolve_qname(node, tok)? {
            (Some(ns), local) if !ns.is_empty() => Some(QName { ns, local }),
            _ => None,
        })
        .collect()
}

/// The trimmed, capped text of the first child element `{ns}local`, if it has any.
pub fn child_text(node: Node<'_, '_>, ns: &str, local: &str) -> Option<String> {
    child(node, ns, local).map(text).filter(|t| !t.is_empty())
}

/// `true`/`1` and `false`/`0` (xs:boolean), nothing else.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Escape text for an element or a double-quoted attribute; characters XML 1.0 cannot carry are
/// dropped.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_markup_and_controls() {
        assert_eq!(esc("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
        assert_eq!(esc("x\u{0}y\u{8}z\tq"), "xyz\tq");
    }

    #[test]
    fn depth_is_bounded_and_ignores_comments_cdata_and_quoted_angle_brackets() {
        let deep = "<a>".repeat(MAX_DEPTH + 1);
        assert!(check_depth(&deep).is_err());
        assert!(check_depth(&"<a>".repeat(MAX_DEPTH)).is_ok());
        let decoys = format!("<r><!-- {} --><![CDATA[{}]]><e x=\"{}\"/></r>", "<a>".repeat(500), "<a>".repeat(500), "<a>".repeat(500));
        assert!(check_depth(&decoys).is_ok());
        // An unterminated tag ends the scan instead of looping.
        assert!(check_depth("<a x=\"").is_ok());
        assert!(check_depth("<!-- never closed").is_ok());
    }

    #[test]
    fn dtds_and_entity_bombs_are_refused() {
        let bomb = "<?xml version=\"1.0\"?><!DOCTYPE l [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]><l>&b;&b;</l>";
        let text = checked_text(bomb.as_bytes()).unwrap();
        assert!(matches!(parse(text), Err(Error::Xml(_))));
    }

    #[test]
    fn size_encoding_and_node_caps_apply() {
        assert!(matches!(checked_text(&vec![b' '; MAX_XML_BYTES + 1]), Err(Error::TooLarge(..))));
        assert!(matches!(checked_text(&[0xff, 0xfe, 0x00]), Err(Error::Xml(_))));
        let many = format!("<r>{}</r>", "<e/>".repeat(MAX_NODES as usize + 10));
        let text = checked_text(many.as_bytes()).unwrap();
        assert!(parse(text).is_err());
    }

    #[test]
    fn qnames_resolve_in_scope() {
        let doc = parse("<r xmlns:p=\"urn:p\" xmlns=\"urn:d\"><e/></r>").unwrap();
        let e = doc.root_element().first_element_child().unwrap();
        assert_eq!(resolve_qname(e, "p:x"), Some((Some("urn:p".into()), "x".into())));
        assert_eq!(resolve_qname(e, "x"), Some((Some("urn:d".into()), "x".into())));
        assert_eq!(resolve_qname(e, "q:x"), None);
        assert_eq!(resolve_qname(e, ":x"), None);
        assert_eq!(resolve_qname(e, "a:b:c"), None);
        assert_eq!(resolve_qname(e, ""), None);
    }

    #[test]
    fn qname_lists_resolve_per_element_and_skip_undeclared_prefixes() {
        let doc = parse("<r xmlns:a=\"urn:a\"><t xmlns:b=\"urn:b\"> a:One b:Two c:Three  plain a:One </t></r>").unwrap();
        let t = doc.root_element().first_element_child().unwrap();
        let q = qname_list(t, 10);
        assert_eq!(q, vec![QName::new("urn:a", "One"), QName::new("urn:b", "Two"), QName::new("urn:a", "One")]);
        assert_eq!(qname_list(t, 1).len(), 1);
        assert!(qname_list(t, 0).is_empty());
        assert_eq!(q[0].to_string(), "{urn:a}One");
        assert!(q[0].is("urn:a", "One") && !q[0].is("urn:b", "One"));
    }

    #[test]
    fn ncnames_are_checked() {
        for ok in ["a", "_x", "Foo-bar.1"] {
            assert!(is_ncname(ok), "{ok}");
        }
        for bad in ["", "1a", "a:b", "a b", "a<b", "\u{e9}", &"x".repeat(200)] {
            assert!(!is_ncname(bad), "{bad}");
        }
    }

    #[test]
    fn text_is_trimmed_and_capped() {
        let big = format!("<r>  hi {} </r>", "x".repeat(MAX_TEXT * 2));
        let doc = parse(&big).unwrap();
        let t = text(doc.root_element());
        assert!(t.starts_with("hi x") && t.chars().count() == MAX_TEXT);
        assert_eq!(parse_bool(" true "), Some(true));
        assert_eq!(parse_bool("yes"), None);
    }
}
