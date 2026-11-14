//! The display-name scanner: words an app must not show users, looked for in the text users read.
//!
//! The app supplies the words, the files and folders to read, the literals that may keep a word,
//! and the marker that exempts a line. The library reads:
//!
//! - [`Kind::Rust`]: the string literals of Rust sources, with escapes decoded, outside comments and
//!   test-only code. A folder is read recursively for `.rs` files.
//! - [`Kind::Manifest`]: the package descriptions of a `Cargo.toml`.
//! - [`Kind::Text`] and [`Kind::TextMarkAbove`]: every line of a text file.
//!
//! It returns the [`Finding`]s; the app prints them and decides the exit status. The scan is
//! lexical: it does not resolve macros or evaluate `cfg` expressions other than `cfg(test)`, and it
//! cannot tell whether a literal is shown. It never starts a process or calls git.

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The limits [`Policy::new`] sets: 8 MiB per file, 512 MiB in all, folders 64 deep.
pub const DEFAULT_LIMITS: Limits = Limits { max_file_bytes: 8 << 20, max_total_bytes: 512 << 20, max_depth: 64 };

/// How a banned word is compared with the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchCase {
    /// Character for character.
    Exact,
    /// ASCII letters compare without case; other characters compare exactly.
    AsciiInsensitive,
}

/// Which Rust literal forms besides `"…"` and `r"…"` are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Literals {
    /// `b"…"` and `br"…"`.
    pub byte_strings: bool,
    /// `c"…"` and `cr"…"`.
    pub c_strings: bool,
}

/// Bounds on what [`scan`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest file read, in bytes.
    pub max_file_bytes: u64,
    /// The most bytes read by one scan, across all files.
    pub max_total_bytes: u64,
    /// The deepest folder read below a [`Kind::Rust`] folder input.
    pub max_depth: usize,
}

/// The app's rules for one scan.
#[derive(Clone, Copy)]
pub struct Policy<'a> {
    /// The words users must not see. A word matches only as a whole word: the characters next to it
    /// are not letters, digits or `_`.
    pub banned: &'a [&'a str],
    /// How the words are compared.
    pub case: MatchCase,
    /// Whole literals (or text lines, trimmed) that may keep a banned word.
    pub allowed: &'a [&'a str],
    /// Functions that return `true` for a whole literal or trimmed text line that may keep a banned
    /// word.
    pub exceptions: &'a [fn(&str) -> bool],
    /// A marker that exempts a line. In Rust it counts only inside a comment on a line the literal
    /// spans; in a text file it counts anywhere on the line, and in [`Kind::TextMarkAbove`] files also
    /// on a `#` line just above.
    pub allow_marker: Option<&'a str>,
    /// The optional Rust literal forms.
    pub literals: Literals,
    /// Returns `true` for a Rust file in a folder input to leave out, given its path below that
    /// folder. [`test_path`] is the default. A file input is read whatever this returns.
    pub skip: fn(&Path) -> bool,
    /// Bounds on what is read.
    pub limits: Limits,
}

impl<'a> Policy<'a> {
    /// A policy with these banned words, exact comparison, no allowed literals, exceptions or
    /// marker, only `"…"` and `r"…"` literals, [`test_path`] and [`DEFAULT_LIMITS`].
    pub const fn new(banned: &'a [&'a str]) -> Self {
        Policy {
            banned,
            case: MatchCase::Exact,
            allowed: &[],
            exceptions: &[],
            allow_marker: None,
            literals: Literals { byte_strings: false, c_strings: false },
            skip: test_path,
            limits: DEFAULT_LIMITS,
        }
    }

    /// The banned words in `text`, in the order of [`Policy::banned`]. Empty when `text` is allowed
    /// or an exception returns `true` for it.
    pub fn words_in(&self, text: &str) -> Vec<&'a str> {
        if self.allowed.contains(&text) || self.exceptions.iter().any(|exempt| exempt(text)) {
            return Vec::new();
        }
        self.banned.iter().copied().filter(|word| contains_word(text, word, self.case)).collect()
    }

    fn validate(&self) -> Result<(), Error> {
        if self.banned.is_empty() {
            return Err(Error::Policy("the list of banned words is empty"));
        }
        if self.banned.iter().any(|word| word.trim().is_empty()) {
            return Err(Error::Policy("a banned word is empty"));
        }
        if self.allow_marker.is_some_and(|marker| marker.trim().is_empty()) {
            return Err(Error::Policy("the allow marker is empty"));
        }
        if self.limits.max_file_bytes == 0 || self.limits.max_total_bytes == 0 {
            return Err(Error::Policy("a byte limit is zero"));
        }
        Ok(())
    }
}

/// The apps' rule for test code: a file named `tests.rs` or ending in `_tests.rs`, or a file under
/// a folder named `tests`.
pub fn test_path(path: &Path) -> bool {
    let file = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    file == "tests.rs" || file.ends_with("_tests.rs") || path.parent().is_some_and(|dir| dir.components().any(|part| part.as_os_str() == "tests"))
}

/// What a path holds and how it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A Rust file, or a folder whose `.rs` files are read.
    Rust,
    /// A `Cargo.toml`, whose `package.description` and `workspace.package.description` values are
    /// read, decoded when they are TOML strings.
    Manifest,
    /// A text file, every line.
    Text,
    /// A text file whose comments take whole `#` lines (a desktop entry): a `#` line with the marker
    /// also exempts the line below it.
    TextMarkAbove,
}

/// One path to scan, relative to the base folder [`scan`] takes, or absolute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input<'a> {
    /// How the path is read.
    pub kind: Kind,
    /// The file or, for [`Kind::Rust`], folder.
    pub path: &'a Path,
}

impl<'a> Input<'a> {
    /// A Rust file or folder.
    pub fn rust(path: &'a Path) -> Self {
        Input { kind: Kind::Rust, path }
    }

    /// A `Cargo.toml`.
    pub fn manifest(path: &'a Path) -> Self {
        Input { kind: Kind::Manifest, path }
    }

    /// A text file.
    pub fn text(path: &'a Path) -> Self {
        Input { kind: Kind::Text, path }
    }

    /// A text file whose `#` lines can mark the line below them.
    pub fn text_mark_above(path: &'a Path) -> Self {
        Input { kind: Kind::TextMarkAbove, path }
    }
}

/// A banned word found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The file, as the input named it, joined with the path below a folder input.
    pub path: PathBuf,
    /// The 1-based line the literal starts on, or the text line.
    pub line: usize,
    /// The banned word, as [`Policy::banned`] lists it.
    pub word: String,
    /// The decoded literal or description, or the trimmed text line.
    pub text: String,
    /// How the file was read.
    pub kind: Kind,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {:?} in {:?}", self.path.display(), self.line, self.word, self.text)
    }
}

/// A Rust source the scanner cannot read as tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// The 1-based line the problem starts on.
    pub line: usize,
    /// What is wrong.
    pub message: &'static str,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Why a scan did not finish.
