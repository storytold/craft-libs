//! Header fields: reading them (unfolding, parameters) and writing them (folding).

use std::iter::Peekable;
use std::str::Chars;

use crate::words::decode_words;
use crate::{Limits, charset};

/// Most parameters read from one `Content-Type` or `Content-Disposition`.
pub const MAX_PARAMS: usize = 64;

/// Where a written header line is folded, when there is white space to fold at.
const FOLD_AT: usize = 78;

/// A header field: its name as written, and its value unfolded and trimmed (encoded-words not decoded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub value: String,
}

/// The header fields of a message or of one MIME part, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(pub Vec<Field>);

impl Headers {
    /// The value of the first field called `name` (any letter case).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|f| f.name.eq_ignore_ascii_case(name)).map(|f| f.value.as_str())
    }

    /// The values of every field called `name` (any letter case), in order.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.0.iter().filter(move |f| f.name.eq_ignore_ascii_case(name)).map(|f| f.value.as_str())
    }
}

/// The line that starts at `pos`, without its line break, and where the next one starts.
pub(crate) fn line_at(bytes: &[u8], pos: usize) -> (&[u8], usize) {
    let rest = bytes.get(pos..).unwrap_or_default();
    let (line, next) = match rest.iter().position(|b| *b == b'\n') {
        Some(i) => (rest.get(..i).unwrap_or_default(), pos.saturating_add(i).saturating_add(1)),
        None => (rest, bytes.len()),
    };
    (line.strip_suffix(b"\r").unwrap_or(line), next)
}

/// Split a message (or a MIME part) into its header fields and its body.
///
/// Takes CRLF or bare LF line ends. Folded lines are joined. The header block ends at the first
/// empty line, or at the first line that can't be a header field (some senders forget the empty
/// line). At the `top` of a message a leading mbox `From ` line is skipped. A value in raw
/// 8-bit bytes is read as UTF-8 when it is valid UTF-8, else as Windows-1252.
///
/// Bounded by `limits`: fields past [`Limits::max_header_fields`] or
/// [`Limits::max_header_block`] are read past, and a value stops growing at
/// [`Limits::max_header_value`]. Fields named in [`Limits::reserved_fields`] are kept past the
/// first two caps (see there).
pub fn split_headers<'a>(bytes: &'a [u8], top: bool, limits: &Limits) -> (Headers, &'a [u8]) {
    let (headers, body, _) = split_counted(bytes, top, limits);
    (headers, body)
}

/// What [`split_counted`] keeps as it goes.
struct Kept<'l> {
    limits: &'l Limits,
    fields: Vec<Field>,
    /// Bytes of names and values kept so far.
    bytes: usize,
    /// How many of each reserved field were kept.
    reserved: Vec<usize>,
    /// Whether a limit left something out.
    cut: bool,
}

impl Kept<'_> {
    fn push(&mut self, current: Option<(String, Vec<u8>)>) {
        let Some((name, value)) = current else { return };
        // Reserved fields get room past both caps, so that padding can't push them out; each is
        // cut short, with a mark so that a cut value never reads as a shorter whole one.
        let reserved = self.limits.reserved_fields.iter().position(|r| name.eq_ignore_ascii_case(r));
        if let Some(count) = reserved.and_then(|i| self.reserved.get_mut(i)) {
            if *count < self.limits.max_reserved_each {
                *count = count.saturating_add(1);
                let value = value.trim_ascii();
                let mut text = charset::guess(value.get(..self.limits.max_reserved_value).unwrap_or(value));
                if value.len() > self.limits.max_reserved_value {
                    text.push('…');
                    self.cut = true;
                }
                self.fields.push(Field { name, value: text });
            } else {
                self.cut = true;
            }
            return;
        }
        let size = name.len().saturating_add(value.len());
        if self.fields.len() < self.limits.max_header_fields && self.bytes.saturating_add(size) <= self.limits.max_header_block {
            self.bytes = self.bytes.saturating_add(size);
            self.fields.push(Field { name, value: charset::guess(value.trim_ascii()) });
        } else {
            self.cut = true;
        }
    }
}

