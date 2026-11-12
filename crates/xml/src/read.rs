//! Reading: the pull reader over `quick-xml`, the tree builder on top of it, and streaming.

use std::borrow::Cow;
use std::ops::Range;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event as Q};

use crate::Error;
use crate::tree::{El, Node, Pat, local_of};
use crate::write::is_xml_char;

/// The namespace the `xml` prefix is always bound to.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// What the reader refuses to exceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most bytes of input. Default 256 MiB.
    pub max_input: usize,
    /// Deepest element nesting: the root is at depth 1. Default 256.
    pub max_depth: usize,
    /// Most nodes in a tree: elements, text stretches and whatever else is kept. Elements
    /// handed to a streaming callback leave the count with everything inside them. Default
    /// 2,000,000: a tree node costs about 72 bytes before its strings, and text lies between
    /// elements at most, so this is about a million elements of an indented document.
    pub max_nodes: usize,
    /// Most attributes on one start tag, namespace declarations and malformed ones included.
    /// With [`Names::Canonical`] it is also the most namespace declarations in scope at once.
    /// Default 1024.
    pub max_attrs: usize,
    /// Most attributes kept in a tree at once, over all its elements. An attribute costs 48
    /// bytes before its strings and five bytes of input (` a=""`), so without this a document
    /// inside every other limit could take ten times its size in attribute headers. Elements
    /// handed to a streaming callback take their attributes out of the count. Default
    /// 4,000,000: two for every node at the node limit.
    pub max_attrs_total: usize,
}

impl Limits {
    pub const DEFAULT: Limits =
        Limits { max_input: 256 * 1024 * 1024, max_depth: 256, max_nodes: 2_000_000, max_attrs: 1024, max_attrs_total: 4_000_000 };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// How element and attribute names are read.
#[derive(Clone, Copy, Default)]
pub enum Names<'a> {
    /// The qualified name as written: `p:sp`, `r:id`. Match with a `&str` when the prefixes
    /// are known, or with [`crate::Local`] and [`El::attr_local`] when they are not.
    #[default]
    AsWritten,
    /// The local name only: `sp`, `id`.
    Local,
    /// The caller's prefix for the name's namespace and the local name: `w:p`, whatever
    /// prefix the document used. See [`Canonical`].
    Canonical(&'a Canonical<'a>),
}

/// An element name being given its canonical prefix.
#[derive(Clone, Copy, Debug)]
pub struct NsName<'s> {
    /// The prefix [`Canonical::namespaces`] has for `uri`, or [`Canonical::unknown`].
    pub prefix: &'s str,
    pub uri: &'s str,
    pub local: &'s str,
    /// The canonical name of the enclosing element; `None` at the root.
    pub parent: Option<&'s str>,
}

/// See [`Canonical::element`].
pub type ElementHook<'a> = dyn Fn(&NsName<'_>) -> Option<&'static str> + 'a;
/// See [`Canonical::attr_value`].
pub type AttrValueHook<'a> = dyn Fn(&str, &str, &str, &Scope<'_>) -> Option<String> + 'a;

/// Canonical prefixes by namespace URI.
///
/// - An element in a listed namespace is named `prefix:local`; in an unlisted one,
///   `unknown:local`; in no namespace (or with a prefix nothing declares), `local`. A namespace
///   listed with the empty prefix reads unprefixed too: a vocabulary's own namespace.
/// - A prefixed attribute is named the same way. An unprefixed attribute is in no namespace
///   and keeps its name.
/// - The `xml` prefix needs no entry: `xml:space` stays `xml:space`.
#[derive(Clone, Copy)]
pub struct Canonical<'a> {
    /// (prefix, namespace URI). The first entry with a URI wins, so several URIs may share a
    /// prefix (the Strict and Transitional forms of a vocabulary).
    pub namespaces: &'a [(&'a str, &'a str)],
    /// The prefix for a namespace that is not listed. `"?"` by default: it cannot be part of an
    /// XML name, so such names never collide with real ones and [`El::write_xml`] leaves them
    /// out.
    pub unknown: &'a str,
    /// Overrides the prefix of an element, for vocabularies where one namespace is read as
    /// several depending on the element and its parent. `None` keeps [`NsName::prefix`]. The
    /// prefixes it returns are constants of the caller's.
    pub element: Option<&'a ElementHook<'a>>,
    /// Rewrites an attribute value while the namespace declarations in scope are still known:
    /// for values that hold prefixes (`mc:Choice/@Requires`, `xsi:type`). Called with the
    /// element's name, the attribute's name, the value and the scope; `None` keeps the value.
    pub attr_value: Option<&'a AttrValueHook<'a>>,
}

