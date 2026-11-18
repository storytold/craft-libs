//! Lightweight namespace-aware XML DOM with source byte ranges, backed by `craft_xml`.

use std::ops::Range;

const MAX_NAMESPACE_BYTES: usize = 64 << 20;
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

#[derive(Clone, Debug)]
pub struct ParsingOptions {
    pub allow_dtd: bool,
    pub nodes_limit: u32,
}

impl Default for ParsingOptions {
    fn default() -> Self {
        Self { allow_dtd: false, nodes_limit: u32::MAX }
    }
}

#[derive(Clone, Debug)]
struct AttrData {
    ns: Option<String>,
    name: String,
    value: String,
}

#[derive(Clone, Debug)]
enum NodeKind {
    Element {
        ns: Option<String>,
        name: String,
        attrs: Vec<AttrData>,
        /// In-scope `(prefix, uri)` bindings at this element (`""` for default namespace).
        namespaces: Vec<(String, String)>,
        range: Range<usize>,
    },
    Text(String),
}

#[derive(Clone, Debug)]
struct NodeData {
    kind: NodeKind,
    children: Vec<usize>,
}

pub struct Document {
    nodes: Vec<NodeData>,
    root: usize,
}

#[derive(Clone, Copy)]
pub struct Node<'a> {
    doc: &'a Document,
    id: usize,
}

impl PartialEq for Node<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.doc, other.doc)
    }
}

impl Eq for Node<'_> {}

#[derive(Clone, Copy)]
pub struct ExpandedName<'a> {
    ns: Option<&'a str>,
    name: &'a str,
}

impl<'a> ExpandedName<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }

    pub fn namespace(&self) -> Option<&'a str> {
        self.ns
    }
}

pub trait AttributeKey {
    fn matches(&self, ns: Option<&str>, name: &str) -> bool;
}

impl AttributeKey for &str {
    fn matches(&self, ns: Option<&str>, name: &str) -> bool {
        ns.is_none() && name == *self
    }
}

impl AttributeKey for (&str, &str) {
    fn matches(&self, ns: Option<&str>, name: &str) -> bool {
        ns == Some(self.0) && name == self.1
    }
}

#[derive(Clone, Copy)]
pub struct Attribute<'a> {
    attr: &'a AttrData,
}

impl<'a> Attribute<'a> {
    pub fn name(&self) -> &'a str {
        &self.attr.name
    }

    pub fn namespace(&self) -> Option<&'a str> {
        self.attr.ns.as_deref()
    }

    pub fn value(&self) -> &'a str {
        &self.attr.value
    }
}

#[derive(Clone, Copy)]
pub struct Namespace<'a> {
    prefix: &'a str,
    uri: &'a str,
}

impl<'a> Namespace<'a> {
    pub fn name(&self) -> Option<&'a str> {
        (!self.prefix.is_empty()).then_some(self.prefix)
    }

    pub fn uri(&self) -> &'a str {
        self.uri
    }
}

fn split_qname(qname: &str) -> (&str, &str) {
    match qname.split_once(':') {
        Some((p, l)) => (p, l),
        None => ("", qname),
    }
}

// One long inherited URI otherwise gets copied into every descendant. Charge retained
// namespace strings and snapshot entry headers before copying, across the whole document.
fn charge_namespace(bytes: &mut usize, amount: usize) -> Result<(), String> {
    *bytes = bytes.checked_sub(amount).ok_or_else(|| "namespace retention budget exceeded".to_string())?;
    Ok(())
}

fn copy_namespace(uri: &str, bytes: &mut usize) -> Result<Option<String>, String> {
    charge_namespace(bytes, uri.len())?;
    Ok(Some(uri.to_string()))
}

