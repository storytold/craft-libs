//! The PDF object model, its lexer and its serializer.

use super::*;

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Obj {
    Null,
    Bool(bool),
    /// A number as written, so it is written back the same.
    Num(Vec<u8>),
    /// A name, `#xx` escapes decoded.
    Name(Vec<u8>),
    /// A string as written, delimiters included (still encrypted, if the file is).
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    Ref(u32, u16),
}

pub(super) type Dict = Vec<(Vec<u8>, Obj)>;

pub(super) fn get<'d>(dict: &'d Dict, key: &[u8]) -> Option<&'d Obj> {
    dict.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

impl Obj {
    pub(super) fn num(&self) -> Option<f64> {
        match self {
            Obj::Num(raw) => std::str::from_utf8(raw).ok()?.parse::<f64>().ok().filter(|v| v.is_finite()),
            _ => None,
        }
    }
    pub(super) fn int(&self) -> Option<i64> {
        match self {
            Obj::Num(raw) => std::str::from_utf8(raw).ok()?.parse::<i64>().ok(),
            _ => None,
        }
    }
    pub(super) fn name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    pub(super) fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }
    pub(super) fn array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }
}

fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delim(b)
}

pub(super) struct Lexer<'a> {
    pub(super) s: &'a [u8],
    pub(super) pos: usize,
    pub(super) nodes: usize,
    pub(super) tokens: usize,
    max_nodes: usize,
    max_tokens: usize,
    pub(super) exhausted: bool,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(s: &'a [u8], pos: usize) -> Self {
        Lexer { s, pos, nodes: 0, tokens: 0, max_nodes: MAX_OBJECT_NODES, max_tokens: MAX_OBJECT_TOKENS, exhausted: false }
    }

    pub(super) fn bounded(s: &'a [u8], pos: usize, budget: &Budget) -> Self {
        let mut lx = Self::new(s, pos);
        lx.max_nodes = MAX_OBJECT_NODES.min(MAX_NODES.saturating_sub(budget.nodes));
        lx.max_tokens = MAX_OBJECT_TOKENS.min(MAX_TOKENS.saturating_sub(budget.tokens));
        lx
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    pub(super) fn rest(&self) -> &'a [u8] {
        self.s.get(self.pos..).unwrap_or(&[])
    }

    /// Skips whitespace and comments, charged to the token budget like the tokens they sit
    /// between: an object behind a long comment is otherwise scanned again for every reference
    /// to it, outside every budget.
    pub(super) fn skip_ws(&mut self) {
        let start = self.pos;
        let end = self.pos.saturating_add(self.max_tokens.saturating_sub(self.tokens));
        while let Some(b) = self.peek() {
            if self.pos >= end {
                self.exhausted = true;
                break;
            }
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(b) = self.peek() {
                    if b == b'\n' || b == b'\r' || self.pos >= end {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        self.tokens = self.tokens.saturating_add(self.pos - start);
    }

    /// A run of regular characters (a number, keyword or name body).
    pub(super) fn word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        self.s.get(start..self.pos).unwrap_or(&[])
    }

    /// The keyword `kw` as a whole word, consumed only when it is there.
    pub(super) fn keyword(&mut self, kw: &[u8]) -> bool {
        self.skip_ws();
        let rest = self.rest();
        if rest.starts_with(kw) && rest.get(kw.len()).is_none_or(|b| !is_regular(*b)) {
            self.pos += kw.len();
            true
        } else {
            false
        }
    }

    /// A non-negative integer, consumed only when it is one.
    pub(super) fn unsigned(&mut self) -> Option<u64> {
        self.skip_ws();
        let start = self.pos;
        let word = self.word();
        match std::str::from_utf8(word).ok().filter(|w| w.bytes().all(|b| b.is_ascii_digit())).and_then(|w| w.parse().ok()) {
            Some(v) => Some(v),
            None => {
                self.pos = start;
                None
            }
        }
    }

    pub(super) fn object(&mut self, depth: usize) -> Option<Obj> {
        self.nodes = self.nodes.checked_add(1)?;
        if self.nodes > self.max_nodes {
            self.exhausted = true;
            return None;
        }
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_ws();
        match self.peek()? {
            b'/' => {
                self.pos += 1;
                Some(Obj::Name(self.name()?))
            }
            b'(' => self.literal().map(Obj::Str),
            b'<' if self.s.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                self.dict_body(depth).map(Obj::Dict)
            }
            b'<' => self.hex().map(Obj::Str),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        return Some(Obj::Array(items));
                    }
                    items.push(self.object(depth + 1)?);
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number_or_ref(),
            _ => match self.word() {
                b"true" => Some(Obj::Bool(true)),
                b"false" => Some(Obj::Bool(false)),
                b"null" => Some(Obj::Null),
                _ => None,
            },
        }
    }

    fn dict_body(&mut self, depth: usize) -> Option<Dict> {
        // A dictionary's last occurrence of a key wins, including its position. Scanning
        // all earlier keys for each insertion makes a small hostile dictionary quadratic.
        // Keep bounded slots while reading, then compact them once in final occurrence order.
        let mut positions = HashMap::new();
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.rest().starts_with(b">>") {
                self.pos += 2;
                return Some(items.into_iter().flatten().collect());
            }
            if self.peek()? != b'/' {
                return None;
            }
            self.pos += 1;
            let key = self.name()?;
            let value = self.object(depth + 1)?;
            if let Some(previous) = positions.insert(key.clone(), items.len()) {
                *items.get_mut(previous)? = None;
            }
            items.push(Some((key, value)));
        }
    }

    pub(super) fn name(&mut self) -> Option<Vec<u8>> {
        let raw = self.word();
        self.tokens = self.tokens.checked_add(raw.len())?;
        if self.tokens > self.max_tokens {
            self.exhausted = true;
            return None;
        }
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 0;
        while let Some(&b) = raw.get(i) {
            let hex = raw.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            match (b, hex) {
                (b'#', Some(v)) => {
                    out.push(v);
                    i += 3;
                }
                _ => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        Some(out)
    }

    pub(super) fn token(&mut self, start: usize, end: usize) -> Option<Vec<u8>> {
        let raw = self.s.get(start..end)?;
        self.tokens = self.tokens.checked_add(raw.len())?;
        if self.tokens > self.max_tokens {
            self.exhausted = true;
            return None;
        }
        Some(raw.to_vec())
    }

    /// A literal string, kept as written: balanced parentheses, `\` escaping the next byte.
    fn literal(&mut self) -> Option<Vec<u8>> {
        let start = self.pos;
        let mut depth = 0usize;
        loop {
            let b = self.peek()?;
            self.pos += 1;
            match b {
                b'\\' => self.pos += 1,
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return self.token(start, self.pos);
                    }
                }
                _ => {}
            }
        }
    }

    fn hex(&mut self) -> Option<Vec<u8>> {
        let start = self.pos;
        let end = self.rest().iter().position(|&b| b == b'>')?;
        self.pos += end + 1;
        self.token(start, self.pos)
    }

    /// A number, or `num gen R`.
    fn number_or_ref(&mut self) -> Option<Obj> {
        let word = self.word();
        if word.is_empty() || !word.iter().all(|b| matches!(b, b'+' | b'-' | b'.' | b'0'..=b'9')) || !word.iter().any(u8::is_ascii_digit) {
            return None;
        }
        if word.iter().all(u8::is_ascii_digit) {
            let (after, tokens, exhausted) = (self.pos, self.tokens, self.exhausted);
            if let (Some(num), Some(generation)) = (std::str::from_utf8(word).ok().and_then(|w| w.parse::<u32>().ok()), self.unsigned())
                && let Ok(generation) = u16::try_from(generation)
                && self.keyword(b"R")
            {
                return Some(Obj::Ref(num, generation));
            }
            // The caller rescans what the lookahead skipped: it must not be charged twice.
            (self.pos, self.tokens, self.exhausted) = (after, tokens, exhausted);
        }
        self.tokens = self.tokens.checked_add(word.len())?;
        if self.tokens > self.max_tokens {
            self.exhausted = true;
            return None;
        }
        Some(Obj::Num(word.to_vec()))
    }
}