impl<'a> Canonical<'a> {
    pub const fn new(namespaces: &'a [(&'a str, &'a str)]) -> Canonical<'a> {
        Canonical { namespaces, unknown: "?", element: None, attr_value: None }
    }
    /// The prefix for `uri`: the listed one, `xml` for [`XML_NS`], else [`Canonical::unknown`].
    pub fn prefix(&self, uri: &str) -> &'a str {
        match self.namespaces.iter().find(|(_, u)| *u == uri) {
            Some((p, _)) => p,
            None if uri == XML_NS => "xml",
            None => self.unknown,
        }
    }
}

/// The namespace declarations in scope at an element.
pub struct Scope<'s> {
    bindings: &'s [(String, String)],
}

impl<'s> Scope<'s> {
    /// The namespace URI `prefix` is bound to; `""` is the default namespace.
    pub fn uri(&self, prefix: &str) -> Option<&'s str> {
        if prefix == "xml" {
            return Some(XML_NS);
        }
        self.bindings.iter().rev().find(|(p, _)| p == prefix).map(|(_, u)| u.as_str()).filter(|u| !u.is_empty())
    }
}

/// What to do with a reference to an entity that is neither predefined nor numeric. Entities a
/// document declares are never expanded, so `&name;` for a declared entity is one of these too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnknownEntity {
    /// Leave it out.
    #[default]
    Drop,
    /// Keep it as the text `&name;`.
    Literal,
    /// Refuse the document ([`Error::UnknownEntity`]).
    Error,
}

/// What to do with a document type declaration. Whatever is chosen, the declaration is never
/// processed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Doctype {
    /// Skip it.
    #[default]
    Ignore,
    /// Refuse a declaration with an internal subset (`<!DOCTYPE x [ … ]>`), skip any other.
    RejectInternalSubset,
    /// Refuse any declaration.
    Reject,
}

/// How to read a document. `Options::default()` keeps names as written and is lenient.
#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub limits: Limits,
    pub names: Names<'a>,
    /// Keep namespace declarations (`xmlns`, `xmlns:p`) as attributes, as written. On by
    /// default; [`Options::local`] and [`Options::canonical`] turn it off.
    pub ns_decls: bool,
    /// Refuse what a lenient read repairs. Lenient: an end tag closes the innermost open
    /// element whatever its name, an end tag with nothing open is skipped, elements still open
    /// at the end of the input are closed, reading stops when the root element ends, a
    /// malformed attribute is skipped, and bytes that are not UTF-8 become U+FFFD. Strict: each
    /// of those is an error, as are a second root element and text outside the root.
    pub strict: bool,
    pub unknown_entity: UnknownEntity,
    pub doctype: Doctype,
    /// Keep CDATA sections as [`Node::CData`] instead of joining them to the text around them.
    pub cdata: bool,
    /// Keep processing instructions inside the root element as [`Node::Pi`].
    pub pis: bool,
    /// Keep comments inside the root element as [`Node::Comment`].
    pub comments: bool,
    /// Drop the text of an element that has child elements when all of it is whitespace: the
    /// indentation of an indented document.
    pub drop_blank_text: bool,
    /// For [`Options::parse_streaming`]: how far below the root an element may be and still be
    /// handed out. 2 by default: children and grandchildren of the root.
    pub split_depth: usize,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options {
            limits: Limits::DEFAULT,
            names: Names::AsWritten,
            ns_decls: true,
            strict: false,
            unknown_entity: UnknownEntity::Drop,
            doctype: Doctype::Ignore,
            cdata: false,
            pis: false,
            comments: false,
            drop_blank_text: false,
            split_depth: 2,
        }
    }
}

