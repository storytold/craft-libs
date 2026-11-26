//! Platform regression guards checking source code for known defects and lifecycle hazards.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::rc::Rc;

/// A diagnostic finding reported by a source guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// File path where the violation was detected.
    pub path: String,
    /// 1-based line number of the violation.
    pub line: usize,
    /// Diagnostic context (the offending code snippet or float literal).
    pub text: String,
}

impl Finding {
    /// Create a new diagnostic finding.
    pub fn new(path: impl Into<String>, line: usize, text: impl Into<String>) -> Self {
        Self { path: path.into(), line, text: text.into() }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() { write!(f, "{}: {}", self.line, self.text) } else { write!(f, "{}:{}: {}", self.path, self.line, self.text) }
    }
}

/// Abstraction over a source file path and its text content.
pub trait AsFileSource {
    /// Path of the source file.
    fn path(&self) -> &str;
    /// Text content of the source file.
    fn content(&self) -> &str;
}

impl AsFileSource for (String, String) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for &(String, String) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for (&str, &str) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for &(&str, &str) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for (String, &str) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for &(String, &str) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for (&str, String) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for &(&str, String) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

/// The deepest `use` group nesting and `cfg` predicate nesting the guards follow. Deeper input is
/// read as far as this depth, so hostile nesting cannot exhaust the stack.
const MAX_NESTING: usize = 64;

/// The most segments of a `use` path that [`use_leaves`] keeps. The guards compare paths of at most
/// three segments, so a leaf with a longer path keeps only the name it binds, and a long prefix shared
/// by many leaves is not copied into each of them.
const MAX_USE_PATH: usize = 8;

/// The longest segment, in bytes, of a `use` path that [`use_leaves`] keeps. The longest segment the
/// guards compare is `PredefinedMenuItem`, so a leaf with a longer segment keeps only the name it binds,
/// and a long name shared by many leaves is not copied into each of them.
const MAX_USE_SEGMENT: usize = 32;

/// The kind of a Rust token. Comments are dropped; string, character and number literals keep no text
/// except the decoded contents of strings, which `cfg` predicates compare. A string with an escape
/// that is not valid is `Other`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Ident(String),
    Punct(char),
    Str(String),
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    line: usize,
}

impl Token {
    fn is_ident(&self, name: &str) -> bool {
        matches!(&self.kind, Kind::Ident(s) if s == name)
    }

    fn is_punct(&self, c: char) -> bool {
        self.kind == Kind::Punct(c)
    }
}

/// Whether `c` can start a Rust identifier: `_`, a letter, or another non-ASCII character that is not white
/// space (see [`non_ascii_word`]).
fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic() || non_ascii_word(c)
}

/// Whether `c` can continue a Rust identifier: `_`, a letter or digit, or another non-ASCII character that is
/// not white space, such as the combining mark U+0301.
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || non_ascii_word(c)
}

/// Whether `c` is a non-ASCII character other than Rust's white space and the byte order mark U+FEFF. Outside
/// literals and comments, Rust allows such a character only in an identifier, so it is read as part of one.
fn non_ascii_word(c: char) -> bool {
    !c.is_ascii() && !c.is_whitespace() && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{FEFF}')
}

/// Split Rust source into tokens with 1-based line numbers.
///
/// Line comments, nested block comments, string literals (normal, byte, C and raw strings with any
/// number of `#`), character and byte literals, lifetimes and raw identifiers are recognized, so a quote
/// or comment marker inside one of them does not hide the code after it. Unterminated literals and
/// comments run to the end of the input.
fn lex_rust(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let at = |i: usize| chars.get(i).copied();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line = 1usize;

    while i < len {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() || matches!(c, '\u{200E}' | '\u{200F}' | '\u{FEFF}') {
            // Rust also reads the left-to-right and right-to-left marks as white space, and skips a byte
            // order mark at the start of a file.
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 1usize;
            i += 2;
            while i < len && depth > 0 {
                if chars[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if is_ident_start(c) {
            let start = i;
            while i < len && is_ident_continue(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let token_line = line;
            if matches!(word.as_str(), "r" | "br" | "cr") && matches!(at(i), Some('"' | '#')) {
                let mut j = i;
                while j < len && chars[j] == '#' {
                    j += 1;
                }
                let hashes = j - i;
                if at(j) == Some('"') {
                    let (end, text) = scan_raw_string(&chars, j + 1, hashes, &mut line);
                    tokens.push(Token { kind: Kind::Str(text), line: token_line });
                    i = end;
                    continue;
                }
                if word == "r" && hashes == 1 && at(j).is_some_and(is_ident_start) {
                    let name_start = j;
                    while j < len && is_ident_continue(chars[j]) {
                        j += 1;
                    }
                    tokens.push(Token { kind: Kind::Ident(raw_identifier(&chars[name_start..j])), line: token_line });
                    i = j;
                    continue;
                }
            }
            if matches!(word.as_str(), "b" | "c") && at(i) == Some('"') {
                let (end, kind) = scan_string(&chars, i + 1, &mut line);
                tokens.push(Token { kind, line: token_line });
                i = end;
                continue;
            }
            if word == "b"
                && at(i) == Some('\'')
                && let Some(end) = char_literal_end(&chars, i)
            {
                tokens.push(Token { kind: Kind::Other, line: token_line });
                i = end;
                continue;
            }
            tokens.push(Token { kind: Kind::Ident(word), line: token_line });
        } else if c.is_ascii_digit() {
            while i < len && (is_ident_continue(chars[i]) || (chars[i] == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit()))) {
                i += 1;
            }
            tokens.push(Token { kind: Kind::Other, line });
        } else if c == '"' {
            let token_line = line;
            let (end, kind) = scan_string(&chars, i + 1, &mut line);
            tokens.push(Token { kind, line: token_line });
            i = end;
        } else if c == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                i = end;
            } else {
                // A lifetime or a loop label.
                i += 1;
                while i < len && is_ident_continue(chars[i]) {
                    i += 1;
                }
            }
            tokens.push(Token { kind: Kind::Other, line });
        } else {
            tokens.push(Token { kind: Kind::Punct(c), line });
            i += 1;
        }
    }
    tokens
}

/// The name of a raw identifier written after `r#`. A raw identifier spelled like a keyword the guards
/// read (`r#let`, `r#in`, `r#match`, `r#true`) keeps its prefix, so it is read as a name rather than as
/// that keyword or as a `cfg` literal.
fn raw_identifier(name: &[char]) -> String {
    let name: String = name.iter().collect();
    let keyword = |list: &[&str]| list.contains(&name.as_str());
    if keyword(&["true", "false"]) || keyword(KEYWORDS) || keyword(BLOCK_KEYWORDS) || keyword(VALUE_KEYWORDS) { format!("r#{name}") } else { name }
}

/// Read a normal string body starting after its opening quote. Returns the index after the closing
/// quote and the token for the body: `Str` with its escapes decoded, or `Other` when an escape is not
/// valid in a string.
fn scan_string(chars: &[char], mut i: usize, line: &mut usize) -> (usize, Kind) {
    let mut text = Some(String::new());
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let (next, decoded) = string_escape(chars, i + 1, line);
                match decoded {
                    Ok(Some(c)) => text.iter_mut().for_each(|t| t.push(c)),
                    Ok(None) => {}
                    Err(()) => text = None,
                }
                i = next;
            }
            '"' => return (i + 1, text.map_or(Kind::Other, Kind::Str)),
            c => {
                if c == '\n' {
                    *line += 1;
                }
                text.iter_mut().for_each(|t| t.push(c));
                i += 1;
            }
        }
    }
    (chars.len(), text.map_or(Kind::Other, Kind::Str))
}

/// Decode the string escape whose `\` is right before `chars[i]`
/// (<https://doc.rust-lang.org/reference/tokens.html#string-literals>). Returns the index after the
/// escape and the character it stands for, `None` for a line continuation, or an error when it is not
/// a valid escape.
fn string_escape(chars: &[char], i: usize, line: &mut usize) -> (usize, Result<Option<char>, ()>) {
    let hex = |k: usize| chars.get(k).and_then(|c| c.to_digit(16));
    match chars.get(i) {
        Some('n') => (i + 1, Ok(Some('\n'))),
        Some('r') => (i + 1, Ok(Some('\r'))),
        Some('t') => (i + 1, Ok(Some('\t'))),
        Some('0') => (i + 1, Ok(Some('\0'))),
        Some(&c @ ('\\' | '\'' | '"')) => (i + 1, Ok(Some(c))),
        Some('x') => match (hex(i + 1), hex(i + 2)) {
            (Some(high), Some(low)) if high < 8 => (i + 3, Ok(char::from_u32(high * 16 + low))),
            _ => (i + 1, Err(())),
        },
        Some('u') if chars.get(i + 1) == Some(&'{') => {
            // At most six hex digits, with underscores, before the closing brace.
            let mut value = 0u32;
            let mut digits = 0;
            let mut j = i + 2;
            while let Some(&c) = chars.get(j) {
                if c == '}' {
                    break;
                }
                if c != '_' {
                    match c.to_digit(16) {
                        Some(d) if digits < 6 => {
                            value = value * 16 + d;
                            digits += 1;
                        }
                        _ => return (j, Err(())),
                    }
                }
                j += 1;
            }
            match (chars.get(j), char::from_u32(value)) {
                (Some('}'), Some(c)) if digits > 0 && chars.get(i + 2) != Some(&'_') => (j + 1, Ok(Some(c))),
                _ => (j.min(chars.len()), Err(())),
            }
        }
        Some('\n') => {
            // A line continuation skips the line break and the whitespace after it.
            let mut j = i;
            while let Some(&c) = chars.get(j).filter(|c| matches!(c, ' ' | '\t' | '\n' | '\r')) {
                if c == '\n' {
                    *line += 1;
                }
                j += 1;
            }
            (j, Ok(None))
        }
        Some('\r') if chars.get(i + 1) == Some(&'\n') => {
            let (j, _) = string_escape(chars, i + 1, line);
            (j, Ok(None))
        }
        Some(_) => (i + 1, Err(())),
        None => (i, Err(())),
    }
}

/// Read a raw string body starting after its opening quote; it ends at a quote followed by `hashes` `#`.
fn scan_raw_string(chars: &[char], mut i: usize, hashes: usize, line: &mut usize) -> (usize, String) {
    let start = i;
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|h| chars.get(i + h) == Some(&'#')) {
            return (i + 1 + hashes, chars[start..i].iter().collect());
        }
        if chars[i] == '\n' {
            *line += 1;
        }
        i += 1;
    }
    (chars.len(), chars[start.min(chars.len())..].iter().collect())
}

/// The index after a character or byte literal whose opening quote is at `quote`, or `None` when the
/// quote starts a lifetime or a label.
fn char_literal_end(chars: &[char], quote: usize) -> Option<usize> {
    match chars.get(quote + 1) {
        Some('\\') => {
            // An escape: `\n`, `\'`, `\x7f`, `\u{10FFFF}`. The longest is twelve characters.
            let mut j = quote + 3;
            while j < chars.len() && j <= quote + 12 && chars[j] != '\'' && chars[j] != '\n' {
                j += 1;
            }
            (chars.get(j) == Some(&'\'')).then_some(j + 1)
        }
        Some('\n') | None => None,
        Some(_) => (chars.get(quote + 2) == Some(&'\'')).then_some(quote + 3),
    }
}

/// Rust keywords that can come right before a path that starts with `::`, as in `return ::std::time`.
/// `crate`, `self`, `super` and `Self` are path segments and are not listed.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "type", "unsafe", "use", "where", "while", "yield",
];

/// Whether `toks[i]` follows `::` that itself follows a path segment, as `time` does in `a::time`.
/// The `>` of `->` and `=>` ends no path, so `-> ::std::time` starts one.
fn continues_a_path(toks: &[Token], i: usize) -> bool {
    i >= 3
        && toks[i - 1].is_punct(':')
        && toks[i - 2].is_punct(':')
        && match &toks[i - 3].kind {
            Kind::Ident(s) => !KEYWORDS.contains(&s.as_str()),
            Kind::Punct('>') => !(i >= 4 && (toks[i - 4].is_punct('-') || toks[i - 4].is_punct('='))),
            _ => false,
        }
}

/// Whether `toks[i..]` starts with `::` followed by the identifier `name`.
fn then_path(toks: &[Token], i: usize, name: impl Fn(&str) -> bool) -> bool {
    toks.get(i).is_some_and(|t| t.is_punct(':'))
        && toks.get(i + 1).is_some_and(|t| t.is_punct(':'))
        && toks.get(i + 2).is_some_and(|t| matches!(&t.kind, Kind::Ident(s) if name(s)))
}

/// Whether the path that ends at `toks[i]` starts right after the `<` of a qualified path, as in
/// `<muda::PredefinedMenuItem>::quit`, rather than after the `<` of generic arguments (`Vec<T>::new`,
/// `f::<T>`).
fn opens_qualified_path(toks: &[Token], i: usize) -> bool {
    let mut start = i;
    while start >= 2 && toks[start - 1].is_punct(':') && toks[start - 2].is_punct(':') {
        if start >= 3 && matches!(toks[start - 3].kind, Kind::Ident(_)) {
            start -= 3;
        } else {
            start -= 2;
            break;
        }
    }
    start >= 1
        && toks[start - 1].is_punct('<')
        && match start.checked_sub(2).map(|j| &toks[j].kind) {
            None => true,
            Some(Kind::Ident(s)) => KEYWORDS.contains(&s.as_str()),
            Some(Kind::Punct(c)) => !matches!(*c, ':' | '>'),
            Some(_) => false,
        }
}

