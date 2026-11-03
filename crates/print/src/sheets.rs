//! The sheets of a print-ready PDF, read leniently, and the incremental update that gives them
//! one paper shape. Windows prints a job with one PrintTicket, so with one paper size and
//! orientation, and draws each page at the size the PDF gives it. A job whose sheets differ is
//! brought to one shape here, by appending new versions of the pages that need it: `/Rotate`
//! turns a sheet of the other orientation a quarter turn, and for shrink-to-fit a page gets the
//! paper's size with its content scaled down (never up) and centred. The original bytes are
//! kept as they are, so a job that needs nothing is the caller's PDF unchanged.
//!
//! Only what that takes is read: the cross-reference sections (tables and streams, object
//! streams, FlateDecode with PNG predictors), the page tree and each page's boxes and rotation.
//! Every length, count and depth is bounded, and anything unreadable is an [`Error`], never a
//! panic: the caller's PDF is input like any other.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::Error;

/// Nesting of arrays and dictionaries, and of the page tree.
const MAX_DEPTH: usize = 64;
const MAX_PAGES: usize = 100_000;
/// The most a cross-reference or object stream may inflate to.
const MAX_STREAM: usize = 16 << 20;
// A short compressed file can otherwise expand into millions of hash-map entries.
// These are job-wide budgets, including older xref sections and cached object streams.
const MAX_RECORDS: usize = 500_000;
const MAX_DECODED: usize = 32 << 20;
const MAX_NODES: usize = 250_000;
const MAX_TOKENS: usize = 16 << 20;
const MAX_OBJECT_NODES: usize = 32_000;
const MAX_OBJECT_TOKENS: usize = 1 << 20;
const MAX_SECTIONS: usize = 256;
/// The most the page objects of repeated copies ([`Repeat`]) may take, in bytes.
const MAX_REPEATED: usize = 64 << 20;
/// A page without a MediaBox (invalid, but readers accept it) is US Letter.
const DEFAULT_MEDIA: [f64; 4] = [0.0, 0.0, 612.0, 792.0];
/// Sheet sizes this close, in points (0.18 mm), are the same size.
pub(crate) const SAME: f64 = 0.5;

fn unreadable(detail: &str) -> Error {
    Error::Spool(format!("the print-ready PDF could not be read for printing: {detail}"))
}

#[derive(Clone, Debug, PartialEq)]
enum Obj {
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

type Dict = Vec<(Vec<u8>, Obj)>;

fn get<'d>(dict: &'d Dict, key: &[u8]) -> Option<&'d Obj> {
    dict.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

