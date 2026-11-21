//! The USD text format (`.usda`): layer metadata, `def` / `over` / `class` prims with metadata
//! (`references`, `active`), attributes (default values, the first time sample,
//! `interpolation`) and relationships. Variant sets, dictionaries and connections are skipped.

use super::{Budget, Layer, Result, SceneError, Specifier, Val};

/// Deepest value / prim nesting accepted.
const MAX_NESTING: usize = 128;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Punct(u8),
    Ident(String),
    Str(String),
    Num(f64),
    Asset(String),
    Path(String),
    Eof,
}

struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
    peeked: Option<Tok>,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.')
}

impl Lexer<'_> {
    fn at(&self, k: usize) -> u8 {
        self.s.get(self.pos + k).copied().unwrap_or(0)
    }

    fn line(&self) -> usize {
        1 + self.s.get(..self.pos).unwrap_or_default().iter().filter(|&&c| c == b'\n').count()
    }

    fn err(&self, msg: impl std::fmt::Display) -> SceneError {
        SceneError::Malformed(format!("line {}: {msg}", self.line()))
    }

    fn skip_space(&mut self) {
        loop {
            match self.at(0) {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b'#' => self.skip_until(b"\n"),
                b'/' if self.at(1) == b'/' => self.skip_until(b"\n"),
                b'/' if self.at(1) == b'*' => {
                    self.pos += 2;
                    self.skip_until(b"*/");
                    self.pos = (self.pos + 2).min(self.s.len());
                }
                _ => return,
            }
        }
    }

    /// Advance to the next `end` (or the end of input).
    fn skip_until(&mut self, end: &[u8]) {
        let rest = self.s.get(self.pos..).unwrap_or_default();
        self.pos += rest.windows(end.len()).position(|w| w == end).unwrap_or(rest.len());
    }

    /// Bytes up to the next `end`, consuming the delimiter.
    fn delimited(&mut self, end: &[u8]) -> Result<&[u8]> {
        let start = self.pos;
        self.skip_until(end);
        if self.pos >= self.s.len() {
            return Err(self.err("unterminated string, asset path or path"));
        }
        let body = self.s.get(start..self.pos).unwrap_or_default();
        self.pos += end.len();
        Ok(body)
    }

    fn string(&mut self, quote: u8) -> Result<String> {
        if self.at(0) == quote && self.at(1) == quote {
            self.pos += 2;
            let body = self.delimited(&[quote; 3])?;
            return Ok(String::from_utf8_lossy(body).into_owned());
        }
        let mut out = Vec::new();
        loop {
            match self.at(0) {
                0 if self.pos >= self.s.len() => return Err(self.err("unterminated string")),
                b'\n' => return Err(self.err("newline in a string")),
                b'\\' => {
                    out.push(match self.at(1) {
                        b'n' => b'\n',
                        b't' => b'\t',
                        c => c,
                    });
                    self.pos += 2;
                }
                c if c == quote => {
                    self.pos += 1;
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                c => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn lex(&mut self) -> Result<Tok> {
        self.skip_space();
        let c = self.at(0);
        if self.pos >= self.s.len() {
            return Ok(Tok::Eof);
        }
        let text = |s: &[u8]| String::from_utf8_lossy(s).into_owned();
        Ok(match c {
            b'"' | b'\'' => {
                self.pos += 1;
                Tok::Str(self.string(c)?)
            }
            b'@' if self.at(1) == b'@' && self.at(2) == b'@' => {
                self.pos += 3;
                Tok::Asset(text(self.delimited(b"@@@")?))
            }
            b'@' => {
                self.pos += 1;
                Tok::Asset(text(self.delimited(b"@")?))
            }
            b'<' => {
                self.pos += 1;
                Tok::Path(text(self.delimited(b">")?))
            }
            b'0'..=b'9' | b'-' | b'+' | b'.' => {
                let start = self.pos;
                self.pos += 1;
                while self.at(0).is_ascii_alphanumeric() || matches!(self.at(0), b'.' | b'-' | b'+') {
                    self.pos += 1;
                }
                let word = std::str::from_utf8(self.s.get(start..self.pos).unwrap_or_default()).unwrap_or_default();
                Tok::Num(word.parse().map_err(|_| self.err(format!("bad number \"{word}\"")))?)
            }
            c if is_ident_start(c) => {
                let start = self.pos;
                while is_ident(self.at(0)) {
                    self.pos += 1;
                }
                Tok::Ident(text(self.s.get(start..self.pos).unwrap_or_default()))
            }
            c => {
                self.pos += 1;
                Tok::Punct(c)
            }
        })
    }

    fn next(&mut self) -> Result<Tok> {
        match self.peeked.take() {
            Some(t) => Ok(t),
            None => self.lex(),
        }
    }

    fn peek(&mut self) -> Result<&Tok> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lex()?);
        }
        Ok(self.peeked.get_or_insert(Tok::Eof))
    }

    fn eat(&mut self, p: u8) -> Result<bool> {
        let hit = *self.peek()? == Tok::Punct(p);
        if hit {
            self.peeked = None;
        }
        Ok(hit)
    }

    fn expect(&mut self, p: u8) -> Result<()> {
        match self.next()? {
            Tok::Punct(c) if c == p => Ok(()),
            t => Err(self.err(format!("expected '{}', found {t:?}", p as char))),
        }
    }
}