/// An object written back: the value it was read as.
pub(super) fn write_obj(out: &mut Vec<u8>, obj: &Obj) {
    match obj {
        Obj::Null => out.extend_from_slice(b"null"),
        Obj::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Obj::Num(raw) | Obj::Str(raw) => out.extend_from_slice(raw),
        Obj::Name(name) => write_name(out, name),
        Obj::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_obj(out, item);
            }
            out.push(b']');
        }
        Obj::Dict(dict) => write_dict(out, dict),
        Obj::Ref(num, generation) => out.extend_from_slice(format!("{num} {generation} R").as_bytes()),
    }
}

fn write_name(out: &mut Vec<u8>, name: &[u8]) {
    out.push(b'/');
    for &b in name {
        if (0x21..=0x7e).contains(&b) && !is_delim(b) && b != b'#' {
            out.push(b);
        } else {
            out.extend_from_slice(format!("#{b:02X}").as_bytes());
        }
    }
}

pub(super) fn write_dict(out: &mut Vec<u8>, dict: &Dict) {
    out.extend_from_slice(b"<<");
    for (key, value) in dict {
        write_name(out, key);
        out.push(b' ');
        write_obj(out, value);
    }
    out.extend_from_slice(b">>");
}

/// A number for a content stream or a box: at most four decimals, no exponent.
pub(super) fn number(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".into() } else { s.to_string() }
}

pub(super) fn num_obj(v: f64) -> Obj {
    Obj::Num(number(v).into_bytes())
}