impl Obj {
    fn num(&self) -> Option<f64> {
        match self {
            Obj::Num(raw) => std::str::from_utf8(raw).ok()?.parse::<f64>().ok().filter(|v| v.is_finite()),
            _ => None,
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Obj::Num(raw) => std::str::from_utf8(raw).ok()?.parse::<i64>().ok(),
            _ => None,
        }
    }
    fn name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }
    fn array(&self) -> Option<&[Obj]> {
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

struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
    nodes: usize,
    tokens: usize,
    max_nodes: usize,
    max_tokens: usize,
    exhausted: bool,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a [u8], pos: usize) -> Self {
        Lexer { s, pos, nodes: 0, tokens: 0, max_nodes: MAX_OBJECT_NODES, max_tokens: MAX_OBJECT_TOKENS, exhausted: false }
    }

    fn bounded(s: &'a [u8], pos: usize, budget: &Budget) -> Self {
        let mut lx = Self::new(s, pos);
        lx.max_nodes = MAX_OBJECT_NODES.min(MAX_NODES.saturating_sub(budget.nodes));
        lx.max_tokens = MAX_OBJECT_TOKENS.min(MAX_TOKENS.saturating_sub(budget.tokens));
        lx
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn rest(&self) -> &'a [u8] {
        self.s.get(self.pos..).unwrap_or(&[])
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(b) = self.peek() {
                    if b == b'\n' || b == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// A run of regular characters (a number, keyword or name body).
    fn word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        self.s.get(start..self.pos).unwrap_or(&[])
    }

    /// The keyword `kw` as a whole word, consumed only when it is there.
    fn keyword(&mut self, kw: &[u8]) -> bool {
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
    fn unsigned(&mut self) -> Option<u64> {
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

    fn object(&mut self, depth: usize) -> Option<Obj> {
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

    fn name(&mut self) -> Option<Vec<u8>> {
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

    fn token(&mut self, start: usize, end: usize) -> Option<Vec<u8>> {
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
            let after = self.pos;
            if let (Some(num), Some(generation)) = (std::str::from_utf8(word).ok().and_then(|w| w.parse::<u32>().ok()), self.unsigned())
                && let Ok(generation) = u16::try_from(generation)
                && self.keyword(b"R")
            {
                return Some(Obj::Ref(num, generation));
            }
            self.pos = after;
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
fn write_obj(out: &mut Vec<u8>, obj: &Obj) {
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

fn write_dict(out: &mut Vec<u8>, dict: &Dict) {
    out.extend_from_slice(b"<<");
    for (key, value) in dict {
        write_name(out, key);
        out.push(b' ');
        write_obj(out, value);
    }
    out.extend_from_slice(b">>");
}

/// A number for a content stream or a box: at most four decimals, no exponent.
fn number(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".into() } else { s.to_string() }
}

fn num_obj(v: f64) -> Obj {
    Obj::Num(number(v).into_bytes())
}

/// Where an object is: at an offset, or the `index`th object of an object stream.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Free,
    At(usize),
    In(u32, u32),
}

/// An object stream, inflated: each object's number and offset within `data`.
struct ObjStm {
    data: Vec<u8>,
    objects: Vec<(u32, usize)>,
}

#[derive(Default)]
struct Budget {
    records: usize,
    decoded: usize,
    nodes: usize,
    tokens: usize,
    exhausted: bool,
}

impl Budget {
    fn charge(&mut self, records: usize, decoded: usize, nodes: usize, tokens: usize) -> Option<()> {
        if self.exhausted {
            return None;
        }
        let fits = self.records.checked_add(records).is_some_and(|n| n <= MAX_RECORDS)
            && self.decoded.checked_add(decoded).is_some_and(|n| n <= MAX_DECODED)
            && self.nodes.checked_add(nodes).is_some_and(|n| n <= MAX_NODES)
            && self.tokens.checked_add(tokens).is_some_and(|n| n <= MAX_TOKENS);
        if !fits {
            self.exhausted = true;
            return None;
        }
        self.records += records;
        self.decoded += decoded;
        self.nodes += nodes;
        self.tokens += tokens;
        Some(())
    }

    fn parsed(&mut self, lx: &Lexer<'_>) -> Option<()> {
        if lx.exhausted {
            self.exhausted = true;
            return None;
        }
        self.charge(0, 0, lx.nodes, lx.tokens)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    entries: HashMap<u32, Entry>,
    objstms: HashMap<u32, Option<ObjStm>>,
    budget: Budget,
}

/// An indirect object read at an offset: its number, value and stream data.
struct Indirect<'a> {
    num: u32,
    obj: Obj,
    stream: Option<&'a [u8]>,
}

fn indirect<'a>(data: &'a [u8], offset: usize, budget: &mut Budget) -> Option<Indirect<'a>> {
    let mut lx = Lexer::bounded(data, offset, budget);
    let num = u32::try_from(lx.unsigned()?).ok()?;
    lx.unsigned()?;
    if !lx.keyword(b"obj") {
        return None;
    }
    let obj = lx.object(0);
    budget.parsed(&lx)?;
    let obj = obj?;
    let mut stream = None;
    if let Obj::Dict(dict) = &obj
        && lx.keyword(b"stream")
    {
        // The data starts after the end of line following `stream` (CR LF or LF; a lone CR is
        // tolerated).
        match lx.rest() {
            [b'\r', b'\n', ..] => lx.pos += 2,
            [b'\n' | b'\r', ..] => lx.pos += 1,
            _ => {}
        }
        let start = lx.pos;
        let by_length = get(dict, b"Length")
            .and_then(Obj::int)
            .and_then(|l| usize::try_from(l).ok())
            .and_then(|l| start.checked_add(l))
            .filter(|&end| Lexer::new(data, end).keyword(b"endstream"));
        let end = match by_length {
            Some(end) => end,
            // An indirect or wrong /Length: the data runs to `endstream`, less its end of line.
            None => {
                let rest = data.get(start..)?;
                let at = rest.windows(9).position(|w| w == b"endstream")?;
                let mut end = start + at;
                if data.get(end.wrapping_sub(1)) == Some(&b'\n') {
                    end -= 1;
                }
                if data.get(end.wrapping_sub(1)) == Some(&b'\r') {
                    end -= 1;
                }
                end.max(start)
            }
        };
        stream = data.get(start..end);
    }
    Some(Indirect { num, obj, stream })
}

/// How an inflate ended.
enum Inflated {
    /// The whole stream, within the limit.
    Done(Vec<u8>),
    /// The output would pass the limit.
    Exhausted,
    /// A damaged or truncated stream: only the bytes actually written before the damage.
    Damaged(Vec<u8>),
}

/// Inflate `raw` (zlib-wrapped or bare deflate) to at most `limit` bytes. Unlike the
/// `decompress_to_vec*` helpers it reports the bytes the decompressor really wrote, so a stream
/// rejected early yields an empty `Damaged`, never the zero-filled output buffer.
fn inflate(raw: &[u8], zlib: bool, limit: usize) -> Inflated {
    use miniz_oxide::inflate::TINFLStatus;
    use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF | if zlib { inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER } else { 0 };
    let mut out = vec![0u8; raw.len().saturating_mul(2).max(64).min(limit)];
    let mut state = Box::<DecompressorOxide>::default();
    let (mut input, mut written) = (raw, 0usize);
    loop {
        let (status, consumed, wrote) = decompress(&mut state, input, &mut out, written, flags);
        written = written.saturating_add(wrote).min(out.len());
        match status {
            TINFLStatus::Done => {
                out.truncate(written);
                return Inflated::Done(out);
            }
            TINFLStatus::HasMoreOutput => {
                if out.len() >= limit {
                    return Inflated::Exhausted;
                }
                let Some(rest) = input.get(consumed..) else {
                    out.truncate(written);
                    return Inflated::Damaged(out);
                };
                input = rest;
                out.resize(out.len().saturating_mul(2).min(limit), 0);
            }
            _ => {
                out.truncate(written);
                return Inflated::Damaged(out);
            }
        }
    }
}

/// A stream's data with its filters undone: FlateDecode (with PNG predictors), or none.
fn decode(dict: &Dict, raw: &[u8], budget: &mut Budget) -> Option<Vec<u8>> {
    if budget.exhausted {
        return None;
    }
    let limit = MAX_STREAM.min(MAX_DECODED.saturating_sub(budget.decoded));
    if limit == 0 {
        budget.exhausted = true;
        return None;
    }
    let filters: Vec<&[u8]> = match get(dict, b"Filter") {
        None => Vec::new(),
        Some(Obj::Name(n)) => vec![n.as_slice()],
        Some(Obj::Array(a)) => a.iter().map(Obj::name).collect::<Option<_>>()?,
        Some(_) => return None,
    };
    let parms = match get(dict, b"DecodeParms") {
        Some(Obj::Dict(d)) => Some(d),
        Some(Obj::Array(a)) => a.first().and_then(Obj::dict),
        _ => None,
    };
    let data = match filters.as_slice() {
        [] => {
            if raw.len() > limit {
                budget.exhausted = true;
                return None;
            }
            Some(raw.to_vec())
        }
        [b"FlateDecode" | b"Fl"] => {
            let inflated = match inflate(raw, true, limit) {
                Inflated::Done(v) => v,
                Inflated::Exhausted => {
                    budget.exhausted = true;
                    return None;
                }
                // A damaged stream: what inflated before the damage, as other readers do.
                Inflated::Damaged(v) if !v.is_empty() => v,
                // Rejected before any output: retry as a bare deflate stream.
                Inflated::Damaged(_) => match inflate(raw, false, limit) {
                    Inflated::Done(v) => v,
                    Inflated::Exhausted => {
                        budget.exhausted = true;
                        return None;
                    }
                    Inflated::Damaged(_) => return None,
                },
            };
            unpredict(&inflated, parms, limit)
        }
        _ => None,
    }?;
    budget.charge(0, data.len(), 0, 0)?;
    Some(data)
}

/// Undo a PNG predictor (`/Predictor` 10–15), which cross-reference streams use.
fn unpredict(data: &[u8], parms: Option<&Dict>, limit: usize) -> Option<Vec<u8>> {
    let int = |key: &[u8], default: i64| parms.and_then(|p| get(p, key)).and_then(Obj::int).unwrap_or(default);
    let predictor = int(b"Predictor", 1);
    if predictor < 10 {
        return (predictor == 1).then(|| data.to_vec());
    }
    let colors = usize::try_from(int(b"Colors", 1)).ok().filter(|c| (1..=32).contains(c))?;
    let bits = usize::try_from(int(b"BitsPerComponent", 8)).ok().filter(|b| [1, 2, 4, 8, 16].contains(b))?;
    let columns = usize::try_from(int(b"Columns", 1)).ok().filter(|c| (1..=1 << 20).contains(c))?;
    let bpp = (colors * bits).div_ceil(8).max(1);
    let row = (columns * colors * bits).div_ceil(8);
    if row > limit || !data.len().is_multiple_of(row + 1) || data.len() / (row + 1) * row > limit {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (&kind, line) = chunk.split_first()?;
        let mut cur = line.to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let a = if i >= bpp { cur.get(i - bpp).copied().unwrap_or(0) } else { 0 };
            let b = prev.get(i).copied().unwrap_or(0);
            let c = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) } else { 0 };
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return None,
            };
            if let Some(v) = cur.get_mut(i) {
                *v = v.wrapping_add(add);
            }
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Some(out)
}

/// One cross-reference section and its trailer (for a stream, its dictionary).
fn xref_section(data: &[u8], offset: usize, entries: &mut HashMap<u32, Entry>, budget: &mut Budget) -> Option<(Dict, bool)> {
    let mut lx = Lexer::bounded(data, offset, budget);
    if !lx.keyword(b"xref") {
        let x = indirect(data, offset, budget)?;
        let dict = x.obj.dict()?.clone();
        xref_stream(&dict, x.stream?, entries, budget)?;
        return Some((dict, true));
    }
    let mut table = Vec::new();
    let trailer = loop {
        if lx.keyword(b"trailer") {
            let trailer = lx.object(0);
            budget.parsed(&lx)?;
            break trailer?.dict()?.clone();
        }
        let start = u32::try_from(lx.unsigned()?).ok()?;
        let count = u32::try_from(lx.unsigned()?).ok()?;
        budget.charge(usize::try_from(count).ok()?, 0, 0, 0)?;
        start.checked_add(count.saturating_sub(1))?;
        for i in 0..count {
            let offset = usize::try_from(lx.unsigned()?).ok()?;
            lx.unsigned()?;
            lx.skip_ws();
            let kind = lx.word();
            let Some(num) = start.checked_add(i) else { break };
            table.push((num, if kind == b"n" { Entry::At(offset) } else { Entry::Free }));
        }
    };
    // A hybrid file's stream lists the objects its table leaves out (ISO 32000-1, 7.5.8.4).
    if let Some(at) = get(&trailer, b"XRefStm").and_then(Obj::int).and_then(|v| usize::try_from(v).ok())
        && let Some(x) = indirect(data, at, budget)
        && let (Some(dict), Some(stream)) = (x.obj.dict(), x.stream)
    {
        xref_stream(dict, stream, entries, budget)?;
    }
    for (num, entry) in table {
        entries.entry(num).or_insert(entry);
    }
    Some((trailer, false))
}

fn xref_stream(dict: &Dict, raw: &[u8], entries: &mut HashMap<u32, Entry>, budget: &mut Budget) -> Option<()> {
    if get(dict, b"Type").and_then(Obj::name) != Some(b"XRef") {
        return None;
    }

    let widths: Vec<usize> =
        get(dict, b"W")?.array()?.iter().map(|w| w.int().and_then(|w| usize::try_from(w).ok()).filter(|w| *w <= 8)).collect::<Option<_>>()?;
    let [w0, w1, w2] = widths.as_slice() else { return None };
    let entry_len = w0 + w1 + w2;
    if entry_len == 0 {
        return None;
    }
    let size = get(dict, b"Size").and_then(Obj::int).unwrap_or(0);
    let index: Vec<i64> = match get(dict, b"Index").and_then(Obj::array) {
        Some(a) => a.iter().map(Obj::int).collect::<Option<_>>()?,
        None => vec![0, size],
    };
    if !index.len().is_multiple_of(2) {
        return None;
    }
    let mut count = 0usize;
    for &[start, n] in index.as_chunks::<2>().0 {
        let (start, n) = (u32::try_from(start).ok()?, u32::try_from(n).ok()?);
        start.checked_add(n.saturating_sub(1))?;
        count = count.checked_add(usize::try_from(n).ok()?)?;
    }
    // Check counts before inflating or expanding the xref map, including duplicate records.
    budget.charge(count, 0, 0, 0)?;
    let data = decode(dict, raw, budget)?;
    let field = |bytes: &[u8]| bytes.iter().fold(0u64, |v, &b| v.wrapping_shl(8) | u64::from(b));
    let mut records = data.chunks_exact(entry_len);
    for &[start, count] in index.as_chunks::<2>().0 {
        let (Ok(start), Ok(count)) = (u32::try_from(start), u32::try_from(count)) else { return None };
        for i in 0..count {
            let Some(record) = records.next() else { return Some(()) };
            let (kind, rest) = record.split_at_checked(*w0)?;
            let (second, third) = rest.split_at_checked(*w1)?;
            let kind = if *w0 == 0 { 1 } else { field(kind) };
            let (second, third) = (field(second), field(third));
            let entry = match kind {
                0 => Entry::Free,
                1 => Entry::At(usize::try_from(second).ok()?),
                2 => Entry::In(u32::try_from(second).ok()?, u32::try_from(third).ok()?),
                _ => continue,
            };
            let Some(num) = start.checked_add(i) else { break };
            entries.entry(num).or_insert(entry);
        }
    }
    Some(())
}

/// The offset `startxref` names.
fn startxref(data: &[u8]) -> Option<usize> {
    let at = data.windows(9).rposition(|w| w == b"startxref")?;
    usize::try_from(Lexer::new(data, at + 9).unsigned()?).ok()
}

impl Reader<'_> {
    fn object(&mut self, num: u32) -> Option<Obj> {
        // Geometry fallback must not resume parsing after any job-wide budget is spent.
        if self.budget.exhausted {
            return None;
        }
        match *self.entries.get(&num)? {
            Entry::Free => None,
            Entry::At(offset) => indirect(self.data, offset, &mut self.budget).filter(|x| x.num == num).map(|x| x.obj),
            Entry::In(stm, index) => {
                if !self.objstms.contains_key(&stm) {
                    let loaded = self.load_objstm(stm);
                    self.objstms.insert(stm, loaded);
                }
                let objstm = self.objstms.get(&stm)?.as_ref()?;
                let &(found, offset) = objstm.objects.get(usize::try_from(index).ok()?)?;
                if found != num {
                    return None;
                }
                let mut lx = Lexer::bounded(&objstm.data, offset, &self.budget);
                let obj = lx.object(0);
                self.budget.parsed(&lx)?;
                obj
            }
        }
    }

    fn load_objstm(&mut self, stm: u32) -> Option<ObjStm> {
        let Entry::At(offset) = *self.entries.get(&stm)? else { return None };
        let x = indirect(self.data, offset, &mut self.budget).filter(|x| x.num == stm)?;
        let dict = x.obj.dict()?;
        if get(dict, b"Type").and_then(Obj::name) != Some(b"ObjStm") {
            return None;
        }
        let n = get(dict, b"N").and_then(Obj::int).and_then(|n| usize::try_from(n).ok())?;
        self.budget.charge(n, 0, 0, 0)?;
        let data = decode(dict, x.stream?, &mut self.budget)?;
        let first = get(dict, b"First").and_then(Obj::int).and_then(|f| usize::try_from(f).ok())?;
        let mut lx = Lexer::new(&data, 0);
        let mut objects = Vec::new();
        for _ in 0..n {
            let (Some(num), Some(at)) = (lx.unsigned(), lx.unsigned()) else { break };
            let (Ok(num), Some(at)) = (u32::try_from(num), usize::try_from(at).ok().and_then(|a| a.checked_add(first))) else { break };
            objects.push((num, at));
        }
        Some(ObjStm { data, objects })
    }

    /// The object a value names, when it is a reference (a few levels at most).
    fn resolve(&mut self, obj: &Obj) -> Option<Obj> {
        let mut current = obj.clone();
        for _ in 0..8 {
            match current {
                Obj::Ref(num, _) => current = self.object(num)?,
                other => return Some(other),
            }
        }
        None
    }

    /// Like [`resolve`](Self::resolve), but a direct object is borrowed, not copied. Inherited
    /// page attributes are read once per page, so copying a large inline value would repeat that
    /// work for every page outside the budget.
    fn resolve_borrowed<'o>(&mut self, obj: &'o Obj) -> Option<Cow<'o, Obj>> {
        match obj {
            Obj::Ref(..) => self.resolve(obj).map(Cow::Owned),
            direct => Some(Cow::Borrowed(direct)),
        }
    }

    fn rect(&mut self, obj: Option<&Obj>) -> Option<[f64; 4]> {
        let obj = self.resolve_borrowed(obj?)?;
        let items = obj.array()?;
        // The shape is checked before reading any element.
        if items.len() != 4 {
            return None;
        }
        let mut v = [0.0; 4];
        for (slot, item) in v.iter_mut().zip(items) {
            *slot = self.resolve_borrowed(item)?.num()?;
        }
        let [x0, y0, x1, y1] = v;
        Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
    }
}

