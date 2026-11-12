//! XML for the xmeml and FCPXML modules: bounded reading and pretty writing through craft-xml,
//! with the small node view their readers are written against.

pub(crate) use craft_xml::esc_attr as escape_attr;
use craft_xml::{Doctype, El, Limits, Options, UnknownEntity};

/// The reader's limits: craft-xml's defaults, which stop a document at 256 levels of nesting
/// (real timelines nest a few levels per nested sequence), 2 000 000 nodes and 256 MiB.
pub const LIMITS: Limits = Limits::DEFAULT;

/// Read a document strictly. A plain `<!DOCTYPE xmeml>` is skipped; a declaration with an
/// internal subset is refused, and so is a reference to an entity XML does not define, so
/// nothing in a document can expand beyond its own size.
pub(crate) fn parse(text: &str) -> Result<El, String> {
    let options =
        Options { doctype: Doctype::RejectInternalSubset, unknown_entity: UnknownEntity::Error, ..Options::default() }.strict().with_limits(LIMITS);
    options.parse(text.as_bytes()).map_err(|e| e.to_string())
}

/// An element of a parsed document.
#[derive(Clone, Copy)]
pub(crate) struct Node<'a>(pub &'a El);

impl<'a> Node<'a> {
    pub fn name(self) -> &'a str {
        self.0.local()
    }
    pub fn has_tag_name(self, name: &str) -> bool {
        self.0.local() == name
    }
    pub fn attribute(self, name: &str) -> Option<&'a str> {
        self.0.attr(name)
    }
    /// Child elements.
    pub fn children(self) -> impl Iterator<Item = Node<'a>> {
        self.0.elements().map(Node)
    }
    /// This element and every element below it, in document order.
    pub fn descendants(self) -> impl Iterator<Item = Node<'a>> {
        std::iter::once(self).chain(self.0.descendants().map(Node))
    }
    /// The text this element starts with, if it starts with text.
    pub fn text(self) -> Option<&'a str> {
        match self.0.children.first() {
            Some(craft_xml::Node::Text(t) | craft_xml::Node::CData(t)) => Some(t.as_str()),
            _ => None,
        }
    }
    /// A number that is this element's alone while the document is alive.
    pub fn id(self) -> usize {
        std::ptr::from_ref(self.0) as usize
    }
}

pub(crate) fn child<'a>(n: Node<'a>, name: &str) -> Option<Node<'a>> {
    n.children().find(|c| c.name() == name)
}

pub(crate) fn children<'a>(n: Node<'a>, name: &'static str) -> impl Iterator<Item = Node<'a>> {
    n.children().filter(move |c| c.name() == name)
}

pub(crate) fn elements<'a>(n: Node<'a>) -> impl Iterator<Item = Node<'a>> {
    n.children()
}

/// Trimmed text of a node's direct text content.
pub(crate) fn text(n: Node<'_>) -> &str {
    n.text().map(str::trim).unwrap_or("")
}

/// Text of the element at `path` below `n`.
pub(crate) fn path_text<'a>(n: Node<'a>, path: &[&str]) -> Option<&'a str> {
    let mut cur = n;
    for p in path {
        cur = child(cur, p)?;
    }
    Some(text(cur))
}

pub(crate) fn child_text<'a>(n: Node<'a>, name: &str) -> Option<&'a str> {
    child(n, name).map(text)
}

pub(crate) fn child_i64(n: Node<'_>, name: &str) -> Option<i64> {
    child_text(n, name).and_then(|t| t.parse::<i64>().ok().or_else(|| t.parse::<f64>().ok().map(|f| f.round() as i64)))
}

pub(crate) fn child_f64(n: Node<'_>, name: &str) -> Option<f64> {
    child_text(n, name).and_then(|t| t.parse::<f64>().ok())
}

pub(crate) fn child_bool(n: Node<'_>, name: &str) -> Option<bool> {
    child_text(n, name).map(|t| t.eq_ignore_ascii_case("true") || t == "1")
}

pub(crate) fn bool_str(b: bool) -> &'static str {
    if b { "TRUE" } else { "FALSE" }
}

/// Format an f64 compactly (no trailing zeros, no exponent for normal ranges).
pub(crate) fn num(v: f64) -> String {
    if v == v.round() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    if !v.is_finite() {
        return "0".into();
    }
    // Shortest representation that parses back to the same f64.
    format!("{v}")
}

/// Pretty-printing XML writer (two-space indent, one element per line).
pub(crate) struct XmlWriter(craft_xml::Writer);

impl XmlWriter {
    pub fn new(doctype: Option<&str>) -> Self {
        let mut w = craft_xml::Writer::pretty("  ");
        w.raw("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        if let Some(d) = doctype {
            w.doctype(d);
        }
        Self(w)
    }
    pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.0.open(name, attrs);
    }
    pub fn close(&mut self) {
        self.0.close();
    }
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.0.empty(name, attrs);
    }
    /// `<name attrs>value</name>`.
    pub fn text_attrs(&mut self, name: &str, attrs: &[(&str, &str)], value: &str) {
        self.0.leaf(name, attrs, value);
    }
    pub fn text(&mut self, name: &str, value: impl std::fmt::Display) {
        self.text_attrs(name, &[], &value.to_string());
    }
    pub fn finish(self) -> String {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_roundtrip() {
        let mut w = XmlWriter::new(Some("xmeml"));
        w.open("a", &[("x", "1 & 2")]);
        w.text("b", "<hi>");
        w.empty("c", &[]);
        let s = w.finish();
        assert_eq!(s, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<a x=\"1 &amp; 2\">\n  <b>&lt;hi&gt;</b>\n  <c/>\n</a>\n");
        let d = parse(&s).unwrap();
        let a = Node(&d);
        assert_eq!(a.attribute("x"), Some("1 & 2"));
        assert_eq!(child_text(a, "b"), Some("<hi>"));
        assert!(child(a, "c").is_some());
        assert_eq!(a.descendants().map(Node::name).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_ne!(child(a, "b").map(Node::id), child(a, "c").map(Node::id));
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(0.1 + 0.2).parse::<f64>().unwrap(), 0.1 + 0.2);
    }

    #[test]
    fn attr_whitespace_roundtrip() {
        let mut w = XmlWriter::new(None);
        w.open("a", &[("name", "x\ny\tz\r")]);
        w.text("b", "a\nb");
        let s = w.finish();
        let d = parse(&s).unwrap();
        let a = Node(&d);
        assert_eq!(a.attribute("name"), Some("x\ny\tz\r"));
        assert_eq!(child(a, "b").and_then(Node::text), Some("a\nb"));
    }

    #[test]
    fn rejects_internal_dtd_entity_expansion() {
        let text = "<!DOCTYPE root [<!ENTITY word 'expanded'>]><root>&word;</root>";
        assert!(parse(text).is_err());
        assert!(parse("<root>&word;</root>").is_err(), "an entity XML does not define");
        assert!(parse("<!DOCTYPE xmeml>\n<xmeml version=\"4\"/>").is_ok());
    }

    /// Elements nested beyond the limit are an error, not a deep recursion.
    #[test]
    fn nesting_is_bounded() {
        let nested = |depth: usize| format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
        assert!(parse(&nested(LIMITS.max_depth)).is_ok());
        let e = parse(&nested(LIMITS.max_depth + 1)).unwrap_err();
        assert!(e.contains("deeper"), "{e}");
        assert!(parse(&nested(100_000)).is_err());
    }
}