/// One name a `use` declaration brings into scope.
#[derive(Debug)]
struct UseLeaf {
    /// The full path, `self` removed and without a leading `::`. It is empty for a path longer than
    /// [`MAX_USE_PATH`] segments or with a segment longer than [`MAX_USE_SEGMENT`] bytes, whose leaf then
    /// records only the name it binds.
    path: Vec<String>,
    /// The name it is bound to, `_` for an underscore import; `None` for a glob import. The name is
    /// shared with the other leaves that bind the same segment of one path.
    name: Option<Rc<str>>,
    /// The first segment of the path, kept when the path is not.
    root: Option<Rc<str>>,
    line: usize,
    /// Index of the `use` keyword.
    start: usize,
    /// Index of the alias of `std::time` that the path was read through, when [`use_leaves`] was given one.
    alias: Option<usize>,
    /// Whether the path starts with `::`, so that its first segment names an external crate.
    absolute: bool,
}

impl UseLeaf {
    /// The name the leaf binds in scope: `None` for a glob import and for an underscore import, which
    /// imports an item without binding a name.
    fn binding(&self) -> Option<&str> {
        self.name.as_deref().filter(|name| *name != "_")
    }
}

/// Every name the `use` declarations of a file bring into scope. A path that starts with a name in
/// `time_aliases` is read as a path that starts with `std::time`, so the alias, at any length, is not
/// copied into the leaves and the limits do not drop them.
fn use_leaves(toks: &[Token], time_aliases: &BTreeSet<String>) -> Vec<UseLeaf> {
    let mut leaves = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        // `use<...>` is a precise capturing bound on `impl Trait`, not a `use` declaration.
        if toks[i].is_ident("use") && !continues_a_path(toks, i) && !toks.get(i + 1).is_some_and(|t| t.is_punct('<')) {
            let start = i;
            let end = (i + 1..toks.len()).find(|&j| toks[j].is_punct(';')).unwrap_or(toks.len());
            let mut pos = i + 1;
            let mut prefix = Vec::new();
            parse_use_tree(&toks[..end], &mut pos, &mut prefix, (start, None, false), &mut leaves, time_aliases, 0);
            i = end;
        } else {
            i += 1;
        }
    }
    leaves
}

fn parse_use_tree(
    toks: &[Token],
    pos: &mut usize,
    prefix: &mut Vec<Rc<str>>,
    // The index of the `use` keyword, the alias of `std::time` that `prefix` was read through, and whether
    // the path starts with `::`.
    (start, mut via, mut absolute): (usize, Option<usize>, bool),
    out: &mut Vec<UseLeaf>,
    time_aliases: &BTreeSet<String>,
    depth: usize,
) {
    let base = prefix.len();
    if toks.get(*pos).is_some_and(|t| t.is_punct(':')) && toks.get(*pos + 1).is_some_and(|t| t.is_punct(':')) {
        absolute |= prefix.is_empty();
        *pos += 2;
    }
    // A leaf is reported on the line where its path starts in this tree, not where its last name is.
    let line = toks.get(*pos).map_or(0, |t| t.line);
    while let Some(tok) = toks.get(*pos) {
        match &tok.kind {
            Kind::Ident(name) => {
                *pos += 1;
                if toks.get(*pos).is_some_and(|t| t.is_punct(':')) && toks.get(*pos + 1).is_some_and(|t| t.is_punct(':')) {
                    // After a leading `::` the first segment names an external crate, never a local alias.
                    if prefix.is_empty() && !absolute && time_aliases.contains(name) {
                        prefix.extend([Rc::from("std"), Rc::from("time")]);
                        via = Some(*pos - 1);
                    } else {
                        prefix.push(Rc::from(name.as_str()));
                    }
                    *pos += 2;
                    continue;
                }
                let length = prefix.len() + usize::from(name != "self");
                let alias = match (toks.get(*pos), toks.get(*pos + 1).map(|t| &t.kind)) {
                    (Some(t), Some(Kind::Ident(alias))) if t.is_ident("as") => {
                        *pos += 2;
                        Some(alias)
                    }
                    _ => None,
                };
                // The name the leaf binds: its alias, its last name, or for `self` the last segment of
                // the prefix, shared rather than copied.
                let bound = match alias {
                    Some(alias) => Some(Rc::from(alias.as_str())),
                    None if name != "self" => Some(Rc::from(name.as_str())),
                    None => prefix.last().cloned(),
                };
                // A path past the limits is not copied, but the leaf still binds its name, so that the name
                // does not read as unbound.
                let path = if length <= MAX_USE_PATH && short_segments(prefix) && name.len() <= MAX_USE_SEGMENT {
                    let mut path: Vec<String> = prefix.iter().map(|segment| segment.to_string()).collect();
                    if name != "self" {
                        path.push(name.clone());
                    }
                    path
                } else {
                    Vec::new()
                };
                if bound.is_some() {
                    let root = prefix.first().cloned().or_else(|| Some(Rc::from(name.as_str())));
                    out.push(UseLeaf { path, name: bound, root, line, start, alias: via, absolute });
                }
                break;
            }
            Kind::Punct('*') => {
                if prefix.len() <= MAX_USE_PATH && short_segments(prefix) {
                    let path = prefix.iter().map(|segment| segment.to_string()).collect();
                    out.push(UseLeaf { path, name: None, root: prefix.first().cloned(), line, start, alias: via, absolute });
                }
                *pos += 1;
                break;
            }
            Kind::Punct('{') => {
                *pos += 1;
                while let Some(t) = toks.get(*pos) {
                    if t.is_punct('}') {
                        *pos += 1;
                        break;
                    }
                    let before = *pos;
                    if !t.is_punct(',') && depth < MAX_NESTING {
                        parse_use_tree(toks, pos, prefix, (start, via, absolute), out, time_aliases, depth + 1);
                    }
                    if *pos == before {
                        *pos += 1;
                    }
                }
                break;
            }
            _ => break,
        }
    }
    prefix.truncate(base);
}

/// Whether every segment of `path` is at most [`MAX_USE_SEGMENT`] bytes long.
fn short_segments(path: &[Rc<str>]) -> bool {
    path.iter().all(|segment| segment.len() <= MAX_USE_SEGMENT)
}

/// A value of a `cfg` predicate on `wasm32-unknown-unknown`, where the value of a feature or of
/// `test` is not known from the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    fn from_bool(b: bool) -> Self {
        if b { Self::True } else { Self::False }
    }

    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// Evaluate the `cfg` predicate at `toks[*pos]` for `wasm32-unknown-unknown`, or with `every` for every
/// configuration at once, where only `true`, `false` and the operators over them have a known value.
fn eval_cfg(toks: &[Token], pos: &mut usize, depth: usize, every: bool) -> Tri {
    let Some(Kind::Ident(name)) = toks.get(*pos).map(|t| &t.kind) else {
        return Tri::Unknown;
    };
    *pos += 1;
    if toks.get(*pos).is_some_and(|t| t.is_punct('=')) {
        let value = match toks.get(*pos + 1).map(|t| &t.kind) {
            Some(Kind::Str(v)) => v.clone(),
            _ => return Tri::Unknown,
        };
        *pos += 2;
        if every {
            return Tri::Unknown;
        }
        let expected = match name.as_str() {
            "target_arch" => "wasm32",
            "target_os" | "target_vendor" => "unknown",
            "target_family" => "wasm",
            "target_env" => "",
            "target_pointer_width" => "32",
            "target_endian" => "little",
            _ => return Tri::Unknown,
        };
        return Tri::from_bool(value == expected);
    }
    if toks.get(*pos).is_some_and(|t| t.is_punct('(')) {
        if depth >= MAX_NESTING {
            // Too deep to read: the predicate is skipped as a whole, and its value is not known.
            *pos = matching_close(toks, *pos).saturating_add(1);
            return Tri::Unknown;
        }
        *pos += 1;
        let mut values = Vec::new();
        while let Some(t) = toks.get(*pos) {
            if t.is_punct(')') {
                *pos += 1;
                break;
            }
            if t.is_punct(',') {
                *pos += 1;
                continue;
            }
            let before = *pos;
            values.push(eval_cfg(toks, pos, depth + 1, every));
            if *pos == before {
                *pos += 1;
            }
        }
        return match name.as_str() {
            "not" => values.first().map_or(Tri::Unknown, |v| v.not()),
            "all" if values.contains(&Tri::False) => Tri::False,
            "all" if values.contains(&Tri::Unknown) => Tri::Unknown,
            "all" => Tri::True,
            "any" if values.contains(&Tri::True) => Tri::True,
            "any" if values.contains(&Tri::Unknown) => Tri::Unknown,
            "any" => Tri::False,
            _ => Tri::Unknown,
        };
    }
    match name.as_str() {
        "false" => Tri::False,
        "unix" | "windows" if !every => Tri::False,
        "true" => Tri::True,
        _ => Tri::Unknown,
    }
}