/// A value, flattened: numbers, strings (tokens, asset paths) and paths in document order.
#[derive(Default)]
struct Acc {
    nums: Vec<f64>,
    strs: Vec<String>,
    paths: Vec<String>,
    /// The last item was an asset path (a path right after one names a prim in that asset).
    after_asset: bool,
    none: bool,
}

impl Acc {
    fn into_val(self) -> Option<Val> {
        if self.none {
            None
        } else if !self.paths.is_empty() {
            Some(Val::Path(self.paths))
        } else if !self.strs.is_empty() {
            Some(Val::Tok(self.strs))
        } else {
            Some(Val::Num(self.nums))
        }
    }
}

struct Parser<'a, 'b> {
    lx: Lexer<'a>,
    layer: Layer,
    budget: &'b mut Budget,
}

/// Metadata keys whose values are kept.
const KEPT_METADATA: [&str; 7] = ["metersPerUnit", "upAxis", "subLayers", "references", "active", "interpolation", "payload"];

/// Parse a `.usda` text layer, charging decoded allocations against `budget`.
pub fn read(bytes: &[u8], budget: &mut Budget) -> Result<Layer> {
    let mut lx = Lexer { s: bytes, pos: 0, peeked: None };
    // The header is a comment: `#usda 1.0`.
    lx.skip_space();
    let mut p = Parser { lx, layer: Layer::new(), budget };
    if *p.lx.peek()? == Tok::Punct(b'(') {
        for (key, acc) in p.metadata()? {
            match (key.as_str(), acc.into_val()) {
                ("metersPerUnit", Some(Val::Num(v))) => p.layer.meters_per_unit = v.first().copied(),
                ("upAxis", Some(Val::Tok(v))) => p.layer.up_axis = v.into_iter().next(),
                ("subLayers", Some(Val::Tok(v))) if !v.is_empty() => p.layer.warn("sublayers are not loaded"),
                _ => {}
            }
        }
    }
    loop {
        match p.lx.next()? {
            Tok::Eof => break,
            Tok::Ident(kw) if matches!(kw.as_str(), "def" | "over" | "class") => p.prim(0, &kw, 0)?,
            Tok::Punct(b';') => {}
            t => return Err(p.lx.err(format!("expected a prim, found {t:?}"))),
        }
    }
    Ok(p.layer)
}