fn resolve_ns(scope: &[(String, String)], prefix: &str, is_attr: bool, bytes: &mut usize) -> Result<Option<String>, String> {
    if prefix == "xml" {
        return copy_namespace(XML_NS, bytes);
    }
    if prefix == "xmlns" {
        return copy_namespace(XMLNS_NS, bytes);
    }
    if prefix.is_empty() {
        if is_attr {
            return Ok(None);
        }
        return match scope.iter().rev().find(|(p, _)| p.is_empty()) {
            Some((_, u)) if !u.is_empty() => copy_namespace(u, bytes),
            _ => Ok(None),
        };
    }
    match scope.iter().rev().find(|(p, _)| p == prefix) {
        Some((_, u)) if !u.is_empty() => copy_namespace(u, bytes),
        _ => Err(format!("undeclared namespace prefix: {prefix}")),
    }
}
impl Document {
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::parse_with_options(text, ParsingOptions::default())
    }

    pub fn parse_with_options(text: &str, options: ParsingOptions) -> Result<Self, String> {
        Self::parse_with_namespace_budget(text, options, MAX_NAMESPACE_BYTES)
    }

    fn parse_with_namespace_budget(text: &str, options: ParsingOptions, mut namespace_bytes: usize) -> Result<Self, String> {
        let mut opts = craft_xml::Options {
            strict: true,
            doctype: if options.allow_dtd { craft_xml::Doctype::Ignore } else { craft_xml::Doctype::Reject },
            ..Default::default()
        };
        opts.limits.max_nodes = options.nodes_limit as usize;
        opts.limits.max_depth = crate::MAX_XML_DEPTH;

        // quick-xml excludes a leading UTF-8 BOM from its buffer positions.
        // Parse it explicitly so every stored range still addresses the original input.
        let input = text.strip_prefix('\u{feff}').unwrap_or(text);
        // A second U+FEFF is content outside the root, not another encoding marker.
        if input.starts_with('\u{feff}') {
            return Err("multiple leading XML byte-order marks".to_string());
        }
        let source_offset = text.len() - input.len();
        let mut events = opts.events(input.as_bytes());
        let mut nodes: Vec<NodeData> = Vec::new();
        let mut open: Vec<(usize, usize)> = Vec::new(); // (node_id, scope_len_before)
        let mut scope: Vec<(String, String)> = Vec::new();
        let mut root: Option<usize> = None;

        while let Some(ev) = events.next().map_err(|e| e.to_string())? {
            let span = events.span();
            let span = span.start + source_offset..span.end + source_offset;
            match ev {
                craft_xml::Event::Start { name, attrs } => {
                    if nodes.len() >= options.nodes_limit as usize {
                        return Err("nodes limit reached".to_string());
                    }
                    let scope_mark = scope.len();
                    for (k, v) in &attrs {
                        if k == "xmlns" {
                            scope.push((String::new(), v.clone()));
                        } else if let Some(prefix) = k.strip_prefix("xmlns:") {
                            scope.push((prefix.to_string(), v.clone()));
                        }
                    }
                    let (el_prefix, el_local) = split_qname(&name);
                    let el_ns = resolve_ns(&scope, el_prefix, false, &mut namespace_bytes)?;
                    let mut el_attrs = Vec::new();
                    for (k, v) in attrs {
                        if k == "xmlns" || k.starts_with("xmlns:") {
                            continue;
                        }
                        let (ap, al) = split_qname(&k);
                        let ans = resolve_ns(&scope, ap, true, &mut namespace_bytes)?;
                        el_attrs.push(AttrData { ns: ans, name: al.to_string(), value: v });
                    }
                    // Deduplicate in-scope namespace bindings, keeping the innermost binding per prefix.
                    let mut namespaces: Vec<(String, String)> = Vec::new();
                    for (p, u) in scope.iter().rev() {
                        if !namespaces.iter().any(|(ep, _)| ep == p) {
                            charge_namespace(&mut namespace_bytes, std::mem::size_of::<(String, String)>())?;
                            charge_namespace(&mut namespace_bytes, p.len())?;
                            charge_namespace(&mut namespace_bytes, u.len())?;
                            namespaces.push((p.clone(), u.clone()));
                        }
                    }
                    namespaces.retain(|(_, u)| !u.is_empty());
                    let id = nodes.len();
                    nodes.push(NodeData {
                        kind: NodeKind::Element { ns: el_ns, name: el_local.to_string(), attrs: el_attrs, namespaces, range: span.start..span.end },
                        children: Vec::new(),
                    });
                    if let Some(&(parent, _)) = open.last() {
                        nodes[parent].children.push(id);
                    } else if root.is_none() {
                        root = Some(id);
                    }
                    open.push((id, scope_mark));
                }
                craft_xml::Event::End { .. } => {
                    if let Some((id, scope_mark)) = open.pop() {
                        if let NodeKind::Element { range, .. } = &mut nodes[id].kind {
                            range.end = span.end;
                        }
                        scope.truncate(scope_mark);
                    }
                }
                craft_xml::Event::Text(t) | craft_xml::Event::CData(t) => {
                    if let Some(&(parent, _)) = open.last() {
                        if let Some(&last_child) = nodes[parent].children.last()
                            && let NodeKind::Text(existing) = &mut nodes[last_child].kind
                        {
                            existing.push_str(&t);
                            continue;
                        }
                        if nodes.len() >= options.nodes_limit as usize {
                            return Err("nodes limit reached".to_string());
                        }
                        let id = nodes.len();
                        nodes.push(NodeData { kind: NodeKind::Text(t), children: Vec::new() });
                        nodes[parent].children.push(id);
                    }
                }
                craft_xml::Event::Pi { .. } | craft_xml::Event::Comment(_) => {}
            }
        }

        let Some(root) = root else {
            return Err("empty XML document".to_string());
        };
        Ok(Self { nodes, root })
    }

    pub fn root_element(&self) -> Node<'_> {
        Node { doc: self, id: self.root }
    }

    pub fn get_node(&self, id: NodeId) -> Option<Node<'_>> {
        (id.0 < self.nodes.len()).then_some(Node { doc: self, id: id.0 })
    }

    pub fn descendants(&self) -> Descendants<'_> {
        Descendants { doc: self, stack: vec![self.root] }
    }
}

