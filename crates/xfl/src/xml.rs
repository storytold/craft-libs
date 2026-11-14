//! AniCraft XML (layer L0): a small, strict-enough XML element reader shared by the importers.
//!
//! XFL and SVG keep everything that matters in elements and attributes (XFL even stores shape
//! geometry as an attribute), so the reader builds a tree of elements with attributes. An
//! element's own character data (text and CDATA, such as XFL's `<characters>`) is kept in
//! [`Element::text`], except whitespace-only text between child elements (indentation). It
//! handles the prolog, comments, CDATA, doctype, processing instructions, the five standard
//! entities and numeric character references. Limits on input size, depth and
//! element count keep a hostile file from exhausting memory or the stack. Never panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;

/// Largest document read (bytes).
pub const MAX_INPUT: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 256;
const MAX_ELEMENTS: usize = 4_000_000;

#[derive(Debug, Default, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<Element>,
    /// The element's character data, entities resolved (XFL's `<characters>`). Whitespace
    /// between child elements is dropped.
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn f32(&self, name: &str) -> Option<f32> {
        self.attr(name).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
    }
    pub fn u32(&self, name: &str) -> Option<u32> {
        self.attr(name).and_then(|v| v.trim().parse::<u32>().ok())
    }
    pub fn bool(&self, name: &str) -> Option<bool> {
        self.attr(name).map(|v| v.trim() == "true")
    }
    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
    /// Children of the first child named `name` (XFL wraps lists: `<layers><DOMLayer/>…`).
    pub fn list(&self, name: &str) -> &[Element] {
        self.child(name).map_or(&[], |c| c.children.as_slice())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum XmlError {
    #[error("the XML is too large")]
    TooLarge,
    #[error("the XML is nested too deeply")]
    TooDeep,
    #[error("the XML has too many elements")]
    TooMany,
    #[error("malformed XML at byte {0}: {1}")]
    Syntax(usize, &'static str),
}

/// The name of the pseudo-elements that hold character data in mixed content (see
/// [`parse_mixed`]).
pub const TEXT_NODE: &str = "#text";

struct P<'a> {
    s: &'a [u8],
    i: usize,
    count: usize,
    /// Elements (by name without namespace prefix) whose character data is also kept in order
    /// among their children, as [`TEXT_NODE`] children.
    mixed: &'a [&'a str],
}

impl P<'_> {
    fn err<T>(&self, what: &'static str) -> Result<T, XmlError> {
        Err(XmlError::Syntax(self.i, what))
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn starts(&self, pat: &[u8]) -> bool {
        self.s.get(self.i..).is_some_and(|r| r.starts_with(pat))
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    /// Skips past `end` (used for comments, CDATA, PIs, doctype).
    fn skip_until(&mut self, end: &[u8]) -> Result<(), XmlError> {
        let rest = self.s.get(self.i..).unwrap_or_default();
        match rest.windows(end.len()).position(|w| w == end) {
            Some(p) => {
                self.i += p + end.len();
                Ok(())
            }
            None => self.err("unterminated construct"),
        }
    }
    fn name(&mut self) -> Result<String, XmlError> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80) {
            self.i += 1;
        }
        if self.i == start {
            return self.err("expected a name");
        }
        Ok(String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or_default()).into_owned())
    }
    /// Skips text, comments and other non-element content up to the next tag, adding text and
    /// CDATA to `text` when there is one.
    fn skip_misc(&mut self, mut text: Option<&mut String>) -> Result<(), XmlError> {
        loop {
            let start = self.i;
            while self.peek().is_some_and(|c| c != b'<') {
                self.i += 1;
            }
            if let Some(t) = text.as_deref_mut()
                && self.i > start
            {
                let raw = String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or_default());
                if raw.contains('&') {
                    t.push_str(&unescape(&raw));
                } else {
                    t.push_str(&raw);
                }
            }
            if self.starts(b"<!--") {
                self.skip_until(b"-->")?;
            } else if self.starts(b"<![CDATA[") {
                let from = self.i + 9;
                self.skip_until(b"]]>")?;
                if let Some(t) = text.as_deref_mut() {
                    t.push_str(&String::from_utf8_lossy(self.s.get(from..self.i.saturating_sub(3)).unwrap_or_default()));
                }
            } else if self.starts(b"<?") {
                self.skip_until(b"?>")?;
            } else if self.starts(b"<!") {
                self.skip_until(b">")?;
            } else {
                return Ok(());
            }
        }
    }
    fn element(&mut self, depth: usize) -> Result<Element, XmlError> {
        if depth > MAX_DEPTH {
            return Err(XmlError::TooDeep);
        }
        self.count += 1;
        if self.count > MAX_ELEMENTS {
            return Err(XmlError::TooMany);
        }
        if self.peek() != Some(b'<') {
            return self.err("expected '<'");
        }
        self.i += 1;
        let mut el = Element { name: self.name()?, ..Element::default() };
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.i += 1;
                    if self.peek() != Some(b'>') {
                        return self.err("expected '>' after '/'");
                    }
                    self.i += 1;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) => {
                    let k = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return self.err("expected '=' in an attribute");
                    }
                    self.i += 1;
                    self.skip_ws();
                    let q = match self.peek() {
                        Some(q @ (b'"' | b'\'')) => q,
                        _ => return self.err("expected a quoted attribute value"),
                    };
                    self.i += 1;
                    let start = self.i;
                    while self.peek().is_some_and(|c| c != q) {
                        self.i += 1;
                    }
                    if self.peek().is_none() {
                        return self.err("unterminated attribute value");
                    }
                    let raw = String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or_default()).into_owned();
                    self.i += 1;
                    el.attrs.insert(k, unescape(&raw));
                }
                None => return self.err("unexpected end inside a tag"),
            }
        }
        // Content: children until the matching end tag. In mixed content each stretch of
        // character data also becomes a text node, so it keeps its place among the children.
        let local = el.name.rsplit(':').next().unwrap_or(&el.name);
        let mixed = self.mixed.contains(&local);
        loop {
            let before = el.text.len();
            self.skip_misc(Some(&mut el.text))?;
            if mixed && el.text.len() > before {
                self.count += 1;
                if self.count > MAX_ELEMENTS {
                    return Err(XmlError::TooMany);
                }
                let text = el.text.get(before..).unwrap_or_default().to_string();
                el.children.push(Element { name: TEXT_NODE.to_string(), text, ..Element::default() });
            }
            if self.peek().is_none() {
                return self.err("missing end tag");
            }
            if self.starts(b"</") {
                self.i += 2;
                let n = self.name()?;
                self.skip_ws();
                if self.peek() != Some(b'>') || n != el.name {
                    return self.err("mismatched end tag");
                }
                self.i += 1;
                if !mixed && !el.children.is_empty() && el.text.trim().is_empty() {
                    el.text = String::new();
                }
                return Ok(el);
            }
            el.children.push(self.element(depth + 1)?);
        }
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let tail = rest.get(i..).unwrap_or("");
        let Some(end) = tail.find(';').filter(|e| *e <= 12) else {
            out.push('&');
            rest = tail.get(1..).unwrap_or("");
            continue;
        };
        let ent = tail.get(1..end).unwrap_or("");
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = tail.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses a document and returns its root element.
pub fn parse(src: &[u8]) -> Result<Element, XmlError> {
    parse_mixed(src, &[])
}