impl Parser<'_, '_> {
    /// One value into `acc`.
    fn value(&mut self, acc: &mut Acc, depth: usize) -> Result<()> {
        if depth > MAX_NESTING {
            return Err(self.lx.err("values are nested too deeply"));
        }
        let after_asset = std::mem::take(&mut acc.after_asset);
        match self.lx.next()? {
            Tok::Num(v) => {
                self.budget.take(8)?;
                acc.nums.push(v);
            }
            Tok::Ident(w) => match w.as_str() {
                "true" | "false" => {
                    self.budget.take(8)?;
                    acc.nums.push(if w == "true" { 1.0 } else { 0.0 });
                }
                "inf" | "nan" => {
                    self.budget.take(8)?;
                    acc.nums.push(w.parse().unwrap_or(f64::NAN));
                }
                "None" => acc.none = true,
                _ => self.push_str(acc, w)?,
            },
            Tok::Str(s) => self.push_str(acc, s)?,
            Tok::Asset(s) => {
                self.push_str(acc, s)?;
                acc.after_asset = true;
            }
            // A path right after an asset path names a prim in that (external) layer.
            Tok::Path(_) if after_asset => {}
            Tok::Path(p) => {
                self.budget.take(p.len() + 24)?;
                acc.paths.push(p);
            }
            Tok::Punct(open @ (b'[' | b'(')) => {
                let close = if open == b'[' { b']' } else { b')' };
                loop {
                    match self.lx.peek()? {
                        Tok::Punct(c) if *c == close => break,
                        Tok::Punct(b',' | b';' | b'=') => {
                            acc.after_asset = false;
                            self.lx.next()?;
                        }
                        Tok::Eof => return Err(self.lx.err("unterminated list")),
                        _ => self.value(acc, depth + 1)?,
                    }
                }
                self.lx.next()?;
            }
            Tok::Punct(b'{') => self.skip_block(b'{', b'}')?,
            t => return Err(self.lx.err(format!("unexpected {t:?} in a value"))),
        }
        Ok(())
    }

    fn push_str(&mut self, acc: &mut Acc, s: String) -> Result<()> {
        self.budget.take(s.len() + 24)?;
        acc.strs.push(s);
        Ok(())
    }

    /// Skip a balanced block whose opening token was just read.
    fn skip_block(&mut self, open: u8, close: u8) -> Result<()> {
        let mut depth = 1usize;
        while depth > 0 {
            match self.lx.next()? {
                Tok::Punct(c) if c == open => depth += 1,
                Tok::Punct(c) if c == close => depth -= 1,
                Tok::Eof => return Err(self.lx.err("unterminated block")),
                _ => {}
            }
        }
        Ok(())
    }

    /// A `( ... )` metadata block (the `(` not yet read): the kept entries, by key. List-op
    /// `delete` and `reorder` entries are dropped.
    fn metadata(&mut self) -> Result<Vec<(String, Acc)>> {
        self.lx.expect(b'(')?;
        let mut out = Vec::new();
        loop {
            let (mut key, mut keep) = match self.lx.next()? {
                Tok::Punct(b')') => return Ok(out),
                Tok::Punct(b';' | b',') | Tok::Str(_) => continue,
                Tok::Ident(k) => (k, true),
                t => return Err(self.lx.err(format!("unexpected {t:?} in metadata"))),
            };
            if matches!(key.as_str(), "prepend" | "append" | "add" | "delete" | "reorder") {
                keep = !matches!(key.as_str(), "delete" | "reorder");
                key = match self.lx.next()? {
                    Tok::Ident(k) => k,
                    t => return Err(self.lx.err(format!("expected a metadata key, found {t:?}"))),
                };
            }
            if !self.lx.eat(b'=')? {
                continue;
            }
            let mut acc = Acc::default();
            self.value(&mut acc, 0)?;
            if keep && KEPT_METADATA.contains(&key.as_str()) {
                out.push((key, acc));
            }
        }
    }

    fn prim(&mut self, parent: usize, specifier: &str, depth: usize) -> Result<()> {
        if depth > MAX_NESTING {
            return Err(self.lx.err("prims are nested too deeply"));
        }
        let (type_name, name) = match self.lx.next()? {
            Tok::Ident(t) => match self.lx.next()? {
                Tok::Str(n) => (t, n),
                t => return Err(self.lx.err(format!("expected a prim name, found {t:?}"))),
            },
            Tok::Str(n) => (String::new(), n),
            t => return Err(self.lx.err(format!("expected a prim name, found {t:?}"))),
        };
        self.budget.take(name.len() * 2 + 256)?;
        let id = self.layer.add_prim(parent, &name);
        let mut references = Vec::new();
        let mut active = None;
        if *self.lx.peek()? == Tok::Punct(b'(') {
            for (key, acc) in self.metadata()? {
                match key.as_str() {
                    "references" | "payload" => {
                        if !acc.strs.is_empty() {
                            self.layer.warn("external references and payloads are not loaded");
                        }
                        references.extend(acc.paths);
                    }
                    "active" => active = acc.nums.first().map(|v| *v != 0.0),
                    _ => {}
                }
            }
        }
        if let Some(spec) = self.layer.prims.get_mut(id) {
            spec.type_name = type_name;
            spec.specifier = match specifier {
                "over" => Specifier::Over,
                "class" => Specifier::Class,
                _ => Specifier::Def,
            };
            spec.active = active;
            spec.references = references;
        }
        self.lx.expect(b'{')?;
        loop {
            let word = match self.lx.next()? {
                Tok::Punct(b'}') => return Ok(()),
                Tok::Punct(b';') => continue,
                Tok::Ident(w) => w,
                t => return Err(self.lx.err(format!("unexpected {t:?} in prim \"{name}\""))),
            };
            match word.as_str() {
                "def" | "over" | "class" => self.prim(id, &word, depth + 1)?,
                "variantSet" => {
                    self.layer.warn("variant sets are not applied");
                    while !self.lx.eat(b'{')? {
                        if self.lx.next()? == Tok::Eof {
                            return Err(self.lx.err("unterminated variant set"));
                        }
                    }
                    self.skip_block(b'{', b'}')?;
                }
                "reorder" => {
                    self.lx.next()?;
                    self.lx.expect(b'=')?;
                    self.value(&mut Acc::default(), 0)?;
                }
                _ => self.property(id, word)?,
            }
        }
    }