pub struct Descendants<'a> {
    doc: &'a Document,
    stack: Vec<usize>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        let children = &self.doc.nodes[id].children;
        for &c in children.iter().rev() {
            self.stack.push(c);
        }
        Some(Node { doc: self.doc, id })
    }
}

impl<'a> Node<'a> {
    pub fn id(&self) -> NodeId {
        NodeId(self.id)
    }

    pub fn document(&self) -> &'a Document {
        self.doc
    }

    pub fn is_element(&self) -> bool {
        matches!(self.doc.nodes[self.id].kind, NodeKind::Element { .. })
    }

    pub fn is_text(&self) -> bool {
        matches!(self.doc.nodes[self.id].kind, NodeKind::Text(_))
    }

    pub fn tag_name(&self) -> ExpandedName<'a> {
        match &self.doc.nodes[self.id].kind {
            NodeKind::Element { ns, name, .. } => ExpandedName { ns: ns.as_deref(), name },
            NodeKind::Text(_) => ExpandedName { ns: None, name: "" },
        }
    }

    pub fn attribute<K: AttributeKey>(&self, key: K) -> Option<&'a str> {
        match &self.doc.nodes[self.id].kind {
            NodeKind::Element { attrs, .. } => attrs.iter().find(|a| key.matches(a.ns.as_deref(), &a.name)).map(|a| a.value.as_str()),
            NodeKind::Text(_) => None,
        }
    }

    pub fn attributes(&self) -> impl ExactSizeIterator<Item = Attribute<'a>> + use<'a> {
        let attrs = match &self.doc.nodes[self.id].kind {
            NodeKind::Element { attrs, .. } => attrs.as_slice(),
            NodeKind::Text(_) => &[],
        };
        attrs.iter().map(|attr| Attribute { attr })
    }

    pub fn namespaces(&self) -> impl ExactSizeIterator<Item = Namespace<'a>> + use<'a> {
        let ns = match &self.doc.nodes[self.id].kind {
            NodeKind::Element { namespaces, .. } => namespaces.as_slice(),
            NodeKind::Text(_) => &[],
        };
        ns.iter().map(|(p, u)| Namespace { prefix: p, uri: u })
    }

    pub fn lookup_prefix(&self, uri: &str) -> Option<&'a str> {
        if uri == XML_NS {
            return Some("xml");
        }
        match &self.doc.nodes[self.id].kind {
            NodeKind::Element { namespaces, .. } => namespaces.iter().find(|(_, u)| u == uri).map(|(p, _)| p.as_str()),
            NodeKind::Text(_) => None,
        }
    }

    pub fn lookup_namespace_uri(&self, prefix: Option<&str>) -> Option<&'a str> {
        if prefix == Some("xml") {
            return Some(XML_NS);
        }
        if prefix == Some("xmlns") {
            return Some(XMLNS_NS);
        }
        let target = prefix.unwrap_or("");
        match &self.doc.nodes[self.id].kind {
            NodeKind::Element { namespaces, .. } => namespaces.iter().find(|(p, _)| p == target).map(|(_, u)| u.as_str()),
            NodeKind::Text(_) => None,
        }
    }

    pub fn children(&self) -> impl Iterator<Item = Node<'a>> + use<'a> {
        let doc = self.doc;
        doc.nodes[self.id].children.iter().map(move |&id| Node { doc, id })
    }

    pub fn descendants(&self) -> Descendants<'a> {
        Descendants { doc: self.doc, stack: vec![self.id] }
    }

    pub fn text(&self) -> Option<&'a str> {
        match &self.doc.nodes[self.id].kind {
            NodeKind::Text(t) => Some(t.as_str()),
            NodeKind::Element { .. } => self.doc.nodes[self.id].children.first().and_then(|&cid| match &self.doc.nodes[cid].kind {
                NodeKind::Text(t) => Some(t.as_str()),
                _ => None,
            }),
        }
    }

    pub fn range(&self) -> Range<usize> {
        match &self.doc.nodes[self.id].kind {
            NodeKind::Element { range, .. } => range.clone(),
            NodeKind::Text(_) => 0..0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_ranges_slice_original_input_with_utf8_bom() {
        for prefix in ["", "\u{feff}", "\u{feff}<?xml version='1.0'?>"] {
            for child_xml in ["<é/>", "<é>text</é>"] {
                let root_xml = format!("<r>{child_xml}</r>");
                let input = format!("{prefix}{root_xml}");
                let doc = Document::parse(&input).unwrap();
                let root = doc.root_element();
                let child = root.children().next().unwrap();
                assert_eq!(input.get(root.range()), Some(root_xml.as_str()));
                assert_eq!(input.get(child.range()), Some(child_xml));
                assert_eq!(root.range(), prefix.len()..input.len());
                assert_eq!(child.range(), prefix.len() + 3..prefix.len() + 3 + child_xml.len());
            }
        }
    }

    #[test]
    fn repeated_leading_bom_is_rejected_before_event_parsing() {
        for prefix in ["\u{feff}\u{feff}", "\u{feff}\u{feff}\u{feff}"] {
            for body in ["<r><é/></r>", "<?xml version='1.0'?><r><é>text</é></r>"] {
                assert!(Document::parse(&format!("{prefix}{body}")).is_err());
            }
        }
    }

    #[test]
    fn inherited_namespace_copies_obey_aggregate_byte_admission() {
        let uri = format!("urn:{}", "x".repeat(124));
        let snapshot = std::mem::size_of::<(String, String)>() + 1 + uri.len();
        for (child, expanded_copies) in [("<n/>", 0), ("<p:n/>", 1), ("<n p:a='v'/>", 1), ("<p:n p:a='v'/>", 2)] {
            let input = format!("<r xmlns:p='{uri}'>{}</r>", child.repeat(8));
            let needed = 9 * snapshot + 8 * expanded_copies * uri.len();
            assert!(Document::parse_with_namespace_budget(&input, ParsingOptions::default(), needed).is_ok());
            assert!(Document::parse_with_namespace_budget(&input, ParsingOptions::default(), needed - 1).is_err());
            assert!(Document::parse_with_namespace_budget(&input, ParsingOptions::default(), 0).is_err());
        }
    }
    #[test]
    fn namespace_shadowing_attributes_text_and_source_ranges() {
        let text = "<root xmlns='urn:outer' xmlns:p='urn:p' a='v'><p:child xmlns='urn:inner' p:a='x'>one<![CDATA[two]]>three</p:child><leaf xmlns=''/></root>";
        let doc = Document::parse(text).unwrap();
        let root = doc.root_element();
        assert_eq!(root.tag_name().namespace(), Some("urn:outer"));
        assert_eq!(root.attribute("a"), Some("v"));
        let child = root.children().next().unwrap();
        assert_eq!(child.tag_name().namespace(), Some("urn:p"));
        assert_eq!(child.attribute(("urn:p", "a")), Some("x"));
        assert_eq!(child.lookup_namespace_uri(None), Some("urn:inner"));
        assert_eq!(child.text(), Some("onetwothree"));
        assert_eq!(child.children().count(), 1);
        assert!(text[child.range()].starts_with("<p:child"));
        assert!(text[child.range()].ends_with("</p:child>"));
        assert!(doc.get_node(child.id()).unwrap() == child);
        assert_eq!(root.children().nth(1).unwrap().tag_name().namespace(), None);
        assert_eq!(root.range(), 0..text.len());
    }

    #[test]
    fn traversal_identity_and_accessors_preserve_element_and_text_semantics() {
        let source = "<?before value?><r xmlns='urn:r' xmlns:p='urn:p' a='plain' p:a='qualified' xml:lang='en'>first<!--gap--><?inside value?>second<empty/><p:child>last</p:child></r>";
        let doc = Document::parse(source).unwrap();
        let other = Document::parse(source).unwrap();
        let root = doc.root_element();
        assert!(root != other.root_element());
        assert!(std::ptr::eq(root.document(), &doc));
        assert!(root.is_element());
        assert!(!root.is_text());
        assert_eq!(root.tag_name().name(), "r");
        assert_eq!(root.lookup_prefix("urn:p"), Some("p"));
        assert_eq!(root.lookup_prefix(XML_NS), Some("xml"));
        assert_eq!(root.lookup_prefix("absent"), None);
        assert_eq!(root.lookup_namespace_uri(Some("xml")), Some(XML_NS));
        assert_eq!(root.lookup_namespace_uri(Some("xmlns")), Some(XMLNS_NS));
        assert_eq!(root.lookup_namespace_uri(Some("absent")), None);
        let attrs: Vec<_> = root.attributes().map(|a| (a.namespace(), a.name(), a.value())).collect();
        assert_eq!(attrs, vec![(None, "a", "plain"), (Some("urn:p"), "a", "qualified"), (Some(XML_NS), "lang", "en")]);
        let namespaces: Vec<_> = root.namespaces().map(|n| (n.name(), n.uri())).collect();
        assert!(namespaces.contains(&(None, "urn:r")));
        assert!(namespaces.contains(&(Some("p"), "urn:p")));
    }

    #[test]
    fn descendants_and_text_nodes_preserve_identity_and_empty_accessors() {
        let doc =
            Document::parse("<r xmlns='urn:r' xmlns:p='urn:p'>first<!--gap--><?inside value?>second<empty/><p:child>last</p:child></r>").unwrap();
        let root = doc.root_element();
        let all: Vec<_> = doc.descendants().collect();
        assert_eq!(all.len(), 5);
        assert!(all.iter().zip(root.descendants()).all(|(a, b)| *a == b));
        assert_eq!(all.iter().filter(|n| n.is_element()).map(|n| n.tag_name().name()).collect::<Vec<_>>(), vec!["r", "empty", "child"]);
        let text = root.children().next().unwrap();
        assert!(text.is_text());
        assert!(!text.is_element());
        assert_eq!(text.text(), Some("firstsecond"));
        assert_eq!(text.tag_name().name(), "");
        assert_eq!(text.tag_name().namespace(), None);
        assert_eq!(text.attribute("a"), None);
        assert_eq!(text.attributes().len(), 0);
        assert_eq!(text.namespaces().len(), 0);
        assert_eq!(text.lookup_prefix("urn:p"), None);
        assert_eq!(text.lookup_namespace_uri(Some("p")), None);
        assert_eq!(text.range(), 0..0);
        assert_eq!(text.children().count(), 0);
        assert_eq!(text.descendants().count(), 1);
        assert!(doc.get_node(NodeId(usize::MAX)).is_none());
        assert_eq!(root.children().nth(1).unwrap().text(), None);
        assert_eq!(Document::parse("<r><child/>tail</r>").unwrap().root_element().text(), None);
    }

    #[test]
    fn inherited_namespace_declarations_have_an_aggregate_budget() {
        use std::fmt::Write;
        let mut xml = String::from("<r");
        for i in 0..512 {
            write!(xml, " xmlns:p{i}='u{i}'").unwrap();
        }
        xml.push_str("><child");
        for i in 512..1024 {
            write!(xml, " xmlns:p{i}='u{i}'").unwrap();
        }
        let accepted = format!("{xml}/></r>");
        assert!(Document::parse(&accepted).is_ok());
        let rejected = format!("{xml} xmlns:extra='extra'/></r>");
        assert!(Document::parse(&rejected).is_err());
    }

    #[test]
    fn malformed_dtd_and_resource_limits_are_preserved() {
        // Current shared XML rejects duplicate attributes, unlike the older app pin.
        assert!(Document::parse("<r a='1' a='2'/>").is_err());
        assert!(Document::parse("<r xmlns:a='u' xmlns:b='u' a:x='1' b:x='2'/>").is_err());
        assert!(Document::parse("<p:root/>").is_err());
        assert!(Document::parse("<!DOCTYPE root><root/>").is_err());
        assert!(Document::parse_with_options("<!DOCTYPE root><root/>", ParsingOptions { allow_dtd: true, nodes_limit: 8 }).is_ok());
        assert!(Document::parse_with_options("<root>text</root>", ParsingOptions { allow_dtd: false, nodes_limit: 1 }).is_err());
        assert!(Document::parse_with_options("<root/>", ParsingOptions { allow_dtd: false, nodes_limit: 0 }).is_err());
        let text = format!("{}{}", "<n>".repeat(1025), "</n>".repeat(1025));
        assert!(Document::parse(&text).is_err());
        assert!(Document::parse("").is_err());
    }
}