/// Reads `bytes` with [`Options::default`].
pub fn parse(bytes: &[u8]) -> Result<El, Error> {
    Options::default().parse(bytes)
}

impl<'a> Options<'a> {
    /// Names by local part, namespace declarations dropped.
    pub fn local() -> Options<'static> {
        Options { names: Names::Local, ns_decls: false, ..Options::default() }
    }
    /// Names with canonical prefixes, namespace declarations dropped.
    pub fn canonical(canonical: &'a Canonical<'a>) -> Options<'a> {
        Options { names: Names::Canonical(canonical), ns_decls: false, ..Options::default() }
    }
    /// [`Options::strict`] on, and an undefined entity an error.
    #[must_use]
    pub fn strict(self) -> Self {
        Options { strict: true, unknown_entity: UnknownEntity::Error, ..self }
    }
    #[must_use]
    pub fn with_limits(self, limits: Limits) -> Self {
        Options { limits, ..self }
    }

    /// Reads a document into its root element.
    pub fn parse(&self, bytes: &[u8]) -> Result<El, Error> {
        self.parse_streaming(bytes, &[], |_| Ok::<(), Error>(()))
    }

    /// Reads a document, handing each complete element whose name is in `split` to `each`
    /// instead of keeping it in the tree: worksheet rows, shared strings. Only elements at
    /// most [`Options::split_depth`] levels below the root are handed out; a deeper element of
    /// the same name stays in the tree. `split` names are matched against names as this reader
    /// produces them. Elements handed out do not count toward [`Limits::max_nodes`] once they
    /// are gone, so a part of any length can be read; each one is itself held to the limit.
    ///
    /// An error from `each` stops the read and is returned.
    pub fn parse_streaming<E: From<Error>>(&self, bytes: &[u8], split: &[&str], each: impl FnMut(El) -> Result<(), E>) -> Result<El, E> {
        match self.parse_streaming_partial(bytes, split, each) {
            (_, Some(e)) => Err(e),
            (root, None) => root.ok_or_else(|| Error::NoRoot.into()),
        }
    }

    /// [`Options::parse_streaming`] that also returns what it had read when it failed: the
    /// root with the children that were complete (and not handed out) before the error, and
    /// the elements still open at that point with what they held. A worksheet whose rows
    /// cannot all be read still gives its properties and merged cells. `(None, Some(_))` when
    /// nothing was read; `(Some(_), None)` is a whole document.
    ///
    /// The partial tree is not a document: it stops where the error was, and an open element
    /// named in `split` is attached to it instead of being handed to `each`.
    pub fn parse_streaming_partial<E: From<Error>>(
        &self,
        bytes: &[u8],
        split: &[&str],
        mut each: impl FnMut(El) -> Result<(), E>,
    ) -> (Option<El>, Option<E>) {
        if bytes.len() > self.limits.max_input {
            return (None, Some(Error::TooLarge(self.limits.max_input).into()));
        }
        let text = to_utf8(bytes);
        let mut events = Events::new(*self, &text, false);
        let mut tree = Builder { opts: self, split, stack: Vec::new(), marks: Vec::new(), kept: 0, attrs: 0, root: None, at: 0 };
        let mut err = tree.run(&mut events, &mut each).err();
        // What was open when the read stopped goes into its parent as it is.
        while let Some(el) = tree.stack.pop() {
            match tree.stack.last_mut() {
                Some(parent) => parent.children.push(Node::El(el)),
                None => tree.root = Some(el),
            }
        }
        if tree.root.is_none() && err.is_none() {
            err = Some(Error::NoRoot.into());
        }
        (tree.root, err)
    }

    /// The first element that matches, from its start tag alone: its name and attributes, no
    /// children. Nothing after the tag is read, so unbalanced markup around it neither hides
    /// nor moves it. `None` when there is none or the markup before it cannot be read.
    pub fn first_start(&self, bytes: &[u8], pat: impl Pat) -> Option<El> {
        if bytes.len() > self.limits.max_input {
            return None;
        }
        let text = to_utf8(bytes);
        // No tree is built, and markup that never closes its tags would otherwise look deep.
        let opts = Options { limits: Limits { max_depth: usize::MAX, ..self.limits }, ..*self };
        let mut events = Events::new(opts, &text, false);
        loop {
            match events.next().ok()?? {
                Event::Start { name, attrs } if pat.matches(&name) => return Some(El { name, attrs, children: Vec::new() }),
                _ => {}
            }
        }
    }

    /// A pull reader over `utf8`, which is UTF-8 text: pass other input through [`to_utf8`]
    /// first. No tree is built, so [`Limits::max_nodes`] and [`Limits::max_input`] do not
    /// apply; depth and attribute limits do.
    pub fn events<'i>(&self, utf8: &'i [u8]) -> Events<'a, 'i> {
        Events::new(*self, utf8, true)
    }
}