#[derive(Debug)]
pub enum Error {
    /// The policy cannot be used.
    Policy(&'static str),
    /// A file or folder could not be read.
    Io {
        /// The path, as reported.
        path: PathBuf,
        /// The cause.
        source: io::Error,
    },
    /// A file is not UTF-8.
    InvalidUtf8 {
        /// The path, as reported.
        path: PathBuf,
    },
    /// A file is larger than [`Limits::max_file_bytes`].
    FileTooLarge {
        /// The path, as reported.
        path: PathBuf,
        /// The limit, in bytes.
        limit: u64,
    },
    /// Reading this file took the scan past [`Limits::max_total_bytes`].
    TotalTooLarge {
        /// The path, as reported.
        path: PathBuf,
        /// The limit, in bytes.
        limit: u64,
    },
    /// A folder is deeper than [`Limits::max_depth`].
    TooDeep {
        /// The path, as reported.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// A file is given as two kinds of input.
    Conflict {
        /// The path, as reported.
        path: PathBuf,
    },
    /// A Rust source could not be read as tokens.
    Syntax {
        /// The path, as reported.
        path: PathBuf,
        /// Where and what.
        error: SyntaxError,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Policy(m) => write!(f, "policy: {m}"),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::InvalidUtf8 { path } => write!(f, "{}: not UTF-8", path.display()),
            Error::FileTooLarge { path, limit } => write!(f, "{}: larger than {limit} bytes", path.display()),
            Error::TotalTooLarge { path, limit } => write!(f, "{}: the scan read more than {limit} bytes", path.display()),
            Error::TooDeep { path, limit } => write!(f, "{}: more than {limit} folders deep", path.display()),
            Error::Conflict { path } => write!(f, "{}: given as two kinds of input", path.display()),
            Error::Syntax { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

/// Every finding in `inputs`, read from `base`. Files are read once each, in path order, and the
/// findings come in path order, then in source order, then in the order of [`Policy::banned`].
///
/// A [`Kind::Rust`] folder is read recursively for `.rs` files that [`Policy::skip`] keeps; links to
/// folders are not followed. A file input is read whatever its name. Every error stops the scan: a
/// missing input, a file that cannot be read, is not UTF-8 or is over a limit, and a Rust source the
/// lexer cannot read. Inputs must not include binary files.
pub fn scan(base: &Path, inputs: &[Input<'_>], policy: &Policy<'_>) -> Result<Vec<Finding>, Error> {
    policy.validate()?;
    let mut files = Vec::new();
    for input in inputs {
        if input.kind == Kind::Rust {
            collect_rust(base, input.path, policy, &mut files)?;
        } else {
            files.push((input.path.to_path_buf(), input.kind));
        }
    }
    files.sort();
    files.dedup();
    if let Some(pair) = files.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(Error::Conflict { path: pair[0].0.clone() });
    }
    let mut total = 0;
    let mut findings = Vec::new();
    for (path, kind) in &files {
        let source = read_limited(&base.join(path), path, &policy.limits, &mut total)?;
        findings.extend(scan_source(path, &source, *kind, policy)?);
    }
    Ok(findings)
}

/// The findings in one source that is already in memory. `path` is only reported. A
/// [`Kind::Rust`] source is a single file here.
pub fn scan_source(path: &Path, source: &str, kind: Kind, policy: &Policy<'_>) -> Result<Vec<Finding>, Error> {
    policy.validate()?;
    let finding =
        |line: usize, word: &str, text: &str| Finding { path: path.to_path_buf(), line, word: word.to_string(), text: text.to_string(), kind };
    let mut out = Vec::new();
    if kind == Kind::Rust {
        let lexed = lex(source, policy.literals, policy.allow_marker).map_err(|error| Error::Syntax { path: path.to_path_buf(), error })?;
        for lit in &lexed.literals {
            if (lit.first_line..=lit.last_line).any(|line| lexed.marked_lines.contains(&line)) {
                continue;
            }
            out.extend(policy.words_in(&lit.text).into_iter().map(|word| finding(lit.first_line, word, &lit.text)));
        }
        return Ok(out);
    }
    let lines: Vec<&str> = source.lines().collect();
    let marked = |line: &str| policy.allow_marker.is_some_and(|marker| line.contains(marker));
    if kind == Kind::Manifest {
        for (first, last, text) in descriptions(source) {
            if lines.get(first - 1..last.min(lines.len())).is_some_and(|span| span.iter().any(|line| marked(line))) {
                continue;
            }
            out.extend(policy.words_in(&text).into_iter().map(|word| finding(first, word, &text)));
        }
        return Ok(out);
    }
    for (i, line) in lines.iter().enumerate() {
        let above = i.checked_sub(1).and_then(|j| lines.get(j)).is_some_and(|l| l.trim_start().starts_with('#') && marked(l));
        if marked(line) || (kind == Kind::TextMarkAbove && above) {
            continue;
        }
        let text = line.trim();
        out.extend(policy.words_in(text).into_iter().map(|word| finding(i + 1, word, text)));
    }
    Ok(out)
}

/// The package descriptions of a `Cargo.toml`, the values at the key paths `package.description`
/// and `workspace.package.description`: the 1-based first and last line of each, and its text. A
/// TOML string is decoded; any other value is the rest of its line, trimmed. Table headers count
/// outside strings and arrays; keys inside inline tables are not read.
fn descriptions(source: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let (mut offset, mut number, mut resume, mut depth) = (0, 0, 0, 0);
    // The current table's key path, or `None` under an array of tables or an unreadable header.
    let mut table = Some(Vec::new());
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        number += 1;
        if start < resume {
            continue;
        }
        let top = depth == 0;
        resume = toml_line_end(source, start, line.len(), &mut depth);
        if !top {
            continue;
        }
        // A byte order mark may start the file.
        let body = if start == 0 { line.strip_prefix('\u{feff}').unwrap_or(line) } else { line };
        let trimmed = body.trim_start_matches([' ', '\t']);
        if trimmed.starts_with('[') {
            table = toml_header(trimmed);
            depth = 0;
            continue;
        }
        let Some((key, value)) = toml_key(body).and_then(|(key, rest)| Some((key, rest.strip_prefix('=')?.trim_start()))) else {
            continue;
        };
        let path: Vec<&str> = table.iter().flatten().chain(&key).map(String::as_str).collect();
        if table.is_none() || !matches!(path.as_slice(), ["package", "description"] | ["workspace", "package", "description"]) {
            continue;
        }
        let at = start + line.len() - value.len();
        let (text, len) = toml_string(source.get(at..).unwrap_or_default()).unwrap_or_else(|| (value.trim().to_string(), value.trim_end().len()));
        resume = resume.max(at + len);
        let last = number + source.get(at..at + len).unwrap_or_default().matches('\n').count();
        out.push((number, last, text));
    }
    out
}

/// The key path of a TOML table header (`[a.b]`) at the start of `line`, or `None` for an array of
/// tables (`[[a.b]]`) or a header that cannot be read.
fn toml_header(line: &str) -> Option<Vec<String>> {
    let (parts, rest) = toml_key(line.strip_prefix('[').filter(|rest| !rest.starts_with('['))?)?;
    let rest = rest.strip_prefix(']')?.trim_start_matches([' ', '\t']);
    (rest.is_empty() || rest.starts_with(['#', '\r', '\n'])).then_some(parts)
}

/// The parts of the TOML key (bare, quoted or dotted) at the start of `text`, after any spaces, and
/// the text after the key and the spaces that follow it.
fn toml_key(text: &str) -> Option<(Vec<String>, &str)> {
    let mut parts = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.starts_with("\"\"\"") || rest.starts_with("'''") {
            return None;
        }
        let len = if rest.starts_with(['"', '\'']) {
            let (part, len) = toml_string(rest)?;
            parts.push(part);
            len
        } else {
            let len = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-').len();
            parts.push(rest.get(..len).filter(|part| !part.is_empty())?.to_string());
            len
        };
        rest = rest.get(len..)?.trim_start_matches([' ', '\t']);
        match rest.strip_prefix('.') {
            Some(after) => rest = after,
            None => return Some((parts, rest)),
        }
    }
}

/// The end of the TOML line of `len` bytes at `start`, or past the last string that starts on it,
/// so that the later lines of a multi-line string are not read as keys. `depth` counts the arrays
/// and inline tables open after the line, outside strings and comments.
fn toml_line_end(source: &str, start: usize, len: usize, depth: &mut usize) -> usize {
    let (mut at, mut end) = (start, start + len);
    while at < end {
        let Some(c) = source.get(at..).and_then(|rest| rest.chars().next()) else { break };
        match c {
            '#' => break,
            '[' | '{' => {
                *depth += 1;
                at += 1;
            }
            ']' | '}' => {
                *depth = depth.saturating_sub(1);
                at += 1;
            }
            '"' | '\'' => match toml_string(source.get(at..).unwrap_or_default()) {
                Some((_, string)) => {
                    at += string;
                    if at > end {
                        // The string closes on a later line: read the rest of that line too.
                        end = source.get(at..).and_then(|rest| rest.find('\n')).map_or(source.len(), |i| at + i + 1);
                    }
                }
                None => break,
            },
            _ => at += c.len_utf8(),
        }
    }
    end
}

/// The TOML string at the start of `value`, decoded, and its length in bytes. `None` when `value`
/// does not start with a complete, valid string.
fn toml_string(value: &str) -> Option<(String, usize)> {
    let delim = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|d| value.starts_with(d))?;
    let (basic, multi) = (delim.starts_with('"'), delim.len() == 3);
    let mut at = delim.len();
    if multi {
        let after = value.get(at..)?;
        at += if after.starts_with("\r\n") { 2 } else { usize::from(after.starts_with('\n')) };
    }
    let mut text = String::new();
    loop {
        let rest = value.get(at..)?;
        let c = rest.chars().next()?;
        if rest.starts_with(delim) {
            if !multi {
                return Some((text, at + 1));
            }
            // Up to two quotes just before the closing three belong to the text.
            let extra = rest.chars().take_while(|&q| q == c).count().saturating_sub(3).min(2);
            text.extend(std::iter::repeat_n(c, extra));
            return Some((text, at + extra + 3));
        }
        match c {
            '\\' if basic => at = toml_escape(value, at + 1, multi, &mut text)?,
            '\n' if !multi => return None,
            '\r' if rest.starts_with("\r\n") => at += 1,
            _ => {
                text.push(c);
                at += c.len_utf8();
            }
        }
    }
}

/// The escape after a backslash at `at` in a TOML basic string, decoded into `text`, and the index
/// after it.
fn toml_escape(value: &str, at: usize, multi: bool, text: &mut String) -> Option<usize> {
    let rest = value.get(at..)?;
    let c = rest.chars().next()?;
    let simple = match c {
        'b' => Some('\u{8}'),
        't' => Some('\t'),
        'n' => Some('\n'),
        'f' => Some('\u{c}'),
        'r' => Some('\r'),
        'e' => Some('\u{1b}'),
        '"' | '\\' => Some(c),
        _ => None,
    };
    if let Some(decoded) = simple {
        text.push(decoded);
        return Some(at + 1);
    }
    let digits = match c {
        'x' => 2,
        'u' => 4,
        'U' => 8,
        _ => 0,
    };
    if digits > 0 {
        let hex = rest.get(1..1 + digits).filter(|hex| hex.chars().all(|h| h.is_ascii_hexdigit()))?;
        text.push(u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)?);
        return Some(at + 1 + digits);
    }
    // A backslash at the end of a line in a multi-line string: the line break and the whitespace
    // after it are left out.
    if multi && rest.trim_start_matches([' ', '\t']).starts_with(['\n', '\r']) {
        return Some(at + rest.len() - rest.trim_start_matches([' ', '\t', '\n', '\r']).len());
    }
    None
}

/// A Rust string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    /// The 1-based line of its opening quote.
    pub first_line: usize,
    /// The 1-based line of its closing quote.
    pub last_line: usize,
    /// Its text, with escapes decoded (not in raw literals) and CRLF read as LF.
    pub text: String,
}