    /// An attribute or relationship whose first word was read.
    fn property(&mut self, prim: usize, mut word: String) -> Result<()> {
        let mut keep = true;
        while matches!(word.as_str(), "custom" | "uniform" | "varying" | "config" | "prepend" | "append" | "add" | "delete") {
            keep &= word != "delete";
            word = match self.lx.next()? {
                Tok::Ident(w) => w,
                t => return Err(self.lx.err(format!("expected a property type, found {t:?}"))),
            };
        }
        let mut type_name = word;
        if self.lx.eat(b'[')? {
            self.lx.expect(b']')?;
            type_name.push_str("[]");
        }
        let full = match self.lx.next()? {
            Tok::Ident(n) => n,
            t => return Err(self.lx.err(format!("expected a property name, found {t:?}"))),
        };
        let (name, suffix) = match full.rsplit_once('.') {
            Some((n, s @ ("connect" | "timeSamples" | "spline" | "default"))) => (n.to_string(), s),
            _ => (full, ""),
        };
        let mut acc = Acc::default();
        let mut has_value = false;
        if self.lx.eat(b'=')? {
            has_value = true;
            if suffix == "timeSamples" || suffix == "spline" {
                self.first_sample(&mut acc)?;
            } else {
                self.value(&mut acc, 0)?;
            }
        }
        let mut interpolation = None;
        if *self.lx.peek()? == Tok::Punct(b'(') {
            for (key, meta) in self.metadata()? {
                if key == "interpolation" {
                    interpolation = meta.strs.into_iter().next();
                }
            }
        }
        if !keep || suffix == "spline" {
            return Ok(());
        }
        let Some(spec) = self.layer.prims.get_mut(prim) else { return Ok(()) };
        let prop = spec.props.entry(name).or_default();
        if suffix == "connect" {
            prop.connect = acc.paths.into_iter().next();
            return Ok(());
        }
        // A plain default and its time samples may both be authored; the default wins.
        if has_value && (suffix != "timeSamples" || prop.value.is_none()) {
            prop.value = acc.into_val();
        }
        prop.type_name = type_name;
        if interpolation.is_some() {
            prop.interpolation = interpolation;
        }
        Ok(())
    }

    /// `{ time: value, ... }`: the first sample's value into `acc`, the rest skipped.
    fn first_sample(&mut self, acc: &mut Acc) -> Result<()> {
        self.lx.expect(b'{')?;
        let mut first = true;
        loop {
            match self.lx.next()? {
                Tok::Punct(b'}') => return Ok(()),
                Tok::Punct(b',') => {}
                Tok::Num(_) => {
                    self.lx.expect(b':')?;
                    if first {
                        self.value(acc, 0)?;
                        first = acc.none;
                        acc.none = false;
                    } else {
                        let mut skip = Acc::default();
                        self.value(&mut skip, 0)?;
                        // Skipped samples do not stay in memory.
                        self.budget.0 = self.budget.0.saturating_add(8 * skip.nums.len());
                    }
                }
                t => return Err(self.lx.err(format!("unexpected {t:?} in time samples"))),
            }
        }
    }
}