/// [`split_headers`], and whether a limit left something out.
pub(crate) fn split_counted<'a>(bytes: &'a [u8], top: bool, limits: &Limits) -> (Headers, &'a [u8], bool) {
    let max_value = limits.max_header_value;
    let mut kept = Kept { limits, fields: Vec::new(), bytes: 0, reserved: vec![0; limits.reserved_fields.len()], cut: false };
    let mut current: Option<(String, Vec<u8>)> = None;
    let mut pos = 0;
    let mut first = true;
    let body = loop {
        if pos >= bytes.len() {
            break bytes.len();
        }
        let (line, next) = line_at(bytes, pos);
        if line.is_empty() {
            break next;
        }
        if matches!(line.first(), Some(b' ' | b'\t')) {
            match &mut current {
                Some((_, value)) => {
                    if value.len().saturating_add(line.len()) <= max_value {
                        value.extend_from_slice(line);
                    } else {
                        kept.cut = true;
                    }
                }
                // A continuation of nothing: this is not a header block.
                None => break pos,
            }
        } else if let Some((name, value)) = field(line) {
            kept.push(current.take());
            kept.cut |= value.len() > max_value;
            current = Some((name, value.get(..max_value).unwrap_or(value).to_vec()));
        } else if !(top && first && (line.starts_with(b"From ") || line.starts_with(b">From "))) {
            break pos;
        }
        first = false;
        pos = next;
    };
    kept.push(current.take());
    (Headers(kept.fields), bytes.get(body..).unwrap_or_default(), kept.cut)
}

/// A header line as a name and a raw value, when it is one.
fn field(line: &[u8]) -> Option<(String, &[u8])> {
    let colon = line.iter().position(|b| *b == b':')?;
    let name = line.get(..colon)?.trim_ascii_end();
    if !is_field_name(name) {
        return None;
    }
    Some((String::from_utf8_lossy(name).into_owned(), line.get(colon.saturating_add(1)..)?))
}

/// Whether bytes can be a header field name: printable ASCII without spaces or a colon.
pub fn is_field_name(name: &[u8]) -> bool {
    !name.is_empty() && name.iter().all(|b| (33..=126).contains(b) && *b != b':')
}

/// A structured header value: `text/plain; charset=utf-8`, `attachment; filename="a.txt"`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params {
    /// The value before the first `;`, lower case.
    pub value: String,
    params: Vec<Param>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Param {
    name: String,
    value: String,
    /// Came in the RFC 2231 form (`name*=`), which says its own character set.
    extended: bool,
}

impl Params {
    /// A parameter as written (quotes removed). `name` in lower case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|p| p.name == name).map(|p| p.value.as_str())
    }

    /// A parameter that holds text for people (a file name): RFC 2231 decoded, or RFC 2047
    /// encoded-words decoded, which many senders put there although they shouldn't.
    pub fn text(&self, name: &str) -> Option<String> {
        self.params.iter().find(|p| p.name == name).map(|p| if p.extended { p.value.clone() } else { decode_words(&p.value) })
    }

    /// The names of the parameters, lower case, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.params.iter().map(|p| p.name.as_str())
    }
}

/// One `name=value` piece of a parameter list, before RFC 2231 pieces are put together.
struct Piece {
    name: String,
    /// The position of a continuation (`name*1`).
    index: Option<u32>,
    /// Percent-encoded (`name*=` or `name*1*=`).
    encoded: bool,
    value: String,
}

/// Step over the `(comment)` that `chars` stands at, nested ones and backslash escapes included.
/// Nothing is consumed, and `false` comes back, when it never closes: then the `(` is text, and
/// `open` (whether comments are still looked for) is cleared so that the scan stays linear.
fn skip_comment(chars: &mut Peekable<Chars<'_>>, open: &mut bool) -> bool {
    if !*open || chars.peek() != Some(&'(') {
        return false;
    }
    let mut ahead = chars.clone();
    let mut depth = 0usize;
    while let Some(c) = ahead.next() {
        match c {
            '\\' => {
                ahead.next();
            }
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    *chars = ahead;
                    return true;
                }
            }
            _ => {}
        }
    }
    *open = false;
    false
}

