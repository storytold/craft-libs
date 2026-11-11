//! Writing: escaping, serialising a tree, and a writer for documents built without a tree.

use std::fmt::{self, Display, Write as _};

use crate::tree::{El, Node, Step, walk};

/// The XML declaration, with a newline.
pub const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
/// The XML declaration the office formats use (`standalone="yes"`), with a newline.
pub const DECL_STANDALONE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// May `c` appear in an XML 1.0 document? Control characters other than tab, LF and CR, and
/// U+FFFE and U+FFFF, may not, even as character references.
pub fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// Is `s` an XML 1.0 name (an element, attribute or processing-instruction target name)?
pub fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(is_name_start)
        && chars.all(|c| is_name_start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'))
}

/// `s` as element content: `&`, `<` and `>` escaped, a carriage return written as `&#13;` (a
/// literal one would be read back as a line feed), characters XML cannot carry dropped.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    push_esc(&mut o, s);
    o
}

/// Appends [`esc`]`(s)` to `o`.
pub fn push_esc(o: &mut String, s: &str) {
    // Most text is plain ASCII with nothing to escape.
    if s.bytes().all(|b| matches!(b, b'\t' | b'\n' | 0x20..=0x7E) && !matches!(b, b'&' | b'<' | b'>')) {
        o.push_str(s);
        return;
    }
    o.reserve(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\r' => o.push_str("&#13;"),
            c if is_xml_char(c) => o.push(c),
            _ => {}
        }
    }
}

/// `s` as a double-quoted attribute value: `&`, `<`, `>` and `"` escaped, tab, LF and CR written
/// as character references (literal ones would be read back as spaces), characters XML cannot
/// carry dropped.
pub fn esc_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    push_esc_attr(&mut o, s);
    o
}

/// Appends [`esc_attr`]`(s)` to `o`.
pub fn push_esc_attr(o: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\t' => o.push_str("&#9;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            c if is_xml_char(c) => o.push(c),
            _ => {}
        }
    }
}

/// Escapes whatever is formatted into it as an attribute value.
struct AttrSink<'a>(&'a mut String);

impl fmt::Write for AttrSink<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        push_esc_attr(self.0, s);
        Ok(())
    }
}

fn push_attr(o: &mut String, name: &str, value: impl Display) {
    o.push(' ');
    o.push_str(name);
    o.push_str("=\"");
    let _ = write!(AttrSink(o), "{value}");
    o.push('"');
}

fn push_pi(o: &mut String, target: &str, data: &str) {
    // A target that is not a name, the reserved target `xml`, or data that would end the
    // instruction early cannot be written.
    if !is_name(target) || target.eq_ignore_ascii_case("xml") || data.contains("?>") {
        return;
    }
    o.push_str("<?");
    o.push_str(target);
    if !data.is_empty() {
        o.push(' ');
        o.extend(data.chars().filter(|c| is_xml_char(*c)));
    }
    o.push_str("?>");
}

fn push_cdata(o: &mut String, text: &str) {
    o.push_str("<![CDATA[");
    // `]]>` cannot appear in a section: end the section inside it and start another.
    for (i, part) in text.split("]]>").enumerate() {
        if i > 0 {
            o.push_str("]]]]><![CDATA[>");
        }
        o.extend(part.chars().filter(|c| is_xml_char(*c)));
    }
    o.push_str("]]>");
}

fn push_leaf(o: &mut String, node: &Node) {
    match node {
        Node::El(_) => {}
        Node::Text(t) => push_esc(o, t),
        Node::CData(t) => push_cdata(o, t),
        Node::Pi { target, data } => push_pi(o, target, data),
        // `--` cannot appear in a comment and a comment cannot end with `-`.
        Node::Comment(c) if c.contains("--") || c.ends_with('-') => {}
        Node::Comment(c) => {
            o.push_str("<!--");
            o.extend(c.chars().filter(|ch| is_xml_char(*ch)));
            o.push_str("-->");
        }
    }
}

fn push_start(o: &mut String, e: &El) {
    o.push('<');
    o.push_str(&e.name);
    for (k, v) in e.attrs.iter().filter(|(k, _)| is_name(k)) {
        push_attr(o, k, v);
    }
}

fn push_end(o: &mut String, e: &El) {
    o.push_str("</");
    o.push_str(&e.name);
    o.push('>');
}