/// The index of the token that closes the bracket opened at `open`, or `toks.len()`.
fn matching_close(toks: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(open) {
        match t.kind {
            Kind::Punct('(' | '[' | '{') => depth += 1,
            Kind::Punct(')' | ']' | '}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

/// Keywords whose item or expression ends with its closing brace rather than with `;`.
const BLOCK_KEYWORDS: &[&str] =
    &["fn", "mod", "impl", "trait", "struct", "enum", "union", "extern", "macro_rules", "if", "match", "loop", "while", "for", "unsafe"];

/// Keywords that start a statement whose value follows them, so a block keyword after them is part of
/// that value.
const VALUE_KEYWORDS: &[&str] = &["let", "return", "break", "yield", "become"];

/// Where [`item_end`] is in the head of an `if`, `while`, `match` or `for` before its body's `{`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Condition {
    /// Not in such a head: the next `{` outside brackets opens the body.
    Done,
    /// In the pattern of a `let` or a `for`, where a `{` opens a struct pattern.
    Pattern,
    /// In a condition, a scrutinee or an iterator, where the next `{` outside brackets opens the body.
    Expression,
}

/// Whether the `=` at `toks[j]` stands alone, as the `=` between a `let` pattern and its value, rather
/// than as part of `==`, `!=`, `<=`, `>=`, `=>` or the range pattern `..=`.
fn lone_equals(toks: &[Token], j: usize) -> bool {
    let punct = |k: usize, cs: &[char]| toks.get(k).is_some_and(|t| cs.iter().any(|&c| t.is_punct(c)));
    !punct(j + 1, &['=', '>']) && j.checked_sub(1).is_none_or(|k| !punct(k, &['=', '!', '<', '>', '.']))
}

/// Whether a block expression starts at `j`: `{`, a labeled block (`'label: {`) or a `const` block.
fn starts_block(toks: &[Token], j: usize) -> bool {
    let punct = |k: usize, c: char| toks.get(k).is_some_and(|t| t.is_punct(c));
    match toks.get(j).map(|t| &t.kind) {
        Some(Kind::Punct('{')) => true,
        Some(Kind::Other) => punct(j + 1, ':') && punct(j + 2, '{'),
        Some(Kind::Ident(k)) => k == "const" && punct(j + 1, '{'),
        _ => false,
    }
}

/// Whether the `{` at `toks[j]`, in the condition, scrutinee or iterator of an `if`, `while`, `match` or
/// `for`, stands where an operand is still needed, so it opens a block expression rather than the body:
/// right after the keyword, after `in`, after the `=` of a `let`, after an operator or `!`, after
/// `unsafe`, `const`, `loop`, `async` or `async move`, or after a loop label's `:`.
fn operand_expected(toks: &[Token], j: usize) -> bool {
    let Some(k) = j.checked_sub(1) else { return false };
    match &toks[k].kind {
        Kind::Ident(prev) => matches!(prev.as_str(), "if" | "while" | "match" | "in" | "unsafe" | "const" | "loop" | "async" | "move"),
        Kind::Punct('=' | '+' | '-' | '*' | '/' | '%' | '^' | '|' | '&' | '<' | '>' | '!') => true,
        Kind::Punct(':') => k.checked_sub(1).is_some_and(|l| toks[l].kind == Kind::Other),
        _ => false,
    }
}

/// The index after the item, statement, field or match arm that starts at `start`, or with `inner`, the
/// index of the bracket that closes the enclosing scope.
///
/// It ends after a `;` or a `,` outside brackets, after the closing brace of a block item, or before a
/// closing bracket of the enclosing scope. A block item starts with `{`, a labeled block, a `const`
/// block, a keyword in [`BLOCK_KEYWORDS`] or a macro call with braces, at the head of the item or of a
/// match arm's body (after its `=>`). An `async` block is not a block item: it runs on to its `;`. A
/// block keyword or macro brace after a `=`, a binary operator or a keyword in [`VALUE_KEYWORDS`]
/// outside brackets is part of an expression, which runs on to its `;` or `,`. When a `.` or `?`
/// follows the closing brace of a block item, the item runs on to its `;` or `,`. A `,` inside the angle
/// brackets of a field's or a `let`'s type ends the item early, which leaves the rest of it checked.
///
/// The body of an `if`, `while`, `match` or `for` at the head is the first `{` outside brackets after its
/// condition, scrutinee or iterator. A `{` where the condition still needs an operand (see
/// [`operand_expected`]) opens a block expression in it, as in `if { ready() } { ... }`, and the body is
/// the next `{` after that block. A `{` in the pattern of a `let` (`if let S { .. } = s`, also after
/// `&&` in a chain) or of a `for` opens a struct pattern and does not end the item. An `else` chain ends
/// at its last block.
///
/// An `if` or `match` expression in a condition, scrutinee or iterator, as in
/// `for x in if a { b } else { c } { ... }`, is read to the end of its last block (up to [`MAX_NESTING`]
/// levels of such expressions), and the body is the next `{` after it.
///
/// In the head of an `fn`, `impl`, `trait`, `struct`, `enum` or `union` item, a block inside angle
/// brackets (a const generic default or argument such as `<const N: usize = { 1 + 2 }>`, also in a
/// return type or a `where` clause) does not end the item. The `>` of `->` does not close an angle
/// bracket.
fn item_end(toks: &[Token], start: usize, inner: bool) -> usize {
    item_end_at(toks, start, inner, 0)
}

/// [`item_end`] for an item or, with `nested` above 0, for an `if` or `match` expression inside the head of
/// another one, `nested` levels deep, which ends right after its last block.
fn item_end_at(toks: &[Token], start: usize, inner: bool, nested: usize) -> usize {
    let mut depth = 0usize;
    let mut block_item = false;
    let mut arm = false;
    let mut head = true;
    let mut condition = Condition::Done;
    let mut generics = false;
    let mut angle = 0usize;
    let mut j = start;
    while j < toks.len() {
        let at_top = depth == 0 && !inner;
        if j == start && at_top && starts_block(toks, j) {
            block_item = true;
        }
        match &toks[j].kind {
            // An `if` or `match` expression in the condition ends at its last block, so its blocks do not
            // read as the body.
            Kind::Ident(k) if at_top && condition == Condition::Expression && matches!(k.as_str(), "if" | "match") && nested < MAX_NESTING => {
                j = item_end_at(toks, j, false, nested + 1).max(j + 1);
                continue;
            }
            Kind::Ident(k) if at_top && head && (BLOCK_KEYWORDS.contains(&k.as_str()) || starts_block(toks, j)) => {
                if !block_item {
                    condition = match k.as_str() {
                        "if" | "while" | "match" => Condition::Expression,
                        "for" => Condition::Pattern,
                        _ => Condition::Done,
                    };
                }
                if matches!(k.as_str(), "fn" | "impl" | "trait" | "struct" | "enum" | "union") {
                    generics = true;
                }
                block_item = true;
            }
            Kind::Punct('<') if at_top && generics => angle += 1,
            Kind::Punct('>') if at_top && generics && !j.checked_sub(1).is_some_and(|k| toks[k].is_punct('-')) => {
                angle = angle.saturating_sub(1);
            }
            Kind::Ident(k) if at_top && condition == Condition::Expression && k == "let" => condition = Condition::Pattern,
            Kind::Ident(k) if at_top && condition == Condition::Pattern && k == "in" => condition = Condition::Expression,
            Kind::Punct('=') if at_top && condition == Condition::Pattern && lone_equals(toks, j) => condition = Condition::Expression,
            Kind::Punct('{') if at_top && condition == Condition::Expression => {
                // A `{` where the condition still needs an operand opens a block expression inside it
                // (`if { ready() } { ... }`); the body's `{` comes after the condition's last operand.
                if !operand_expected(toks, j) {
                    condition = Condition::Done;
                }
                depth += 1;
            }
            Kind::Ident(k) if at_top && VALUE_KEYWORDS.contains(&k.as_str()) => head = false,
            Kind::Punct('!') if at_top && head && toks.get(j + 1).is_some_and(|t| t.is_punct('{')) => block_item = true,
            Kind::Punct('=') if at_top && toks.get(j + 1).is_some_and(|t| t.is_punct('>')) => {
                // A match arm's body has a head of its own: a block or block keyword there ends the arm.
                arm = true;
                head = true;
                block_item = starts_block(toks, j + 2);
                condition = Condition::Done;
                j += 2;
                continue;
            }
            // The `:` after a field's or a static's name starts its type, where `fn` is a function pointer
            // type rather than an item. The `:` of a path (`::`) and the one after a loop label are not.
            Kind::Punct(':')
                if at_top
                    && !block_item
                    && !toks.get(j + 1).is_some_and(|t| t.is_punct(':'))
                    && j.checked_sub(1).is_none_or(|k| !toks[k].is_punct(':') && toks[k].kind != Kind::Other) =>
            {
                head = false;
            }
            Kind::Punct('=' | '+' | '-' | '*' | '/' | '%' | '^' | '|' | '&' | '<' | '>' | '!') if at_top && !block_item => head = false,
            Kind::Punct('(' | '[' | '{') => depth += 1,
            Kind::Punct(c @ (')' | ']' | '}')) => {
                if depth == 0 {
                    return j;
                }
                depth -= 1;
                if depth == 0 && *c == '}' && block_item && !inner && condition == Condition::Done && angle == 0 {
                    if toks.get(j + 1).is_some_and(|t| t.is_ident("else")) {
                        // `else if` has a condition of its own before its body; `else {` opens the body.
                        // The `if` is stepped over, so it does not read as an `if` inside the condition.
                        if toks.get(j + 2).is_some_and(|t| t.is_ident("if")) {
                            condition = Condition::Expression;
                            j += 3;
                        } else {
                            j += 1;
                        }
                        continue;
                    }
                    if nested > 0 {
                        return j + 1;
                    }
                    if toks.get(j + 1).is_some_and(|t| t.is_punct('.') || t.is_punct('?')) {
                        // A method call, field or `?` after the block continues the expression to its `;` or `,`.
                        block_item = false;
                        head = false;
                        j += 1;
                        continue;
                    }
                    let comma = toks.get(j + 1).is_some_and(|t| t.is_punct(','));
                    return if arm && comma { j + 2 } else { j + 1 };
                }
            }
            Kind::Punct(';') if at_top => return j + 1,
            Kind::Punct(',') if at_top && (arm || !block_item) => return j + 1,
            _ => {}
        }
        j += 1;
    }
    toks.len()
}

/// Mark the tokens of every item whose `cfg` attribute is false on `wasm32-unknown-unknown`.
///
/// An outer attribute (`#[cfg(...)]`) covers the item after it; an inner attribute (`#![cfg(...)]`)
/// covers the rest of the module or block it is in.
fn native_only(toks: &[Token]) -> Vec<bool> {
    cfg_covered(toks, Cover::NativeOnly)
}

/// Which `cfg` attributes [`cfg_covered`] marks the items of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cover {
    /// Every `cfg` attribute, whatever its predicate.
    Any,
    /// A `cfg` attribute whose predicate is false on `wasm32-unknown-unknown`, as [`native_only`] reads it.
    NativeOnly,
    /// A `cfg` attribute whose predicate is false on every configuration, such as `cfg(any())` or
    /// `cfg(false)`, so the item is never compiled.
    Never,
}

/// Mark the tokens of every item that a `cfg` attribute selected by `cover` covers.
fn cfg_covered(toks: &[Token], cover: Cover) -> Vec<bool> {
    let mut skip = vec![false; toks.len()];
    let mut i = 0;
    while i < toks.len() {
        if !toks[i].is_punct('#') {
            i += 1;
            continue;
        }
        let inner = toks.get(i + 1).is_some_and(|t| t.is_punct('!'));
        let open = if inner { i + 2 } else { i + 1 };
        if !toks.get(open).is_some_and(|t| t.is_punct('[')) {
            i += 1;
            continue;
        }
        let close = matching_close(toks, open);
        if close >= toks.len() {
            // An unclosed attribute: no later bracket closes it, so nothing after it is an item.
            break;
        }
        let excluded = toks.get(open + 1).is_some_and(|t| t.is_ident("cfg")) && toks.get(open + 2).is_some_and(|t| t.is_punct('(')) && {
            let mut pos = open + 3;
            cover == Cover::Any || eval_cfg(&toks[..close], &mut pos, 0, cover == Cover::Never) == Tri::False
        };
        if !excluded {
            i = close + 1;
            continue;
        }
        // Further outer attributes on the same item.
        let mut item = close + 1;
        while !inner && item < toks.len() && toks[item].is_punct('#') && toks.get(item + 1).is_some_and(|t| t.is_punct('[')) {
            item = matching_close(toks, item + 1).saturating_add(1);
        }
        let end = item_end(toks, item.min(toks.len()), inner);
        for s in skip.iter_mut().take(end).skip(i) {
            *s = true;
        }
        i = end.max(i + 1);
    }
    skip
}

/// The index of the keyword of every item that carries an outer `cfg` attribute of its own whose predicate
/// is not known to be true on `wasm32-unknown-unknown`, so the item may be absent from the web build: the
/// token after the item's outer attributes and its visibility (`pub`, `pub(...)`).
fn cfg_items(toks: &[Token]) -> BTreeSet<usize> {
    let mut items = BTreeSet::new();
    let mut i = 0;
    while i < toks.len() {
        let cfg = toks[i].is_punct('#')
            && toks.get(i + 1).is_some_and(|t| t.is_punct('['))
            && toks.get(i + 2).is_some_and(|t| t.is_ident("cfg"))
            && toks.get(i + 3).is_some_and(|t| t.is_punct('('));
        if !cfg {
            i += 1;
            continue;
        }
        let close = matching_close(toks, i + 1);
        if close >= toks.len() {
            break;
        }
        let mut pos = i + 4;
        if eval_cfg(&toks[..close], &mut pos, 0, false) == Tri::True {
            i = close + 1;
            continue;
        }
        let mut item = close + 1;
        while item < toks.len() && toks[item].is_punct('#') && toks.get(item + 1).is_some_and(|t| t.is_punct('[')) {
            item = matching_close(toks, item + 1).saturating_add(1);
        }
        if toks.get(item).is_some_and(|t| t.is_ident("pub")) {
            item += 1;
            if toks.get(item).is_some_and(|t| t.is_punct('(')) {
                item = matching_close(toks, item).saturating_add(1);
            }
        }
        if item < toks.len() {
            items.insert(item);
        }
        i = item.max(i + 1);
    }
    items
}

/// A module declaration, `mod name;` or the head of `mod name { ... }`.
#[derive(Debug, Clone)]
struct ModName {
    name: String,
    /// The string of a `#[path = "..."]` attribute on the declaration.
    path_attribute: Option<String>,
}

/// An out-of-line module declaration, `mod name;`.
#[derive(Debug)]
struct ModDecl {
    module: ModName,
    /// The inline modules (`mod a { ... }`) the declaration is inside, outermost first, shared between the
    /// declarations inside them.
    inline: Vec<Rc<ModName>>,
    /// Whether the `skip` tokens given to [`module_decls`] cover the declaration: for [`std_instant`], an
    /// item whose `cfg` is false on `wasm32-unknown-unknown`, and for [`predefined_quit`], one that is never
    /// compiled.
    native: bool,
}

/// The most tokens [`path_attribute`] reads back from one `mod` keyword, so that hostile nesting cannot
/// make the scan quadratic.
const ATTRIBUTE_SCAN_LIMIT: usize = 4096;

/// The index of the bracket `open` that matches the bracket `close` at `toks[end - 1]`, reading back
/// from `end`, or `None` when it is not found within `budget` tokens.
fn matching_open_back(toks: &[Token], end: usize, open: char, close: char, budget: &mut usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = end;
    while j > 0 && *budget > 0 {
        j -= 1;
        *budget -= 1;
        if toks[j].is_punct(close) {
            depth += 1;
        } else if toks[j].is_punct(open) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// The string of a `#[path = "..."]` outer attribute on the item whose `mod` keyword is at `toks[m]`.
fn path_attribute(toks: &[Token], m: usize) -> Option<String> {
    let mut budget = ATTRIBUTE_SCAN_LIMIT;
    let mut k = m;
    if k > 0 && toks[k - 1].is_punct(')') {
        // A visibility such as `pub(crate)`.
        let j = matching_open_back(toks, k, '(', ')', &mut budget)?;
        if j > 0 && toks[j - 1].is_ident("pub") {
            k = j - 1;
        }
    } else if k > 0 && toks[k - 1].is_ident("pub") {
        k -= 1;
    }
    let mut found = None;
    while k > 0 && toks[k - 1].is_punct(']') {
        let Some(j) = matching_open_back(toks, k, '[', ']', &mut budget) else { break };
        if j == 0 || !toks[j - 1].is_punct('#') {
            break;
        }
        if toks.get(j + 1).is_some_and(|t| t.is_ident("path"))
            && toks.get(j + 2).is_some_and(|t| t.is_punct('='))
            && let Some(Kind::Str(s)) = toks.get(j + 3).map(|t| &t.kind)
        {
            found = Some(s.clone());
        }
        k = j - 1;
    }
    found
}

/// What an open brace in [`module_decls`] opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Brace {
    /// A block that is not an inline module.
    Block,
    /// An inline module, pushed on the inline stack.
    Module,
    /// An inline module deeper than [`MAX_NESTING`], not tracked.
    TooDeep,
}

/// The name of a module's file or folder: a raw identifier such as `r#match` names `match`.
fn module_name(name: &str) -> String {
    name.strip_prefix("r#").unwrap_or(name).to_owned()
}

/// Every out-of-line module declaration in a file, in one pass that tracks the inline modules around it.
///
/// Declarations inside more than [`MAX_NESTING`] inline modules are left out, so their files are checked.
fn module_decls(toks: &[Token], skip: &[bool]) -> Vec<ModDecl> {
    let mut decls = Vec::new();
    let mut braces: Vec<Brace> = Vec::new();
    let mut inline: Vec<Rc<ModName>> = Vec::new();
    let mut too_deep = 0usize;
    for (i, tok) in toks.iter().enumerate() {
        match &tok.kind {
            Kind::Punct('{') => {
                let brace = match i.checked_sub(2).map(|j| (&toks[j], &toks[j + 1].kind)) {
                    Some((m, Kind::Ident(name))) if m.is_ident("mod") => {
                        if too_deep == 0 && inline.len() < MAX_NESTING {
                            inline.push(Rc::new(ModName { name: module_name(name), path_attribute: path_attribute(toks, i - 2) }));
                            Brace::Module
                        } else {
                            too_deep += 1;
                            Brace::TooDeep
                        }
                    }
                    _ => Brace::Block,
                };
                braces.push(brace);
            }
            Kind::Punct('}') => match braces.pop() {
                Some(Brace::Module) => {
                    inline.pop();
                }
                Some(Brace::TooDeep) => too_deep -= 1,
                _ => {}
            },
            Kind::Ident(k) if k == "mod" && too_deep == 0 => {
                if let Some(Kind::Ident(name)) = toks.get(i + 1).map(|t| &t.kind)
                    && toks.get(i + 2).is_some_and(|t| t.is_punct(';'))
                {
                    decls.push(ModDecl {
                        module: ModName { name: module_name(name), path_attribute: path_attribute(toks, i) },
                        inline: inline.clone(),
                        native: skip.get(i).copied().unwrap_or(false),
                    });
                }
            }
            _ => {}
        }
    }
    decls
}

/// A path with `\` read as `/`, empty and `.` components removed and `..` applied to the component before it.
fn normalize_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if absolute { format!("/{joined}") } else { joined }
}

/// `child` below the folder `parent`, normalized. A `child` that is an absolute path (`/x`, `\x` or `C:...`)
/// stands alone. `\\` is read as one separator, since a string literal keeps its escapes.
fn join_path(parent: &str, child: &str) -> String {
    let child = child.replace("\\\\", "/");
    if parent.is_empty() || child.starts_with('/') || child.starts_with('\\') || child.as_bytes().get(1) == Some(&b':') {
        normalize_path(&child)
    } else {
        normalize_path(&format!("{parent}/{child}"))
    }
}

/// The paths, normalized, where the module `decl` declared in the file at `parent` (normalized) can be.
///
/// The modules of `lib.rs`, `main.rs` and `mod.rs`, and with `path_loaded`, of a file that a `#[path]`
/// attribute names, are in the file's folder, and those of any other file are in the folder named after
/// its stem. Each inline module around the declaration adds a folder named
/// after it, or with a `path` attribute, moves to that path relative to the current folder (without the
/// stem folder). The module is then `<name>.rs` or `<name>/mod.rs` there, or with a `path` attribute, that
/// path relative to the current folder, again without the stem folder.
fn module_files(parent: &str, decl: &ModDecl, path_loaded: bool) -> Vec<String> {
    let (dir, file) = match parent.rsplit_once('/') {
        Some(("", file)) => ("/", file),
        Some(split) => split,
        None => ("", parent),
    };
    let mut folder = dir.to_string();
    let mut stem = (!path_loaded && !matches!(file, "lib.rs" | "main.rs" | "mod.rs")).then(|| file.strip_suffix(".rs").unwrap_or(file));
    for inline in &decl.inline {
        if let Some(attr) = &inline.path_attribute {
            folder = join_path(&folder, attr);
            stem = None;
        } else {
            if let Some(stem) = stem.take() {
                folder = join_path(&folder, stem);
            }
            folder = join_path(&folder, &inline.name);
        }
    }
    if let Some(attr) = &decl.module.path_attribute {
        return vec![join_path(&folder, attr)];
    }
    if let Some(stem) = stem {
        folder = join_path(&folder, stem);
    }
    let name = &decl.module.name;
    vec![join_path(&folder, &format!("{name}.rs")), join_path(&folder, &format!("{name}/mod.rs"))]
}

/// For each file, whether it is native-only: a `mod` declaration in a native-only item, or any declaration
/// in a native-only file, names it, and no chain of declarations outside native-only items reaches it from
/// a crate root (a file that no declaration names, or in a cycle of declarations that no file outside the
/// cycle names, its `lib.rs` or `main.rs`, or every member when it has neither).
fn native_only_files(paths: &[String], decls: &[Vec<ModDecl>]) -> Vec<bool> {
    let mut index: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        index.entry(path.as_str()).or_default().push(i);
    }
    // rustc reads a file that a `#[path]` attribute names as if it were a `mod.rs`, so the modules it
    // declares are in its own folder rather than its stem folder. Each file is therefore two nodes: node
    // `2 * file` for the file loaded by a plain `mod name;` (or as a crate root), and node `2 * file + 1`
    // for the file loaded through a `#[path]` attribute. Each node lists the nodes its declarations name,
    // with whether the declaration is in a native-only item.
    let edges: Vec<Vec<(bool, usize)>> = (0..paths.len() * 2)
        .map(|node| {
            let (parent, path_loaded) = (node / 2, node % 2 == 1);
            let mut targets = Vec::new();
            for decl in &decls[parent] {
                let by_path = usize::from(decl.module.path_attribute.is_some());
                for candidate in module_files(&paths[parent], decl, path_loaded) {
                    for &j in index.get(candidate.as_str()).into_iter().flatten() {
                        targets.push((decl.native, 2 * j + by_path));
                    }
                }
            }
            targets
        })
        .collect();

    // A group of files that name each other in a cycle, and that no file outside the group names, has no
    // member that a crate root can reach. rustc does not load a cycle of enabled declarations, so such a
    // cycle runs through a declaration the build leaves out. When the group holds a `lib.rs` or `main.rs`,
    // those files are its crate roots and the declarations that name them are read as left out; otherwise
    // no declaration of the group names its own members, and each of them can be a crate root.
    let file_edges: Vec<Vec<usize>> =
        (0..paths.len()).map(|file| edges[2 * file].iter().chain(&edges[2 * file + 1]).map(|&(_, target)| target / 2).collect()).collect();
    let component = components(&file_edges);
    let root_file = |file: usize| matches!(paths[file].rsplit('/').next(), Some("lib.rs" | "main.rs"));
    let mut entered = vec![false; paths.len()];
    let mut has_root = vec![false; paths.len()];
    for (file, targets) in file_edges.iter().enumerate() {
        has_root[component[file]] |= root_file(file);
        for &target in targets {
            if component[target] != component[file] {
                entered[component[target]] = true;
            }
        }
    }
    let names = |node: usize, target: usize| {
        let group = component[target / 2];
        component[node / 2] != group || entered[group] || (has_root[group] && !root_file(target / 2))
    };

    // The nodes that are loaded: a file that no loaded node names is a crate root, loaded plainly, and a
    // node is loaded when a loaded node names it. Every file starts out loaded plainly, and the set is
    // narrowed until it stops changing, or after `MAX_MODULE_ROUNDS` rounds.
    let mut loaded: Vec<bool> = (0..paths.len() * 2).map(|node| node % 2 == 0).collect();
    let mut named = vec![false; paths.len()];
    for _ in 0..MAX_MODULE_ROUNDS {
        let mut reached = vec![false; paths.len() * 2];
        for (node, targets) in edges.iter().enumerate() {
            if loaded[node] {
                for &(_, target) in targets {
                    if names(node, target) {
                        reached[target] = true;
                    }
                }
            }
        }
        named = (0..paths.len()).map(|j| reached[2 * j] || reached[2 * j + 1]).collect();
        let next: Vec<bool> = (0..paths.len() * 2).map(|node| reached[node] || (node % 2 == 0 && !named[node / 2])).collect();
        if next == loaded {
            break;
        }
        loaded = next;
    }

    // Files the web build compiles: crate roots, and every file a declaration outside a native-only item
    // of such a file names.
    let mut web = vec![false; paths.len() * 2];
    let mut queue: Vec<usize> = (0..paths.len()).filter(|&j| !named[j]).map(|j| 2 * j).collect();
    for &node in &queue {
        web[node] = true;
    }
    while let Some(parent) = queue.pop() {
        for &(_, target) in edges[parent].iter().filter(|(is_native, _)| !is_native) {
            if !web[target] {
                web[target] = true;
                queue.push(target);
            }
        }
    }

    // Files a native-only declaration of a loaded node names, and their descendants.
    let mut native = vec![false; paths.len() * 2];
    let starts = edges.iter().enumerate().filter(|&(node, _)| loaded[node]).flat_map(|(_, targets)| targets.iter());
    for &(_, target) in starts.filter(|(is_native, _)| *is_native) {
        if !native[target] {
            native[target] = true;
            queue.push(target);
        }
    }
    while let Some(parent) = queue.pop() {
        for &(_, target) in &edges[parent] {
            if !native[target] {
                native[target] = true;
                queue.push(target);
            }
        }
    }
    (0..paths.len()).map(|j| (native[2 * j] || native[2 * j + 1]) && !web[2 * j] && !web[2 * j + 1]).collect()
}

/// The most rounds [`native_only_files`] spends narrowing the set of loaded files.
const MAX_MODULE_ROUNDS: usize = 16;

/// For each node of the graph `edges`, the number of its strongly connected component (Tarjan's
/// algorithm, with an explicit stack in place of recursion).
fn components(edges: &[Vec<usize>]) -> Vec<usize> {
    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; edges.len()];
    let mut low = vec![0; edges.len()];
    let mut on_stack = vec![false; edges.len()];
    let mut component = vec![UNSEEN; edges.len()];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut count = 0;
    for root in 0..edges.len() {
        if index[root] != UNSEEN {
            continue;
        }
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        // Each frame is a node and the position of the next edge of it to follow.
        let mut work = vec![(root, 0usize)];
        while let Some(frame) = work.last_mut() {
            let node = frame.0;
            if let Some(&target) = edges[node].get(frame.1) {
                frame.1 += 1;
                if index[target] == UNSEEN {
                    index[target] = next;
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    work.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component[member] = count;
                    if member == node {
                        break;
                    }
                }
                count += 1;
            }
        }
    }
    component
}

