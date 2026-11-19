//! A tolerant HTML tokenizer: text, start tags and end tags.
//!
//! It follows the general syntax of the HTML Living Standard closely enough for documents, mail
//! and clipboard fragments, and reads anything: a `<` that starts no tag is text, a tag or
//! comment that never ends runs to the end of the input. Comments, doctypes and processing
//! instructions never leave it.

use crate::entity::unescape_attr;

/// What a [`Lexer`] caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most attributes kept per tag; the rest are read and dropped. Default 64.
    pub max_attrs: usize,
    /// Longest tag name kept, in bytes; a longer one is no real element and its name is empty.
    /// Default 64.
    pub max_tag_name: usize,
}

impl Limits {
    pub const DEFAULT: Limits = Limits { max_attrs: 64, max_tag_name: 64 };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// How a [`Lexer`] reads. Element names are lower case.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options<'o> {
    pub limits: Limits,
    /// Elements that never leave the lexer: the start tag, the content (raw text, not markup)
    /// and the end tag are skipped. For `script`, `style`, `title` and the like, when the caller
    /// has no use for them. A caller that wants a `style`'s text leaves it out of this list and
    /// calls [`Lexer::raw_content`] on its start tag.
    pub drop: &'o [&'o str],
    /// Elements whose content is text, not markup, and is given: after the start tag comes one
    /// [`Token::Text`] with everything up to the end tag. For `textarea`, `xmp`, `plaintext`.
    pub raw_text: &'o [&'o str],
}

/// A start tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag<'a> {
    /// Lower case; empty for a name longer than [`Limits::max_tag_name`].
    pub name: String,
    /// Names and values as written, in the order written.
    pub attrs: Vec<(&'a str, &'a str)>,
    /// The tag ends in `/>`.
    pub self_close: bool,
}

impl Tag<'_> {
    /// An attribute's value with character references decoded ([`unescape_attr`]); the first
    /// of a name wins, as in browsers. Names match in any letter case.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.raw_attr(name).map(unescape_attr)
    }

    /// An attribute's value as written, character references and all.
    pub fn raw_attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
    }

    /// Whether the tag has the attribute, with or without a value.
    pub fn has(&self, name: &str) -> bool {
        self.attrs.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// Every attribute in the order written: the name as written, the value decoded.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, String)> + '_ {
        self.attrs.iter().map(|(n, v)| (*n, unescape_attr(v)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    /// Character data as written: references are not decoded ([`crate::unescape`]).
    Text(&'a str),
    Start(Tag<'a>),
    /// An end tag's name, lower case (empty when longer than [`Limits::max_tag_name`]).
    End(String),
}

/// The tokenizer: an iterator of [`Token`]s over a string.
pub struct Lexer<'a, 'o> {
    src: &'a str,
    pos: usize,
    opts: Options<'o>,
    /// Set after the start tag of a raw-text element: its content comes next.
    raw: Option<String>,
}

impl<'a> Lexer<'a, 'static> {
    /// A lexer with [`Options::default`]: nothing dropped, no raw-text elements.
    pub fn new(src: &'a str) -> Lexer<'a, 'static> {
        Lexer::with(src, Options::default())
    }
}

impl<'a, 'o> Lexer<'a, 'o> {
    pub fn with(src: &'a str, opts: Options<'o>) -> Lexer<'a, 'o> {
        Lexer { src, pos: 0, opts, raw: None }
    }

    /// How far into the input the tokens given so far go, in bytes.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Goes back (or forward) to `pos`, a byte offset an earlier [`Lexer::pos`] gave, to read
    /// from there again: for a reader that looked at a token and hands it back. An offset past the
    /// end, or inside a character, moves to the next character boundary.
    pub fn set_pos(&mut self, pos: usize) {
        let mut pos = pos.min(self.src.len());
        while !self.src.is_char_boundary(pos) {
            pos += 1;
        }
        self.pos = pos;
        self.raw = None;
    }

    /// The raw content of element `name`, whose start tag was the last token: everything up to
    /// its end tag (any letter case), or to the end of the input when it has none. The end tag
    /// is consumed. For `style` and `script`, whose content is not markup.
    pub fn raw_content(&mut self, name: &str) -> &'a str {
        self.raw = None;
        let b = self.bytes();
        let end = find_end_tag(b, self.pos, name).unwrap_or(b.len());
        let text = self.slice(self.pos, end);
        self.pos = find_byte(b, end, b'>').map_or(b.len(), |i| i + 1);
        text
    }

    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.src.get(start..end).unwrap_or("")
    }

    /// A tag name in lower case; empty when it is too long to be a real element.
    fn tag_name(&self, raw: &str) -> String {
        if raw.len() > self.opts.limits.max_tag_name { String::new() } else { raw.to_ascii_lowercase() }
    }

    /// Whether the `<` at `i` starts a tag, a comment or a declaration (rather than being text).
    fn markup_at(&self, i: usize) -> bool {
        let b = self.bytes();
        match b.get(i + 1) {
            Some(c) if c.is_ascii_alphabetic() => true,
            Some(b'!' | b'?') => true,
            Some(b'/') => matches!(b.get(i + 2), Some(c) if c.is_ascii_alphabetic() || *c == b'>'),
            _ => false,
        }
    }

    /// The end of the text run that starts at `self.pos`: the next `<` that starts markup.
    fn text_end(&self) -> usize {
        let b = self.bytes();
        let mut i = self.pos + 1;
        loop {
            match find_byte(b, i, b'<') {
                Some(j) if self.markup_at(j) => return j,
                Some(j) => i = j + 1,
                None => return b.len(),
            }
        }
    }

    /// Skip a comment, doctype or processing instruction at `self.pos`.
    fn skip_declaration(&mut self) {
        let b = self.bytes();
        let end = if b.get(self.pos..self.pos + 4) == Some(b"<!--") {
            let body = self.pos + 4;
            if b.get(body) == Some(&b'>') {
                body + 1
            } else if b.get(body..body + 2) == Some(b"->") {
                body + 2
            } else {
                find_sub(b, body, b"-->").map_or(b.len(), |i| i + 3)
            }
        } else {
            find_byte(b, self.pos, b'>').map_or(b.len(), |i| i + 1)
        };
        self.pos = end.min(b.len());
    }

    /// Read the end tag at `self.pos` (`</name ...>`).
    fn close_tag(&mut self) -> Option<Token<'a>> {
        let b = self.bytes();
        let start = self.pos + 2;
        let mut i = start;
        while let Some(c) = b.get(i) {
            if c.is_ascii_whitespace() || *c == b'/' || *c == b'>' {
                break;
            }
            i += 1;
        }
        let name = self.slice(start, i);
        self.pos = find_byte(b, i, b'>').map_or(b.len(), |j| j + 1);
        // `</>` is nothing at all.
        (!name.is_empty()).then(|| Token::End(self.tag_name(name)))
    }

    /// Read the start tag at `self.pos` (`<name attr=value ...>`).
    fn open_tag(&mut self) -> Tag<'a> {
        let b = self.bytes();
        let start = self.pos + 1;
        let mut i = start;
        while let Some(c) = b.get(i) {
            if c.is_ascii_whitespace() || *c == b'/' || *c == b'>' {
                break;
            }
            i += 1;
        }
        let name = self.tag_name(self.slice(start, i));
        let mut attrs = Vec::new();
        let mut self_close = false;
        loop {
            while matches!(b.get(i), Some(c) if c.is_ascii_whitespace() || *c == b'/') {
                self_close = b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'>');
                i += 1;
            }
            match b.get(i) {
                None => break,
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(_) => {}
            }
            // An attribute name runs to white space, `/`, `>` or `=`; a leading `=` is part of it.
            let name_start = i;
            i += 1;
            while let Some(c) = b.get(i) {
                if c.is_ascii_whitespace() || matches!(*c, b'/' | b'>' | b'=') {
                    break;
                }
                i += 1;
            }
            let attr = self.slice(name_start, i);
            let (value, next) = self.attr_value(i);
            i = next;
            if attrs.len() < self.opts.limits.max_attrs {
                attrs.push((attr, value));
            }
        }
        self.pos = i.min(b.len());
        Tag { name, attrs, self_close }
    }

    /// The value after an attribute name that ends at `i` (empty when it has none), and where
    /// reading goes on.
    fn attr_value(&self, i: usize) -> (&'a str, usize) {
        let b = self.bytes();
        let mut j = i;
        while matches!(b.get(j), Some(c) if c.is_ascii_whitespace()) {
            j += 1;
        }
        if b.get(j) != Some(&b'=') {
            return ("", j);
        }
        j += 1;
        while matches!(b.get(j), Some(c) if c.is_ascii_whitespace()) {
            j += 1;
        }
        match b.get(j) {
            Some(&q) if q == b'"' || q == b'\'' => {
                let end = find_byte(b, j + 1, q).unwrap_or(b.len());
                (self.slice(j + 1, end), (end + 1).min(b.len()))
            }
            _ => {
                let mut end = j;
                while let Some(c) = b.get(end) {
                    if c.is_ascii_whitespace() || *c == b'>' {
                        break;
                    }
                    end += 1;
                }
                (self.slice(j, end), end)
            }
        }
    }
}