/// Parses a document whose `mixed` elements (named without namespace prefix, such as SVG's
/// `text` and `tspan`) have mixed content: their character data is also kept among their
/// children, in order, as [`TEXT_NODE`] elements whose `text` is that stretch (whitespace
/// included). Everything else is read as [`parse`] reads it.
pub fn parse_mixed(src: &[u8], mixed: &[&str]) -> Result<Element, XmlError> {
    if src.len() > MAX_INPUT {
        return Err(XmlError::TooLarge);
    }
    // A UTF-8 byte-order mark is allowed.
    let src = src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(src);
    let mut p = P { s: src, i: 0, count: 0, mixed };
    p.skip_misc(None)?;
    if p.peek().is_none() {
        return p.err("no root element");
    }
    p.element(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_entities() {
        let x = br#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment -->
<DOMDocument xmlns="http://ns.adobe.com/xfl/2008/" width="640" frameRate='30'>
  <timelines><DOMTimeline name="Scene &amp; &quot;1&quot; &#65;&#x42;"><layers/></DOMTimeline></timelines>
  <![CDATA[ <ignored/> ]]>
  text is ignored
</DOMDocument>"#;
        let root = parse(x).unwrap();
        assert_eq!(root.name, "DOMDocument");
        assert_eq!(root.f32("width"), Some(640.0));
        assert_eq!(root.u32("frameRate"), Some(30));
        let tl = &root.list("timelines")[0];
        assert_eq!(tl.attr("name"), Some("Scene & \"1\" AB"));
        assert!(tl.list("layers").is_empty());
        assert_eq!(root.children.len(), 1, "CDATA and text are not elements");
        assert!(root.text.contains(" <ignored/> ") && root.text.contains("text is ignored"), "{:?}", root.text);
    }

    #[test]
    fn character_data_is_kept_and_indentation_dropped() {
        let x = b"<t>\n  <characters>a &lt;b&gt;&#xD;line 2</characters>\n  <characters> </characters><c><![CDATA[x<y]]>z</c>\n</t>";
        let root = parse(x).unwrap();
        assert_eq!(root.text, "", "whitespace between elements");
        let texts: Vec<&str> = root.children.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["a <b>\rline 2", " ", "x<yz"]);
    }

    #[test]
    fn mixed_content_keeps_text_in_order() {
        let x = b"<svg><text x='1'>Hello <tspan>big</tspan> world&amp;<!-- c --> more</text>
  <g>
</g></svg>";
        let root = parse_mixed(x, &["text", "tspan"]).unwrap();
        let t = &root.children[0];
        let parts: Vec<(&str, &str)> = t.children.iter().map(|c| (c.name.as_str(), c.text.as_str())).collect();
        assert_eq!(parts, [(TEXT_NODE, "Hello "), ("tspan", "big"), (TEXT_NODE, " world& more")]);
        assert_eq!(t.children[1].children.len(), 1, "a tspan's own text is a node too");
        assert_eq!(t.text, "Hello  world& more", "the concatenation is kept as before");
        assert_eq!(root.children.len(), 2, "elements not listed keep dropping whitespace");
        assert!(root.children[1].children.is_empty());
        assert_eq!(parse(x).unwrap().children[0].children.len(), 1, "plain parsing makes no text nodes");
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        for bad in [&b""[..], b"<", b"<a", b"<a b>", b"<a b='1>", b"<a></b>", b"<a><b></a>", b"<!-- open", b"<a>\xff\xfe"] {
            assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let deep = "<a>".repeat(400) + &"</a>".repeat(400);
        assert_eq!(parse(deep.as_bytes()).unwrap_err(), XmlError::TooDeep);
        assert_eq!(unescape("a &unknown; b &amp c"), "a &unknown; b &amp c");
    }
}