/// The innermost `{` around each token, and for each `}` the `{` it closes.
fn brace_scopes(toks: &[Token]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut around = vec![None; toks.len()];
    let mut opener = vec![None; toks.len()];
    let mut open = Vec::new();
    for (j, t) in toks.iter().enumerate() {
        around[j] = open.last().copied();
        if t.is_punct('{') {
            open.push(j);
        } else if t.is_punct('}') {
            opener[j] = open.pop();
        }
    }
    (around, opener)
}

/// Look a name up from `toks[at]` outward through the braces around it (`around`, from
/// [`brace_scopes`]): the first value that `get` returns for a pair of braces, given the index of its
/// `{`, or for the file, given `None`. The search ends with the body of the `mod` it starts in, since a
/// module does not see the names its parent binds. `Some(None)` means no braces bind the name; `None`
/// means the search passed [`MAX_NESTING`] levels.
fn lookup<T>(toks: &[Token], around: &[Option<usize>], at: usize, get: impl Fn(Option<usize>) -> Option<T>) -> Option<Option<T>> {
    let mut scope = around.get(at).copied().flatten();
    for _ in 0..=MAX_NESTING {
        if let Some(value) = get(scope) {
            return Some(Some(value));
        }
        match scope {
            Some(o) if o >= 2 && toks[o - 2].is_ident("mod") && matches!(toks[o - 1].kind, Kind::Ident(_)) => return Some(None),
            Some(o) => scope = around[o],
            None => return Some(None),
        }
    }
    None
}

/// Add one finding for each line in `lines`, in line order, with the trimmed text of that line.
fn report(findings: &mut Vec<Finding>, path: &str, content: &str, lines: &BTreeSet<usize>) {
    if lines.is_empty() {
        return;
    }
    let text: Vec<&str> = content.lines().collect();
    for &line in lines {
        let line_text = text.get(line.saturating_sub(1)).map_or("", |s| s.trim());
        findings.push(Finding::new(path, line, line_text));
    }
}

/// For each pair of braces (`None` for the file) and a name: whether a binding in those braces binds the
/// name to the muda crate, and whether one binds it to something else.
type CrateBindings<'a> = BTreeMap<(&'a str, Option<usize>), (bool, bool)>;

/// For each token, whether it is in a macro's input, `m!(...)`, `m![...]` or `m! { ... }`, in an
/// attribute, `#[...]` or `#![...]`, or in the body of a `macro_rules!` definition, whose templates are not
/// expanded where they are written. Tokens in a block inside other parentheses or brackets, such as a call's
/// argument, are not marked.
fn macro_tokens(toks: &[Token]) -> Vec<bool> {
    let mut marked = Vec::with_capacity(toks.len());
    // For each open delimiter, whether it starts a macro's input, an attribute or a `macro_rules!` body; and
    // how many of those are open.
    let mut open = Vec::new();
    let mut in_macro = 0usize;
    for i in 0..toks.len() {
        if toks[i].is_punct('(') || toks[i].is_punct('[') || toks[i].is_punct('{') {
            let ident = |k: usize| matches!(toks[k].kind, Kind::Ident(_));
            let definition = i >= 3 && ident(i - 1) && toks[i - 2].is_punct('!') && toks[i - 3].is_ident("macro_rules");
            let input = i >= 2 && toks[i - 1].is_punct('!') && ident(i - 2);
            let attribute =
                toks[i].is_punct('[') && i >= 1 && (toks[i - 1].is_punct('#') || (i >= 2 && toks[i - 1].is_punct('!') && toks[i - 2].is_punct('#')));
            let skip = input || definition || attribute;
            open.push(skip);
            in_macro += usize::from(skip);
        } else if (toks[i].is_punct(')') || toks[i].is_punct(']') || toks[i].is_punct('}')) && open.pop() == Some(true) {
            in_macro = in_macro.saturating_sub(1);
        }
        marked.push(in_macro > 0);
    }
    marked
}

/// Each `extern crate c;` or `extern crate c as n;` in a file: whether `c` is muda, the name it binds, and
/// the index of that name. A declaration in an item that `never` marks (see [`Cover::Never`]) is left out,
/// and so is one in a macro's input, an attribute or a `macro_rules!` body (see [`macro_tokens`]).
fn extern_crates<'a>(toks: &'a [Token], never: &[bool]) -> Vec<(bool, &'a str, usize)> {
    let mut externs = Vec::new();
    let in_macro = macro_tokens(toks);
    for i in 0..toks.len() {
        if !in_macro[i]
            && !never.get(i).copied().unwrap_or(false)
            && toks[i].is_ident("extern")
            && toks.get(i + 1).is_some_and(|t| t.is_ident("crate"))
            && let Some(Kind::Ident(krate)) = toks.get(i + 2).map(|t| &t.kind)
        {
            let at = if toks.get(i + 3).is_some_and(|t| t.is_ident("as")) { i + 4 } else { i + 2 };
            if let Some(Kind::Ident(name)) = toks.get(at).map(|t| &t.kind)
                && name != "_"
            {
                externs.push((krate == "muda", name.as_str(), at));
            }
        }
    }
    externs
}