/// Parse a structured header value and its parameters, RFC 2231 forms included
/// (`filename*=utf-8''na%C3%AFve.txt`, and the continuations `filename*0=…; filename*1=…`).
///
/// Comments (RFC 5322 §3.2.2) are stepped over where one can stand without being part of a
/// value: around the value before the first `;`, before a parameter's name, and after a quoted
/// value. So a `;` or a `"` inside a comment neither ends the value nor starts a parameter.
/// Inside an unquoted value a `(` is text (`filename=report (1).pdf`).
pub fn parse_params(s: &str) -> Params {
    let mut chars = s.chars().peekable();
    let mut comments = true;
    let mut main = String::new();
    while chars.peek().is_some() {
        if skip_comment(&mut chars, &mut comments) {
            main.push(' ');
        } else if let Some(c) = chars.next() {
            if c == ';' {
                break;
            }
            main.push(c);
        }
    }
    let value = main.split(|c: char| c.is_whitespace() || c == '(').find(|t| !t.is_empty()).unwrap_or("").to_ascii_lowercase();

    let mut pieces: Vec<Piece> = Vec::new();
    while pieces.len() < MAX_PARAMS {
        loop {
            while chars.next_if(|c| c.is_whitespace() || *c == ';').is_some() {}
            if !skip_comment(&mut chars, &mut comments) {
                break;
            }
        }
        if chars.peek().is_none() {
            break;
        }
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| *c != '=' && *c != ';') {
            name.push(c);
        }
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            if chars.next_if_eq(&'"').is_some() {
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        // Only `\"` and `\\` are escapes; a lone backslash (a Windows path) stays.
                        '\\' => value.push(chars.next_if(|n| *n == '"' || *n == '\\').unwrap_or('\\')),
                        c => value.push(c),
                    }
                }
                // Anything between the closing quote and the next `;` is noise.
                while chars.peek().is_some_and(|c| *c != ';') {
                    if !skip_comment(&mut chars, &mut comments) {
                        chars.next();
                    }
                }
            } else {
                while let Some(c) = chars.next_if(|c| *c != ';') {
                    value.push(c);
                }
                value.truncate(value.trim_end().len());
            }
        }
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let mut parts = name.split('*');
        let base = parts.next().unwrap_or("").to_string();
        let piece = match (parts.next(), parts.next()) {
            (None, _) => Piece { name: base, index: None, encoded: false, value },
            (Some(""), _) => Piece { name: base, index: None, encoded: true, value },
            (Some(n), star) => match n.parse::<u32>() {
                Ok(index) => Piece { name: base, index: Some(index), encoded: star == Some(""), value },
                Err(_) => continue,
            },
        };
        pieces.push(piece);
    }

    let mut params: Vec<Param> = Vec::new();
    for piece in &pieces {
        if params.iter().any(|p| p.name == piece.name) {
            continue;
        }
        let mut extended: Vec<&Piece> = pieces.iter().filter(|p| p.name == piece.name && (p.encoded || p.index.is_some())).collect();
        if extended.is_empty() {
            params.push(Param { name: piece.name.clone(), value: piece.value.clone(), extended: false });
            continue;
        }
        // The RFC 2231 form wins over a plain parameter of the same name.
        extended.sort_by_key(|p| p.index.unwrap_or(0));
        let mut charset = String::new();
        let mut bytes = Vec::new();
        for (i, p) in extended.iter().enumerate() {
            if !p.encoded {
                bytes.extend_from_slice(p.value.as_bytes());
                continue;
            }
            // The first piece starts with `charset'language'`.
            let mut text = p.value.as_str();
            if i == 0 {
                let mut it = text.splitn(3, '\'');
                if let (Some(cs), Some(_language), Some(rest)) = (it.next(), it.next(), it.next()) {
                    charset = cs.to_string();
                    text = rest;
                }
            }
            bytes.extend(percent_decode(text));
        }
        params.push(Param { name: piece.name.clone(), value: charset::decode(&bytes, &charset), extended: true });
    }
    Params { value, params }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).and_then(|c| char::from(*c).to_digit(16)).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        match (c, hex(i.saturating_add(1)), hex(i.saturating_add(2))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h << 4) | l);
                i = i.saturating_add(3);
            }
            _ => {
                out.push(c);
                i = i.saturating_add(1);
            }
        }
    }
    out
}

/// Remove what must never reach a header: CR, LF and NUL.
pub fn clean(value: &str) -> String {
    value.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect()
}