/// One step of a document. See [`Options::events`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A start tag. An empty element (`<a/>`) is a `Start` followed by its `End`.
    Start {
        name: String,
        attrs: Vec<(String, String)>,
    },
    /// The end of the element named `name` (the name of its start tag).
    End {
        name: String,
    },
    /// Character data, references resolved. One stretch of text may arrive in several events.
    Text(String),
    /// The content of a CDATA section.
    CData(String),
    Pi {
        target: String,
        data: String,
    },
    Comment(String),
}

struct Open {
    /// Where this element's namespace declarations start in the scope.
    scope: usize,
    /// The name as written, kept for a strict read to check the end tag against.
    raw: Vec<u8>,
    /// The name as produced, kept when an end event or a canonical parent needs it.
    name: String,
}

/// A pull reader. See [`Options::events`].
pub struct Events<'o, 'i> {
    reader: Reader<&'i [u8]>,
    opts: Options<'o>,
    /// Whether [`Event::End`] carries the name.
    end_names: bool,
    open: Vec<Open>,
    scope: Vec<(String, String)>,
    /// The end of an empty element, due after its start.
    pending: bool,
    /// Whether a root element has started.
    rooted: bool,
    span: Range<usize>,
}

fn at(pos: u64) -> usize {
    usize::try_from(pos).unwrap_or(usize::MAX)
}

fn line_ends(s: Cow<'_, str>) -> String {
    // XML 1.0 §2.11: a literal CR LF or CR is read as LF.
    if s.contains('\r') { s.replace("\r\n", "\n").replace('\r', "\n") } else { s.into_owned() }
}

fn is_ns_decl(key: &[u8]) -> bool {
    key == b"xmlns" || key.starts_with(b"xmlns:")
}

impl<'o, 'i> Events<'o, 'i> {
    fn new(opts: Options<'o>, utf8: &'i [u8], end_names: bool) -> Self {
        let mut reader = Reader::from_reader(utf8);
        let cfg = reader.config_mut();
        // The checks a strict read wants are made here, where the answer can be lenient.
        cfg.check_end_names = false;
        cfg.allow_unmatched_ends = true;
        cfg.allow_dangling_amp = true;
        cfg.trim_text_start = false;
        cfg.trim_text_end = false;
        cfg.expand_empty_elements = false;
        Events { reader, opts, end_names, open: Vec::new(), scope: Vec::new(), pending: false, rooted: false, span: 0..0 }
    }