/// For each of `paths`, the names that an `extern crate muda as m;` outside any braces of its crate root adds
/// to the crate's extern prelude, so that a path starting with `::m` names muda anywhere in the crate.
///
/// The crate roots follow Cargo's layout: `src/lib.rs` and `src/main.rs`, except one below another package's
/// `src` folder that a declaration outside its own package folder names; each file that [`cargo_target`]
/// accepts; another `build.rs`, `lib.rs` or `main.rs` when no `mod` declaration in another of the files names
/// it, plainly or through a `#[path]` attribute; and each other file directly in a `bin`, `examples`, `tests`
/// or `benches` folder other than `mod.rs` when no package folder holds it and no declaration without a
/// `#[path]` attribute names it. A root's
/// names reach every file that a chain of declarations
/// from it names, where a crate root and a file loaded through a `#[path]` attribute have their modules in their
/// own folder, as [`module_files`] reads them. A declaration in an item whose `cfg` is false on every
/// configuration neither adds a name nor loads a file.
fn extern_prelude_names(paths: &[String], contents: &[&str]) -> Vec<BTreeSet<String>> {
    let mut index: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        index.entry(path.as_str()).or_default().push(i);
    }
    let mut decls: Vec<Vec<ModDecl>> = Vec::with_capacity(paths.len());
    let mut aliases: Vec<Vec<String>> = Vec::with_capacity(paths.len());
    for content in contents {
        let toks = lex_rust(content);
        // A declaration in an item that is never compiled loads no file, and its alias is not in the prelude.
        let never = cfg_covered(&toks, Cover::Never);
        // Nor does one in a macro's input, an attribute or a `macro_rules!` body.
        let skip: Vec<bool> = macro_tokens(&toks).into_iter().zip(&never).map(|(in_macro, &never)| in_macro || never).collect();
        decls.push(module_decls(&toks, &skip).into_iter().filter(|decl| !decl.native).collect());
        let (around, _) = brace_scopes(&toks);
        aliases.push(
            extern_crates(&toks, &never)
                .into_iter()
                .filter(|&(muda, _, at)| muda && around[at].is_none())
                .map(|(_, name, _)| name.to_owned())
                .collect(),
        );
    }
    // Node `2 * file` is the file loaded plainly (or as a crate root) and node `2 * file + 1` the file loaded
    // through a `#[path]` attribute. Each node lists the nodes its declarations name.
    let mut named = vec![false; paths.len()];
    let mut named_plainly = vec![false; paths.len()];
    // Whether a `src/lib.rs` or `src/main.rs` is named by a declaration in a file other than its package's
    // `build.rs` and the files in its `src`, `tests`, `examples` and `benches` folders.
    let mut named_from_outside = vec![false; paths.len()];
    let edges: Vec<Vec<usize>> = (0..paths.len() * 2)
        .map(|node| {
            let (file, path_loaded) = (node / 2, node % 2 == 1);
            let mut targets = Vec::new();
            for decl in &decls[file] {
                let by_path = usize::from(decl.module.path_attribute.is_some());
                for candidate in module_files(&paths[file], decl, path_loaded) {
                    for &j in index.get(candidate.as_str()).into_iter().flatten() {
                        if j != file {
                            named[j] = true;
                            named_plainly[j] |= by_path == 0;
                            let package = paths[j].strip_suffix("src/lib.rs").or_else(|| paths[j].strip_suffix("src/main.rs"));
                            let own = |package: &str| {
                                paths[file].strip_prefix(package).is_some_and(|rest| {
                                    rest == "build.rs" || ["src/", "tests/", "examples/", "benches/"].iter().any(|dir| rest.starts_with(dir))
                                })
                            };
                            named_from_outside[j] |= package.is_none_or(|package| !own(package));
                            targets.push(2 * j + by_path);
                        }
                    }
                }
            }
            targets
        })
        .collect();
    let packages = package_folders(paths);
    // Whether `path` is below the `src` folder of a package, other than directly in it.
    let in_package_src = |path: &str| {
        packages.iter().any(|package| {
            let src = if package.is_empty() { "src/".to_owned() } else { format!("{package}/src/") };
            path.strip_prefix(src.as_str()).is_some_and(|rest| rest.contains('/'))
        })
    };
    let root = |file: usize| {
        if cargo_target(&paths[file], &packages) {
            return true;
        }
        let mut segments = paths[file].rsplit('/');
        match (segments.next(), segments.next()) {
            // A `src/lib.rs` that a declaration in a package's `src` folder loads is a module there.
            (Some("lib.rs" | "main.rs"), Some("src")) => !(named_from_outside[file] && in_package_src(&paths[file])),
            (Some("lib.rs" | "main.rs" | "build.rs"), _) => !named[file],
            // In a known package, `cargo_target` has already placed the target folders.
            (Some(name), Some("bin" | "examples" | "tests" | "benches")) => {
                name.ends_with(".rs") && name != "mod.rs" && !named_plainly[file] && package_relative(&paths[file], &packages).is_none()
            }
            _ => false,
        }
    };
    let mut prelude = vec![BTreeSet::new(); paths.len()];
    for start in (0..paths.len()).filter(|&file| !aliases[file].is_empty() && root(file)) {
        // A crate root declares its modules in its own folder, as a file loaded through `#[path]` does, so
        // `mod child;` in `src/bin/tool.rs` names `src/bin/child.rs`.
        let mut seen = vec![false; paths.len() * 2];
        let mut queue = vec![2 * start + 1];
        seen[2 * start + 1] = true;
        while let Some(node) = queue.pop() {
            prelude[node / 2].extend(aliases[start].iter().cloned());
            for &next in &edges[node] {
                if !seen[next] {
                    seen[next] = true;
                    queue.push(next);
                }
            }
        }
    }
    prelude
}

/// The package folders among `paths`: each folder that holds a `src/lib.rs` or `src/main.rs`, and each folder
/// that holds a file directly in `src/bin` and is not inside the `src` folder of one of the first kind. A
/// relative `src/lib.rs` gives the empty folder.
fn package_folders(paths: &[String]) -> BTreeSet<&str> {
    fn folder(dir: &str) -> Option<&str> {
        dir.strip_suffix('/').or_else(|| dir.is_empty().then_some(""))
    }
    let roots: BTreeSet<&str> =
        paths.iter().filter_map(|path| path.strip_suffix("src/lib.rs").or_else(|| path.strip_suffix("src/main.rs"))).filter_map(folder).collect();
    let in_src = |dir: &str| {
        roots.iter().any(|root| {
            let src = if root.is_empty() { "src".to_owned() } else { format!("{root}/src") };
            dir == src || dir.starts_with(&format!("{src}/"))
        })
    };
    let bins: Vec<&str> = paths
        .iter()
        .filter_map(|path| path.rsplit_once('/').map(|(dir, _)| dir))
        .filter_map(|dir| dir.strip_suffix("src/bin"))
        .filter_map(folder)
        .filter(|dir| !in_src(dir))
        .collect();
    roots.into_iter().chain(bins).collect()
}

/// Whether Cargo builds `path` as a crate root of its own, also when another file declares it as a module:
/// each file directly in a `src/bin` folder and each `main.rs` one folder below it; and in a package folder,
/// `build.rs`, each file directly in its `tests`, `examples` or `benches` folder, and each `main.rs` one
/// folder below those. Cargo's auto-discovery takes a `mod.rs` directly in those folders as a target too.
/// The package folder is the nearest folder above `path` in `packages` (see [`package_folders`]). Without
/// one, a package folder is one that is not directly inside `src` and has no `bin`, `tests`, `examples` or
/// `benches` folder above it, so a folder of that name inside `src` or inside one of those folders holds
/// modules. With one, only those places directly below it count, so `src/foo/tests/helper.rs` and
/// `src/foo/src/bin/helper.rs` hold modules.
fn cargo_target(path: &str, packages: &BTreeSet<&str>) -> bool {
    let inside = package_relative(path, packages);
    let mut segments = inside.unwrap_or(path).rsplit('/');
    let Some(name) = segments.next() else { return false };
    if !name.ends_with(".rs") {
        return false;
    }
    let folders: Vec<&str> = segments.collect();
    if inside.is_some() {
        return match folders.as_slice() {
            ["bin", "src"] | ["tests" | "examples" | "benches"] => true,
            [_, "bin", "src"] | [_, "tests" | "examples" | "benches"] => name == "main.rs",
            [] => name == "build.rs",
            _ => false,
        };
    }
    let package = |rest: &[&str]| rest.first() != Some(&"src") && !rest.iter().any(|s| matches!(*s, "bin" | "tests" | "examples" | "benches"));
    match folders.as_slice() {
        ["bin", "src", ..] => true,
        [_, "bin", "src", ..] if name == "main.rs" => true,
        ["tests" | "examples" | "benches", rest @ ..] => package(rest),
        [_, "tests" | "examples" | "benches", rest @ ..] if name == "main.rs" => package(rest),
        rest if name == "build.rs" => package(rest),
        _ => false,
    }
}

/// `path` relative to the nearest folder above it in `packages` (see [`package_folders`]), or `None` when no
/// package folder holds it.
fn package_relative<'a>(path: &'a str, packages: &BTreeSet<&str>) -> Option<&'a str> {
    let inside = path.rmatch_indices('/').map(|(i, _)| i).find(|&i| i > 0 && packages.contains(&path[..i])).map(|i| &path[i + 1..]);
    inside.or_else(|| (packages.contains("") && !path.starts_with('/')).then_some(path))
}

/// The names of the muda crate in a file, and where each is bound. The names are `muda` itself, each name
/// in `prelude` (see [`extern_prelude_names`]), which names the crate as `muda` does, and each name that
/// `use muda as m;`, `use muda::{self as m};` or `extern crate muda as m;` gives it. For each
/// name and each pair of braces, the bindings record whether a `use` or an `extern crate` in them binds
/// the name to the crate, and whether a `use`, an `extern crate` or a `mod` in them binds it to something
/// else. A binding to something else inside an item with a `cfg` attribute, or through `crate::`, `self::`
/// or `super::` (which may be a re-export of the crate), is left out, and so is an `extern crate` in an
/// item that `never` marks, in a macro's input, in an attribute or in a `macro_rules!` body (see [`extern_crates`]).
fn muda_crate_bindings<'a>(
    toks: &'a [Token],
    leaves: &'a [UseLeaf],
    around: &[Option<usize>],
    gated: &[bool],
    never: &[bool],
    prelude: &'a BTreeSet<String>,
) -> (BTreeSet<&'a str>, CrateBindings<'a>) {
    let externs = extern_crates(toks, never);
    // Whether a `use` path names the crate itself: `muda`, or a name of the crate in the extern prelude.
    let crate_path = |leaf: &UseLeaf| leaf.path.len() == 1 && (leaf.path[0] == "muda" || prelude.contains(&leaf.path[0]));
    let mut names: BTreeSet<&'a str> = BTreeSet::from(["muda"]);
    names.extend(prelude.iter().map(String::as_str));
    names.extend(leaves.iter().filter(|leaf| crate_path(leaf)).filter_map(UseLeaf::binding));
    names.extend(externs.iter().filter(|&&(muda, _, _)| muda).map(|&(_, name, _)| name));
    let mut bound: CrateBindings<'a> = BTreeMap::new();
    // `use muda as m;` binds `m` to the crate only where `muda` itself names the crate, so it is read
    // after every other binding. The same holds for a name of the crate in the extern prelude.
    let renames = |leaf: &UseLeaf| crate_path(leaf) && leaf.binding() != Some(leaf.path[0].as_str());
    for leaf in leaves.iter().filter(|leaf| !renames(leaf)) {
        let Some(name) = leaf.binding().filter(|name| names.contains(name)) else { continue };
        let local = leaf.root.as_deref().is_some_and(|s| matches!(s, "crate" | "self" | "super"));
        let entry = bound.entry((name, around.get(leaf.start).copied().flatten()));
        if crate_path(leaf) {
            entry.or_default().0 = true;
        } else if !local && !gated.get(leaf.start).copied().unwrap_or(false) {
            entry.or_default().1 = true;
        }
    }
    for &(muda, name, at) in &externs {
        if !names.contains(name) {
            continue;
        }
        if muda {
            bound.entry((name, around[at])).or_default().0 = true;
        } else if !gated[at] {
            bound.entry((name, around[at])).or_default().1 = true;
        }
    }
    for (i, t) in toks.iter().enumerate() {
        if let Kind::Ident(name) = &t.kind
            && names.contains(name.as_str())
            && i.checked_sub(1).is_some_and(|k| toks[k].is_ident("mod"))
            && !gated[i]
        {
            bound.entry((name, around[i])).or_default().1 = true;
        }
    }
    let mut renamed = Vec::new();
    for leaf in leaves.iter().filter(|leaf| renames(leaf)) {
        let Some(name) = leaf.binding() else { continue };
        let crate_here = leaf.absolute
            || !matches!(lookup(toks, around, leaf.start, |scope| bound.get(&(leaf.path[0].as_str(), scope)).copied()), Some(Some((false, true))));
        let local = leaf.root.as_deref().is_some_and(|s| matches!(s, "crate" | "self" | "super"));
        let scope = around.get(leaf.start).copied().flatten();
        if crate_here {
            renamed.push((name, scope, true));
        } else if !local && !gated.get(leaf.start).copied().unwrap_or(false) {
            renamed.push((name, scope, false));
        }
    }
    for (name, scope, muda) in renamed {
        let entry = bound.entry((name, scope)).or_default();
        if muda {
            entry.0 = true;
        } else {
            entry.1 = true;
        }
    }
    (names, bound)
}

