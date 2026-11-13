//! The owned element tree.

use std::borrow::Cow;

use crate::value;

/// An element: its name, its attributes in document order and its children in document order.
///
/// What `name` and the attribute names hold depends on [`crate::Names`]: the qualified name as
/// written (`p:sp`), the local name (`sp`), or a canonical prefix and the local name (`w:p`).
///
/// A tree the reader built is never deeper than [`crate::Limits::max_depth`]. A tree built by
/// hand is as deep as its builder made it; the methods here do not recurse, but dropping a tree
/// does, once per level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct El {
    pub name: String,
    /// Attributes in document order. A name the document repeats is kept each time.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A child of an element.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    El(El),
    /// Character data, references resolved. Adjacent stretches are one node.
    Text(String),
    /// A CDATA section. The reader makes these only with [`crate::Options::cdata`]; otherwise a
    /// section's content is part of the surrounding [`Node::Text`].
    CData(String),
    /// A processing instruction (`<?target data?>`), kept only with [`crate::Options::pis`].
    Pi {
        target: String,
        data: String,
    },
    /// A comment, kept only with [`crate::Options::comments`].
    Comment(String),
}

impl From<El> for Node {
    fn from(e: El) -> Node {
        Node::El(e)
    }
}

impl From<&str> for Node {
    fn from(t: &str) -> Node {
        Node::Text(t.to_string())
    }
}

impl From<String> for Node {
    fn from(t: String) -> Node {
        Node::Text(t)
    }
}

/// What a name is matched against: a `&str` is the whole name, [`Local`] the part after the
/// prefix.
pub trait Pat: Copy {
    fn matches(self, name: &str) -> bool;
}

impl Pat for &str {
    fn matches(self, name: &str) -> bool {
        self == name
    }
}

impl Pat for &String {
    fn matches(self, name: &str) -> bool {
        self == name
    }
}

/// Matches a name by its local part, whatever its prefix: `Local("sp")` matches `sp` and `p:sp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local<'a>(pub &'a str);

impl Pat for Local<'_> {
    fn matches(self, name: &str) -> bool {
        local_of(name) == self.0
    }
}