/// One sheet of the print-ready PDF.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sheet {
    /// The page object (number, generation); `None` for a page written inside its parent.
    obj: Option<(u32, u16)>,
    dict: Dict,
    /// The MediaBox, inherited or not.
    media: [f64; 4],
    /// The visible box (CropBox within MediaBox), in the page's own unrotated space.
    pub(crate) bounds: [f64; 4],
    /// `/Resources` inherited from the page tree; `None` when the page has its own or none.
    resources: Option<Arc<Obj>>,
    /// `/Rotate`, 0, 90, 180 or 270.
    pub(crate) rotate: u32,
    /// Annotation flags match the renderer's visible printing behavior.
    print_annotations_safe: bool,
    /// `/Annots` lists at least one annotation.
    annotated: bool,
}

impl Sheet {
    /// Width and height as the sheet is shown and printed (rotation applied), in points.
    pub(crate) fn size(&self) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bounds;
        if self.rotate % 180 == 90 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) }
    }
}

/// A print-ready PDF's sheets, and what an update to it needs.
pub(crate) struct Sheets<'a> {
    data: &'a [u8],
    pub(crate) sheets: Vec<Sheet>,
    trailer: Dict,
    catalog: Dict,
    /// The newest cross-reference section is a stream (so is the update's).
    xref_stream: bool,
    startxref: usize,
    /// The first unused object number.
    size: u32,
}