/// The string literals of a Rust source outside comments and test-only code, in source order.
///
/// Test-only code is the code under `#[cfg(test)]` and the rest of a module that starts with
/// `#![cfg(test)]`. Under `#[cfg(test)]`, a `const`, `static` or `let` runs up to its `;`, another
/// item up to its closing `}` or its `;`, and a field, argument, arm or statement up to its `,` or
/// `;`. A block expression (a block, a macro's braces, a labeled block, or `if`, `match`, `loop`,
/// `while`, `for`, `unsafe`, `async` or `const`), as the whole statement or as an arm's expression
/// after `=>`, ends at its `}`, or at the last `}` of an `if` with `else` branches. When a method
/// call, field or `?` follows that `}`, it runs up to its `,` or `;` instead. Character, byte and
/// lifetime tokens are read, so a quote in them does not start a string.
pub fn literals(source: &str, kinds: Literals) -> Result<Vec<Literal>, SyntaxError> {
    lex(source, kinds, None).map(|lexed| lexed.literals)
}

fn contains_word(text: &str, word: &str, case: MatchCase) -> bool {
    match case {
        MatchCase::Exact => text.match_indices(word).any(|(at, _)| whole_word(text, at, word.len())),
        MatchCase::AsciiInsensitive => {
            let lower = text.to_ascii_lowercase();
            let word = word.to_ascii_lowercase();
            lower.match_indices(&word).any(|(at, _)| whole_word(text, at, word.len()))
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn whole_word(text: &str, at: usize, len: usize) -> bool {
    let before = text.get(..at).and_then(|s| s.chars().next_back());
    let after = text.get(at + len..).and_then(|s| s.chars().next());
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

fn collect_rust(base: &Path, path: &Path, policy: &Policy<'_>, out: &mut Vec<(PathBuf, Kind)>) -> Result<(), Error> {
    let meta = fs::metadata(base.join(path)).map_err(io_error(path))?;
    if meta.is_dir() {
        walk(base, path, path, policy, 0, out)
    } else {
        out.push((path.to_path_buf(), Kind::Rust));
        Ok(())
    }
}

fn walk(base: &Path, input: &Path, dir: &Path, policy: &Policy<'_>, depth: usize, out: &mut Vec<(PathBuf, Kind)>) -> Result<(), Error> {
    if depth > policy.limits.max_depth {
        return Err(Error::TooDeep { path: dir.to_path_buf(), limit: policy.limits.max_depth });
    }
    let mut entries = fs::read_dir(base.join(dir)).and_then(|entries| entries.collect::<io::Result<Vec<_>>>()).map_err(io_error(dir))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = dir.join(entry.file_name());
        let file_type = entry.file_type().map_err(io_error(&path))?;
        if file_type.is_dir() {
            walk(base, input, &path, policy, depth + 1, out)?;
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "rs") || (policy.skip)(path.strip_prefix(input).unwrap_or(&path)) {
            continue;
        }
        // A link to a folder is not followed; a link to a file is read like the file.
        let is_file = if file_type.is_symlink() { fs::metadata(base.join(&path)).map_err(io_error(&path))?.is_file() } else { file_type.is_file() };
        if is_file {
            out.push((path, Kind::Rust));
        }
    }
    Ok(())
}

fn read_limited(full: &Path, path: &Path, limits: &Limits, total: &mut u64) -> Result<String, Error> {
    let file = File::open(full).map_err(io_error(path))?;
    let len = file.metadata().map_err(io_error(path))?.len();
    if len > limits.max_file_bytes {
        return Err(Error::FileTooLarge { path: path.to_path_buf(), limit: limits.max_file_bytes });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(limits.max_file_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(io_error(path))?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > limits.max_file_bytes {
        return Err(Error::FileTooLarge { path: path.to_path_buf(), limit: limits.max_file_bytes });
    }
    *total = total.saturating_add(read);
    if *total > limits.max_total_bytes {
        return Err(Error::TotalTooLarge { path: path.to_path_buf(), limit: limits.max_total_bytes });
    }
    let mut text = String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8 { path: path.to_path_buf() })?;
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    Ok(text)
}

struct Lexed {
    literals: Vec<Literal>,
    marked_lines: HashSet<usize>,
}

/// The literal forms, which differ in the escapes they take and in whether the scan reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Str,
    Char,
    Byte,
    C,
}

/// The keywords that start an item, whose header can hold commas.
const ITEM_KEYWORDS: &[&str] =
    &["const", "enum", "extern", "fn", "impl", "let", "macro_rules", "mod", "static", "struct", "trait", "type", "union", "use"];

/// The keywords of items that end at their `;`, whatever braces their value holds.
const SEMICOLON_KEYWORDS: &[&str] = &["const", "let", "static"];

/// The keywords that start an expression ending at the `}` of its block (or of its last `else`).
const BLOCK_KEYWORDS: &[&str] = &["async", "const", "for", "if", "loop", "match", "unsafe", "while"];

/// Test-only code being skipped.
#[derive(Debug, Clone, Copy)]
enum TestRegion {
    /// An item, field, argument, arm or statement under `#[cfg(test)]`, at this delimiter depth.
    /// `start` is the index where its code (or an arm's expression, after `=>`) starts; `opened` is
    /// set once its first `{` is read, `item` once an item keyword (`fn`, `impl`, `mod`…) is read,
    /// and `semi` while the last such keyword is `const`, `let` or `static`, whose items end at their
    /// `;`. Without an item keyword, `block` is set when that first `{` starts a block expression (a
    /// block, a macro's braces, or `if`, `match`, `loop` and the like), which ends at its `}` unless
    /// an `else` follows. A `.` or `?` after that `}` clears `block`, so the expression ends at its
    /// `;` or `,`.
    Item { level: usize, start: usize, opened: bool, item: bool, semi: bool, block: bool },
    /// The rest of the module that `#![cfg(test)]` starts, at this delimiter depth.
    Inner { level: usize },
}

struct Lexer<'s> {
    chars: Vec<char>,
    at: usize,
    line: usize,
    kinds: Literals,
    marker: Option<&'s str>,
    /// Open delimiters and the line of each.
    delims: Vec<(char, usize)>,
    test: Option<TestRegion>,
    out: Lexed,
}

fn lex(source: &str, kinds: Literals, marker: Option<&str>) -> Result<Lexed, SyntaxError> {
    let mut lexer = Lexer {
        chars: source.chars().collect(),
        at: 0,
        line: 1,
        kinds,
        marker,
        delims: Vec::new(),
        test: None,
        out: Lexed { literals: Vec::new(), marked_lines: HashSet::new() },
    };
    lexer.run()?;
    Ok(lexer.out)
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.at += 1;
        Some(c)
    }

    fn error(line: usize, message: &'static str) -> SyntaxError {
        SyntaxError { line, message }
    }

    fn run(&mut self) -> Result<(), SyntaxError> {
        while let Some(c) = self.peek(0) {
            match c {
                '\n' => {
                    self.line += 1;
                    self.at += 1;
                }
                '/' if self.peek(1) == Some('/') => self.line_comment(),
                '/' if self.peek(1) == Some('*') => self.block_comment()?,
                '"' => {
                    self.at += 1;
                    self.string(Form::Str)?;
                }
                '\'' => self.quote(Form::Char)?,
                '#' => self.hash(),
                '{' | '(' | '[' => self.open(c),
                '}' | ')' | ']' => self.close(c)?,
                ';' => self.item_end(false),
                ',' => self.item_end(true),
                '=' if self.peek(1) == Some('>') => self.arrow(),
                c if c.is_ascii_digit() => self.number(),
                c if is_word(c) => self.word()?,
                _ => self.at += 1,
            }
        }
        match self.delims.last() {
            Some(&(_, line)) => Err(Self::error(line, "unclosed delimiter")),
            None => Ok(()),
        }
    }

    fn comment_text(&mut self, first: usize, start: usize) {
        if let Some(marker) = self.marker {
            let text: String = self.chars.get(start..self.at).unwrap_or_default().iter().collect();
            let marked = text.split('\n').enumerate().filter(|(_, line)| line.contains(marker));
            self.out.marked_lines.extend(marked.map(|(i, _)| first + i));
        }
    }

    fn line_comment(&mut self) {
        let start = self.at;
        while self.peek(0).is_some_and(|c| c != '\n') {
            self.at += 1;
        }
        self.comment_text(self.line, start);
    }

    fn block_comment(&mut self) -> Result<(), SyntaxError> {
        let (first, start) = (self.line, self.at);
        self.at += 2;
        let mut depth = 1_usize;
        while depth > 0 {
            match (self.next(), self.peek(0)) {
                (None, _) => return Err(Self::error(first, "unterminated block comment")),
                (Some('/'), Some('*')) => {
                    self.at += 1;
                    depth += 1;
                }
                (Some('*'), Some('/')) => {
                    self.at += 1;
                    depth -= 1;
                }
                (Some('\n'), _) => self.line += 1,
                _ => {}
            }
        }
        self.comment_text(first, start);
        Ok(())
    }

    fn reads(&self, form: Form) -> bool {
        match form {
            Form::Str => true,
            Form::Byte => self.kinds.byte_strings,
            Form::C => self.kinds.c_strings,
            Form::Char => false,
        }
    }

    fn push(&mut self, first: usize, text: String, form: Form) {
        if self.reads(form) && self.test.is_none() {
            self.out.literals.push(Literal { first_line: first, last_line: self.line, text });
        }
    }

    /// A `"…"` body after its opening quote.
    fn string(&mut self, form: Form) -> Result<(), SyntaxError> {
        let first = self.line;
        let mut text = String::new();
        loop {
            match self.next() {
                None => return Err(Self::error(first, "unterminated string literal")),
                Some('"') => break,
                Some('\\') => self.escape(form, &mut text, first)?,
                Some('\r') if self.peek(0) == Some('\n') => {}
                Some('\n') => {
                    self.line += 1;
                    text.push('\n');
                }
                Some(c) => text.push(c),
            }
        }
        self.push(first, text, form);
        Ok(())
    }

    /// The escape after a backslash, decoded into `text`.
    fn escape(&mut self, form: Form, text: &mut String, first: usize) -> Result<(), SyntaxError> {
        let decoded = match self.next() {
            None => return Err(Self::error(first, "unterminated escape")),
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('0') => '\0',
            Some(c @ ('\\' | '\'' | '"')) => c,
            Some('x') => self.hex_escape(form)?,
            Some('u') if form != Form::Byte => self.unicode_escape()?,
            Some('\n') if form != Form::Char => {
                self.continuation();
                return Ok(());
            }
            Some('\r') if form != Form::Char && self.peek(0) == Some('\n') => {
                self.at += 1;
                self.continuation();
                return Ok(());
            }
            Some(_) => return Err(Self::error(self.line, "unknown character escape")),
        };
        text.push(decoded);
        Ok(())
    }

    /// A backslash before a line break: the break and the whitespace after it are left out.
    fn continuation(&mut self) {
        self.line += 1;
        while let Some(c @ (' ' | '\t' | '\n' | '\r')) = self.peek(0) {
            self.line += usize::from(c == '\n');
            self.at += 1;
        }
    }

    fn hex_escape(&mut self, form: Form) -> Result<char, SyntaxError> {
        let digits = (self.peek(0).and_then(|c| c.to_digit(16)), self.peek(1).and_then(|c| c.to_digit(16)));
        let (Some(high), Some(low)) = digits else {
            return Err(Self::error(self.line, "a \\x escape needs two hex digits"));
        };
        self.at += 2;
        let value = high * 16 + low;
        match (value, form) {
            (0..=0x7f, _) => Ok(char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER)),
            (_, Form::Byte | Form::C) => Ok(char::REPLACEMENT_CHARACTER),
            _ => Err(Self::error(self.line, "a \\x escape above 0x7F")),
        }
    }

    fn unicode_escape(&mut self) -> Result<char, SyntaxError> {
        let bad = Self::error(self.line, "an invalid \\u{...} escape");
        if self.next() != Some('{') || self.peek(0) == Some('_') {
            return Err(bad);
        }
        let (mut value, mut digits) = (0_u32, 0_usize);
        loop {
            match self.next() {
                Some('}') => break,
                Some('_') => {}
                Some(c) if digits < 6 => {
                    value = value * 16 + c.to_digit(16).ok_or_else(|| bad.clone())?;
                    digits += 1;
                }
                _ => return Err(bad),
            }
        }
        if digits == 0 {
            return Err(bad);
        }
        char::from_u32(value).ok_or(bad)
    }

    /// A quote: a character or byte literal, or the start of a lifetime or label.
    fn quote(&mut self, form: Form) -> Result<(), SyntaxError> {
        match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => {
                let first = self.line;
                self.at += 2;
                self.escape(form, &mut String::new(), first)?;
                if self.next() != Some('\'') {
                    return Err(Self::error(first, "unterminated character literal"));
                }
            }
            (Some(c), Some('\'')) if c != '\'' && c != '\n' => self.at += 3,
            _ => self.at += 1,
        }
        Ok(())
    }

    fn number(&mut self) {
        while self.peek(0).is_some_and(is_word) {
            self.at += 1;
        }
    }

    /// An identifier, or the prefix of a byte, C or raw literal.
    fn word(&mut self) -> Result<(), SyntaxError> {
        let start = self.at;
        self.number();
        let word: String = self.chars.get(start..self.at).unwrap_or_default().iter().collect();
        // `const { .. }` is a block expression, not an item.
        let const_block = word == "const" && self.chars.get(self.skip_space(self.at)) == Some(&'{');
        if let Some(TestRegion::Item { level, opened: false, item, semi, .. }) = &mut self.test
            && *level == self.delims.len()
            && ITEM_KEYWORDS.contains(&word.as_str())
            && !const_block
        {
            *item = true;
            *semi = SEMICOLON_KEYWORDS.contains(&word.as_str());
        }
        match (word.as_str(), self.peek(0)) {
            ("b", Some('\'')) => self.quote(Form::Byte),
            ("b", Some('"')) => {
                self.at += 1;
                self.string(Form::Byte)
            }
            ("c", Some('"')) => {
                self.at += 1;
                self.string(Form::C)
            }
            ("r", Some('"' | '#')) => self.raw(Form::Str),
            ("br", Some('"' | '#')) => self.raw(Form::Byte),
            ("cr", Some('"' | '#')) => self.raw(Form::C),
            _ => Ok(()),
        }
    }

    /// A raw literal after its `r`, or a raw identifier (`r#name`), which is left to the caller.
    fn raw(&mut self, form: Form) -> Result<(), SyntaxError> {
        let hashes = self.chars.get(self.at..).unwrap_or_default().iter().take_while(|&&c| c == '#').count();
        if self.peek(hashes) != Some('"') {
            return Ok(());
        }
        let first = self.line;
        self.at += hashes + 1;
        let mut text = String::new();
        loop {
            match self.next() {
                None => return Err(Self::error(first, "unterminated raw string literal")),
                Some('"') if (0..hashes).all(|n| self.peek(n) == Some('#')) => break,
                Some('\r') if self.peek(0) == Some('\n') => {}
                Some(c) => {
                    self.line += usize::from(c == '\n');
                    text.push(c);
                }
            }
        }
        self.at += hashes;
        self.push(first, text, form);
        Ok(())
    }

    /// The index after the whitespace and comments at `at`.
    fn skip_space(&self, mut at: usize) -> usize {
        loop {
            match (self.chars.get(at), self.chars.get(at + 1)) {
                (Some(c), _) if c.is_whitespace() => at += 1,
                (Some('/'), Some('/')) => {
                    while self.chars.get(at).is_some_and(|&c| c != '\n') {
                        at += 1;
                    }
                }
                (Some('/'), Some('*')) => {
                    let mut depth = 0_usize;
                    loop {
                        match (self.chars.get(at), self.chars.get(at + 1)) {
                            (None, _) => return at,
                            (Some('/'), Some('*')) => {
                                depth += 1;
                                at += 2;
                            }
                            (Some('*'), Some('/')) => {
                                depth -= 1;
                                at += 2;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => at += 1,
                        }
                    }
                }
                _ => return at,
            }
        }
    }

    /// The end of a `#[cfg(test)]` or `#![cfg(test)]` attribute at the current `#`, and whether it
    /// is the inner form.
    fn cfg_test_end(&self) -> Option<(usize, bool)> {
        let mut at = self.at + 1;
        let inner = self.chars.get(at) == Some(&'!');
        at += usize::from(inner);
        for token in ["[", "cfg", "(", "test", ")", "]"] {
            at = self.skip_space(at);
            let end = at + token.chars().count();
            if !self.chars.get(at..end).is_some_and(|got| got.iter().copied().eq(token.chars())) {
                return None;
            }
            if token.starts_with(is_word) && self.chars.get(end).is_some_and(|&c| is_word(c)) {
                return None;
            }
            at = end;
        }
        Some((at, inner))
    }

    fn hash(&mut self) {
        let Some((end, inner)) = self.cfg_test_end() else {
            self.at += 1;
            return;
        };
        let skipped = self.chars.get(self.at..end).unwrap_or_default();
        self.line += skipped.iter().filter(|&&c| c == '\n').count();
        self.at = end;
        if self.test.is_none() {
            let level = self.delims.len();
            self.test = Some(if inner {
                TestRegion::Inner { level }
            } else {
                TestRegion::Item { level, start: end, opened: false, item: false, semi: false, block: false }
            });
        }
    }

    /// A `=>` at the level of a test-only arm: its expression starts after it.
    fn arrow(&mut self) {
        self.at += 2;
        if let Some(TestRegion::Item { level, item: false, .. }) = self.test
            && level == self.delims.len()
        {
            self.test = Some(TestRegion::Item { level, start: self.at, opened: false, item: false, semi: false, block: false });
        }
    }

    /// Whether the code at `start` is a block expression whose first `{` is at the current index:
    /// the block itself, a macro's braces, a labeled block or loop, or a block keyword's expression.
    /// Outer attributes before it are passed over.
    fn starts_block(&self, start: usize) -> bool {
        let mut at = self.skip_space(start);
        while self.chars.get(at) == Some(&'#') {
            at = self.skip_space(at + 1);
            if self.chars.get(at) != Some(&'[') {
                return false;
            }
            let mut depth = 0_usize;
            loop {
                match self.chars.get(at) {
                    None => return false,
                    Some('[') => depth += 1,
                    Some(']') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            at = self.skip_space(at + 1);
        }
        at == self.at || self.chars.get(at) == Some(&'\'') || BLOCK_KEYWORDS.contains(&self.word_at(at).as_str()) || self.macro_braces(at)
    }

    /// Whether the code at `at` is a macro path, `!` and the `{` at the current index.
    fn macro_braces(&self, mut at: usize) -> bool {
        if self.chars.get(at..at + 2) == Some(&[':', ':'][..]) {
            at = self.skip_space(at + 2);
        }
        loop {
            let len = self.word_at(at).chars().count();
            if len == 0 {
                return false;
            }
            at = self.skip_space(at + len);
            if self.chars.get(at..at + 2) != Some(&[':', ':'][..]) {
                break;
            }
            at = self.skip_space(at + 2);
        }
        self.chars.get(at) == Some(&'!') && self.skip_space(at + 1) == self.at
    }

    fn word_at(&self, at: usize) -> String {
        self.chars.get(at..).unwrap_or_default().iter().take_while(|&&c| is_word(c)).collect()
    }

    fn open(&mut self, c: char) {
        if let Some(TestRegion::Item { level, start, opened: false, item, semi, .. }) = self.test
            && level == self.delims.len()
            && c == '{'
        {
            let block = !item && self.starts_block(start);
            self.test = Some(TestRegion::Item { level, start, opened: true, item, semi, block });
        }
        self.delims.push((c, self.line));
        self.at += 1;
    }

    fn close(&mut self, c: char) -> Result<(), SyntaxError> {
        let expected = match self.delims.pop() {
            Some(('{', _)) => '}',
            Some(('(', _)) => ')',
            Some(_) => ']',
            None => return Err(Self::error(self.line, "unmatched closing delimiter")),
        };
        if c != expected {
            return Err(Self::error(self.line, "mismatched closing delimiter"));
        }
        self.at += 1;
        let depth = self.delims.len();
        let ends = match self.test {
            Some(TestRegion::Item { level, start, opened, item, semi, block }) => {
                let block_end = opened && depth == level && c == '}';
                if block_end && !item && block && self.postfix_follows() {
                    // A method call, field or `?` continues the expression to its `;` or `,`.
                    self.test = Some(TestRegion::Item { level, start, opened, item, semi, block: false });
                    false
                } else {
                    depth < level || (block_end && if item { !semi } else { block && self.word_at(self.skip_space(self.at)) != "else" })
                }
            }
            Some(TestRegion::Inner { level }) => depth < level,
            None => false,
        };
        if ends {
            self.test = None;
        }
        Ok(())
    }

    /// Whether a `?` or a `.` (not `..`) follows the current index, past whitespace and comments.
    fn postfix_follows(&self) -> bool {
        let at = self.skip_space(self.at);
        match self.chars.get(at) {
            Some('?') => true,
            Some('.') => self.chars.get(at + 1) != Some(&'.'),
            _ => false,
        }
    }

    /// A `;` ends a test-only statement, or an item that has no body or ends at its `;`, and a `,`
    /// inside a delimiter ends a test-only field, argument or arm. A `,` in an item's header
    /// (`impl<A, B>`, a `where` clause) does not.
    fn item_end(&mut self, comma: bool) {
        if let Some(TestRegion::Item { level, opened, item, semi, .. }) = self.test
            && level == self.delims.len()
            && if comma { level > 0 && !item } else { !item || !opened || semi }
        {
            self.test = None;
        }
        self.at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNED: &[&str] = &["Amber Tool", "AzureKit", "BlueCore"];
    const ALLOWED: &[&str] = &["AzureKit Design"];
    const POLICY: Policy<'static> = Policy { allowed: ALLOWED, ..Policy::new(BANNED) };

    fn banned_display(text: &str) -> bool {
        !POLICY.words_in(text).is_empty()
    }

    fn literals(source: &str) -> Vec<String> {
        super::literals(source, Literals::default()).unwrap().into_iter().map(|lit| lit.text).collect()
    }

    fn write(dir: &Path, path: &str, text: &str) {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn hits(findings: &[Finding]) -> Vec<(String, usize, &str)> {
        findings.iter().map(|f| (f.path.to_string_lossy().replace('\\', "/"), f.line, f.word.as_str())).collect()
    }

    #[test]
    fn production_display_names_stay_neutral() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("crates/ui");
        write(&base, "src/lib.rs", "pub const LABEL: &str = \"Open\";\n#[cfg(test)]\nmod tests { const S: &str = \"AzureKit\"; }\n");
        write(&base, "src/tests.rs", "const S: &str = \"AzureKit\";\n");
        write(&base, "src/panel_tests.rs", "const S: &str = \"BlueCore\";\n");
        write(&base, "src/tests/helper.rs", "const S: &str = \"Amber Tool\";\n");
        write(&base, "src/notes.txt", "AzureKit\n");
        write(dir.path(), "crates/engine/src/cmd/file.rs", "pub const DESIGN: &str = \"AzureKit Design\";\n");
        let roots = [Path::new("src"), Path::new("../engine/src/cmd")];
        let inputs: Vec<Input<'_>> = roots.iter().map(|root| Input::rust(root)).collect();
        let found = scan(&base, &inputs, &POLICY).unwrap();
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn guard_distinguishes_labels_from_identifiers_and_comments() {
        for name in BANNED {
            assert!(banned_display(name), "{name}");
        }
        for name in ALLOWED {
            assert!(!banned_display(name), "{name}");
        }
        assert!(!banned_display("filter.azureKit"));
        assert!(!banned_display("tool.guidedBlueCore"));
        assert!(!banned_display("insert.amberTool"));
        assert_eq!(literals("// \"ignored\"\n/* \"ignored\" */ \"label\""), ["label"]);
        assert_eq!(literals("r#\"raw label\"#"), ["raw label"]);
    }

    #[test]
    fn guard_scans_production_after_test_only_items() {
        let source = r###"#[cfg(test)] mod tests { let _ = "ignored"; }
        #[cfg(test)] use fixtures::Helper;
        fn production() { let _ = r#"live label"#; }
    "###;
        assert_eq!(literals(source), ["live label"]);
        assert_eq!(literals("#[cfg(test)] mod tests; fn live() { let _ = \"live label\"; }"), ["live label"]);
    }

    #[test]
    fn guard_skips_escaped_character_literals() {
        assert_eq!(literals(r#"let q = '\"'; "label""#), ["label"]);
        assert_eq!(literals(r#"let q = '\''; "label""#), ["label"]);
        assert_eq!(literals(r#"let q = '\\'; "label""#), ["label"]);
        assert_eq!(literals(r#"let q = '\u{22}'; "label""#), ["label"]);
    }

    #[test]
    fn guard_decodes_whitespace_escapes() {
        assert_eq!(literals(r#""one\ntwo\tthree\rfour\0""#), ["one\ntwo\tthree\rfour\0"]);
        assert_eq!(literals("\"joined\\\n   label\""), ["joinedlabel"]);
        for name in BANNED {
            let source = format!("\"\\t{name}\\n\"");
            assert!(literals(&source).iter().any(|label| banned_display(label)));
        }
    }

    #[test]
    fn guard_scans_after_escaped_character_literals() {
        let source = r###"let a = '\"'; let b = '\''; let c = '\\';
        let d = '\u{22}'; let e = '\x22';
        let label = "Amber Tool";
        fn lifetime<'a>(value: &'a str) { 'outer: loop { break 'outer; } let _ = "BlueCore"; }
    "###;
        assert_eq!(literals(source), ["Amber Tool", "BlueCore"]);
    }

    #[test]
    fn guard_scans_production_after_test_only_items_and_parses_raw_labels() {
        let source = r###"#[cfg(test)] mod tests { let _ = "AzureKit"; }
        #[cfg(test)] use fixtures::Helper;
        fn production() { let _ = r#"BlueCore"#; }
    "###;
        assert_eq!(literals(source), ["BlueCore"]);
        assert!(literals(source).iter().any(|label| banned_display(label)));
        assert_eq!(literals("#[cfg(test)] mod tests; fn live() { let _ = \"AzureKit\"; }"), ["AzureKit"]);
    }

    #[test]
    fn guard_skips_escaped_character_literals_without_hiding_labels() {
        let source = r#"let quote = '\"'; let label = "BlueCore"; // "ignored""#;
        assert_eq!(literals(source), ["BlueCore"]);
        assert!(literals(source).iter().any(|label| banned_display(label)));
        assert_eq!(literals(r#"let quote = '\"'; // "BlueCore""#), Vec::<String>::new());
    }

    #[test]
    fn source_read_failures_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "const S: &str = \"Open\";\n");
        // A file cannot be read as a folder, and a missing root is an error, not an empty scan.
        for root in ["src/lib.rs/not-a-folder.rs", "missing"] {
            let err = scan(dir.path(), &[Input::rust(Path::new(root))], &POLICY).unwrap_err();
            assert!(matches!(err, Error::Io { .. }), "{root}: {err}");
        }
        let err = scan(dir.path(), &[Input::text(Path::new("missing.txt"))], &POLICY).unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err}");
    }

    const DENY: &[&str] = &["azurekit", "bluecore", "copperline", "dunemark", "emberly", "fernwave", "glintpad"];
    const VECTOR: Policy<'static> = Policy {
        case: MatchCase::AsciiInsensitive,
        allow_marker: Some("name-ok"),
        literals: Literals { byte_strings: true, c_strings: true },
        ..Policy::new(DENY)
    };

    fn rust_hits(src: &str) -> Vec<(usize, &str)> {
        let found = scan_source(Path::new("lib.rs"), src, Kind::Rust, &VECTOR).unwrap();
        found.iter().map(|f| (f.line, DENY.iter().copied().find(|w| *w == f.word).unwrap())).collect()
    }

    #[test]
    fn rust_literals_skip_comments_identifiers_tests_and_marked_lines() {
        let src = r##"// BlueCore in a comment
/// AzureKit in a doc comment
/* block AzureKit /* nested */ Copperline */
fn f() { let a = "Made for BlueCore users"; let k = "copyEmberly"; let q = '"'; let l: &'static str = r#"raw "Dunemark" text"#; }
const OLD: &str = "BlueCore Defaults"; // name-ok: legacy alias
fn g() -> &'static str { "line\nEMBERLY" }
#[cfg(test)]
mod tests {
    fn t() { let x = "AzureKit"; if true { let y = "Copperline"; } }
}
#[cfg(test)]
mod other;
fn h() { let s = b"Fernwave"; let m = "multi
line Glintpad"; }
"##;
        assert_eq!(rust_hits(src), [(4, "bluecore"), (4, "dunemark"), (6, "emberly"), (13, "fernwave"), (13, "glintpad")]);
        let lits = super::literals(src, VECTOR.literals).unwrap();
        let multi = lits.iter().find(|l| l.text.contains("Glintpad")).unwrap();
        assert_eq!((multi.first_line, multi.last_line), (13, 14));
    }

    #[test]
    fn whole_words_only() {
        assert_eq!(VECTOR.words_in("AzureKit's app"), ["azurekit"]);
        assert_eq!(VECTOR.words_in("an BLUECORE-style UI"), ["bluecore"]);
        assert!(VECTOR.words_in("bluecores bluecoreish copyEmberly emberlyMode").is_empty());
    }

    #[test]
    fn docs_packaging_and_manifests() {
        let scan = |text: &str, kind: Kind| -> Vec<(usize, String)> {
            scan_source(Path::new("f"), text, kind, &VECTOR).unwrap().into_iter().map(|f| (f.line, f.word)).collect()
        };
        let one = |line: usize, word: &str| vec![(line, word.to_string())];
        assert_eq!(scan("Keywords=vector;bluecore;\n", Kind::Text), one(1, "bluecore"));
        assert_eq!(scan("Like AzureKit apps\nnot here: AzureKit # name-ok\n", Kind::Text), one(1, "azurekit"));
        let desktop = "# shared type (name-ok)\nMimeType=application/bluecore;\n# a comment\nMimeType=application/bluecore;\n";
        assert_eq!(scan(desktop, Kind::TextMarkAbove), one(4, "bluecore"), "a `#` line marks the line below it only");
        assert_eq!(
            scan(desktop, Kind::Text),
            [(2, "bluecore".to_string()), (4, "bluecore".to_string())],
            "only mark-above files take a mark from above"
        );
        assert_eq!(scan("x = 1 # name-ok\nBlueCore\n", Kind::TextMarkAbove), one(2, "bluecore"), "a code line's comment marks that line only");
        let manifest = "[package]\nname = \"azurekit-thing\"\ndescription = \"BlueCore-style UI\"\n";
        assert_eq!(scan(manifest, Kind::Manifest), one(3, "bluecore"));
        let multi =
            "[package]\ndescription = \"\"\"\nA BlueCore\n-style UI\"\"\"\nkeywords = [\"azurekit\"]\ndescription = '''one line''' # azurekit\n";
        assert_eq!(scan(multi, Kind::Manifest), one(2, "bluecore"), "a multi-line value is read whole; a comment is not read");
        assert_eq!(scan("[package]\ndescription = '''\nAzureKit'''\nname = \"bluecore\"\n", Kind::Manifest), one(2, "azurekit"));
        let escaped = "[package]\ndescription = \"\"\"say \\\"\"\" and\nBlueCore \\\\\"\"\"\nname = \"azurekit\"\n";
        assert_eq!(scan(escaped, Kind::Manifest), one(2, "bluecore"), "an escaped quote does not close the description");
        assert_eq!(
            scan("[package]\ndescription = \"Blue\\u0043ore and Azure\\U0000004Bit\"\n", Kind::Manifest),
            [(2, "azurekit".to_string()), (2, "bluecore".to_string())]
        );
        assert_eq!(scan("[package]\ndescription = \"\"\"Blue\\\n    Core\"\"\"\n", Kind::Manifest), one(2, "bluecore"));
        assert!(scan("[package]\ndescription = \"Neutral\" # BlueCore\ndescriptions = \"AzureKit\"\n", Kind::Manifest).is_empty());
        let notes = "[package]\nnotes = \"\"\"\n[dependencies]\ndescription = \"BlueCore\"\n\"\"\" # \"\"\"\nquoted = 'a \"\"\" b'\ndescription = \"AzureKit\"\n";
        assert_eq!(scan(notes, Kind::Manifest), one(7, "azurekit"), "lines inside another key's string are not keys or headers");
        let keys = "package.description = 'BlueCore'\n[package]\n\"description\" = \"AzureKit\"\n'description'='Copperline'\n# description = \"Dunemark\"\nother.description = \"Emberly\"\n";
        assert_eq!(scan(keys, Kind::Manifest), [(1, "bluecore".to_string()), (3, "azurekit".to_string()), (4, "copperline".to_string())]);
        let marked =
            "[package]\ndescription = \"\"\"\nBlueCore long\n\"\"\" # name-ok\nname = \"y\"\n[workspace.package]\ndescription = \"BlueCore\"\n";
        assert_eq!(scan(marked, Kind::Manifest), one(7, "bluecore"), "a marker on any line of the value counts");
        let text = scan_source(Path::new("Cargo.toml"), "[package]\ndescription = '''\nBlueCore\nstyle'''\n", Kind::Manifest, &VECTOR).unwrap();
        assert_eq!(text[0].text, "BlueCore\nstyle");
    }

    #[test]
    fn only_package_descriptions_are_read() {
        let scan = |text: &str| -> Vec<(usize, String)> {
            scan_source(Path::new("Cargo.toml"), text, Kind::Manifest, &VECTOR).unwrap().into_iter().map(|f| (f.line, f.word)).collect()
        };
        let other = "description = \"Glintpad\"\n[dependencies.foo]\ndescription = \"BlueCore\"\n[package.metadata.x]\ndescription = \"AzureKit\"\n\
            [[bin]]\ndescription = \"Copperline\"\n[[package]]\ndescription = \"Copperline\"\n[package]\nmetadata.description = \"Dunemark\"\n\
            dependencies = { description = \"Dunemark\" }\n[workspace]\ndescription = \"Emberly\"\n";
        assert!(scan(other).is_empty(), "{:?}", scan(other));
        assert_eq!(scan("\u{feff}[package]\ndescription = \"BlueCore\"\n"), [(2, "bluecore".to_string())], "a byte order mark is skipped");
        assert_eq!(scan("\u{feff}package.description = \"AzureKit\"\n"), [(1, "azurekit".to_string())]);
        let tables = "[ \"package\" ] # x\ndescription = \"BlueCore\"\n[dependencies.foo]\nx = [\n  [\"package\"]\n]\ndescription = \"AzureKit\"\n\
            [workspace . 'package']\ndescription = \"Fernwave\"\n[dependencies]\nworkspace.package.description = \"Emberly\"\n\
            y = [ \"\"\"\n]\n\"\"\", ]\n[package]\ndescription = \"Glintpad\"\n";
        assert_eq!(
            scan(tables),
            [(2, "bluecore".to_string()), (9, "fernwave".to_string()), (16, "glintpad".to_string())],
            "a line inside an array is not a header, and an array closed after a multi-line string is closed"
        );
    }

    #[test]
    fn scope() {
        for (path, test) in [
            ("src/cmd/file.rs", false),
            ("src/tests_file.rs", false),
            ("src/text_tests.rs", true),
            ("src/cms/tests.rs", true),
            ("tests/command_sweep.rs", true),
            ("src/tests/helper.rs", true),
            ("../engine/tests/a.rs", true),
        ] {
            assert_eq!(test_path(Path::new(path)), test, "{path}");
        }
        let dir = tempfile::tempdir().unwrap();
        for file in ["src/a.rs", "src/b.txt", "src/c.rs.bak", "src/d/e.rs", "src/tests/f.rs", "src/g_tests.rs"] {
            write(dir.path(), file, "\"AzureKit\"\n");
        }
        let found = scan(dir.path(), &[Input::rust(Path::new("src"))], &POLICY).unwrap();
        assert_eq!(hits(&found), [("src/a.rs".to_string(), 1, "AzureKit"), ("src/d/e.rs".to_string(), 1, "AzureKit")]);
        let named = scan(dir.path(), &[Input::rust(Path::new("src/g_tests.rs"))], &POLICY).unwrap();
        assert_eq!(hits(&named), [("src/g_tests.rs".to_string(), 1, "AzureKit")], "a file input is read whatever its name");
        write(dir.path(), "tests/app/src/lib.rs", "\"AzureKit\"\n");
        let below = scan(dir.path(), &[Input::rust(Path::new("tests/app/src"))], &POLICY).unwrap();
        assert_eq!(hits(&below), [("tests/app/src/lib.rs".to_string(), 1, "AzureKit")], "folders above the input do not count");
    }

    #[test]
    fn injected_strings_fail_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "crates/x/src/lib.rs", "pub const LABEL: &str = \"Export for BlueCore\"; // a comment: AzureKit\n");
        write(root, "crates/x/Cargo.toml", "[package]\nname = \"x\"\ndescription = \"A Copperline-style crate\"\n");
        write(root, "docs/x.md", "Works like Copperline.\n");
        write(root, "docs/x.txt", "Works like Copperline.\n");
        write(root, "packaging/x.desktop", "Keywords=vector;bluecore;\n");
        let marked = "# shared type (name-ok)\nMimeType=application/bluecore;\n";
        write(root, "packaging/linux/app.desktop", marked);
        write(root, "packaging/x.yaml", marked);
        write(root, "README.md", "AzureKit trademark notice\n");
        let path = Path::new;
        let inputs = [
            Input::rust(path("crates")),
            Input::manifest(path("crates/x/Cargo.toml")),
            Input::text(path("docs/x.txt")),
            Input::text(path("packaging/x.desktop")),
            Input::text_mark_above(path("packaging/linux/app.desktop")),
            Input::text(path("packaging/x.yaml")),
        ];
        let found = scan(root, &inputs, &VECTOR).unwrap();
        let expected = [
            ("crates/x/Cargo.toml", 3, "copperline"),
            ("crates/x/src/lib.rs", 1, "bluecore"),
            ("docs/x.txt", 1, "copperline"),
            ("packaging/x.desktop", 1, "bluecore"),
            ("packaging/x.yaml", 2, "bluecore"),
        ];
        assert_eq!(hits(&found), expected.map(|(p, l, w)| (p.to_string(), l, w)));
    }

    #[test]
    fn the_tree_is_clean() {
        // This crate's own sources hold none of the synthetic words outside their tests, and the
        // lexer reads every one of them.
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = scan(base, &[Input::rust(Path::new("src"))], &VECTOR).unwrap();
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn escapes_are_decoded_before_matching() {
        for source in [r#""A\x7AureKit""#, r#""A\u{7A}ureKit""#, r#""A\u{7_A}ureKit""#, r#""Azure\u{4B}it""#] {
            assert_eq!(literals(source), ["AzureKit"], "{source}");
        }
        assert_eq!(literals(r#""Blue\x43ore""#), ["BlueCore"]);
        // A continuation leaves out the line break and the indentation, and adds no space.
        assert_eq!(literals("\"Blue\\\n      Core\""), ["BlueCore"]);
        assert_eq!(literals("\"Blue\\\r\n\tCore\""), ["BlueCore"]);
        assert_eq!(literals(r#""say \"BlueCore\" \\ ok""#), [r#"say "BlueCore" \ ok"#]);
    }

    #[test]
    fn test_only_items_of_every_form_are_skipped() {
        let source = r#"#[cfg(test)] pub mod t { const S: &str = "AzureKit"; }
#[cfg(test)] fn f() { let _ = "AzureKit"; }
#[cfg(test)] const C: &str = "AzureKit";
#[cfg(test)] static D: [&str; 2] = ["AzureKit", "AzureKit"];
#[cfg(test)]
#[allow(dead_code)]
fn g() -> [u8; 2] { let _ = "AzureKit"; [0, 1] }
# [ cfg ( test ) ] mod spaced { "AzureKit"; }
struct S { #[cfg(test)] probe: &'static str, label: &'static str }
fn h() { let s = S { #[cfg(test)] probe: "AzureKit", label: "one" }; }
#[cfg(test)] mod declared;
mod generic {
    #[cfg(test)]
    impl<A, B> Probe for Pair<A, B> where A: Clone, B: Copy { fn p() { let _ = "AzureKit"; } }
}
mod inner {
    #![cfg(test)]
    fn i() { let _ = "AzureKit"; }
}
#[cfg(test)] const PICK: &str = if true { "AzureKit" } else { "AzureKit" };
#[cfg(/* a reason */ test)] const NOTED: &str = "AzureKit";
#[cfg( // a reason
    test)] const NOTED_LINE: &str = "AzureKit";
#[cfg(test)] const fn k() -> u8 { 1 }
#[cfg(not(test))] const E: &str = "two";
#[cfg(tests)] const F: &str = "three";
fn live() { let _ = "four"; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four"]);
        assert_eq!(literals("#![cfg(test)]\nfn a() { \"AzureKit\"; }\nfn b() { \"AzureKit\"; }\n"), Vec::<String>::new());
    }

    #[test]
    fn test_only_arms_and_statements_end_after_their_last_block() {
        let source = r#"fn pick(x: u8, c: bool) -> &'static str {
    match x {
        #[cfg(test)] 0 => if c { "AzureKit" } else { "AzureKit" },
        #[cfg(test)] 1 => if c { "AzureKit" } else if !c { "AzureKit" } else { "AzureKit" }
        #[cfg(test)] 2 => { "AzureKit" }
        3 => "one",
        #[cfg(test)] 4 => Probe { a: "AzureKit" }.label("AzureKit"),
        #[cfg(test)] Probe { a: 5 } => "AzureKit",
        #[cfg(test)] 6 => match c { true => "AzureKit", false => "AzureKit" }
        #[cfg(test)] #[allow(unused)] 7 => unsafe { "AzureKit" }
        _ => "two",
    }
}
fn statements(c: bool) {
    #[cfg(test)] if c { "AzureKit"; } else { "AzureKit"; }
    let _ = "three";
    #[cfg(test)] { "AzureKit"; }
    #[cfg(test)] probe! { "AzureKit" }
    #[cfg(test)] crate::probe! /* noted */ { "AzureKit" }
    let _ = "four";
    #[cfg(test)] Probe { a: "AzureKit" }.run("AzureKit");
    #[cfg(test)] 'outer: loop { "AzureKit"; }
    #[cfg(test)] const { "AzureKit"; }
    let _ = "five";
}
fn tail() -> Probe {
    #[cfg(test)] Probe { a: "AzureKit" }.with("AzureKit")
}
fn after() { let _ = "six"; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six"]);
    }

    #[test]
    fn test_only_blocks_keep_their_method_calls_and_question_marks() {
        let source = r#"fn statements(c: bool) -> Result<(), Probe> {
    #[cfg(test)] { "x" }.replace("x", "AzureKit");
    let _ = "one";
    #[cfg(test)] unsafe { probe("x") }?;
    #[cfg(test)] unsafe { probe("x") }?.with("AzureKit");
    let _ = "two";
    #[cfg(test)] { "x" } /* noted */ .replace("x", "AzureKit");
    #[cfg(test)] { "x" } // noted
        .replace("x", "AzureKit");
    #[cfg(test)] match c { _ => probe("x") }?.with("AzureKit");
    #[cfg(test)] probe! { "x" }.with("AzureKit");
    #[cfg(test)] { (1, "x") }.0.with("AzureKit");
    let _ = "three";
    #[cfg(test)] { "AzureKit"; }
    ..;
    let _ = "four";
    Ok(())
}
fn pick(x: u8, c: bool) -> String {
    match x {
        #[cfg(test)] 8 => if c { "x" } else { "y" }.to_string().replace("x", "AzureKit"),
        9 => "five".into(),
        #[cfg(test)] 10 => { "x" }.replace("x", "AzureKit"),
        _ => "six".into(),
    }
}
fn tail() -> String {
    #[cfg(test)] { "x" }.replace("x", "AzureKit")
}
fn after() { let _ = "seven"; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six", "seven"]);
    }

    #[test]
    fn the_marker_counts_only_in_a_comment() {
        let policy = Policy { allow_marker: Some("name-ok"), ..POLICY };
        let rust = |src: &str| scan_source(Path::new("lib.rs"), src, Kind::Rust, &policy).unwrap().len();
        assert_eq!(rust("let label = \"AzureKit name-ok\";"), 1);
        assert_eq!(rust("let label = \"AzureKit\"; // name-ok: compatibility"), 0);
        assert_eq!(rust("let label = \"AzureKit\"; /* name-ok */"), 0);
        assert_eq!(rust("/* name-ok */ let label = \"AzureKit\";"), 0);
        assert_eq!(rust("/* name-ok\n */ let label = \"AzureKit\";"), 1, "a block comment marks only its lines that hold the marker");
        assert_eq!(rust("/* a\n b name-ok */ let label = \"AzureKit\";"), 0);
        assert_eq!(rust("/* a\n\n name-ok */ let label = \"AzureKit\";"), 0, "the marker's line is the comment's third");
        assert_eq!(rust("/* a\n name-ok\n */ let label = \"AzureKit\nx\";"), 1);
        assert_eq!(rust("let a = \"name-ok\"; let b = \"AzureKit\";"), 1);
        assert_eq!(rust("// name-ok\nlet label = \"AzureKit\";"), 1, "the line above does not count in Rust");
        assert_eq!(rust("let label = \"Azure\nKit\nAzureKit\"; // name-ok"), 0, "a mark on any line of the literal counts");
    }

    #[test]
    fn letters_of_any_script_are_part_of_a_word() {
        for policy in [POLICY, Policy { case: MatchCase::AsciiInsensitive, ..POLICY }] {
            assert!(policy.words_in("éAzureKit").is_empty());
            assert!(policy.words_in("AzureKitß").is_empty());
            assert!(policy.words_in("AzureKit_2").is_empty());
            assert_eq!(policy.words_in("é AzureKit, é"), ["AzureKit"]);
        }
    }

    #[test]
    fn case_follows_the_policy() {
        assert!(POLICY.words_in("azurekit").is_empty());
        assert_eq!(Policy { case: MatchCase::AsciiInsensitive, ..POLICY }.words_in("azureKIT and amber TOOL"), ["Amber Tool", "AzureKit"]);
    }

    #[test]
    fn allowed_literals_and_exceptions_match_whole_text() {
        fn group(text: &str) -> bool {
            text.strip_prefix("AzureKit Design › ").is_some_and(|rest| !banned_display(rest))
        }
        let policy = Policy { exceptions: &[group], ..POLICY };
        assert!(policy.words_in("AzureKit Design").is_empty());
        assert_eq!(policy.words_in("AzureKit Design extra"), ["AzureKit"]);
        assert!(policy.words_in("AzureKit Design › Layout").is_empty());
        assert_eq!(policy.words_in("AzureKit Design › BlueCore Styles"), ["AzureKit", "BlueCore"]);
    }

    #[test]
    fn byte_and_c_strings_are_read_only_when_asked() {
        let source = r##"let a = b"AzureKit"; let b = br#"AzureKit"#; let c = c"BlueCore"; let d = cr"BlueCore"; let e = b'"'; let f = b'\xFF'; let g = r#"plain"#;"##;
        assert_eq!(literals(source), ["plain"]);
        let bytes = super::literals(source, Literals { byte_strings: true, c_strings: false }).unwrap();
        assert_eq!(bytes.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["AzureKit", "AzureKit", "plain"]);
        let both = super::literals(source, Literals { byte_strings: true, c_strings: true }).unwrap();
        assert_eq!(both.len(), 5);
        assert_eq!(literals("let r#type = 1; let s = \"BlueCore\";"), ["BlueCore"]);
    }

    #[test]
    fn raw_strings_close_at_the_same_number_of_hashes() {
        assert_eq!(literals(r###"r##"a "# b"## "next""###), [r##"a "# b"##, "next"]);
        assert_eq!(literals(r#"r"\x41""#), [r"\x41"]);
    }

    #[test]
    fn malformed_source_is_an_error() {
        for (source, line) in [
            ("let a = \"open", 1),
            ("\n let a = r#\"open\"", 2),
            ("/* /* */", 1),
            ("fn f() { }}", 1),
            ("fn f() {\n", 1),
            ("fn f( ]", 1),
            (r#""\q""#, 1),
            (r#""\x80""#, 1),
            (r#""\x4""#, 1),
            (r#""\u{D800}""#, 1),
            (r#""\u{1234567}""#, 1),
            (r#""\u{}""#, 1),
            (r"let c = '\q';", 1),
            (r"let c = '\n", 1),
        ] {
            let err = super::literals(source, Literals::default()).unwrap_err();
            assert_eq!(err.line, line, "{source}: {err}");
        }
        let err = scan_source(Path::new("bad.rs"), "\"open", Kind::Rust, &POLICY).unwrap_err();
        assert!(matches!(err, Error::Syntax { .. }), "{err}");
    }

    #[test]
    fn lines_count_crlf_and_words_at_literal_edges() {
        let src = "fn a() {}\r\n\r\nconst A: &str = \"AzureKit\";\r\nconst B: &str = \"x\r\ny BlueCore\";\r\n";
        let found = scan_source(Path::new("lib.rs"), src, Kind::Rust, &POLICY).unwrap();
        assert_eq!(found.iter().map(|f| (f.line, f.word.as_str())).collect::<Vec<_>>(), [(3, "AzureKit"), (4, "BlueCore")]);
        assert_eq!(found[1].text, "x\ny BlueCore");
        let text = scan_source(Path::new("a.txt"), "one\r\nBlueCore\r\n", Kind::Text, &POLICY).unwrap();
        assert_eq!((text[0].line, text[0].text.as_str()), (2, "BlueCore"));
    }

    #[test]
    fn unreadable_and_oversized_inputs_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("bad")).unwrap();
        fs::write(root.join("bad/lib.rs"), b"const S: &str = \"\xff\";").unwrap();
        let err = scan(root, &[Input::rust(Path::new("bad"))], &POLICY).unwrap_err();
        assert!(matches!(err, Error::InvalidUtf8 { .. }), "{err}");

        write(root, "big/lib.rs", &"/* */".repeat(40));
        let small = Policy { limits: Limits { max_file_bytes: 199, ..DEFAULT_LIMITS }, ..POLICY };
        let err = scan(root, &[Input::rust(Path::new("big"))], &small).unwrap_err();
        assert!(matches!(err, Error::FileTooLarge { limit: 199, .. }), "{err}");
        let exact = Policy { limits: Limits { max_file_bytes: 200, ..DEFAULT_LIMITS }, ..POLICY };
        assert!(scan(root, &[Input::rust(Path::new("big"))], &exact).unwrap().is_empty());

        write(root, "two/a.rs", "\"one\"");
        write(root, "two/b.rs", "\"two\"");
        let total = Policy { limits: Limits { max_total_bytes: 9, ..DEFAULT_LIMITS }, ..POLICY };
        let err = scan(root, &[Input::rust(Path::new("two"))], &total).unwrap_err();
        assert!(matches!(&err, Error::TotalTooLarge { path, .. } if path.ends_with("b.rs")), "{err}");

        write(root, "deep/a/b/c/lib.rs", "\"one\"");
        let shallow = Policy { limits: Limits { max_depth: 2, ..DEFAULT_LIMITS }, ..POLICY };
        let err = scan(root, &[Input::rust(Path::new("deep"))], &shallow).unwrap_err();
        assert!(matches!(err, Error::TooDeep { limit: 2, .. }), "{err}");
        let enough = Policy { limits: Limits { max_depth: 3, ..DEFAULT_LIMITS }, ..POLICY };
        assert!(scan(root, &[Input::rust(Path::new("deep"))], &enough).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "outside/lib.rs", "\"AzureKit\"");
        write(root, "src/lib.rs", "\"BlueCore\"");
        std::os::unix::fs::symlink(root.join("outside"), root.join("src/linked")).unwrap();
        std::os::unix::fs::symlink(root.join("outside/lib.rs"), root.join("src/file_link.rs")).unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("src/dangling.txt")).unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("src/tests.rs")).unwrap();
        let found = scan(root, &[Input::rust(Path::new("src"))], &POLICY).unwrap();
        assert_eq!(hits(&found), [("src/file_link.rs".to_string(), 1, "AzureKit"), ("src/lib.rs".to_string(), 1, "BlueCore")]);
    }

    #[test]
    fn findings_are_sorted_and_files_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/b.rs", "\"BlueCore\"\n\"AzureKit BlueCore Amber Tool\"");
        write(root, "src/a.rs", "\"AzureKit\"");
        let src = Path::new("src");
        let inputs = [Input::rust(Path::new("src/b.rs")), Input::rust(src), Input::rust(src)];
        let found = scan(root, &inputs, &POLICY).unwrap();
        let expected = [
            ("src/a.rs", 1, "AzureKit"),
            ("src/b.rs", 1, "BlueCore"),
            ("src/b.rs", 2, "Amber Tool"),
            ("src/b.rs", 2, "AzureKit"),
            ("src/b.rs", 2, "BlueCore"),
        ];
        assert_eq!(hits(&found), expected.map(|(p, l, w)| (p.to_string(), l, w)));
        assert_eq!(found[0].to_string(), format!("{}:1: \"AzureKit\" in \"AzureKit\"", Path::new("src").join("a.rs").display()));
        let twice = [Input::rust(src), Input::text(Path::new("src/a.rs"))];
        let err = scan(root, &twice, &POLICY).unwrap_err();
        assert!(matches!(&err, Error::Conflict { path } if path.ends_with("a.rs")), "{err}");
    }

    #[test]
    fn policies_that_match_nothing_are_errors() {
        let empty: &[&str] = &[];
        for policy in [Policy::new(empty), Policy::new(&["AzureKit", " "]), Policy { allow_marker: Some(""), ..POLICY }] {
            let err = scan_source(Path::new("f"), "", Kind::Text, &policy).unwrap_err();
            assert!(matches!(err, Error::Policy(_)), "{err}");
        }
    }
}