    /// The bytes of the input the last event was read from. Both events of an empty element
    /// have the whole element.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    fn syntax<T>(&self, msg: impl Into<String>) -> Result<T, Error> {
        Err(Error::Syntax { at: self.span.start, msg: msg.into() })
    }

    fn decode<'b>(&self, bytes: &'b [u8]) -> Result<Cow<'b, str>, Error> {
        match std::str::from_utf8(bytes) {
            Ok(s) => Ok(Cow::Borrowed(s)),
            Err(e) if self.opts.strict => Err(Error::Encoding(self.span.start.saturating_add(e.valid_up_to()))),
            Err(_) => Ok(String::from_utf8_lossy(bytes)),
        }
    }

    fn unknown(&self, name: &str) -> Result<Option<String>, Error> {
        match self.opts.unknown_entity {
            UnknownEntity::Drop => Ok(None),
            UnknownEntity::Literal => Ok(Some(format!("&{name};"))),
            UnknownEntity::Error => Err(Error::UnknownEntity { at: self.span.start, name: name.to_string() }),
        }
    }

    /// The next event, or `None` at the end of the input.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Event>, Error> {
        if self.pending {
            self.pending = false;
            return Ok(Some(self.end()));
        }
        loop {
            let from = at(self.reader.buffer_position());
            let ev = match self.reader.read_event() {
                Ok(ev) => ev,
                Err(e) => return Err(Error::Syntax { at: at(self.reader.error_position()), msg: e.to_string() }),
            };
            self.span = from..at(self.reader.buffer_position());
            if let Some(out) = self.step(ev)? {
                return Ok(out);
            }
        }
    }

    /// `None` to read on, `Some(None)` at the end of the input.
    fn step(&mut self, ev: Q<'i>) -> Result<Option<Option<Event>>, Error> {
        let event = match ev {
            Q::Start(e) => self.start(&e)?,
            Q::Empty(e) => {
                self.pending = true;
                self.start(&e)?
            }
            Q::End(e) => {
                let Some(top) = self.open.last() else {
                    return if self.opts.strict { self.syntax("an end tag has no start tag") } else { Ok(None) };
                };
                if self.opts.strict && top.raw != e.name().as_ref() {
                    return self.syntax(format!("the end tag `{}` does not match its start tag", String::from_utf8_lossy(e.name().as_ref())));
                }
                self.end()
            }
            Q::Text(t) => {
                if t.is_empty() {
                    return Ok(None);
                }
                // Outside the root element only white space may stand (XML 1.0 [1], [27]).
                if self.outside_root() && t.iter().any(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
                    return self.syntax("text outside the root element");
                }
                Event::Text(line_ends(self.decode(&t)?))
            }
            Q::CData(_) | Q::GeneralRef(_) if self.outside_root() => return self.syntax("text outside the root element"),
            Q::CData(t) => Event::CData(line_ends(self.decode(&t)?)),
            Q::GeneralRef(r) => {
                let name = self.decode(&r)?;
                match entity(&name) {
                    Some(c) => Event::Text(c.to_string()),
                    None => match self.unknown(&name)? {
                        Some(literal) => Event::Text(literal),
                        None => return Ok(None),
                    },
                }
            }
            Q::PI(p) => Event::Pi { target: self.decode(p.target())?.into_owned(), data: self.decode(p.content())?.trim_start().to_string() },
            Q::Comment(c) => Event::Comment(self.decode(&c)?.into_owned()),
            Q::DocType(d) => {
                let refuse = match self.opts.doctype {
                    Doctype::Ignore => false,
                    // A `[` in a quoted identifier is refused too, which fails safe.
                    Doctype::RejectInternalSubset => d.contains(&b'['),
                    Doctype::Reject => true,
                };
                return if refuse { Err(Error::Doctype) } else { Ok(None) };
            }
            Q::Decl(_) => return Ok(None),
            Q::Eof => {
                return if self.opts.strict && !self.open.is_empty() { self.syntax("an element is not closed") } else { Ok(Some(None)) };
            }
        };
        Ok(Some(Some(event)))
    }

    fn end(&mut self) -> Event {
        let (scope, name) = self.open.pop().map_or((0, String::new()), |o| (o.scope, o.name));
        self.scope.truncate(scope);
        Event::End { name }
    }

    /// The URI `prefix` is bound to here; `""` is the default namespace.
    fn uri(&self, prefix: &str) -> Option<&str> {
        Scope { bindings: &self.scope }.uri(prefix)
    }

    fn element_name(&self, raw: &str) -> String {
        match self.opts.names {
            Names::AsWritten => raw.to_string(),
            Names::Local => local_of(raw).to_string(),
            Names::Canonical(canon) => {
                let (prefix, local) = raw.split_once(':').unwrap_or(("", raw));
                let Some(uri) = self.uri(prefix) else { return local.to_string() };
                let listed = canon.prefix(uri);
                let parent = self.open.last().map(|o| o.name.as_str());
                let chosen = canon.element.and_then(|hook| hook(&NsName { prefix: listed, uri, local, parent })).unwrap_or(listed);
                if chosen.is_empty() { local.to_string() } else { format!("{chosen}:{local}") }
            }
        }
    }

    fn attr_name(&self, raw: &str) -> String {
        match self.opts.names {
            Names::AsWritten => raw.to_string(),
            Names::Local => local_of(raw).to_string(),
            Names::Canonical(canon) => match raw.split_once(':') {
                Some((prefix, local)) => match self.uri(prefix).map(|uri| canon.prefix(uri)) {
                    Some(p) if !p.is_empty() => format!("{p}:{local}"),
                    _ => local.to_string(),
                },
                None => raw.to_string(),
            },
        }
    }

    fn attr_value(&self, raw: &[u8]) -> Result<String, Error> {
        match decode_refs(&self.decode(raw)?, true, self.opts.unknown_entity) {
            Ok(v) => Ok(v),
            Err(name) => Err(Error::UnknownEntity { at: self.span.start, name }),
        }
    }

    /// Reads the tag's namespace declarations into the scope, so that the tag's own name and
    /// attributes resolve against them.
    fn declare(&mut self, e: &BytesStart<'_>) -> Result<(), Error> {
        let mut attrs = e.attributes();
        attrs.with_checks(false);
        // The attribute pass that follows counts every attribute; this one only has to stop.
        for a in attrs.take(self.opts.limits.max_attrs).flatten() {
            let key = a.key.as_ref();
            if is_ns_decl(key) {
                let prefix = self.decode(key.get(6..).unwrap_or_default())?.into_owned();
                let uri = self.attr_value(&a.value)?;
                self.scope.push((prefix, uri));
            }
        }
        // Every name is resolved against the scope, so the scope as a whole is bounded too:
        // declarations piled up over many levels would make each lookup long.
        if self.scope.len() > self.opts.limits.max_attrs {
            return Err(Error::TooManyAttrs(self.opts.limits.max_attrs));
        }
        Ok(())
    }

    /// In a strict read: is the reader before or after the root element?
    fn outside_root(&self) -> bool {
        self.opts.strict && self.open.is_empty()
    }

    fn start(&mut self, e: &BytesStart<'_>) -> Result<Event, Error> {
        let limits = self.opts.limits;
        if self.outside_root() && self.rooted {
            return self.syntax("more than one root element");
        }
        self.rooted = true;
        if self.open.len() >= limits.max_depth {
            return Err(Error::TooDeep(limits.max_depth));
        }
        let canonical = match self.opts.names {
            Names::Canonical(c) => Some(c),
            _ => None,
        };
        let scope = self.scope.len();
        if canonical.is_some() {
            self.declare(e)?;
        }
        let raw = e.name();
        let name = self.element_name(&self.decode(raw.as_ref())?);
        let mut out = Vec::new();
        let mut attrs = e.attributes();
        attrs.with_checks(false);
        for (seen, a) in attrs.enumerate() {
            if seen >= limits.max_attrs {
                return Err(Error::TooManyAttrs(limits.max_attrs));
            }
            let a = match a {
                Ok(a) => a,
                Err(err) if self.opts.strict => return self.syntax(err.to_string()),
                Err(_) => continue,
            };
            let key = a.key.as_ref();
            let decl = is_ns_decl(key);
            if decl && !self.opts.ns_decls {
                continue;
            }
            let key = self.decode(key)?;
            let key = if decl { key.into_owned() } else { self.attr_name(&key) };
            let mut value = self.attr_value(&a.value)?;
            if let Some(hook) = canonical.and_then(|c| c.attr_value)
                && let Some(rewritten) = hook(&name, &key, &value, &Scope { bindings: &self.scope })
            {
                value = rewritten;
            }
            out.push((key, value));
        }
        let keep_name = self.end_names || canonical.is_some_and(|c| c.element.is_some());
        self.open.push(Open {
            scope,
            raw: if self.opts.strict { raw.as_ref().to_vec() } else { Vec::new() },
            name: if keep_name { name.clone() } else { String::new() },
        });
        Ok(Event::Start { name, attrs: out })
    }
}