fn push_el(o: &mut String, root: &El) {
    // Where each open element's content starts: one that wrote none is closed as `<a/>`, so
    // that what is written reads back as the same text however much of it was left out.
    let mut content = Vec::new();
    walk(root, |step| {
        match step {
            Step::Enter(e) => {
                if !is_name(&e.name) {
                    return false;
                }
                push_start(o, e);
                if e.children.is_empty() {
                    o.push_str("/>");
                    return false;
                }
                o.push('>');
                content.push(o.len());
            }
            Step::Leave(e) => {
                if content.pop() == Some(o.len()) {
                    o.pop();
                    o.push_str("/>");
                } else {
                    push_end(o, e);
                }
            }
            Step::Leaf(n) => push_leaf(o, n),
        }
        true
    });
}

fn is_blank(n: &Node) -> bool {
    matches!(n, Node::Text(t) if t.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')))
}

/// Children that are all elements (with at most blank text between them) go one per line.
fn is_block(e: &El) -> bool {
    let mut any = false;
    for n in &e.children {
        match n {
            Node::El(c) => any |= is_name(&c.name),
            n if is_blank(n) => {}
            _ => return false,
        }
    }
    any
}

fn push_pretty(o: &mut String, root: &El, indent: &str, base: usize) {
    let mut depth = base;
    let pad = |o: &mut String, depth: usize| (0..depth).for_each(|_| o.push_str(indent));
    walk(root, |step| {
        match step {
            Step::Enter(e) => {
                if !is_name(&e.name) {
                    return false;
                }
                pad(o, depth);
                if !is_block(e) {
                    push_el(o, e);
                    o.push('\n');
                    return false;
                }
                push_start(o, e);
                o.push_str(">\n");
                depth += 1;
            }
            Step::Leave(e) => {
                depth = depth.saturating_sub(1);
                pad(o, depth);
                push_end(o, e);
                o.push('\n');
            }
            // Blank text between the children of a block.
            Step::Leaf(_) => {}
        }
        true
    });
}

impl El {
    /// The element as XML on one line, with no declaration. See [`El::write_xml`].
    pub fn to_xml(&self) -> String {
        let mut s = String::new();
        self.write_xml(&mut s);
        s
    }

    /// Appends the element as XML. Text is escaped with [`esc`] and attribute values with
    /// [`esc_attr`]; an element without children is written `<a/>`. An element or attribute
    /// whose name is not an XML name ([`is_name`]) is left out with everything inside it, so a
    /// name can never inject markup. Reading the result back gives an equal tree, apart from
    /// such names and characters XML cannot carry.
    pub fn write_xml(&self, out: &mut String) {
        push_el(out, self);
    }

    /// The element as indented XML, ending with a newline. See [`El::write_xml_pretty`].
    pub fn to_xml_pretty(&self, indent: &str) -> String {
        let mut s = String::new();
        self.write_xml_pretty(&mut s, indent);
        s
    }

    /// Appends the element as indented XML: an element whose children are all elements has them
    /// one per line, `indent` deeper each level, and any other element is written on one line
    /// exactly as [`El::write_xml`] writes it. Character data is therefore never changed, but
    /// whitespace-only text between the children of an element-only element is not kept.
    pub fn write_xml_pretty(&self, out: &mut String, indent: &str) {
        push_pretty(out, self, indent, 0);
    }
}

/// Builds XML text without a tree. Element names are closed for the caller, attribute values and
/// text are escaped, and with [`Writer::pretty`] each element goes on its own line.
///
/// Names are written as given: they are the caller's own constants, not checked.
///
/// ```
/// let mut w = craft_xml::Writer::document();
/// w.tag("w:p").attr("w:id", 7).open();
/// w.leaf("w:t", &[("xml:space", "preserve")], " a < b ");
/// w.empty("w:br", &[]);
/// assert_eq!(
///     w.finish(),
///     "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:p w:id=\"7\"><w:t xml:space=\"preserve\"> a &lt; b </w:t><w:br/></w:p>"
/// );
/// ```
#[derive(Clone, Debug, Default)]
pub struct Writer {
    out: String,
    /// The names of the open elements, one after another, and where each starts.
    names: String,
    open: Vec<usize>,
    indent: Option<String>,
}

/// A start tag being written: add attributes, then end it with [`Tag::open`], [`Tag::empty`] or
/// [`Tag::text`].
#[must_use = "end the tag with `open`, `empty` or `text`"]
pub struct Tag<'w> {
    w: &'w mut Writer,
    name: &'w str,
}