impl<'a> Iterator for Lexer<'a, '_> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let b = self.bytes();
            if self.pos >= b.len() {
                return None;
            }
            if let Some(name) = self.raw.take() {
                let end = find_end_tag(b, self.pos, &name).unwrap_or(b.len());
                let text = self.slice(self.pos, end);
                self.pos = end;
                if !text.is_empty() {
                    return Some(Token::Text(text));
                }
                continue;
            }
            if b.get(self.pos) != Some(&b'<') || !self.markup_at(self.pos) {
                let start = self.pos;
                self.pos = self.text_end();
                return Some(Token::Text(self.slice(start, self.pos)));
            }
            match b.get(self.pos + 1) {
                Some(b'!' | b'?') => self.skip_declaration(),
                Some(b'/') => {
                    if let Some(token) = self.close_tag() {
                        return Some(token);
                    }
                }
                _ => {
                    let tag = self.open_tag();
                    if self.opts.drop.contains(&tag.name.as_str()) {
                        self.raw_content(&tag.name);
                        continue;
                    }
                    if self.opts.raw_text.contains(&tag.name.as_str()) {
                        self.raw = Some(tag.name.clone());
                    }
                    return Some(Token::Start(tag));
                }
            }
        }
    }
}

fn find_byte(b: &[u8], from: usize, byte: u8) -> Option<usize> {
    b.get(from..)?.iter().position(|c| *c == byte).map(|i| i + from)
}

fn find_sub(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Where `</name` starts (any letter case, followed by white space, `/`, `>` or the end), at or after `from`.
fn find_end_tag(b: &[u8], from: usize, name: &str) -> Option<usize> {
    let mut i = from;
    while let Some(lt) = find_byte(b, i, b'<') {
        let after = lt + 2 + name.len();
        let named = b.get(lt + 1) == Some(&b'/') && b.get(lt + 2..after).is_some_and(|n| n.eq_ignore_ascii_case(name.as_bytes()));
        if named && b.get(after).is_none_or(|c| c.is_ascii_whitespace() || matches!(*c, b'/' | b'>')) {
            return Some(lt);
        }
        i = lt + 1;
    }
    None
}

#[cfg(test)]
mod tests;