struct Builder<'b, 'o> {
    opts: &'b Options<'o>,
    split: &'b [&'b str],
    stack: Vec<El>,
    /// The node and attribute counts when each open element started.
    marks: Vec<(usize, usize)>,
    kept: usize,
    /// Attributes in the elements built so far and not handed out.
    attrs: usize,
    root: Option<El>,
    /// Where the event being handled starts, for errors.
    at: usize,
}

impl Builder<'_, '_> {
    fn run<E: From<Error>>(&mut self, events: &mut Events<'_, '_>, each: &mut impl FnMut(El) -> Result<(), E>) -> Result<(), E> {
        while let Some(ev) = events.next()? {
            self.at = events.span().start;
            self.event(ev, each)?;
            if !self.opts.strict && self.root.is_some() {
                break;
            }
        }
        // A lenient read keeps what a truncated document had; a strict one failed above.
        while let Some(el) = self.stack.pop() {
            self.finish(el, each)?;
        }
        Ok(())
    }

    fn count(&mut self) -> Result<(), Error> {
        self.kept = self.kept.saturating_add(1);
        if self.kept > self.opts.limits.max_nodes { Err(Error::TooManyNodes(self.opts.limits.max_nodes)) } else { Ok(()) }
    }

    fn push(&mut self, node: Node) -> Result<(), Error> {
        if !self.stack.is_empty() {
            self.count()?;
        }
        if let Some(top) = self.stack.last_mut() {
            top.children.push(node);
        }
        Ok(())
    }