/// Guard: detects direct calls to `muda`'s predefined Quit menu item.
///
/// Using `PredefinedMenuItem::quit` connects the macOS menu directly to `NSApplication terminate:`,
/// which bypasses dirty document checks, unsaved changes save prompts, and lifecycle cleanup.
/// Applications should route Quit through custom menu items or interceptors connected to `craft-quit`.
///
/// The guard reads Rust tokens, so comments, strings and character literals are ignored, and whitespace
/// or line breaks around `::` do not hide a call. A qualified path such as `<muda::PredefinedMenuItem>::quit`
/// is reported too. A path that names the type through another crate or module, such as
/// `other::PredefinedMenuItem::quit`, is not, and neither is a path through a name `muda` that the innermost
/// braces binding it bind only to something else (`use other_menu as muda;`, `mod muda`), read like a name
/// used on its own below; `::muda::PredefinedMenuItem` always names the crate. A name that the crate is
/// imported under (`use muda as menu;`, `use muda::{self as menu};`, `extern crate muda as menu;`) is read
/// like `muda` in the braces that bind it, so `menu::PredefinedMenuItem::quit` is reported there. An
/// `extern crate muda as menu;` outside any braces of a crate root (`src/lib.rs` and `src/main.rs`; `build.rs`,
/// another `lib.rs` or `main.rs`, or a file outside any package folder directly in a `bin`, `examples`, `tests`
/// or `benches` folder other than `mod.rs`, unless a `mod` declaration in another of `files` names it, for a file in those folders one
/// without a `#[path]` attribute; a file directly in `src/bin`, a package's `build.rs`, a file directly in its
/// `tests`, `examples` or `benches` folder, `mod.rs` included, and a `main.rs` one folder below those, are crate
/// roots even when a declaration names them, where the package folder is the nearest folder above the file
/// that holds a `src/lib.rs` or `src/main.rs` of `files`, and without one, a folder not directly inside `src`
/// with no `bin`, `tests`, `examples` or `benches` folder above it) also adds `menu` to
/// the crate's extern prelude, so in that file and in every file of `files` that a chain of `mod`
/// declarations from it names, `menu` is read like `muda` and `::menu::PredefinedMenuItem::quit` is
/// reported. An `extern crate` or `mod` declaration in an item whose `cfg` is false on every configuration
/// (`cfg(any())`, `cfg(false)`) is read as absent, and so are the tokens of an `extern crate` in a macro's
/// input (`m!(...)`, `m![...]`, `m! { ... }`), in an attribute or in a `macro_rules!` body. Elsewhere a path that starts with `::` names the crate
/// only through `muda`. It also
/// follows a `use` or a `type` alias that renames muda's type (`use muda::PredefinedMenuItem as Item;` then
/// `Item::quit`); a rename of another crate's `PredefinedMenuItem` is not followed, including one through a
/// name `muda` that the braces around the `use` or `type` bind to something else, read as for a qualified
/// path.
///
/// A name used on its own (`PredefinedMenuItem::quit`, `Item::quit`) is read in the innermost braces
/// around it, within its module, or the file, that bind the name. It is reported when those braces
/// import or alias muda's type under it, and not when they only import another type under it
/// (`use other::PredefinedMenuItem;`) or define one (`struct PredefinedMenuItem`). Where no braces bind
/// it, `PredefinedMenuItem` is reported and a renamed name is not. An import or definition inside an
/// item with a `cfg` attribute, an import or a type alias through `crate::`, `self::` or `super::`
/// (which may be a re-export of muda's type), and an associated `type` in an `impl` or `trait` do not
/// bind the name. Each finding names the line where the type's name starts, once per line, in line order.
pub fn predefined_quit<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let files: Vec<I::Item> = files.into_iter().collect();
    let paths: Vec<String> = files.iter().map(|file| normalize_path(file.path())).collect();
    let contents: Vec<&str> = files.iter().map(|file| file.content()).collect();
    let prelude = extern_prelude_names(&paths, &contents);
    let mut findings = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let path = file.path();
        let content = file.content();
        let toks = lex_rust(content);
        let leaves = use_leaves(&toks, &BTreeSet::new());
        let (around, opener) = brace_scopes(&toks);
        let gated = cfg_covered(&toks, Cover::Any);
        let never = cfg_covered(&toks, Cover::Never);
        let (crate_names, crate_bound) = muda_crate_bindings(&toks, &leaves, &around, &gated, &never, &prelude[index]);
        let names_crate = |s: &str| crate_names.contains(s);
        // Whether `name`, read at token `i`, names the crate. After a leading `::`, `muda` does, and so
        // does a name that `extern crate muda as name;` outside any braces of the file's crate root gives it
        // (a name in the extern prelude). Otherwise `muda` and a name in the extern prelude do unless the
        // innermost braces around `i` that bind the name bind it only to something else, and another name
        // of the crate does when those braces bind it to the crate, or only to nothing else.
        let in_prelude = |name: &str| name == "muda" || prelude[index].contains(name);
        let crate_name_at = |name: &str, i: usize, absolute: bool| {
            if absolute || !names_crate(name) {
                return in_prelude(name);
            }
            match lookup(&toks, &around, i, |scope| crate_bound.get(&(name, scope)).copied()) {
                None => true,
                Some(Some((muda, other))) => muda || !other,
                Some(None) => in_prelude(name),
            }
        };
        // Whether the identifier at `i` names the crate.
        let crate_is_muda = |i: usize| {
            let Kind::Ident(name) = &toks[i].kind else { return false };
            let absolute = i >= 2 && toks[i - 1].is_punct(':') && toks[i - 2].is_punct(':');
            crate_name_at(name, i, absolute)
        };
        // A `use` path to muda's type, read through a name of the crate bound where the `use` is. After
        // a leading `::` the first segment is an external crate, so only a name in the extern prelude names it.
        let is_muda =
            |leaf: &UseLeaf| leaf.path.len() == 2 && leaf.path[1] == "PredefinedMenuItem" && crate_name_at(&leaf.path[0], leaf.start, leaf.absolute);
        // Whether the braces opened at `o` are the body of an `impl` or a `trait`, whose `type` items are
        // associated types rather than names in scope. The head is read back from `o` to the `;`, `{` or
        // `}` before it, stepping over a block in angle brackets (`impl Foo<{ N }>`), for at most
        // `ATTRIBUTE_SCAN_LIMIT` tokens.
        let assoc_body = |o: usize| {
            let mut j = o;
            let mut depth = 0usize;
            let mut start = None;
            for _ in 0..ATTRIBUTE_SCAN_LIMIT {
                let Some(k) = j.checked_sub(1) else {
                    start = Some(0);
                    break;
                };
                let t = &toks[k];
                if depth == 0 && (t.is_punct(';') || t.is_punct('{')) {
                    start = Some(k + 1);
                    break;
                }
                if depth == 0 && t.is_punct('}') {
                    let generic = opener[k].filter(|&b| b.checked_sub(1).is_some_and(|p| ['<', ',', '='].iter().any(|&c| toks[p].is_punct(c))));
                    if let Some(b) = generic {
                        j = b;
                        continue;
                    }
                    start = Some(k + 1);
                    break;
                }
                match t.kind {
                    Kind::Punct(')' | ']') => depth += 1,
                    Kind::Punct('(' | '[') => depth = depth.saturating_sub(1),
                    _ => {}
                }
                j = k;
            }
            let Some(mut s) = start else { return false };
            // Attributes and modifiers before the keyword.
            while s < o {
                let next = |c: char| toks.get(s + 1).is_some_and(|t| t.is_punct(c));
                if (toks[s].is_punct('#') && next('[')) || (toks[s].is_ident("pub") && next('(')) {
                    // `#[...]` or `pub(...)`.
                    s = matching_close(&toks, s + 1).saturating_add(1);
                } else if ["pub", "unsafe", "auto", "default"].iter().any(|m| toks[s].is_ident(m)) {
                    s += 1;
                } else {
                    break;
                }
            }
            s < o && (toks[s].is_ident("impl") || toks[s].is_ident("trait"))
        };
        // Whether the `type` name at `i` is followed by `= muda::PredefinedMenuItem;` (or `::muda::...`, or
        // a name of the crate in place of `muda`).
        let aliases_muda = |i: usize| {
            let mut j = i + 1;
            if !toks.get(j).is_some_and(|t| t.is_punct('=')) {
                return false;
            }
            j += 1;
            if toks.get(j).is_some_and(|t| t.is_punct(':')) && toks.get(j + 1).is_some_and(|t| t.is_punct(':')) {
                j += 2;
            }
            j < toks.len()
                && crate_is_muda(j)
                && then_path(&toks, j + 1, |s| s == "PredefinedMenuItem")
                && toks.get(j + 4).is_some_and(|t| t.is_punct(';'))
        };
        let definition =
            |i: usize| i.checked_sub(1).is_some_and(|k| ["struct", "enum", "union", "type", "trait"].iter().any(|kw| toks[k].is_ident(kw)));
        // The names of muda's type: its own, and each name a `use` or a `type` alias gives it.
        let mut names: BTreeSet<&str> = BTreeSet::from(["PredefinedMenuItem"]);
        names.extend(leaves.iter().filter(|leaf| is_muda(leaf)).filter_map(UseLeaf::binding));
        for (i, t) in toks.iter().enumerate() {
            if let Kind::Ident(name) = &t.kind
                && i.checked_sub(1).is_some_and(|k| toks[k].is_ident("type"))
                && aliases_muda(i)
            {
                names.insert(name);
            }
        }
        // For each of those names and each pair of braces (`None` for the file), whether a `use` or a
        // type alias in it binds the name to muda's type, and whether a `use` or an item definition in it
        // binds the name to another type. A binding to another type inside an item with a `cfg`
        // attribute may not apply on the platform the guard protects, so it is left out.
        let mut bound: BTreeMap<(&str, Option<usize>), (bool, bool)> = BTreeMap::new();
        for leaf in &leaves {
            let Some(name) = leaf.binding().filter(|name| names.contains(name)) else { continue };
            let muda = is_muda(leaf);
            // A path into this crate (`crate::`, `self::`, `super::`) may lead to a re-export of muda's
            // type, so it is not counted as another type.
            let local = leaf.root.as_deref().is_some_and(|s| matches!(s, "crate" | "self" | "super"));
            if muda || (!local && !gated.get(leaf.start).copied().unwrap_or(false)) {
                let entry = bound.entry((name, around.get(leaf.start).copied().flatten())).or_default();
                if muda {
                    entry.0 = true;
                } else {
                    entry.1 = true;
                }
            }
        }
        for (i, t) in toks.iter().enumerate() {
            let Kind::Ident(name) = &t.kind else { continue };
            let Some(k) = i.checked_sub(1) else { continue };
            if !names.contains(name.as_str()) || !definition(i) {
                continue;
            }
            if toks[k].is_ident("type") && around[i].is_some_and(assoc_body) {
                continue;
            }
            // An alias of a path into this crate may name a re-export of muda's type, as an import does.
            let local_alias = toks.get(i + 1).is_some_and(|t| t.is_punct('='))
                && toks.get(i + 2).is_some_and(|t| t.is_ident("crate") || t.is_ident("self") || t.is_ident("super"));
            if toks[k].is_ident("type") && local_alias {
                continue;
            }
            let key = (name.as_str(), around[i]);
            if toks[k].is_ident("type") && aliases_muda(i) {
                bound.entry(key).or_default().0 = true;
            } else if !gated[i] {
                bound.entry(key).or_default().1 = true;
            }
        }
        // A name used on its own is muda's type when the innermost braces around it that bind it (within
        // its module) bind it to muda's type, or only to nothing else. Where no braces bind it, only
        // `PredefinedMenuItem` itself is muda's type. Past `MAX_NESTING` levels it is read as muda's type.
        let bare_is_muda = |name: &str, i: usize| match lookup(&toks, &around, i, |scope| bound.get(&(name, scope)).copied()) {
            None => true,
            Some(Some((muda, other))) => muda || !other,
            Some(None) => name == "PredefinedMenuItem",
        };
        let mut lines = BTreeSet::new();
        for (i, tok) in toks.iter().enumerate() {
            let Kind::Ident(name) = &tok.kind else { continue };
            if !names.contains(name.as_str()) {
                continue;
            }
            // A qualified name counts only as `muda::PredefinedMenuItem` (or `::muda::...`, or a name of the
            // crate in place of `muda`), not as another crate's or module's type of the same name.
            let named = if continues_a_path(&toks, i) {
                name == "PredefinedMenuItem" && !continues_a_path(&toks, i - 3) && crate_is_muda(i - 3)
            } else {
                bare_is_muda(name, i)
            };
            let quit = |at: usize| then_path(&toks, at, |s| s == "quit" || s == "Quit");
            // `<muda::PredefinedMenuItem>::quit`, a qualified path to the same function.
            let qualified = || toks.get(i + 1).is_some_and(|t| t.is_punct('>')) && quit(i + 2) && opens_qualified_path(&toks, i);
            if named && (quit(i + 1) || qualified()) {
                lines.insert(tok.line);
            }
        }
        report(&mut findings, path, content, &lines);
    }
    findings
}