/// What [`read`] inherits down the page tree.
#[derive(Clone, Default)]
struct Inherited {
    media: Option<Arc<Obj>>,
    crop: Option<Arc<Obj>>,
    rotate: Option<Arc<Obj>>,
    resources: Option<Arc<Obj>>,
}

/// Read the sheets of a print-ready PDF.
pub(crate) fn read(data: &[u8]) -> Result<Sheets<'_>, Error> {
    let start = startxref(data).ok_or_else(|| unreadable("no cross-reference table"))?;
    let mut entries = HashMap::new();
    let mut budget = Budget::default();
    let mut trailer = None;
    let mut xref_stream = false;
    let mut seen = HashSet::new();
    let mut next = Some(start);
    while let Some(offset) = next {
        if !seen.insert(offset) || seen.len() > MAX_SECTIONS {
            break;
        }
        let Some((section, is_stream)) = xref_section(data, offset, &mut entries, &mut budget) else {
            // An older section that can't be read only loses objects the newer ones replaced.
            if budget.exhausted || trailer.is_none() {
                return Err(unreadable("the cross-reference table is damaged"));
            }
            break;
        };
        next = get(&section, b"Prev").and_then(Obj::int).and_then(|v| usize::try_from(v).ok());
        if trailer.is_none() {
            xref_stream = is_stream;
            trailer = Some(section);
        }
    }
    let trailer = trailer.ok_or_else(|| unreadable("no trailer"))?;
    let size = entries
        .keys()
        .max()
        .map_or(0, |m| m.saturating_add(1))
        .max(get(&trailer, b"Size").and_then(Obj::int).and_then(|s| u32::try_from(s).ok()).unwrap_or(0));
    let mut reader = Reader { data, entries, objstms: HashMap::new(), budget };
    let root = get(&trailer, b"Root").cloned().ok_or_else(|| unreadable("no document catalog"))?;
    let catalog = reader.resolve(&root).and_then(|c| c.dict().cloned()).ok_or_else(|| unreadable("no document catalog"))?;
    let pages_root = get(&catalog, b"Pages").cloned().ok_or_else(|| unreadable("no page tree"))?;

    let mut sheets = Vec::new();
    let mut visited = HashSet::new();
    // Depth-first in page order: kids are pushed last to first.
    let mut stack = vec![(pages_root, Inherited::default(), 0usize)];
    while let Some((node, inherited, depth)) = stack.pop() {
        if depth > MAX_DEPTH || sheets.len() >= MAX_PAGES {
            return Err(unreadable("the page tree is too deep or too large"));
        }
        let obj = match &node {
            Obj::Ref(num, generation) => {
                if !visited.insert(*num) {
                    return Err(unreadable("the page tree loops"));
                }
                Some((*num, *generation))
            }
            _ => None,
        };
        let mut dict = reader.resolve(&node).and_then(|d| d.dict().cloned()).ok_or_else(|| unreadable("a page is missing"))?;
        // A null value is the same as no entry (ISO 32000-1, 7.3.9), so it inherits.
        let own = |key: &[u8]| get(&dict, key).filter(|v| **v != Obj::Null).cloned().map(Arc::new);
        let parent_resources = inherited.resources.clone();
        let here = Inherited {
            media: own(b"MediaBox").or(inherited.media),
            crop: own(b"CropBox").or(inherited.crop),
            rotate: own(b"Rotate").or(inherited.rotate),
            resources: own(b"Resources").or(inherited.resources),
        };
        let kids = get(&dict, b"Kids").cloned().and_then(|k| reader.resolve(&k));
        let is_node = get(&dict, b"Type").and_then(Obj::name) == Some(b"Pages") || (get(&dict, b"Type").is_none() && kids.is_some());
        if is_node {
            let kids = kids.and_then(|k| k.array().map(<[Obj]>::to_vec)).unwrap_or_default();
            if stack.len().checked_add(kids.len()).is_none_or(|n| n > MAX_PAGES) {
                return Err(unreadable("the page tree is too large"));
            }
            stack.extend(kids.into_iter().rev().map(|kid| (kid, here.clone(), depth + 1)));
            continue;
        }
        // Windows.Data.Pdf ignores nondefault UserUnit. Refuse it before submitting a
        // job rather than silently printing the wrong physical dimensions.
        if let Some(unit) = get(&dict, b"UserUnit") {
            let unit = reader.resolve(unit).and_then(|u| u.num());
            if unit != Some(1.0) {
                return Err(unreadable("nondefault or invalid UserUnit is not supported"));
            }
        }
        if let Some(contents) = get(&dict, b"Contents").cloned() {
            let resolved = reader.resolve(&contents).ok_or_else(|| unreadable("page content is missing"))?;
            // An indirect array can live in an object stream. Keep only its stream
            // references in the wrapper; an array reference there renders blank.
            if let Obj::Array(items) = resolved {
                if !items.iter().all(|item| matches!(item, Obj::Ref(..))) {
                    return Err(unreadable("a content array must contain stream references"));
                }
                dict.retain(|(key, _)| key != b"Contents");
                dict.push((b"Contents".to_vec(), Obj::Array(items)));
            } else if resolved == Obj::Null {
                dict.retain(|(key, _)| key != b"Contents");
            } else if !matches!(contents, Obj::Ref(..)) || !matches!(resolved, Obj::Dict(_)) {
                return Err(unreadable("page content must be a stream or an array of streams"));
            }
        }
        let media = reader.rect(here.media.as_deref()).filter(|r| r[2] > r[0] && r[3] > r[1]).unwrap_or(DEFAULT_MEDIA);
        let bounds = reader
            .rect(here.crop.as_deref())
            .map(|c| [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])])
            .filter(|c| c[2] > c[0] && c[3] > c[1])
            .unwrap_or(media);
        let rotate = here.rotate.as_ref().and_then(|r| reader.resolve_borrowed(r).and_then(|r| r.int())).unwrap_or(0).rem_euclid(360);
        // Not a multiple of 90 is invalid; such a page is shown unrotated.
        let rotate = if rotate % 90 == 0 { u32::try_from(rotate).unwrap_or(0) } else { 0 };
        // Windows renders viewer visibility, not annotation print visibility. Only a plain
        // visible printable annotation is safe; flatten other flags before native submission.
        let (print_annotations_safe, annotated) = match get(&dict, b"Annots") {
            None | Some(Obj::Null) => (true, false),
            Some(annotations) => match reader.resolve(annotations) {
                Some(Obj::Null) => (true, false),
                Some(Obj::Array(items)) => (
                    items.iter().all(|item| {
                        reader
                            .resolve(item)
                            .and_then(|a| a.dict().cloned())
                            .is_some_and(|annotation| get(&annotation, b"F").and_then(|f| reader.resolve(f)).and_then(|f| f.int()) == Some(4))
                    }),
                    !items.is_empty(),
                ),
                _ => (false, true),
            },
        };
        // A reference to a missing object is null too. Only the cross-reference table is
        // looked at: resolving a shared Resources dictionary for every page would parse it again.
        let own_resources = match get(&dict, b"Resources") {
            None | Some(Obj::Null) => false,
            Some(Obj::Ref(num, _)) => matches!(reader.entries.get(num), Some(Entry::At(_) | Entry::In(..))),
            Some(_) => true,
        };
        let resources = if own_resources { None } else { parent_resources };
        sheets.push(Sheet { obj, dict, media, bounds, resources, rotate, print_annotations_safe, annotated });
    }
    if reader.budget.exhausted {
        return Err(unreadable("the parser memory budget was exceeded"));
    }
    if sheets.is_empty() {
        return Err(Error::Spool("the print-ready PDF has no pages".into()));
    }
    Ok(Sheets { data, sheets, trailer, catalog, xref_stream, startxref: start, size })
}