    fn text(&mut self, text: String) -> Result<(), Error> {
        match self.stack.last_mut() {
            Some(top) => {
                if let Some(Node::Text(prev)) = top.children.last_mut() {
                    prev.push_str(&text);
                    return Ok(());
                }
                self.push(Node::Text(text))
            }
            None if self.opts.strict && !text.trim().is_empty() => Err(Error::Syntax { at: self.at, msg: "text outside the root element".into() }),
            None => Ok(()),
        }
    }

    fn event<E: From<Error>>(&mut self, ev: Event, each: &mut impl FnMut(El) -> Result<(), E>) -> Result<(), E> {
        match ev {
            Event::Start { name, attrs } => {
                if self.root.is_some() {
                    return Err(Error::Syntax { at: self.at, msg: "more than one root element".into() }.into());
                }
                self.marks.push((self.kept, self.attrs));
                self.attrs = self.attrs.saturating_add(attrs.len());
                if self.attrs > self.opts.limits.max_attrs_total {
                    return Err(Error::TooManyAttrsTotal(self.opts.limits.max_attrs_total).into());
                }
                self.stack.push(El { name, attrs, children: Vec::new() });
            }
            Event::End { .. } => {
                if let Some(el) = self.stack.pop() {
                    self.finish(el, each)?;
                }
            }
            Event::Text(t) => self.text(t)?,
            Event::CData(t) if self.opts.cdata => self.push(Node::CData(t))?,
            Event::CData(t) => self.text(t)?,
            Event::Pi { target, data } if self.opts.pis => self.push(Node::Pi { target, data })?,
            Event::Comment(c) if self.opts.comments => self.push(Node::Comment(c))?,
            Event::Pi { .. } | Event::Comment(_) => {}
        }
        Ok(())
    }

    /// Attaches a complete element to its parent, hands it out, or makes it the root.
    fn finish<E: From<Error>>(&mut self, mut el: El, each: &mut impl FnMut(El) -> Result<(), E>) -> Result<(), E> {
        let (mark, attrs_mark) = self.marks.pop().unwrap_or((0, 0));
        if self.opts.drop_blank_text && el.children.iter().any(|n| matches!(n, Node::El(_))) {
            let is_text = |n: &Node| matches!(n, Node::Text(_));
            if el.children.iter().all(|n| !is_text(n) || matches!(n, Node::Text(t) if t.trim().is_empty())) {
                let before = el.children.len();
                el.children.retain(|n| !is_text(n));
                self.kept = self.kept.saturating_sub(before - el.children.len());
            }
        }
        let depth = self.stack.len();
        if depth == 0 {
            self.count()?;
            self.root = Some(el);
        } else if depth <= self.opts.split_depth && self.split.contains(&el.name.as_str()) {
            each(el)?;
            // The element and everything inside it have left the tree.
            self.kept = mark;
            self.attrs = attrs_mark;
        } else {
            self.push(Node::El(el))?;
        }
        Ok(())
    }
}