/// Guard: detects uses of `std::time::Instant` in code paths running on the web.
///
/// On `wasm32-unknown-unknown`, `std::time::Instant::now()` panics with "time not implemented
/// on this platform". Crates running in the browser should use `web_time::Instant`, which maps to
/// `std::time::Instant` on native platforms and `performance.now()` on the web.
///
/// ### Syntactic Scope and Limitations
///
/// * **Targeted files**: Callers should pass files from crates that compile into the application's web build.
/// * **What is reported**: the paths `std::time::Instant` and `::std::time::Instant`; `<m>::Instant` where a
///   `use` outside a native-only item binds `m` to `std::time` (`use std::time;`, `use std::time as t;`,
///   `use std::time::{self as t}`);
///   and every `use` that imports `std::time::Instant` or `std::time::*`, under any name, directly or through
///   such an `m` of any length (`use m::{Instant as Clock};`, `use m::*;`). Whitespace, comments
///   and line breaks may separate the parts of a path. Each finding names the line where the path starts (in a
///   `use` group, the line where the name's part of the group starts), once per line, in line order.
/// * **What is skipped**: comments, string and character literals, other crates' `Instant` (`web_time::Instant`,
///   a `time` module that is not `std::time`), and items whose `cfg` attribute is false on `wasm32-unknown-unknown`
///   (for example `#[cfg(not(target_arch = "wasm32"))]`, `#[cfg(all(unix, ...))]`, `#[cfg(target_os = "macos")]`,
///   or `#![cfg(...)]` for the rest of a module). Such an attribute covers only its item: code before the
///   attribute or after the item on the same line is checked. A statement whose value holds a block, such as
///   `let n = match x { ... } as u64 + ...;`, is covered up to its `;`. `cfg(false)` is false and `cfg(true)`
///   is true. A predicate that depends on a feature or on `test`, or that is nested more than 64 levels deep,
///   is not known from the source, so the item is checked. Escapes in a predicate's string values are decoded.
/// * **Module files**: a `mod name;` inside such an item makes the module's file native-only, and so are the
///   files of the modules declared in it, when they are among `files`, unless a chain of declarations outside
///   such items also reaches the file from a crate root (a file that no declaration names, or in a cycle of
///   declarations that no file outside the cycle names, such as `lib.rs` naming itself through `#[path]`, the
///   cycle's `lib.rs` or `main.rs`, or every file of the cycle when it has neither). The module's file is `name.rs` or
///   `name/mod.rs` next to `lib.rs`, `main.rs`, `mod.rs` or a file that a `#[path]` attribute names, and in
///   the folder named after any other file's stem, below the folders of enclosing inline modules; a `#[path = "..."]` attribute on the declaration
///   or on an enclosing inline module is followed. Paths are compared as text with `\` read as `/`, so the files must be named the same way
///   (for example all from one `rust_sources` call), and a crate root other than `lib.rs` or `main.rs`
///   (such as `src/bin/tool.rs`) is read as an ordinary module file. A declaration inside more than 64 inline
///   modules is not followed, so its file is checked.
/// * **Limitations**: This is a lexical source guard. It does not expand macros, resolve paths through other
///   modules or re-exports, or rename `std` itself (`use std as s;`). An alias of `std::time` counts inside the
///   braces of its `use` (or the file), down to but not into the body of a nested `mod`; nearer braces whose
///   `use` or `mod` binds the same name to something else hide it, unless that `use` or `mod` carries a `cfg`
///   attribute of its own that is not known to be true on `wasm32-unknown-unknown`, which may leave it out of
///   the web build. A path that starts with `::` names an external crate, so `::m::Instant` and
///   `use ::m::Instant;` are not read through an alias `m`.
pub fn std_instant<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let files: Vec<I::Item> = files.into_iter().collect();
    let paths: Vec<String> = files.iter().map(|file| normalize_path(file.path())).collect();
    // Only the declarations are kept from this pass, so the tokens of every file are never held at once.
    let decls: Vec<Vec<ModDecl>> = files
        .iter()
        .map(|file| {
            let toks = lex_rust(file.content());
            module_decls(&toks, &native_only(&toks))
        })
        .collect();
    let excluded = native_only_files(&paths, &decls);

    let mut findings = Vec::new();
    for (file, excluded) in files.iter().zip(excluded) {
        if excluded {
            continue;
        }
        let path = file.path();
        let content = file.content();
        let toks = lex_rust(content);
        let skip = native_only(&toks);
        let leaves = use_leaves(&toks, &BTreeSet::new());
        let modules: BTreeSet<String> = leaves
            .iter()
            .filter(|leaf| leaf.path == ["std", "time"] && !skip.get(leaf.start).copied().unwrap_or(false))
            .filter_map(|leaf| leaf.binding().map(str::to_owned))
            .collect();
        // For each alias name and each pair of braces (`None` for the file), whether a `use` in it binds
        // the name to `std::time`, and whether a `use` or a `mod` binds the name to something else, unless
        // its own `cfg` attribute may be false on the web.
        let (around, _) = brace_scopes(&toks);
        let gated = cfg_items(&toks);
        let mut bound: BTreeMap<(&str, Option<usize>), (bool, bool)> = BTreeMap::new();
        for leaf in &leaves {
            let Some(name) = leaf.binding().filter(|name| modules.contains(*name)) else { continue };
            let time = leaf.path == ["std", "time"];
            // A `use` of something else that carries a `cfg` attribute of its own that may be false on the
            // web may be absent from the web build, so it does not hide an outer alias.
            if !skip.get(leaf.start).copied().unwrap_or(false) && (time || !gated.contains(&leaf.start)) {
                let entry = bound.entry((name, around.get(leaf.start).copied().flatten())).or_default();
                if time {
                    entry.0 = true;
                } else {
                    entry.1 = true;
                }
            }
        }
        for (i, t) in toks.iter().enumerate() {
            if let Kind::Ident(name) = &t.kind
                && modules.contains(name)
                && !skip[i]
                && i.checked_sub(1).is_some_and(|k| toks[k].is_ident("mod") && !gated.contains(&k))
            {
                bound.entry((name.as_str(), around[i])).or_default().1 = true;
            }
        }
        // Whether `name` at `toks[at]` is an alias of `std::time`: the innermost braces around it that
        // bind the name (within its module) bind it to `std::time`. Past `MAX_NESTING` levels it is.
        let time_alias = |name: &str, at: usize| match lookup(&toks, &around, at, |scope| bound.get(&(name, scope)).copied()) {
            None => true,
            Some(Some((time, _))) => time,
            Some(None) => false,
        };
        // Read again with the aliases of `std::time`, so a path through an alias starts with `std::time`.
        // The leaves read without aliases are checked as well, since an alias does not reach every `use`
        // whose path starts with its name (an alias in a nested module named `std`).
        let aliased = if modules.is_empty() { Vec::new() } else { use_leaves(&toks, &modules) };
        let is_std_time = |path: &[String]| path == ["std", "time"];

        let mut lines = BTreeSet::new();
        for leaf in leaves.iter().chain(&aliased) {
            if skip.get(leaf.start).copied().unwrap_or(false) {
                continue;
            }
            let imports_instant = if leaf.name.is_none() {
                is_std_time(&leaf.path)
            } else {
                leaf.path.split_last().is_some_and(|(last, parent)| last == "Instant" && is_std_time(parent))
            };
            // A path read through an alias counts only where the alias names `std::time`.
            let through_other = leaf.alias.is_some_and(|a| matches!(&toks[a].kind, Kind::Ident(root) if !time_alias(root, leaf.start)));
            if imports_instant && !through_other {
                lines.insert(leaf.line);
            }
        }
        for (i, tok) in toks.iter().enumerate() {
            if skip[i] || continues_a_path(&toks, i) {
                continue;
            }
            let Kind::Ident(name) = &tok.kind else { continue };
            let hit = if name == "std" {
                then_path(&toks, i + 1, |s| s == "time") && then_path(&toks, i + 4, |s| s == "Instant")
            } else {
                // After a leading `::` the name is an external crate, not a local alias of `std::time`.
                let absolute = i >= 2 && toks[i - 1].is_punct(':') && toks[i - 2].is_punct(':');
                !absolute && modules.contains(name) && then_path(&toks, i + 1, |s| s == "Instant") && time_alias(name, i)
            };
            if hit {
                lines.insert(tok.line);
            }
        }
        report(&mut findings, path, content, &lines);
    }
    findings
}