pub(crate) fn local_of(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

fn is_ns_decl(name: &str) -> bool {
    name == "xmlns" || name.starts_with("xmlns:")
}

impl El {
    /// An element with no attributes and no children.
    pub fn new(name: impl Into<String>) -> El {
        El { name: name.into(), attrs: Vec::new(), children: Vec::new() }
    }

    // ---------------------------------------------------------------- building and changing

    /// [`El::set_attr`], for chaining.
    #[must_use]
    pub fn with_attr(mut self, name: &str, value: impl ToString) -> El {
        self.set_attr(name, value);
        self
    }
    /// Appends a child element.
    #[must_use]
    pub fn with_child(mut self, child: El) -> El {
        self.children.push(Node::El(child));
        self
    }
    /// Appends text.
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> El {
        self.children.push(Node::Text(text.into()));
        self
    }
    /// Appends any node (a CDATA section, a processing instruction…).
    #[must_use]
    pub fn with_node(mut self, node: impl Into<Node>) -> El {
        self.children.push(node.into());
        self
    }
    /// Sets the attribute named exactly `name`, replacing its first occurrence or appending it.
    pub fn set_attr(&mut self, name: &str, value: impl ToString) {
        let value = value.to_string();
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((name.to_string(), value)),
        }
    }
    /// Removes every attribute named exactly `name`; the value of the first, if there was one.
    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let first = self.attrs.iter().position(|(k, _)| k == name)?;
        let (_, value) = self.attrs.remove(first);
        self.attrs.retain(|(k, _)| k != name);
        Some(value)
    }
    /// Appends a child node.
    pub fn push(&mut self, node: impl Into<Node>) {
        self.children.push(node.into());
    }
    /// Child elements, for changing them.
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut El> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::El(e) => Some(e),
            _ => None,
        })
    }
    /// The first child element that matches, for changing it.
    pub fn child_mut(&mut self, pat: impl Pat) -> Option<&mut El> {
        self.elements_mut().find(|e| pat.matches(&e.name))
    }

    // ---------------------------------------------------------------- names

    /// The name without its prefix.
    pub fn local(&self) -> &str {
        local_of(&self.name)
    }
    /// The prefix, or `""`.
    pub fn prefix(&self) -> &str {
        self.name.split_once(':').map_or("", |(p, _)| p)
    }
    /// Does the name match?
    pub fn is(&self, pat: impl Pat) -> bool {
        pat.matches(&self.name)
    }
    /// The namespace declarations on this element as (prefix, URI); the default namespace has
    /// the prefix `""`. They are attributes, so the reader must have kept them
    /// ([`crate::Options::ns_decls`]).
    pub fn ns_decls(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attrs.iter().filter(|(k, _)| is_ns_decl(k)).map(|(k, v)| (k.strip_prefix("xmlns:").unwrap_or(""), v.as_str()))
    }

    // ---------------------------------------------------------------- attributes

    /// The first attribute named exactly `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// [`El::attr`]; failing that, for a prefixed `name`, the unprefixed attribute with the same
    /// local name. Some producers leave the prefix out (`val` for `w:val`).
    pub fn attr_or_plain(&self, name: &str) -> Option<&str> {
        self.attr(name).or_else(|| {
            let local = local_of(name);
            if local == name { None } else { self.attr(local) }
        })
    }
    /// An attribute by local name, for names kept as written. An unprefixed `name` is the
    /// attribute of exactly that name, else the first one with that local name under any
    /// prefix. A prefixed `name` (`r:id`) is the first *prefixed* attribute with that local
    /// name, whatever its prefix. Namespace declarations never match.
    pub fn attr_local(&self, name: &str) -> Option<&str> {
        let by_local = |local: &str, prefixed_only: bool| {
            self.attrs.iter().find(|(k, _)| !is_ns_decl(k) && local_of(k) == local && (!prefixed_only || k.contains(':'))).map(|(_, v)| v.as_str())
        };
        match name.split_once(':') {
            Some((_, local)) => by_local(local, true),
            None => self.attr(name).or_else(|| by_local(name, false)),
        }
    }
    pub fn has_attr(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }
    /// [`El::attr`] as a number: see [`value::to_u32`].
    pub fn attr_u32(&self, name: &str) -> Option<u32> {
        self.attr(name).and_then(value::to_u32)
    }
    /// See [`value::to_i32`].
    pub fn attr_i32(&self, name: &str) -> Option<i32> {
        self.attr(name).and_then(value::to_i32)
    }
    /// See [`value::to_i64`].
    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name).and_then(value::to_i64)
    }
    /// See [`value::to_f64`]: finite values only.
    pub fn attr_f64(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(value::to_f64)
    }
    /// See [`value::to_bool`].
    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        self.attr(name).and_then(value::to_bool)
    }
    /// [`El::attr_bool`], or `default` when the attribute is missing or not a boolean.
    pub fn flag(&self, name: &str, default: bool) -> bool {
        self.attr_bool(name).unwrap_or(default)
    }

    // ---------------------------------------------------------------- children

    /// Child elements in document order.
    pub fn elements(&self) -> impl Iterator<Item = &El> {
        self.children.iter().filter_map(|n| match n {
            Node::El(e) => Some(e),
            _ => None,
        })
    }
    /// The first child element that matches.
    pub fn child(&self, pat: impl Pat) -> Option<&El> {
        self.elements().find(|e| pat.matches(&e.name))
    }
    /// Every child element that matches.
    pub fn children<'a, P: Pat + 'a>(&'a self, pat: P) -> impl Iterator<Item = &'a El> + 'a {
        self.elements().filter(move |e| pat.matches(&e.name))
    }
    /// Follows a chain of child names, taking the first match at each step.
    pub fn path<P: Pat>(&self, names: &[P]) -> Option<&El> {
        names.iter().try_fold(self, |e, n| e.child(*n))
    }
    /// An attribute of the first matching child (`<x val="…"/>`).
    pub fn child_attr(&self, pat: impl Pat, attr: &str) -> Option<&str> {
        self.child(pat).and_then(|c| c.attr(attr))
    }
    /// [`El::text`] of the first matching child.
    pub fn child_text(&self, pat: impl Pat) -> Option<Cow<'_, str>> {
        self.child(pat).map(El::text)
    }
    /// How many nodes this element is, itself included: what [`crate::Limits::max_nodes`]
    /// counted when it was read. A reader of many parts charges this against a budget for the
    /// whole package and reads the next part with what is left as its `max_nodes`.
    pub fn node_count(&self) -> usize {
        let mut count = 1_usize;
        let mut open = vec![self];
        while let Some(el) = open.pop() {
            count = count.saturating_add(el.children.len());
            open.extend(el.elements());
        }
        count
    }
    /// Every descendant element, depth first in document order, without this element.
    pub fn descendants(&self) -> Descendants<'_> {
        Descendants { stack: vec![self.children.iter()] }
    }
    /// The first descendant that matches, depth first.
    pub fn find(&self, pat: impl Pat) -> Option<&El> {
        self.descendants().find(|e| pat.matches(&e.name))
    }
    /// Every descendant that matches, depth first. A match inside a match is found too.
    pub fn find_all<'a, P: Pat + 'a>(&'a self, pat: P) -> impl Iterator<Item = &'a El> + 'a {
        self.descendants().filter(move |e| pat.matches(&e.name))
    }

    // ---------------------------------------------------------------- text

    /// This element's own character data: its text and CDATA children joined, not its
    /// descendants'. Borrowed when there is at most one such child.
    pub fn text(&self) -> Cow<'_, str> {
        let mut parts = self.children.iter().filter_map(|n| match n {
            Node::Text(t) | Node::CData(t) => Some(t.as_str()),
            _ => None,
        });
        let first = parts.next().unwrap_or("");
        match parts.next() {
            None => Cow::Borrowed(first),
            Some(second) => {
                let mut s = String::from(first);
                s.push_str(second);
                parts.for_each(|t| s.push_str(t));
                Cow::Owned(s)
            }
        }
    }
    /// The character data of this element and every descendant, in document order.
    pub fn deep_text(&self) -> String {
        let mut s = String::new();
        walk(self, |step| {
            if let Step::Leaf(Node::Text(t) | Node::CData(t)) = step {
                s.push_str(t);
            }
            true
        });
        s
    }
}