/// How sheets of another size than the paper are printed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    /// Every sheet must be the paper's size (either way round).
    Exact,
    /// Sheets are shrunk (never enlarged) to fit inside `margin` points of the paper's edges, and
    /// centred on it.
    Shrink { margin: f64 },
}

/// A page's new version: its rotation, and for a page brought to the paper's size, the new
/// geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edit {
    pub(crate) sheet: usize,
    pub(crate) rotate: u32,
    pub(crate) frame: Option<Frame>,
}

/// A page reframed onto the paper: its new MediaBox (unrotated space), the matrix placing the
/// old content on it, and the old visible box, which stays the clip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    pub(crate) media: (f64, f64),
    pub(crate) matrix: [f64; 6],
    pub(crate) clip: [f64; 4],
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() <= SAME && (a.1 - b.1).abs() <= SAME
}

/// The edits that put every sheet on `paper` (width and height as printed, in points; its
/// orientation is the job's). A sheet of the other orientation turns a quarter: a landscape sheet
/// in a portrait job anticlockwise, as drivers put a landscape job on the paper, a portrait sheet
/// in a landscape job clockwise. `Exact` refuses a sheet of another size; `Shrink` reframes it.
pub(crate) fn plan(sheets: &[Sheet], paper: (f64, f64), fit: Fit) -> Result<Vec<Edit>, Error> {
    let (pw, ph) = paper;
    if !(pw.is_finite() && ph.is_finite() && pw > 0.0 && ph > 0.0) {
        return Err(Error::Spool("the printer reported a paper size that is not usable".into()));
    }
    let mut edits = Vec::new();
    for (i, sheet) in sheets.iter().enumerate() {
        let (w, h) = sheet.size();
        let turn = if pw > ph + SAME && h > w + SAME {
            90
        } else if ph > pw + SAME && w > h + SAME {
            270
        } else {
            0
        };
        let rotate = (sheet.rotate + turn) % 360;
        let (w, h) = if turn == 0 { (w, h) } else { (h, w) };
        let frame = match fit {
            Fit::Exact if close((w, h), paper) => None,
            Fit::Exact => {
                return Err(Error::Spool(format!(
                    "Windows prints one paper size per job: sheet {} is {:.2} × {:.2} in, the job's paper {:.2} × {:.2} in; print the sheets of each size separately",
                    i + 1,
                    w / 72.0,
                    h / 72.0,
                    pw / 72.0,
                    ph / 72.0
                )));
            }
            Fit::Shrink { margin } => {
                let margin = if margin.is_finite() && margin > 0.0 && 2.0 * margin < pw.min(ph) { margin } else { 0.0 };
                let scale = ((pw - 2.0 * margin) / w).min((ph - 2.0 * margin) / h).min(1.0);
                if scale >= 1.0 && close((w, h), paper) {
                    None
                } else {
                    let [x0, y0, x1, y1] = sheet.bounds;
                    let (cw, ch) = (x1 - x0, y1 - y0);
                    // The paper in the page's unrotated space.
                    let (uw, uh) = if rotate % 180 == 90 { (ph, pw) } else { (pw, ph) };
                    let tx = (uw - scale * cw) / 2.0 - scale * x0;
                    let ty = (uh - scale * ch) / 2.0 - scale * y0;
                    Some(Frame { media: (uw, uh), matrix: [scale, 0.0, 0.0, scale, tx, ty], clip: [x0, y0, cw, ch] })
                }
            }
        };
        if turn != 0 || frame.is_some() {
            edits.push(Edit { sheet: i, rotate, frame });
        }
    }
    Ok(edits)
}