impl Tag<'_> {
    /// An attribute; `value` is formatted and escaped.
    pub fn attr(self, name: &str, value: impl Display) -> Self {
        push_attr(&mut self.w.out, name, value);
        self
    }
    /// An attribute when there is a value.
    pub fn attr_opt(self, name: &str, value: Option<impl Display>) -> Self {
        match value {
            Some(v) => self.attr(name, v),
            None => self,
        }
    }
    /// Several attributes.
    pub fn attrs(self, attrs: &[(&str, &str)]) -> Self {
        attrs.iter().fold(self, |tag, (k, v)| tag.attr(k, v))
    }
    /// `<name …>`: the element stays open until [`Writer::close`].
    pub fn open(self) {
        self.w.out.push('>');
        self.w.open.push(self.w.names.len());
        self.w.names.push_str(self.name);
        self.w.newline();
    }
    /// `<name …/>`.
    pub fn empty(self) {
        self.w.out.push_str("/>");
        self.w.newline();
    }
    /// `<name …>text</name>`.
    pub fn text(self, text: &str) {
        self.w.out.push('>');
        push_esc(&mut self.w.out, text);
        self.w.out.push_str("</");
        self.w.out.push_str(self.name);
        self.w.out.push('>');
        self.w.newline();
    }
}

impl Writer {
    /// An empty writer, for a fragment.
    pub fn new() -> Writer {
        Writer::default()
    }
    /// A writer that starts with [`DECL_STANDALONE`].
    pub fn document() -> Writer {
        Writer { out: String::from(DECL_STANDALONE), ..Writer::default() }
    }
    /// A writer that puts each element on its own line, `indent` deeper per open element.
    /// Elements written with [`Writer::leaf`] or [`Tag::text`] stay on one line. Text written
    /// with [`Writer::text`] is not indented.
    pub fn pretty(indent: &str) -> Writer {
        Writer { indent: Some(indent.to_string()), ..Writer::default() }
    }

    fn pad(&mut self) {
        if let Some(indent) = &self.indent {
            (0..self.open.len()).for_each(|_| self.out.push_str(indent));
        }
    }
    fn newline(&mut self) {
        if self.indent.is_some() {
            self.out.push('\n');
        }
    }

    /// Writes the XML declaration: [`DECL_STANDALONE`] or [`DECL`].
    pub fn decl(&mut self, standalone: bool) {
        self.out.push_str(if standalone { DECL_STANDALONE } else { DECL });
    }
    /// Writes `<!DOCTYPE name>` and a newline. Nothing is written if `name` is not a name.
    pub fn doctype(&mut self, name: &str) {
        if is_name(name) {
            let _ = writeln!(self.out, "<!DOCTYPE {name}>");
        }
    }
    /// Writes a processing instruction, and a newline when no element is open. Nothing is
    /// written if `target` is not a name or `data` holds `?>`.
    pub fn pi(&mut self, target: &str, data: &str) {
        let before = self.out.len();
        push_pi(&mut self.out, target, data);
        if self.open.is_empty() && self.out.len() > before {
            self.out.push('\n');
        }
    }

    /// Starts a tag: `w.tag("a:off").attr("x", 1).attr("y", 2).empty()`.
    pub fn tag<'w>(&'w mut self, name: &'w str) -> Tag<'w> {
        self.pad();
        self.out.push('<');
        self.out.push_str(name);
        Tag { w: self, name }
    }
    /// `<name attrs>`.
    pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.tag(name).attrs(attrs).open();
    }
    /// `<name attrs/>`.
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.tag(name).attrs(attrs).empty();
    }
    /// `<name attrs>text</name>`.
    pub fn leaf(&mut self, name: &str, attrs: &[(&str, &str)], text: &str) {
        self.tag(name).attrs(attrs).text(text);
    }
    /// Closes the innermost open element. Nothing happens when none is open.
    pub fn close(&mut self) {
        let Some(start) = self.open.pop() else { return };
        self.pad();
        self.out.push_str("</");
        self.out.push_str(self.names.get(start..).unwrap_or_default());
        self.out.push('>');
        self.names.truncate(start);
        self.newline();
    }
    /// Escaped text.
    pub fn text(&mut self, text: &str) {
        push_esc(&mut self.out, text);
    }
    /// A CDATA section.
    pub fn cdata(&mut self, text: &str) {
        push_cdata(&mut self.out, text);
    }
    /// Markup the caller has already written, as it is.
    pub fn raw(&mut self, xml: &str) {
        self.out.push_str(xml);
    }
    /// A tree: on one line, or indented to the current depth with [`Writer::pretty`].
    pub fn tree(&mut self, el: &El) {
        match &self.indent {
            Some(indent) => push_pretty(&mut self.out, el, indent, self.open.len()),
            None => push_el(&mut self.out, el),
        }
    }
    /// What has been written so far.
    pub fn as_str(&self) -> &str {
        &self.out
    }
    /// Closes every element still open and returns the text.
    pub fn finish(mut self) -> String {
        while !self.open.is_empty() {
            self.close();
        }
        self.out
    }
    /// [`Writer::finish`], as bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.finish().into_bytes()
    }
}