/// Replace WGSL comments (`//` and nested `/* ... */`) with spaces, and write each WGSL line break as
/// `\n`: line feed, vertical tab, form feed, carriage return, carriage return and line feed as one
/// break, next line (U+0085), line separator (U+2028) and paragraph separator (U+2029)
/// (<https://www.w3.org/TR/WGSL/#line-break>). A line comment ends at any of them, and `str::lines`
/// then counts lines the way WGSL does.
fn mask_wgsl_comments(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    let mut depth = 0usize;
    let mut in_line = false;
    while i < len {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let line_break = match c {
            '\r' if next == Some('\n') => 2,
            '\n' | '\u{000B}' | '\u{000C}' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}' => 1,
            _ => 0,
        };
        if line_break > 0 {
            in_line = false;
            out.push('\n');
            i += line_break;
        } else if in_line {
            out.push(' ');
            i += 1;
        } else if c == '/' && next == Some('*') {
            depth += 1;
            out.push_str("  ");
            i += 2;
        } else if depth > 0 && c == '*' && next == Some('/') {
            depth -= 1;
            out.push_str("  ");
            i += 2;
        } else if depth > 0 {
            out.push(' ');
            i += 1;
        } else if c == '/' && next == Some('/') {
            in_line = true;
            out.push_str("  ");
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The largest finite value of the type a WGSL float literal with this suffix has.
fn float_limit(suffix: Option<u8>) -> f64 {
    if suffix == Some(b'h') { 65504.0 } else { f64::from(f32::MAX) }
}

/// Read a WGSL hexadecimal literal starting at the `0` of `0x`. Returns the index after it, the length of
/// its text without a suffix, and whether it is a float above the range of its type.
fn scan_wgsl_hex(code: &[u8], start: usize) -> (usize, usize, bool) {
    // Significant hex digits kept; 15 hold 60 bits, more than an f64 mantissa.
    const KEPT: usize = 15;
    let mut i = start + 2;
    let mut mantissa = 0f64;
    let mut kept = 0usize;
    let mut exponent: i64 = 0;
    let mut digits = 0usize;
    let mut digit = |d: u8, fraction: bool, exponent: &mut i64| {
        let value = (d as char).to_digit(16).unwrap_or(0);
        if mantissa == 0.0 && value == 0 {
            if fraction {
                *exponent -= 4;
            }
        } else if kept < KEPT {
            mantissa = mantissa * 16.0 + f64::from(value);
            kept += 1;
            if fraction {
                *exponent -= 4;
            }
        } else if !fraction {
            *exponent += 4;
        }
    };
    while i < code.len() && code[i].is_ascii_hexdigit() {
        digit(code[i], false, &mut exponent);
        digits += 1;
        i += 1;
    }
    let mut is_float = false;
    if code.get(i) == Some(&b'.') {
        is_float = true;
        i += 1;
        while i < code.len() && code[i].is_ascii_hexdigit() {
            digit(code[i], true, &mut exponent);
            digits += 1;
            i += 1;
        }
    }
    let mut has_exponent = false;
    if matches!(code.get(i), Some(b'p' | b'P')) {
        let mut j = i + 1;
        let negative = code.get(j) == Some(&b'-');
        if matches!(code.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if code.get(j).is_some_and(u8::is_ascii_digit) {
            let mut e: i64 = 0;
            while j < code.len() && code[j].is_ascii_digit() {
                e = (e * 10 + i64::from(code[j] - b'0')).min(1_000_000);
                j += 1;
            }
            exponent += if negative { -e } else { e };
            has_exponent = true;
            is_float = true;
            i = j;
        }
    }
    let text_len = i - start;
    let suffix = if has_exponent && matches!(code.get(i), Some(b'f' | b'h')) { code.get(i).copied() } else { None };
    let end = if suffix.is_some() || (!is_float && matches!(code.get(i), Some(b'i' | b'u'))) { i + 1 } else { i };
    if !is_float || digits == 0 || mantissa == 0.0 {
        return (end, text_len, false);
    }
    let scale = i32::try_from(exponent.clamp(-4000, 4000)).unwrap_or(0);
    let value = mantissa * 2f64.powi(scale);
    (end, text_len, !value.is_finite() || value > float_limit(suffix))
}

/// Guard: scans a single WGSL shader string and detects float literals outside their type's finite range.
///
/// WebGPU compilers in browsers reject a float literal above `f32::MAX` (e.g. `3.40282347e38`) with a
/// pipeline build error, which turns off GPU rendering. `f32::MAX` can be written as the hex float
/// `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
///
/// ### Policy and Limitations
///
/// * **Conservative mathematical bound**: The guard compares the literal's value, rounded to `f64`,
///   with `f64::from(f32::MAX)`, or with 65504 (the largest finite `f16`) for an `h` suffix. Any literal
///   above it is reported, even one that a compiler would round down to the maximum. A literal that exceeds
///   the maximum by less than `f64` precision (such as `0x1.fffffe000000001p+127f`) rounds to the maximum and
///   is not reported.
/// * **Which literals**: decimal floats (with a `.`, an exponent or an `f`/`h` suffix) and hexadecimal floats
///   (`0x` with a `.` or a `p` exponent). Integer literals (`123`, `123u`, `0xFFi`, `0x1f`) are not floats and
///   are not reported. A `-` in front is not part of the literal, so `-1e39` is reported as `1e39`.
/// * **Comments**: Comments (`//` and nested `/* ... */`) are masked out prior to scanning. A `//` comment
///   ends at any WGSL line break, a lone carriage return included, and finding lines count every one of them.
/// * **Identifiers**: digits inside an identifier, `π1e39` included, are not a literal.
pub fn wgsl_float_literals(wgsl: &str) -> Vec<Finding> {
    wgsl_float_literals_in([("", wgsl)])
}

/// Guard: scans multiple WGSL shader files and detects float literals outside their type's finite range,
/// with the rules of [`wgsl_float_literals`].
pub fn wgsl_float_literals_in<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let mut findings = Vec::new();
    for file in files {
        let path = file.path();
        let masked = mask_wgsl_comments(file.content());

        for (n, line) in masked.lines().enumerate() {
            let code = line.as_bytes();
            let mut i = 0;
            while i < code.len() {
                let digit_or_dot = code[i].is_ascii_digit() || (code[i] == b'.' && code.get(i + 1).is_some_and(u8::is_ascii_digit));
                // Outside comments, a character that is not ASCII and not blankspace (U+200E, U+200F) is part of an
                // identifier, as in `π1e39`.
                let word_before = digit_or_dot
                    && i > 0
                    && (code[i - 1].is_ascii_alphanumeric()
                        || code[i - 1] == b'_'
                        || code[i - 1] == b'.'
                        || (!code[i - 1].is_ascii() && !line[..i].ends_with(['\u{200E}', '\u{200F}'])));
                if !digit_or_dot || word_before {
                    i += 1;
                    continue;
                }
                let start = i;
                if code[i] == b'0' && matches!(code.get(i + 1), Some(b'x' | b'X')) {
                    let (end, text_len, over) = scan_wgsl_hex(code, i);
                    if over {
                        findings.push(Finding::new(path, n + 1, String::from_utf8_lossy(&code[start..start + text_len]).into_owned()));
                    }
                    i = end.max(i + 1);
                    continue;
                }
                let mut is_float = false;
                while i < code.len() && code[i].is_ascii_digit() {
                    i += 1;
                }
                if code.get(i) == Some(&b'.') {
                    is_float = true;
                    i += 1;
                    while i < code.len() && code[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if matches!(code.get(i), Some(b'e' | b'E')) {
                    let mut j = i + 1;
                    if matches!(code.get(j), Some(b'+' | b'-')) {
                        j += 1;
                    }
                    if code.get(j).is_some_and(u8::is_ascii_digit) {
                        is_float = true;
                        i = j;
                        while i < code.len() && code[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let literal = String::from_utf8_lossy(&code[start..i]).into_owned();
                let suffix = match code.get(i) {
                    Some(&s @ (b'f' | b'h')) => {
                        is_float = true;
                        i += 1;
                        Some(s)
                    }
                    Some(b'i' | b'u') if !is_float => {
                        i += 1;
                        None
                    }
                    _ => None,
                };
                if !is_float {
                    continue;
                }
                let is_overflow = match literal.parse::<f64>() {
                    Ok(v) => !v.is_finite() || v.abs() > float_limit(suffix),
                    Err(_) => true,
                };
                if is_overflow {
                    findings.push(Finding::new(path, n + 1, literal));
                }
            }
        }
    }
    findings
}

/// Size limits for [`rust_sources_with_limits`]. An input over a limit is an error.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLimits {
    /// The largest single file read, in bytes. Default 8 MiB.
    pub max_file_bytes: u64,
    /// The largest total of all files read, in bytes. Default 256 MiB.
    pub max_total_bytes: u64,
    /// The most files read. Default 50,000.
    pub max_files: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for SourceLimits {
    fn default() -> Self {
        Self { max_file_bytes: 8 << 20, max_total_bytes: 256 << 20, max_files: 50_000 }
    }
}

/// The first line of a `CACHEDIR.TAG` file, which marks a folder of generated files
/// (<https://bford.info/cachedir/>). Cargo writes one in each target folder.
#[cfg(not(target_arch = "wasm32"))]
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// Open `path` for reading when it is a regular file. Returns `Ok(None)` when `path` is a symbolic
/// link, a folder, a FIFO or another special file.
///
/// On Unix the open does not follow a symbolic link (`O_NOFOLLOW`) and does not wait for a writer on a
/// FIFO (`O_NONBLOCK`), and the type is then read from the opened file, so a file replaced between a
/// directory listing and the open is checked as it is when opened. On Windows the open does not follow
/// a reparse point (`FILE_FLAG_OPEN_REPARSE_POINT`), so an opened symbolic link or junction is the link
/// itself and returns `Ok(None)`. On other platforms a
/// path that is a symbolic link is skipped before the open, and the opened file must be a regular file.
#[cfg(not(target_arch = "wasm32"))]
fn open_regular_file(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    #[cfg(unix)]
    let opened = {
        use rustix::fs::{Mode, OFlags};
        rustix::fs::open(path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
            .map(std::fs::File::from)
            .map_err(std::io::Error::from)
    };
    #[cfg(windows)]
    let opened = open_without_following(path);
    #[cfg(not(any(unix, windows)))]
    let opened = {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Ok(None);
        }
        std::fs::File::open(path)
    };
    let file = match opened {
        Ok(file) => file,
        // `O_NOFOLLOW` fails on a symbolic link (ELOOP on Linux and macOS, EMLINK on FreeBSD).
        Err(_) if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => return Ok(None),
        Err(err) => return Err(err),
    };
    Ok(file.metadata()?.file_type().is_file().then_some(file))
}

/// The Win32 `FILE_FLAG_OPEN_REPARSE_POINT` flag: open a reparse point itself instead of its target.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Open `path` for reading without following a reparse point: a symbolic link or junction opens as the
/// link itself.
#[cfg(windows)]
fn open_without_following(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)
}

/// The Win32 `FILE_ATTRIBUTE_REPARSE_POINT` attribute of a file that is a reparse point.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

/// Whether `dir` holds a `CACHEDIR.TAG` regular file that starts with [`CACHEDIR_SIGNATURE`]. A marker
/// that is a symbolic link, a folder, a FIFO or another special file is not read and does not count.
/// On Windows a marker that is a reparse point of any kind, cloud placeholders included, does not count.
#[cfg(not(target_arch = "wasm32"))]
fn is_cache_dir(dir: &Path) -> bool {
    use std::io::Read;
    let Ok(Some(file)) = open_regular_file(&dir.join("CACHEDIR.TAG")) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if !file.metadata().is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0) {
            return false;
        }
    }
    let mut head = Vec::with_capacity(CACHEDIR_SIGNATURE.len());
    file.take(CACHEDIR_SIGNATURE.len() as u64).read_to_end(&mut head).is_ok() && head == CACHEDIR_SIGNATURE
}

/// Recursively collect all Rust source files under `dir`, excluding test files and test directories,
/// with the default [`SourceLimits`].
///
/// Excluded files:
/// * `tests.rs`
/// * `tests_*.rs`
/// * `*_tests.rs`
/// * `*_test.rs`
/// * `test.rs`
/// * Any file below `dir` within a directory named `tests`, `benches`, `examples` or `target`, or whose
///   name starts with `.`, or that holds a `CACHEDIR.TAG` regular file starting with the signature line
///   `Signature: 8a477f597d28d172789f06886806bc55` (a build output folder with another name). A
///   `CACHEDIR.TAG` that is a symbolic link or a special file such as a FIFO is not read, and its folder
///   is listed. On Windows a `CACHEDIR.TAG` that is a reparse point of any kind, cloud placeholders
///   included, does not mark its folder either.
/// * Symbolic links (and, on Windows, junctions) to files or folders, which are not followed.
///
/// Results are sorted lexicographically by file path for deterministic behavior across platforms.
#[cfg(not(target_arch = "wasm32"))]
pub fn rust_sources(dir: impl AsRef<Path>) -> std::io::Result<Vec<(String, String)>> {
    rust_sources_with_limits(dir, SourceLimits::default())
}

/// [`rust_sources`] with explicit size limits. A file over `max_file_bytes` returns an error of kind
/// [`std::io::ErrorKind::FileTooLarge`]; more than `max_files` files or `max_total_bytes` bytes in all
/// returns an error of kind [`std::io::ErrorKind::QuotaExceeded`].
#[cfg(not(target_arch = "wasm32"))]
pub fn rust_sources_with_limits(dir: impl AsRef<Path>, limits: SourceLimits) -> std::io::Result<Vec<(String, String)>> {
    use std::io::{Error, ErrorKind, Read};

    let root = dir.as_ref();
    let mut files = Vec::new();
    let mut total: u64 = 0;
    let mut dirs = vec![root.to_path_buf()];

    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let p = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() {
                if name.starts_with('.') || name == "target" || name == "tests" || name == "benches" || name == "examples" || is_cache_dir(&p) {
                    continue;
                }
                dirs.push(p);
            } else if kind.is_file() {
                if !name.ends_with(".rs") {
                    continue;
                }
                if name == "tests.rs" || name.starts_with("tests_") || name.ends_with("_tests.rs") || name.ends_with("_test.rs") || name == "test.rs"
                {
                    continue;
                }
                if files.len() >= limits.max_files {
                    return Err(Error::new(
                        ErrorKind::QuotaExceeded,
                        format!("more than {} Rust source files under {}", limits.max_files, root.display()),
                    ));
                }
                let Some(file) = open_regular_file(&p)? else {
                    continue;
                };
                let over_file = || Error::new(ErrorKind::FileTooLarge, format!("{} is larger than {} bytes", p.display(), limits.max_file_bytes));
                if file.metadata()?.len() > limits.max_file_bytes {
                    return Err(over_file());
                }
                let mut content = String::new();
                file.take(limits.max_file_bytes.saturating_add(1)).read_to_string(&mut content)?;
                let size = content.len() as u64;
                if size > limits.max_file_bytes {
                    return Err(over_file());
                }
                total = total.saturating_add(size);
                if total > limits.max_total_bytes {
                    return Err(Error::new(
                        ErrorKind::QuotaExceeded,
                        format!("Rust sources under {} are larger than {} bytes in all", root.display(), limits.max_total_bytes),
                    ));
                }
                files.push((p.display().to_string(), content));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{MAX_USE_PATH, MAX_USE_SEGMENT, lex_rust, use_leaves};

    fn paths_and_names(source: &str) -> Vec<(Vec<String>, Option<String>)> {
        use_leaves(&lex_rust(source), &BTreeSet::new()).into_iter().map(|leaf| (leaf.path, leaf.name.map(|name| name.to_string()))).collect()
    }

    /// A new empty folder under the system temporary folder, removed when dropped.
    #[cfg(any(unix, windows))]
    struct TempDir(std::path::PathBuf);

    #[cfg(any(unix, windows))]
    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("craft-testkit-unit-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
    }

    #[cfg(any(unix, windows))]
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn open_regular_file_opens_a_regular_file() {
        let dir = TempDir::new("open_regular");
        let tag = dir.0.join("CACHEDIR.TAG");
        std::fs::write(&tag, super::CACHEDIR_SIGNATURE).expect("write tag");
        assert!(super::open_regular_file(&tag).expect("open").is_some());
        assert!(super::is_cache_dir(&dir.0));
    }

    #[cfg(unix)]
    #[test]
    fn open_regular_file_does_not_follow_a_symlink() {
        let dir = TempDir::new("open_symlink");
        let target = dir.0.join("target.tag");
        std::fs::write(&target, super::CACHEDIR_SIGNATURE).expect("write target");
        let link = dir.0.join("CACHEDIR.TAG");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert!(super::open_regular_file(&link).expect("open").is_none());
        assert!(!super::is_cache_dir(&dir.0));
    }

    /// The FIFO has no writer, so an open that waited for one would not return.
    #[cfg(unix)]
    #[test]
    fn open_regular_file_does_not_wait_on_a_fifo() {
        let dir = TempDir::new("open_fifo");
        let fifo = dir.0.join("CACHEDIR.TAG");
        match std::process::Command::new("mkfifo").arg(&fifo).status() {
            Ok(status) if status.success() => {}
            other => {
                eprintln!("skipped: mkfifo is not available here ({other:?})");
                return;
            }
        }
        assert!(super::open_regular_file(&fifo).expect("open").is_none());
        assert!(!super::is_cache_dir(&dir.0));
    }

    /// A file symbolic link to `target` at `link`, or `None` with a printed note when this account may not
    /// create symbolic links.
    #[cfg(windows)]
    fn windows_file_symlink(target: &std::path::Path, link: &std::path::Path) -> Option<()> {
        match std::os::windows::fs::symlink_file(target, link) {
            Ok(()) => Some(()),
            Err(err) => {
                eprintln!("skipped: symbolic links cannot be created here ({err})");
                None
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn open_without_following_opens_the_link_itself() {
        let dir = TempDir::new("open_no_follow");
        let target = dir.0.join("target.tag");
        std::fs::write(&target, super::CACHEDIR_SIGNATURE).expect("write target");
        let link = dir.0.join("CACHEDIR.TAG");
        if windows_file_symlink(&target, &link).is_none() {
            return;
        }
        let opened = super::open_without_following(&link).expect("open");
        assert!(opened.metadata().expect("metadata").file_type().is_symlink());
    }

    #[cfg(windows)]
    #[test]
    fn a_symlinked_cachedir_tag_does_not_mark_its_folder() {
        let dir = TempDir::new("tag_symlink");
        let target = dir.0.join("target.tag");
        std::fs::write(&target, super::CACHEDIR_SIGNATURE).expect("write target");
        let link = dir.0.join("CACHEDIR.TAG");
        if windows_file_symlink(&target, &link).is_none() {
            return;
        }
        assert!(super::open_regular_file(&link).expect("open").is_none());
        assert!(!super::is_cache_dir(&dir.0));
    }

    #[test]
    fn use_leaves_keep_no_path_longer_than_the_limit() {
        let source = format!("use {}{{x, y::z, *}};\nuse std::time::{{Instant, self as t}};\n", "a::".repeat(MAX_USE_PATH));
        let some = |name: &str| Some(name.to_string());
        let path = |segments: &[&str]| segments.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            paths_and_names(&source),
            vec![
                (Vec::new(), some("x")),
                (Vec::new(), some("z")),
                (vec!["a".to_string(); MAX_USE_PATH], None),
                (path(&["std", "time", "Instant"]), some("Instant")),
                (path(&["std", "time"]), some("t")),
            ]
        );
    }

    #[test]
    fn use_leaves_keep_no_segment_longer_than_the_limit() {
        let long = "n".repeat(MAX_USE_SEGMENT + 1);
        let fits = "n".repeat(MAX_USE_SEGMENT);
        let source = format!("use {long}::{{self, x, *}};\nuse a::{long};\nuse {fits}::{{self as b, *}};\n");
        assert_eq!(
            paths_and_names(&source),
            vec![
                (Vec::new(), Some(long.clone())),
                (Vec::new(), Some("x".to_string())),
                (Vec::new(), Some(long)),
                (vec![fits.clone()], Some("b".to_string())),
                (vec![fits], None),
            ]
        );
    }
}