/// Write one header field with its CRLF, folded at white space so lines stay within 78
/// characters where the value allows. CR, LF and NUL in the value are dropped, so a value can
/// never start a new header field. A `name` that can't be a field name ([`is_field_name`])
/// writes nothing and gives `false`.
///
/// The value is written as it is otherwise: text that isn't printable ASCII goes through
/// [`crate::encode_unstructured`] (or [`crate::encode_words`]) first.
pub fn write_field(out: &mut Vec<u8>, name: &str, value: &str) -> bool {
    if !is_field_name(name.as_bytes()) {
        return false;
    }
    let value = clean(value);
    let value = value.as_str();
    out.extend_from_slice(name.as_bytes());
    out.push(b':');
    if !value.is_empty() {
        out.push(b' ');
    }
    let is_space = |c: char| c == ' ' || c == '\t';
    let mut col = name.len().saturating_add(2);
    let mut pos = 0usize;
    while pos < value.len() {
        // A chunk is a run of white space and the word after it.
        let rest = value.get(pos..).unwrap_or("");
        let word = rest.find(|c: char| !is_space(c)).unwrap_or(rest.len());
        let len = rest.get(word..).and_then(|w| w.find(is_space)).map_or(rest.len(), |i| word.saturating_add(i));
        // Fold before the white space, never before the first word and never to leave a blank line.
        if pos > 0 && word > 0 && len > word && col.saturating_add(len) > FOLD_AT {
            out.extend_from_slice(b"\r\n");
            col = 0;
        }
        out.extend_from_slice(rest.get(..len).unwrap_or("").as_bytes());
        col = col.saturating_add(len);
        pos = pos.saturating_add(len.max(1));
    }
    out.extend_from_slice(b"\r\n");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_FIELDS: usize = Limits::DEFAULT.max_header_fields;
    const MAX_VALUE: usize = Limits::DEFAULT.max_header_value;

    fn split(bytes: &[u8], top: bool) -> (Headers, &[u8]) {
        split_headers(bytes, top, &Limits::default())
    }

    fn fields(bytes: &[u8]) -> Vec<(String, String)> {
        split(bytes, true).0.0.into_iter().map(|f| (f.name, f.value)).collect()
    }

    fn pair(n: &str, v: &str) -> (String, String) {
        (n.to_string(), v.to_string())
    }

    fn written(name: &str, value: &str) -> String {
        let mut out = Vec::new();
        assert!(write_field(&mut out, name, value));
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn splits_headers_from_body() {
        let (h, body) = split(b"Subject: Hi\r\nTo: bob@example.org\r\n\r\nBody\r\nmore", true);
        assert_eq!(h.get("subject"), Some("Hi"));
        assert_eq!(h.get("TO"), Some("bob@example.org"));
        assert_eq!(h.get("cc"), None);
        assert_eq!(body, b"Body\r\nmore");
    }

    #[test]
    fn bare_lf_and_no_body() {
        let (h, body) = split(b"A: 1\nB: 2\n\nBody\n", true);
        assert_eq!(h.0.len(), 2);
        assert_eq!(body, b"Body\n");
        assert_eq!(split(b"A: 1", true), (Headers(vec![Field { name: "A".into(), value: "1".into() }]), &b""[..]));
        assert_eq!(split(b"A: 1\r\n", true).1, b"");
        assert_eq!(split(b"", true), (Headers::default(), &b""[..]));
        assert_eq!(split(b"\r\nBody", true), (Headers::default(), &b"Body"[..]));
    }

    #[test]
    fn folded_lines_are_joined() {
        let got = fields(b"Subject: a\r\n b\r\n\tc\r\nX:\r\n folded value\r\n\r\n");
        assert_eq!(got, [pair("Subject", "a b\tc"), pair("X", "folded value")]);
    }

    #[test]
    fn odd_field_lines() {
        assert_eq!(fields(b"Name : spaced\r\nEmpty:\r\nColons: a:b:c\r\n\r\n"), [pair("Name", "spaced"), pair("Empty", ""), pair("Colons", "a:b:c")]);
        // Raw 8-bit values: UTF-8 when valid, else Windows-1252.
        assert_eq!(fields("Subject: caf\u{e9}\n\n".as_bytes()), [pair("Subject", "café")]);
        assert_eq!(fields(b"Subject: caf\xE9\n\n"), [pair("Subject", "café")]);
    }

    #[test]
    fn a_line_that_is_no_header_starts_the_body() {
        let (h, body) = split(b"A: 1\r\nThis is text\r\nB: 2\r\n", true);
        assert_eq!(h.0.len(), 1);
        assert_eq!(body, b"This is text\r\nB: 2\r\n");
        assert_eq!(split(b"just text", true), (Headers::default(), &b"just text"[..]));
        assert_eq!(split(b" leading space: x\r\n", true).1, b" leading space: x\r\n");
        assert_eq!(split(b"Bad Name: x\r\n", true).1, b"Bad Name: x\r\n");
        assert_eq!(split(b": no name\r\n", true).1, b": no name\r\n");
    }

    #[test]
    fn an_mbox_from_line_is_skipped_at_the_top_only() {
        let msg = b"From ada@example.org Sat Jan  3 01:05:34 1996\r\nSubject: x\r\n\r\nB";
        assert_eq!(fields(msg), [pair("Subject", "x")]);
        assert_eq!(split(msg, false).0, Headers::default());
        assert_eq!(split(b"A: 1\r\nFrom here on\r\n", true).1, b"From here on\r\n");
    }

    #[test]
    fn header_counts_and_sizes_are_capped() {
        let many = "X: 1\r\n".repeat(MAX_FIELDS + 50) + "\r\nBody";
        let (h, body) = split(many.as_bytes(), true);
        assert_eq!(h.0.len(), MAX_FIELDS);
        assert_eq!(body, b"Body");
        let long = format!("X: start\r\n{}\r\nbody", " more\r\n".repeat(100_000));
        let (h, body) = split(long.as_bytes(), true);
        assert!(h.get("x").is_some_and(|v| v.len() <= MAX_VALUE));
        assert_eq!(body, b"body");
        let one_line = format!("X: {}\r\n\r\n", "a".repeat(MAX_VALUE * 2));
        // The cut includes the space after the colon, which is then trimmed.
        assert_eq!(split(one_line.as_bytes(), true).0.get("x").map(str::len), Some(MAX_VALUE - 1));
    }

    #[test]
    fn each_header_limit_holds() {
        let small = Limits { max_header_fields: 3, max_header_value: 10, max_header_block: 40, ..Limits::default() };
        let (h, body, cut) = split_counted(b"A: 1\r\nB: 2\r\nC: 3\r\nD: 4\r\n\r\nbody", true, &small);
        assert_eq!((h.0.len(), body, cut), (3, &b"body"[..], true));
        let (h, _, cut) = split_counted(b"A: 0123456789abcdef\r\n more\r\n\r\n", true, &small);
        assert_eq!((h.get("a"), cut), (Some("012345678"), true));
        let (h, _, cut) = split_counted(b"Aaaaaaaaaa: 123456789\r\nBbbbbbbbbb: 123456789\r\nC: 1\r\n\r\n", true, &small);
        // The block cap counts names and values as they came: 20 + 20, and no room for `C`.
        assert_eq!((h.0.len(), h.get("c"), cut), (2, None, true));
        assert!(!split_counted(b"A: 1\r\nB: 2\r\n\r\nbody", true, &small).2);
    }

    #[test]
    fn reserved_fields_survive_padding() {
        let limits = Limits { max_header_fields: 2, reserved_fields: &["X-Stamp"], max_reserved_each: 2, max_reserved_value: 4, ..Limits::default() };
        let text = "P: 1\r\nP: 2\r\nP: 3\r\nx-stamp: Internal\r\nX-Stamp: ok\r\nX-Stamp: third\r\n\r\n";
        let (h, _, cut) = split_counted(text.as_bytes(), true, &limits);
        let got: Vec<(&str, &str)> = h.0.iter().map(|f| (f.name.as_str(), f.value.as_str())).collect();
        // Cut values carry a mark, so that a cut one never reads as a shorter whole one.
        assert_eq!(got, [("P", "1"), ("P", "2"), ("x-stamp", "Inte…"), ("X-Stamp", "ok")]);
        assert!(cut);
    }

    #[test]
    fn a_name_that_is_no_field_name_writes_nothing() {
        for name in ["", "Bad Name", "X:Y", "X\r\nBcc", "Sübject", "X\0"] {
            let mut out = Vec::new();
            assert!(!write_field(&mut out, name, "v"), "{name:?}");
            assert!(out.is_empty());
        }
    }

    #[test]
    fn parameters() {
        let p = parse_params("Text/Plain; charset=\"UTF-8\"; Format=flowed");
        assert_eq!(p.value, "text/plain");
        assert_eq!(p.get("charset"), Some("UTF-8"));
        assert_eq!(p.get("format"), Some("flowed"));
        assert_eq!(p.get("missing"), None);
        assert_eq!(parse_params("attachment").value, "attachment");
        assert_eq!(parse_params("").value, "");
        assert_eq!(parse_params("text/html (a comment); charset=us-ascii").value, "text/html");
    }

    #[test]
    fn quoted_parameters() {
        let p = parse_params(r#"attachment; filename="a \"quoted\" name; v2.txt" ; size=12"#);
        assert_eq!(p.get("filename"), Some("a \"quoted\" name; v2.txt"));
        assert_eq!(p.get("size"), Some("12"));
        assert_eq!(parse_params(r#"x; filename="C:\dir\file.txt""#).get("filename"), Some(r"C:\dir\file.txt"));
        assert_eq!(parse_params(r#"x; a="back\\slash""#).get("a"), Some(r"back\slash"));
        assert_eq!(parse_params(r#"x; a="unterminated"#).get("a"), Some("unterminated"));
        assert_eq!(parse_params("x; filename=my file.pdf; b=2").get("filename"), Some("my file.pdf"));
        assert_eq!(parse_params("x; flag; a=1;;; =novalue; b").get("a"), Some("1"));
    }

    #[test]
    fn rfc2231_extended_parameters() {
        let p = parse_params("attachment; filename*=utf-8''na%C3%AFve%20r%C3%A9sum%C3%A9.pdf; filename=\"fallback.pdf\"");
        assert_eq!(p.text("filename").as_deref(), Some("naïve résumé.pdf"));
        let p = parse_params("attachment; filename=\"fallback.pdf\"; filename*=UTF-8'en'%E2%82%AC.txt");
        assert_eq!(p.text("filename").as_deref(), Some("€.txt"));
        let p = parse_params("attachment; filename*=iso-8859-1''caf%E9.txt");
        assert_eq!(p.text("filename").as_deref(), Some("café.txt"));
        // No character set: guessed.
        assert_eq!(parse_params("x; filename*=''a%20b").text("filename").as_deref(), Some("a b"));
        assert_eq!(parse_params("x; filename*=plain%41").text("filename").as_deref(), Some("plainA"));
        assert_eq!(parse_params("x; filename*=utf-8''bad%ZZ%4").text("filename").as_deref(), Some("bad%ZZ%4"));
    }

    #[test]
    fn rfc2231_continuations() {
        let p = parse_params("attachment; filename*1=\"second half.txt\"; filename*0=\"first half and \"");
        assert_eq!(p.text("filename").as_deref(), Some("first half and second half.txt"));
        let p = parse_params("attachment; filename*0*=utf-8''%E2%82%AC%20; filename*1*=r%C3%A9sum%C3%A9; filename*2=\" (1).pdf\"");
        assert_eq!(p.text("filename").as_deref(), Some("€ résumé (1).pdf"));
        // A character cut between two pieces.
        let p = parse_params("x; n*0*=utf-8''caf%C3; n*1*=%A9");
        assert_eq!(p.text("n").as_deref(), Some("café"));
        assert_eq!(parse_params("x; n*x=1; n*99999999999=2; m=3").get("m"), Some("3"));
    }

    #[test]
    fn encoded_words_in_parameters() {
        let p = parse_params("application/pdf; name=\"=?UTF-8?B?csOpc3Vtw6kucGRm?=\"");
        assert_eq!(p.text("name").as_deref(), Some("résumé.pdf"));
        assert_eq!(p.get("name"), Some("=?UTF-8?B?csOpc3Vtw6kucGRm?="));
        // The extended form is not decoded twice.
        let p = parse_params("x; name*=utf-8''%3D%3FUTF-8%3FQ%3Fa%3F%3D");
        assert_eq!(p.text("name").as_deref(), Some("=?UTF-8?Q?a?="));
    }

    #[test]
    fn comments_neither_end_the_value_nor_start_a_parameter() {
        // WordCraft's MHT reader: a quote or a `;` in a comment must not hide or fake a boundary.
        for comment in [r#"(10" screen)"#, r#"(outer (10" screen); boundary=wrong)"#, r#"(10\)" screen; boundary=wrong)"#] {
            let p = parse_params(&format!("multipart/related {comment}; boundary=\"----=_B\""));
            assert_eq!((p.value.as_str(), p.get("boundary")), ("multipart/related", Some("----=_B")), "{comment}");
            let p = parse_params(&format!("multipart/related; {comment} boundary=right; name=\"a\" {comment}; z=1"));
            assert_eq!((p.get("boundary"), p.get("z")), (Some("right"), Some("1")), "{comment}");
        }
        for name in [r#"name="notes; boundary=wrong""#, r#"name="notes\"; boundary=wrong""#, r#"name="notes (10\" screen; boundary=wrong""#] {
            let p = parse_params(&format!("multipart/related; {name}; boundary=\"----=_B\""));
            assert_eq!(p.get("boundary"), Some("----=_B"), "{name}");
        }
        assert_eq!(parse_params("(first) text/html (second); charset=x").value, "text/html");
        // In an unquoted value a parenthesis is text, and a comment that never closes is too.
        assert_eq!(parse_params("x; filename=report (1).pdf").get("filename"), Some("report (1).pdf"));
        let p = parse_params("text/html (oops; charset=utf-8");
        assert_eq!((p.value.as_str(), p.get("charset")), ("text/html", Some("utf-8")));
        // Linear on brackets that never close.
        let _ = parse_params(&format!("x; a=\"b\"{}", "(".repeat(200_000)));
        let _ = parse_params(&"(;".repeat(100_000));
    }

    #[test]
    fn many_parameters_are_capped() {
        let s = format!("x{}", (0..500).map(|i| format!("; p{i}=v")).collect::<String>());
        let p = parse_params(&s);
        assert_eq!(p.get("p0"), Some("v"));
        assert_eq!(p.get("p499"), None);
    }

    #[test]
    fn short_fields_are_written_on_one_line() {
        assert_eq!(written("Subject", "Hello there"), "Subject: Hello there\r\n");
        assert_eq!(written("X-Empty", ""), "X-Empty:\r\n");
    }

    #[test]
    fn long_fields_fold_at_white_space() {
        let value = "word ".repeat(40);
        let value = value.trim_end();
        let text = written("Subject", value);
        assert!(text.ends_with("\r\n"));
        let lines: Vec<&str> = text.trim_end_matches("\r\n").split("\r\n").collect();
        assert!(lines.len() > 2);
        for (i, line) in lines.iter().enumerate() {
            assert!(line.len() <= 78, "{line}");
            assert_eq!(line.starts_with(' '), i > 0);
            assert!(!line.trim().is_empty());
        }
        // Unfolding gives the value back.
        assert_eq!(fields(format!("{text}\r\n").as_bytes()), [pair("Subject", value)]);
    }

    #[test]
    fn folding_keeps_every_space() {
        for value in [
            "a    b",
            "tab\tseparated\twords",
            &format!("{}  {}   {}", "x".repeat(50), "y".repeat(50), "z".repeat(50)),
            &format!("{} \t {}", "x".repeat(70), "y".repeat(70)),
        ] {
            let text = written("X-Test", value);
            assert_eq!(fields(format!("{text}\r\n").as_bytes()), [pair("X-Test", value)], "{text}");
        }
    }

    #[test]
    fn a_word_that_cannot_fold_stays_whole() {
        let long = "x".repeat(300);
        assert_eq!(written("X", &long), format!("X: {long}\r\n"));
        // No fold before the first word, and none that would leave a blank line.
        assert_eq!(written("X", &format!("{long} ")), format!("X: {long} \r\n"));
        assert_eq!(written("X", &format!(" {long}")), format!("X:  {long}\r\n"));
    }

    #[test]
    fn line_breaks_cannot_be_injected() {
        assert_eq!(written("Subject", "Hi\r\nBcc: eve@example.org"), "Subject: HiBcc: eve@example.org\r\n");
        assert_eq!(written("Subject", "a\nb\rc\0d"), "Subject: abcd\r\n");
        assert_eq!(clean("\r\n\0"), "");
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_split(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..400)) {
            let (h, body) = split(&data, true);
            proptest::prop_assert!(body.len() <= data.len());
            for f in &h.0 {
                proptest::prop_assert!(is_field_name(f.name.as_bytes()));
            }
        }

        #[test]
        fn any_parameters_parse(s in "[a-z*0-9=;\"'%\\\\ é]{0,60}") {
            let p = parse_params(&s);
            let _ = p.text("a");
        }

        #[test]
        fn written_fields_read_back(value in "[ -~\t]{0,300}") {
            let text = written("X-Any", &value);
            proptest::prop_assert!(!text.trim_end_matches("\r\n").split("\r\n").any(|l| l.trim().is_empty()));
            let (h, _) = split(format!("{text}\r\n").as_bytes(), true);
            proptest::prop_assert_eq!(h.get("x-any"), Some(value.trim_matches([' ', '\t'])));
        }
    }
}