/// See [`El::descendants`].
#[derive(Clone, Debug)]
pub struct Descendants<'a> {
    stack: Vec<std::slice::Iter<'a, Node>>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = &'a El;

    fn next(&mut self) -> Option<&'a El> {
        loop {
            match self.stack.last_mut()?.next() {
                Some(Node::El(e)) => {
                    self.stack.push(e.children.iter());
                    return Some(e);
                }
                Some(_) => {}
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

pub(crate) enum Step<'a> {
    Enter(&'a El),
    Leave(&'a El),
    Leaf(&'a Node),
}

/// Walks `root` in document order without recursion. `f` returns whether to go into the element
/// it was just given; an element it declines gets no [`Step::Leave`].
pub(crate) fn walk<'a>(root: &'a El, mut f: impl FnMut(Step<'a>) -> bool) {
    if !f(Step::Enter(root)) {
        return;
    }
    let mut stack = vec![(root, root.children.iter())];
    while let Some((el, rest)) = stack.last_mut() {
        match rest.next() {
            Some(Node::El(child)) => {
                if f(Step::Enter(child)) {
                    stack.push((child, child.children.iter()));
                }
            }
            Some(leaf) => {
                f(Step::Leaf(leaf));
            }
            None => {
                f(Step::Leave(el));
                stack.pop();
            }
        }
    }
}