/// The input as UTF-8 text for the reader: a UTF-8 byte-order mark is removed, and UTF-16 (with
/// a byte-order mark, or recognisable without one by the zero bytes of its first two
/// characters) is transcoded. An unpaired surrogate becomes U+FFFD and a trailing odd byte is
/// dropped. The result is at most 1.5 times the input. Any other input is returned as it is;
/// the encoding an XML declaration names is not consulted.
pub fn to_utf8(bytes: &[u8]) -> Cow<'_, [u8]> {
    let (little_endian, body) = match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => return Cow::Borrowed(rest),
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        [a, 0, b, 0, ..] if *a != 0 && *b != 0 => (true, bytes),
        [0, a, 0, b, ..] if *a != 0 && *b != 0 => (false, bytes),
        _ => return Cow::Borrowed(bytes),
    };
    let units = body.as_chunks::<2>().0.iter().map(|pair| if little_endian { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) });
    Cow::Owned(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect::<String>().into_bytes())
}

/// The character a reference names, without its `&` and `;`: one of the five predefined
/// entities (`amp`, `lt`, `gt`, `quot`, `apos`) or a numeric character reference (`#65`,
/// `#x41`, zero-padded or not). `None` for anything else, for a number that is not a character
/// XML can carry, and for a number with a sign.
pub fn entity(name: &str) -> Option<char> {
    let number = match name {
        "amp" => return Some('&'),
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        _ => name.strip_prefix('#')?,
    };
    let (digits, radix) = match number.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (number, 10),
    };
    if digits.is_empty() {
        return None;
    }
    let code = digits.chars().try_fold(0u32, |n, c| n.checked_mul(radix)?.checked_add(c.to_digit(radix)?))?;
    char::from_u32(code).filter(|c| is_xml_char(*c))
}

/// Resolves the references in `s`, and with `attr` turns literal whitespace into spaces as
/// attribute-value normalisation does. An `&` that starts no reference stays as it is. `Err` is
/// the name of an unknown entity under [`UnknownEntity::Error`].
fn decode_refs(s: &str, attr: bool, unknown: UnknownEntity) -> Result<String, String> {
    let special = |c: char| c == '&' || (attr && matches!(c, '\t' | '\n' | '\r'));
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(special) {
        out.push_str(rest.get(..i).unwrap_or_default());
        let tail = rest.get(i..).unwrap_or_default();
        let Some(after) = tail.strip_prefix('&') else {
            out.push(' ');
            rest = tail.strip_prefix("\r\n").or_else(|| tail.get(1..)).unwrap_or_default();
            continue;
        };
        // Only the bytes a name or a number can have are scanned, so a run of `&` costs one
        // look each however far away the next `;` is.
        let len = after.bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'#' | b'_' | b'-' | b'.' | b':')).count();
        let name = after.get(..len).unwrap_or_default();
        if len == 0 || after.as_bytes().get(len) != Some(&b';') {
            out.push('&');
            rest = after;
            continue;
        }
        match (entity(name), unknown) {
            (Some(c), _) => out.push(c),
            (None, UnknownEntity::Drop) => {}
            (None, UnknownEntity::Literal) => {
                out.push('&');
                out.push_str(name);
                out.push(';');
            }
            (None, UnknownEntity::Error) => return Err(name.to_string()),
        }
        rest = after.get(len + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// `s` with its predefined and numeric references resolved ([`entity`]). Anything else that
/// starts with `&` stays as it is.
pub fn unescape(s: &str) -> String {
    decode_refs(s, false, UnknownEntity::Literal).unwrap_or_else(|_| s.to_string())
}