/// The whole document printed `copies` times in one job, for collated copies the driver can't
/// make: each copy's pages follow the previous copy's. On two-sided paper, a document with an odd
/// number of sheets gets a blank page after every copy but the last, so each copy starts on a
/// sheet of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Repeat {
    pub(crate) copies: u32,
    pub(crate) two_sided: bool,
}

fn has_annotations(dict: &Dict) -> bool {
    get(dict, b"Annots").is_some_and(|a| !matches!(a, Obj::Null) && !matches!(a, Obj::Array(items) if items.is_empty()))
}

fn rect_obj(r: [f64; 4]) -> Obj {
    Obj::Array(r.into_iter().map(num_obj).collect())
}

/// Starts object `num` in an update, recording where it is.
fn new_object(out: &mut Vec<u8>, num: u32, generation: u16, written: &mut Vec<(u32, u16, usize)>) {
    written.push((num, generation, out.len()));
    out.extend_from_slice(format!("{num} {generation} obj\n").as_bytes());
}

/// Boxes a reframed page drops: they were in the old geometry.
const OLD_GEOMETRY: [&[u8]; 6] = [b"MediaBox", b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox", b"Rotate"];

impl Sheets<'_> {
    /// Refuse annotation print/view visibility the Windows renderer cannot preserve.
    pub(crate) fn check_print_annotations(&self) -> Result<(), Error> {
        if self.sheets.iter().all(|s| s.print_annotations_safe) {
            Ok(())
        } else {
            Err(Error::Spool("flatten annotation print visibility into page content before native Windows printing; nothing was sent".into()))
        }
    }

    /// The PDF with `edits` appended as an incremental update (ISO 32000-1, 7.5.6): the
    /// original bytes, then the new versions of the edited pages, the content streams a
    /// reframed page wraps its own in, and a cross-reference section of the same kind as the
    /// newest one, chained to it with `/Prev`.
    ///
    pub(crate) fn update(&self, edits: &[Edit]) -> Result<Vec<u8>, Error> {
        // Only a repeat can be turned down, so there is always a PDF.
        Ok(self.write(edits, None)?.unwrap_or_else(|| self.data.to_vec()))
    }

    /// [`update`](Self::update) with every copy of `repeat` in the one PDF. The edited pages
    /// are not written in place: the update adds a page tree of its own, every copy's pages as
    /// new page objects that share the original content and resources, and a new version of
    /// the catalog pointing to it. Each new page carries the boxes, rotation and resources it
    /// inherited, since its parent is the new tree. `None` when the copies can't be one PDF:
    /// the PDF is encrypted (strings are encrypted with keys made from their object's number),
    /// a page has annotations (an annotation belongs to one page; whether a renderer draws it on
    /// others is unknown), or the job would be too large.
    pub(crate) fn repeated(&self, edits: &[Edit], repeat: Repeat) -> Result<Option<Vec<u8>>, Error> {
        self.write(edits, Some(repeat))
    }

    fn write(&self, edits: &[Edit], repeat: Option<Repeat>) -> Result<Option<Vec<u8>>, Error> {
        let refuse = |why: &str| Error::Spool(format!("the print-ready PDF can't be fitted to the paper: {why}"));
        let mut out = self.data.to_vec();
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        let mut next = self.size;
        // Each new object's number, generation and offset.
        let mut written: Vec<(u32, u16, usize)> = Vec::new();
        let repeat = repeat.filter(|r| r.copies > 1);
        let mut edited = HashMap::new();
        for edit in edits {
            let sheet = self.sheets.get(edit.sheet).ok_or_else(|| refuse("no such sheet"))?;
            let mut dict = sheet.dict.clone();
            dict.retain(|(k, _)| k != b"Rotate");
            if let Some(frame) = &edit.frame {
                if get(&self.trailer, b"Encrypt").is_some() {
                    return Err(refuse("it is encrypted"));
                }
                // Annotation appearances live outside /Contents. Transforming only content
                // would leave them at the old coordinates; deleting them loses printed marks.
                // Require callers to flatten before fitting until appearances can be reframed.
                if has_annotations(&dict) {
                    return Err(refuse("flatten page annotations before using shrink-to-fit"));
                }
                let [a, b, c, d, e, f] = frame.matrix;
                let [x, y, w, h] = frame.clip;
                let prefix = format!(
                    "q {} {} {} {} {} {} cm {} {} {} {} re W n\n",
                    number(a),
                    number(b),
                    number(c),
                    number(d),
                    number(e),
                    number(f),
                    number(x),
                    number(y),
                    number(w),
                    number(h)
                );
                let mut contents = Vec::new();
                for part in [prefix.as_bytes(), b"\nQ\n"] {
                    let stream = next;
                    next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                    new_object(&mut out, stream, 0, &mut written);
                    out.extend_from_slice(format!("<</Length {}>>\nstream\n", part.len()).as_bytes());
                    out.extend_from_slice(part);
                    out.extend_from_slice(b"\nendstream\nendobj\n");
                    contents.push(Obj::Ref(stream, 0));
                }
                // The page's own content, in order, between the two.
                let own = match get(&dict, b"Contents") {
                    Some(Obj::Array(items)) => items.clone(),
                    Some(other) => vec![other.clone()],
                    None => Vec::new(),
                };
                contents.splice(1..1, own);
                dict.retain(|(k, _)| !OLD_GEOMETRY.contains(&k.as_slice()) && k != b"Contents" && k != b"Annots");
                // CropBox too: a parent's would otherwise be inherited, in the old geometry.
                let media = Obj::Array(vec![num_obj(0.0), num_obj(0.0), num_obj(frame.media.0), num_obj(frame.media.1)]);
                dict.push((b"MediaBox".to_vec(), media.clone()));
                dict.push((b"CropBox".to_vec(), media));
                dict.push((b"Contents".to_vec(), Obj::Array(contents)));
            }
            dict.push((b"Rotate".to_vec(), Obj::Num(edit.rotate.to_string().into_bytes())));
            if repeat.is_some() {
                edited.insert(edit.sheet, dict);
            } else {
                let (num, generation) = sheet.obj.ok_or_else(|| refuse("a page is not an object of its own"))?;
                new_object(&mut out, num, generation, &mut written);
                write_dict(&mut out, &dict);
                out.extend_from_slice(b"\nendobj\n");
            }
        }
        if let Some(Repeat { copies, two_sided }) = repeat {
            let refuse = |why: &str| Error::Spool(format!("the copies can't be written: {why}"));
            if get(&self.trailer, b"Encrypt").is_some() || self.sheets.iter().any(|s| s.annotated) {
                return Ok(None);
            }
            let Some(&Obj::Ref(root, root_generation)) = get(&self.trailer, b"Root") else {
                return Ok(None);
            };
            let blank = two_sided && self.sheets.len() % 2 == 1;
            let per_copy = self.sheets.len() + usize::from(blank);
            let count =
                usize::try_from(copies).ok().and_then(|c| per_copy.checked_mul(c)).map(|n| n - usize::from(blank)).filter(|&n| n <= MAX_PAGES);
            let Some(count) = count else { return Ok(None) };
            let tree = next;
            next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
            let parent = (b"Parent".to_vec(), Obj::Ref(tree, 0));
            // Bytes written once, and bytes written once per copy (a page, its xref entry and kid).
            let (mut once, mut each) = (0usize, 0usize);
            let within =
                |once: usize, each: usize| each.checked_mul(copies as usize).and_then(|n| n.checked_add(once)).is_some_and(|n| n <= MAX_REPEATED);
            // Inherited inline resources are written once, as an object of their own, and every
            // page that inherited them refers to it.
            let mut shared: HashMap<*const Obj, Obj> = HashMap::new();
            let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(per_copy);
            let mut last = Dict::new();
            for (i, sheet) in self.sheets.iter().enumerate() {
                let mut dict = edited.remove(&i).unwrap_or_else(|| sheet.dict.clone());
                dict.retain(|(k, _)| k != b"Parent");
                // The boxes as read, not the inherited objects: those can be anything.
                for (key, value) in [(&b"MediaBox"[..], rect_obj(sheet.media)), (b"CropBox", rect_obj(sheet.bounds))] {
                    if get(&dict, key).is_none_or(|v| *v == Obj::Null) {
                        dict.retain(|(k, _)| k != key);
                        dict.push((key.to_vec(), value));
                    }
                }
                if get(&dict, b"Rotate").is_none_or(|v| *v == Obj::Null) {
                    dict.retain(|(k, _)| k != b"Rotate");
                    dict.push((b"Rotate".to_vec(), Obj::Num(sheet.rotate.to_string().into_bytes())));
                }
                if let Some(resources) = &sheet.resources {
                    let value = match &**resources {
                        Obj::Ref(..) => Obj::clone(resources),
                        _ => match shared.get(&Arc::as_ptr(resources)) {
                            Some(r) => r.clone(),
                            None => {
                                let num = next;
                                next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                                let at = out.len();
                                new_object(&mut out, num, 0, &mut written);
                                write_obj(&mut out, resources);
                                out.extend_from_slice(b"\nendobj\n");
                                once += out.len() - at;
                                if !within(once, each) {
                                    return Ok(None);
                                }
                                shared.insert(Arc::as_ptr(resources), Obj::Ref(num, 0));
                                Obj::Ref(num, 0)
                            }
                        },
                    };
                    dict.retain(|(k, _)| k != b"Resources");
                    dict.push((b"Resources".to_vec(), value));
                }
                dict.push(parent.clone());
                let mut body = Vec::new();
                write_dict(&mut body, &dict);
                each += body.len() + 64;
                if !within(once, each) {
                    return Ok(None);
                }
                bodies.push(body);
                last = dict;
            }
            if blank {
                let mut page = vec![(b"Type".to_vec(), Obj::Name(b"Page".to_vec())), parent.clone(), (b"Resources".to_vec(), Obj::Dict(Vec::new()))];
                for key in [&b"MediaBox"[..], b"CropBox", b"Rotate"] {
                    if let Some(value) = get(&last, key) {
                        page.push((key.to_vec(), value.clone()));
                    }
                }
                let mut body = Vec::new();
                write_dict(&mut body, &page);
                bodies.push(body);
            }
            let mut kids = Vec::with_capacity(count);
            for body in bodies.iter().cycle().take(count) {
                let num = next;
                next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
                new_object(&mut out, num, 0, &mut written);
                out.extend_from_slice(body);
                out.extend_from_slice(b"\nendobj\n");
                kids.push(Obj::Ref(num, 0));
            }
            new_object(&mut out, tree, 0, &mut written);
            let node = vec![
                (b"Type".to_vec(), Obj::Name(b"Pages".to_vec())),
                (b"Kids".to_vec(), Obj::Array(kids)),
                (b"Count".to_vec(), Obj::Num(count.to_string().into_bytes())),
            ];
            write_dict(&mut out, &node);
            out.extend_from_slice(b"\nendobj\n");
            let mut catalog = self.catalog.clone();
            // Page labels and the open action name pages of the old tree.
            catalog.retain(|(k, _)| k != b"Pages" && k != b"PageLabels" && k != b"OpenAction");
            catalog.push((b"Pages".to_vec(), Obj::Ref(tree, 0)));
            new_object(&mut out, root, root_generation, &mut written);
            write_dict(&mut out, &catalog);
            out.extend_from_slice(b"\nendobj\n");
        }
        if written.is_empty() {
            return Ok(Some(self.data.to_vec()));
        }
        written.sort_by_key(|(num, _, _)| *num);
        let mut trailer: Dict = Vec::new();
        for key in [&b"Root"[..], b"Info", b"ID", b"Encrypt"] {
            if let Some(v) = get(&self.trailer, key) {
                trailer.push((key.to_vec(), v.clone()));
            }
        }
        trailer.push((b"Prev".to_vec(), Obj::Num(self.startxref.to_string().into_bytes())));
        if self.xref_stream {
            let xref = next;
            next = next.checked_add(1).ok_or_else(|| refuse("too many objects"))?;
            let at = out.len();
            written.push((xref, 0, at));
            let width = (usize::BITS - at.leading_zeros()).div_ceil(8).max(1) as usize;
            let mut data = Vec::new();
            let mut index = Vec::new();
            for (num, generation, offset) in &written {
                index.extend([Obj::Num(num.to_string().into_bytes()), Obj::Num(b"1".to_vec())]);
                data.push(1u8);
                data.extend_from_slice(offset.to_be_bytes().get(std::mem::size_of::<usize>() - width..).unwrap_or(&[]));
                data.extend_from_slice(&generation.to_be_bytes());
            }
            trailer.extend([
                (b"Type".to_vec(), Obj::Name(b"XRef".to_vec())),
                (b"Size".to_vec(), Obj::Num(next.to_string().into_bytes())),
                (b"W".to_vec(), Obj::Array(vec![Obj::Num(b"1".to_vec()), Obj::Num(width.to_string().into_bytes()), Obj::Num(b"2".to_vec())])),
                (b"Index".to_vec(), Obj::Array(index)),
                (b"Length".to_vec(), Obj::Num(data.len().to_string().into_bytes())),
            ]);
            out.extend_from_slice(format!("{xref} 0 obj\n").as_bytes());
            write_dict(&mut out, &trailer);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&data);
            out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
        } else {
            let at = out.len();
            out.extend_from_slice(b"xref\n");
            for (num, generation, offset) in &written {
                out.extend_from_slice(format!("{num} 1\n{offset:010} {generation:05} n\r\n").as_bytes());
            }
            trailer.push((b"Size".to_vec(), Obj::Num(next.to_string().into_bytes())));
            out.extend_from_slice(b"trailer\n");
            write_dict(&mut out, &trailer);
            out.extend_from_slice(format!("\nstartxref\n{at}\n%%EOF\n").as_bytes());
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
#[path = "tests/pdf_bounds.rs"]
mod tests;
