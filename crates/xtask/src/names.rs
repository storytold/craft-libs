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
use std::path::{Component, Path, PathBuf};

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
/// A file is identified by its path joined to `base`, and a relative `base` is joined to the
/// current folder (on Windows, a drive-relative `base` such as `C:src` to the current folder of
/// its drive) before any input is listed. A drive-relative input (`C:a.rs`, `C:src`) is joined to
/// the current folder of its drive instead, once, before it is listed, and the files of a folder
/// input are opened under that path. A relative and an absolute path to a file are one file,
/// read once and reported by the input path that sorts first; `..` and links are not resolved.
///
/// A [`Kind::Rust`] folder is read recursively for `.rs` files that [`Policy::skip`] keeps; links to
/// folders are not followed, also when the link is an input. A file input is read whatever its
/// name. Every error stops the scan: a missing input, a file that cannot be read, is not UTF-8 or
/// is over a limit, and a Rust source the lexer cannot read. Inputs must not include binary files.
pub fn scan(base: &Path, inputs: &[Input<'_>], policy: &Policy<'_>) -> Result<Vec<Finding>, Error> {
    policy.validate()?;
    // The base is made absolute once, so that every input is listed and read under one folder.
    let root = absolute_base(base)?;
    // Each input is resolved once, before it is listed: the path that is opened, its kind, and the
    // path that is reported.
    let mut files = Vec::new();
    for input in inputs {
        if input.kind == Kind::Rust {
            collect_rust(&root, input.path, policy, &mut files)?;
        } else {
            files.push((input_path(&root, input.path)?, input.kind, input.path.to_path_buf()));
        }
    }
    // A file is known by the path that is opened, so a relative and an absolute input for it are one.
    files.sort();
    files.dedup_by(|later, first| later.0 == first.0 && later.1 == first.1);
    if let Some(pair) = files.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(Error::Conflict { path: pair[0].2.clone() });
    }
    files.sort_by(|a, b| a.2.cmp(&b.2));
    let mut total = 0;
    let mut findings = Vec::new();
    for (full, kind, path) in &files {
        let source = read_limited(full, path, &policy.limits, &mut total)?;
        findings.extend(scan_source(path, &source, *kind, policy)?);
    }
    Ok(findings)
}

/// `base` as an absolute path: a relative base is joined to the current folder, and a Windows
/// drive-relative base (`C:src`) to the current folder of its drive. `..` is not resolved.
fn absolute_base(base: &Path) -> Result<PathBuf, Error> {
    if base.is_absolute() {
        return Ok(base.to_path_buf());
    }
    let error = |source: io::Error| Error::Io { path: base.to_path_buf(), source };
    let mut parts = base.components();
    match parts.next() {
        Some(Component::Prefix(drive)) => Ok(std::path::absolute(drive.as_os_str()).map_err(error)?.join(parts.as_path())),
        _ => Ok(std::env::current_dir().map_err(error)?.join(base)),
    }
}

/// The absolute path that `path` opens under the absolute `root`: `path` joined to `root`, or for a
/// Windows drive-relative path (`C:a.rs`), joined to the current folder of its drive.
fn input_path(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    match path.components().next() {
        Some(Component::Prefix(_)) if !path.has_root() => absolute_base(path),
        _ => Ok(root.join(path)),
    }
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
/// `;`; a `,` in generic arguments (`f::<A, B>`, `<Pair<A, B> as Fixture>::LABEL`, a field's
/// `field: Pair<A, B>`, a parameter's `mut p: Pair<A, B>`, or a cast's `ptr as *const Pair<A, B>`)
/// or in the parameters of a closure that starts the expression (after any
/// `move` or `&`) does not end it. A raw identifier (`r#let`) is never a keyword. A block expression (a block, a macro's braces, a
/// labeled block, or `if`, `match`, `loop`, `while`, `for`, `unsafe`, `async` or `const`), as the
/// whole statement or as an arm's expression after `=>`, ends at its `}`, or at the last `}` of an
/// `if` with `else` branches; braces in the condition of an `if`, `while`, `match` or `for` (a
/// macro's, or a block operand such as `if { ready } { .. }`) are not its block. When a method
/// call, field or `?` follows that `}`, it runs up to its `,` or `;` instead. Character, byte and
/// lifetime tokens are read, so a quote in them does not start a string. A shebang first line is
/// skipped.
pub fn literals(source: &str, kinds: Literals) -> Result<Vec<Literal>, SyntaxError> {
    lex(source, kinds, None).map(|lexed| lexed.literals)
}

fn contains_word(text: &str, word: &str, case: MatchCase) -> bool {
    match case {
        MatchCase::Exact => any_whole_word(text, text, word),
        MatchCase::AsciiInsensitive => any_whole_word(&text.to_ascii_lowercase(), text, &word.to_ascii_lowercase()),
    }
}

/// Whether `word` occurs in `haystack` at an index where it is a whole word of `text`, which has the
/// same byte offsets. Every occurrence is checked, overlapping ones included.
fn any_whole_word(haystack: &str, text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(found) = haystack.get(from..).and_then(|rest| rest.find(word)) {
        let at = from + found;
        if whole_word(text, at, word.len()) {
            return true;
        }
        from = at + haystack.get(at..).and_then(|rest| rest.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn whole_word(text: &str, at: usize, len: usize) -> bool {
    let before = text.get(..at).and_then(|s| s.chars().next_back());
    let after = text.get(at + len..).and_then(|s| s.chars().next());
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

fn push_char(text: &mut Vec<u8>, c: char) {
    text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

/// The Rust files of the input `path` under the absolute `root`: the path that is opened, the kind,
/// and the path that is reported. `path` is resolved once, so the folder that is listed is the
/// folder whose files are read.
fn collect_rust(root: &Path, path: &Path, policy: &Policy<'_>, out: &mut Vec<(PathBuf, Kind, PathBuf)>) -> Result<(), Error> {
    let full = input_path(root, path)?;
    let link = fs::symlink_metadata(&full).map_err(io_error(path))?.file_type().is_symlink();
    let meta = fs::metadata(&full).map_err(io_error(path))?;
    if meta.is_dir() {
        // A link to a folder is not followed, also when it is an input.
        if link { Ok(()) } else { walk(&full, path, path, policy, 0, out) }
    } else {
        out.push((full, Kind::Rust, path.to_path_buf()));
        Ok(())
    }
}

/// The Rust files in the folder `full`, which is reported as `dir`, under the input `input`.
fn walk(full: &Path, input: &Path, dir: &Path, policy: &Policy<'_>, depth: usize, out: &mut Vec<(PathBuf, Kind, PathBuf)>) -> Result<(), Error> {
    if depth > policy.limits.max_depth {
        return Err(Error::TooDeep { path: dir.to_path_buf(), limit: policy.limits.max_depth });
    }
    let mut entries = fs::read_dir(full).and_then(|entries| entries.collect::<io::Result<Vec<_>>>()).map_err(io_error(dir))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let (opened, path) = (full.join(entry.file_name()), dir.join(entry.file_name()));
        let file_type = entry.file_type().map_err(io_error(&path))?;
        if file_type.is_dir() {
            walk(&opened, input, &path, policy, depth + 1, out)?;
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "rs") || (policy.skip)(path.strip_prefix(input).unwrap_or(&path)) {
            continue;
        }
        // A link to a folder is not followed; a link to a file is read like the file.
        let is_file = if file_type.is_symlink() { fs::metadata(&opened).map_err(io_error(&path))?.is_file() } else { file_type.is_file() };
        if is_file {
            out.push((opened, Kind::Rust, path));
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
    let remaining = limits.max_total_bytes.saturating_sub(*total);
    let over_total = || Error::TotalTooLarge { path: path.to_path_buf(), limit: limits.max_total_bytes };
    if len > remaining {
        return Err(over_total());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    // One byte past the smaller limit shows a file that grew after its length was read.
    let cap = limits.max_file_bytes.min(remaining);
    file.take(cap.saturating_add(1)).read_to_end(&mut bytes).map_err(io_error(path))?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > limits.max_file_bytes {
        return Err(Error::FileTooLarge { path: path.to_path_buf(), limit: limits.max_file_bytes });
    }
    if read > remaining {
        return Err(over_total());
    }
    *total = total.saturating_add(read);
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
    /// set once its first `{` is read, `item` once its first item keyword (`fn`, `impl`, `mod`…) is
    /// read where only qualifiers (`pub`, `unsafe`, `extern "C"`…) come before it, and `semi` when that keyword is `const` (not `const fn`), `let` or `static`, whose items
    /// end at their `;`. Without an item keyword, `block` is set when that first `{` starts a block
    /// expression (a block, a macro's braces, or `if`, `match`, `loop` and the like), which ends at
    /// its `}` unless an `else` follows. A `.` or `?` after that `}` clears `block`, so the expression
    /// ends at its `;` or `,`. A `{` does not count as the first while `pattern` is set (between the
    /// `let` of an `if` or `while` condition and its `=`, or a `for` and its `in`), while `angle`
    /// generic brackets are open (of an item's header, after a `::` or where an operand starts in an
    /// expression, or in the type after `typed` is set), or while `params` is set (between the `|`s
    /// of a closure that starts the expression). A `,` does not end it while `angle` or `params` is
    /// set either. `typed` is set at the `:` after the name of a field or the pattern of a parameter
    /// (`name: Type`, `mut p: Type`), and `cast` at an `as` until the token that ends its type
    /// (`ptr as *const Pair<A, B>`).
    Item {
        level: usize,
        start: usize,
        opened: bool,
        item: bool,
        semi: bool,
        block: bool,
        pattern: bool,
        angle: usize,
        params: bool,
        typed: bool,
        cast: bool,
    },
    /// The rest of the module that `#![cfg(test)]` starts, at this delimiter depth.
    Inner { level: usize },
}

impl TestRegion {
    /// Test-only code at `level` whose code starts at `start`.
    fn item(level: usize, start: usize) -> Self {
        TestRegion::Item {
            level,
            start,
            opened: false,
            item: false,
            semi: false,
            block: false,
            pattern: false,
            angle: 0,
            params: false,
            typed: false,
            cast: false,
        }
    }
}

/// The keywords that start a block expression whose header can hold item keywords that do not start
/// an item (`if let`, `for … in`).
const HEADER_KEYWORDS: &[&str] = &["for", "if", "loop", "match", "while"];

/// What the fields or parameters in a delimiter are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fields {
    /// None are declared there: the delimiter is an expression's, a pattern's or a block's.
    Untyped,
    /// They are declared with their types: a struct's, union's or variant's named fields, or a
    /// function's parameters.
    Typed,
    /// The types of a tuple struct's or tuple variant's fields.
    Tuple,
    /// An enum's variants, whose `{ .. }` and `( .. )` hold typed fields.
    Variants,
    /// An enum's variants, in the discriminant (`V = f(..)`) that runs from a variant's `=` to the
    /// next `,`: its delimiters are an expression's.
    Discriminant,
}

/// A declaration whose fields or parameters have not opened yet.
#[derive(Debug, Clone, Copy)]
struct Declares {
    /// The delimiter depth of its keyword.
    level: usize,
    /// Its keyword: `struct`, `union`, `enum` or `fn`.
    keyword: &'static str,
    /// The generic brackets of its header that are open (`fn f<F: Fn(u8)>`).
    angle: usize,
    /// Whether its `where` clause has started, after which only a `{` opens its fields.
    after_where: bool,
}

struct Lexer<'s> {
    chars: Vec<char>,
    at: usize,
    line: usize,
    kinds: Literals,
    marker: Option<&'s str>,
    /// Open delimiters, the line of each, and what the fields or parameters in each are.
    delims: Vec<(char, usize, Fields)>,
    /// The declarations after a `struct`, `union`, `enum` or `fn` whose `{` or `(` of fields or
    /// parameters has not opened yet, innermost last: a declaration in another's header
    /// (`struct S<T = (fn(),)>`) is pending until its own delimiter opens, and the outer one after.
    declares: Vec<Declares>,
    test: Option<TestRegion>,
    /// Whether the last token read, past whitespace and comments, is a word.
    after_word: bool,
    /// Whether the last token read, past whitespace and comments, is a macro's `!`: a `!` after a
    /// word, not the `!` of a never type (`-> !`).
    after_bang: bool,
    /// The first character of the last token read, past whitespace and comments.
    prev: Option<char>,
    /// The first character of the token before that one.
    prev_prev: Option<char>,
    /// The index where the last word or number read starts.
    last_word: usize,
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
        declares: Vec::new(),
        test: None,
        after_word: false,
        after_bang: false,
        prev: None,
        prev_prev: None,
        last_word: 0,
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
        // A first line that starts with `#!` is a shebang, unless a `[` follows past whitespace and
        // comments that are not doc comments (an inner attribute).
        let start = usize::from(self.peek(0) == Some('\u{feff}'));
        if self.chars.get(start..start + 2) == Some(&['#', '!'][..]) && self.chars.get(self.skip_plain_space(start + 2)) != Some(&'[') {
            while self.peek(0).is_some_and(|c| c != '\n') {
                self.at += 1;
            }
        }
        while let Some(c) = self.peek(0) {
            let begin = self.at;
            let token = !c.is_whitespace() && !(c == '/' && matches!(self.peek(1), Some('/' | '*')));
            if token {
                self.cast_token(c);
            }
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
                '#' => self.hash()?,
                '{' | '(' | '[' => self.open(c),
                '}' | ')' | ']' => self.close(c)?,
                ';' => self.item_end(false),
                ',' => self.item_end(true),
                '=' if self.peek(1) == Some('>') => self.arrow(),
                '=' => self.equals(),
                '<' | '>' => self.angle(c),
                '|' => self.pipe(),
                ':' => self.colon(),
                c if c.is_ascii_digit() => self.number(),
                c if is_word(c) => self.word()?,
                _ => self.at += 1,
            }
            if token {
                self.after_bang = c == '!' && self.after_word;
                self.after_word = is_word(c);
                self.prev_prev = self.prev;
                self.prev = Some(c);
                if is_word(c) {
                    self.last_word = begin;
                }
            }
        }
        match self.delims.last() {
            Some(&(_, line, _)) => Err(Self::error(line, "unclosed delimiter")),
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

    /// A `"…"` body after its opening quote. The text is collected as bytes, so that the `\x`
    /// escapes of a byte or C string decode as UTF-8 together; bytes that are not UTF-8 become
    /// U+FFFD.
    fn string(&mut self, form: Form) -> Result<(), SyntaxError> {
        let first = self.line;
        let mut text = Vec::new();
        loop {
            match self.next() {
                None => return Err(Self::error(first, "unterminated string literal")),
                Some('"') => break,
                Some('\\') => self.escape(form, &mut text, first)?,
                Some('\r') if self.peek(0) == Some('\n') => {}
                Some('\n') => {
                    self.line += 1;
                    text.push(b'\n');
                }
                Some(c) => push_char(&mut text, c),
            }
        }
        self.push(first, String::from_utf8_lossy(&text).into_owned(), form);
        Ok(())
    }

    /// The escape after a backslash, decoded into `text`.
    fn escape(&mut self, form: Form, text: &mut Vec<u8>, first: usize) -> Result<(), SyntaxError> {
        let decoded = match self.next() {
            None => return Err(Self::error(first, "unterminated escape")),
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('0') => '\0',
            Some(c @ ('\\' | '\'' | '"')) => c,
            Some('x') => {
                text.push(self.hex_escape(form)?);
                return Ok(());
            }
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
        push_char(text, decoded);
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

    /// The byte of a `\x` escape after its `x`.
    fn hex_escape(&mut self, form: Form) -> Result<u8, SyntaxError> {
        let digits = (self.peek(0).and_then(|c| c.to_digit(16)), self.peek(1).and_then(|c| c.to_digit(16)));
        let (Some(high), Some(low)) = digits else {
            return Err(Self::error(self.line, "a \\x escape needs two hex digits"));
        };
        self.at += 2;
        let value = u8::try_from(high * 16 + low).unwrap_or(u8::MAX);
        match (value, form) {
            (0..=0x7f, _) | (_, Form::Byte | Form::C) => Ok(value),
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
                self.escape(form, &mut Vec::new(), first)?;
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

    /// An identifier, or the prefix of a byte, C or raw literal. A raw identifier (`r#let`) is read
    /// whole, so its name is not taken for a keyword.
    fn word(&mut self) -> Result<(), SyntaxError> {
        let start = self.at;
        self.number();
        let raw_name = self.peek(1).is_some_and(|c| is_word(c) && !c.is_ascii_digit());
        if self.at == start + 1 && self.chars.get(start) == Some(&'r') && self.peek(0) == Some('#') && raw_name {
            self.at += 1;
            self.number();
        }
        let word: String = self.chars.get(start..self.at).unwrap_or_default().iter().collect();
        self.header_word(start, &word);
        let level = self.delims.len();
        if word == "as"
            && let Some(TestRegion::Item { level: at, item: false, angle: 0, cast, .. }) = &mut self.test
            && *at == level
        {
            *cast = true;
        }
        // `union` is a keyword only before the union's name.
        let named = word != "union" || !self.word_at(self.skip_space(self.at)).is_empty();
        let keyword = ["struct", "union", "enum", "fn"].into_iter().find(|&k| k == word);
        // A `fn` type in a declaration's header (`struct S<F = fn(u8)>`, `struct S<T = (fn(),)>`)
        // is pending on top of it, and its `(` opens before the declaration's fields.
        if let Some(keyword) = keyword.filter(|_| named) {
            self.declares.push(Declares { level: self.delims.len(), keyword, angle: 0, after_where: false });
        } else if word == "where"
            && let Some(declares) = self.declares.last_mut()
            && declares.level == self.delims.len()
        {
            declares.after_where = true;
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

    /// Reads a word at `at` in the header of test-only code: its first item keyword decides how the
    /// item ends, and `let`, `for` and `in` mark where a pattern's braces are.
    fn header_word(&mut self, at: usize, word: &str) {
        let Some(TestRegion::Item { level, start, opened: false, item, .. }) = self.test else { return };
        if level != self.delims.len() {
            return;
        }
        let code = self.code_start(start);
        let lead = self.word_at(code);
        let next = self.word_at(self.skip_space(self.at));
        let header = HEADER_KEYWORDS.contains(&lead.as_str());
        let (keyword, pattern) = match word {
            "let" if matches!(lead.as_str(), "if" | "while") => (false, Some(true)),
            "for" if lead == "for" && at == code => (false, Some(true)),
            "in" if lead == "for" => (false, Some(false)),
            // `const { .. }` is a block expression, not an item.
            "const" if self.chars.get(self.skip_space(self.at)) == Some(&'{') => (false, None),
            // An item keyword counts only after the item's qualifiers, so `fn` in `f::<fn()>(..)` does not.
            _ => (!item && !header && ITEM_KEYWORDS.contains(&word) && self.item_qualifiers(code, at), None),
        };
        if let Some(TestRegion::Item { item, semi, pattern: reading, .. }) = &mut self.test {
            if keyword {
                *item = true;
                *semi = SEMICOLON_KEYWORDS.contains(&word) && !(word == "const" && matches!(next.as_str(), "async" | "extern" | "fn" | "unsafe"));
            }
            if let Some(pattern) = pattern {
                *reading = pattern;
            }
        }
    }

    /// A `=` that is not part of `==`, `!=`, `<=`, `>=` or `..=` ends a `let` pattern, and among an
    /// enum's variants starts a discriminant.
    fn equals(&mut self) {
        let before = self.at.checked_sub(1).and_then(|i| self.chars.get(i)).copied();
        let compares = matches!(before, Some('=' | '!' | '<' | '>' | '.')) || self.peek(1) == Some('=');
        if let Some((_, _, fields @ Fields::Variants)) = self.delims.last_mut()
            && !compares
        {
            *fields = Fields::Discriminant;
        }
        if let Some(TestRegion::Item { level, pattern, .. }) = &mut self.test
            && *level == self.delims.len()
            && !compares
        {
            *pattern = false;
        }
        self.at += 1;
    }

    /// A `<` or `>` (not the `>` of `->`) opens or closes a generic bracket in an item's header
    /// before its body. In an expression, a `<` opens one after a `::` (`pair::<u8, u16>`), where an
    /// operand starts (`<Pair<u8, u16> as Fixture>::LABEL`), and after a word in the type of a field
    /// or parameter when its `>` follows (`field: Pair<A, B>`), and the brackets stay open until
    /// they close, so that a comparison (`a < b`) does not.
    fn angle(&mut self, c: char) {
        let arrow = c == '>' && self.at.checked_sub(1).and_then(|i| self.chars.get(i)) == Some(&'-');
        let turbofish = c == '<' && self.prev == Some(':') && self.prev_prev == Some(':');
        let level_now = self.delims.len();
        if let Some(declares) = self.declares.last_mut()
            && declares.level == level_now
            && !arrow
        {
            declares.angle = if c == '<' { declares.angle + 1 } else { declares.angle.saturating_sub(1) };
        }
        // A tuple field is a type from its start.
        let tuple = self.delims.last().is_some_and(|&(_, _, fields)| fields == Fields::Tuple);
        let generic = c == '<'
            && match self.test {
                Some(TestRegion::Item { level, start, item: false, angle: 0, typed, cast, .. }) if level == level_now => {
                    self.operand_starts(start) || ((typed || tuple || cast) && self.after_word && self.angle_closes())
                }
                _ => false,
            };
        if let Some(TestRegion::Item { level, opened, item, angle, .. }) = &mut self.test
            && *level == self.delims.len()
            && !arrow
            && if *item { !*opened } else { turbofish || generic || *angle > 0 }
        {
            *angle = if c == '<' { *angle + 1 } else { angle.saturating_sub(1) };
        }
        self.at += 1;
    }

    /// Whether an operand starts at the current index in the test-only expression whose code starts
    /// at `start`: at that code, or after an operator or a keyword that takes one.
    fn operand_starts(&self, start: usize) -> bool {
        if self.after_word {
            return matches!(self.word_at(self.last_word).as_str(), "as" | "break" | "if" | "in" | "match" | "return" | "while");
        }
        self.code_start(start) == self.at || matches!(self.prev, Some('=' | '!' | '&' | '|' | '+' | '-' | '*' | '/' | '%' | '^' | ':'))
    }

    /// Reads the token that starts with `c` at the current index against the type of a cast in
    /// test-only code: outside its generic brackets and other delimiters, the type holds words (with
    /// `::` and lifetimes), `&`, `*`, `(` and `[` after `as`, `const`, `mut`, `dyn` or `fn` (or
    /// another of them), the `->` of a function pointer's return type, and `<` and the `>` that close
    /// them, and any other token ends it.
    fn cast_token(&mut self, c: char) {
        let level = self.delims.len();
        let Some(TestRegion::Item { level: at, angle: 0, cast: true, .. }) = self.test else { return };
        if at != level {
            return;
        }
        let prefix = !self.after_word || matches!(self.word_at(self.last_word).as_str(), "as" | "const" | "mut" | "dyn" | "fn");
        let arrow = (c == '-' && self.peek(1) == Some('>')) || (c == '>' && self.at.checked_sub(1).and_then(|i| self.chars.get(i)) == Some(&'-'));
        let keeps = is_word(c) || arrow || matches!(c, ':' | '\'' | '<') || (matches!(c, '&' | '*' | '(' | '[') && prefix);
        if !keeps && let Some(TestRegion::Item { cast, .. }) = &mut self.test {
            *cast = false;
        }
    }

    /// Whether the `<` at the current index has its `>` before a `;`, a `=>`, a closing delimiter of
    /// its level, or a string or `.` that a type does not hold. Whitespace, comments, groups in
    /// delimiters and character literals are passed over whole.
    fn angle_closes(&self) -> bool {
        let mut depth = 0_usize;
        let mut at = self.at + 1;
        loop {
            at = self.skip_space(at);
            let Some(&c) = self.chars.get(at) else { return false };
            match c {
                '(' | '[' | '{' => {
                    let Some(end) = self.group_end(at) else { return false };
                    at = end;
                    continue;
                }
                ')' | ']' | '}' | ';' | '"' | '.' => return false,
                '=' if self.chars.get(at + 1) == Some(&'>') => return false,
                '-' if self.chars.get(at + 1) == Some(&'>') => at += 1,
                '<' => depth += 1,
                '>' if depth == 0 => return true,
                '>' => depth -= 1,
                '\'' => {
                    at += match (self.chars.get(at + 1), self.chars.get(at + 2)) {
                        (Some('\\'), _) => self.chars.get(at + 3..).and_then(|rest| rest.iter().position(|&q| q == '\'')).map_or(1, |end| end + 4),
                        (Some(_), Some('\'')) => 3,
                        _ => 1,
                    };
                    continue;
                }
                c if is_word(c) => {
                    at += self.word_at(at).chars().count();
                    continue;
                }
                _ => {}
            }
            at += 1;
        }
    }

    /// A `:` (not part of a `::`) at the level of a test-only field or parameter, after its name
    /// (and any `pub`) or its pattern (`mut p`, `(a, b)`, `P { x, .. }`), starts its type
    /// (`name: Type`) in the fields of a struct, union or variant and in a function's parameters,
    /// not in a struct expression. A `:` inside the pattern's own delimiters is at a deeper level.
    fn colon(&mut self) {
        let single = self.peek(1) != Some(':') && self.at.checked_sub(1).and_then(|i| self.chars.get(i)) != Some(&':');
        let declared = self.delims.last().is_some_and(|&(_, _, fields)| fields == Fields::Typed);
        if single
            && declared
            && let Some(TestRegion::Item { level, start, item: false, typed: false, angle: 0, .. }) = self.test
            && level == self.delims.len()
            && self.code_start(start) < self.at
            && let Some(TestRegion::Item { typed, .. }) = &mut self.test
        {
            *typed = true;
        }
        self.at += 1;
    }

    /// A `|` at the start of a test-only expression, after any `async`, `move` and borrow (`&`,
    /// `&mut`), opens the parameters of a closure (`|a, b| ..`), and the next `|` at its level
    /// closes them.
    fn pipe(&mut self) {
        if let Some(TestRegion::Item { level, start, item: false, params, .. }) = self.test
            && level == self.delims.len()
        {
            let mut code = self.code_start(start);
            loop {
                let word = self.word_at(code);
                if matches!(word.as_str(), "async" | "move") {
                    code = self.skip_space(code + word.chars().count());
                } else if self.chars.get(code) == Some(&'&') {
                    code = self.skip_space(code + 1);
                    if self.word_at(code) == "mut" {
                        code = self.skip_space(code + 3);
                    }
                } else {
                    break;
                }
            }
            let opens = !params && code == self.at;
            if let Some(TestRegion::Item { params, .. }) = &mut self.test {
                *params = opens;
            }
        }
        self.at += 1;
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
            if self.chars.get(at).is_some_and(|c| c.is_whitespace()) {
                at += 1;
            } else if let Some(end) = self.comment_end(at) {
                at = end;
            } else {
                return at;
            }
        }
    }

    /// The index after the whitespace and the comments that are not doc comments at `at`.
    fn skip_plain_space(&self, mut at: usize) -> usize {
        loop {
            if self.chars.get(at).is_some_and(|c| c.is_whitespace()) {
                at += 1;
            } else if let Some(end) = self.comment_end(at).filter(|_| !self.doc_comment(at)) {
                at = end;
            } else {
                return at;
            }
        }
    }

    /// The index after the comment that starts at `at`, or `None` when none starts there.
    fn comment_end(&self, mut at: usize) -> Option<usize> {
        match (self.chars.get(at), self.chars.get(at + 1)) {
            (Some('/'), Some('/')) => {
                while self.chars.get(at).is_some_and(|&c| c != '\n') {
                    at += 1;
                }
            }
            (Some('/'), Some('*')) => {
                let mut depth = 0_usize;
                loop {
                    match (self.chars.get(at), self.chars.get(at + 1)) {
                        (None, _) => break,
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
            _ => return None,
        }
        Some(at)
    }

    /// Whether the comment at `at` is a doc comment: `//!`, `/*!`, `///` but not `////`, or `/**`
    /// but not `/***` or `/**/`.
    fn doc_comment(&self, at: usize) -> bool {
        let c = |n: usize| self.chars.get(at + n).copied();
        match (c(0), c(1), c(2)) {
            (Some('/'), Some('/' | '*'), Some('!')) => true,
            (Some('/'), Some('/'), Some('/')) => c(3) != Some('/'),
            (Some('/'), Some('*'), Some('*')) => !matches!(c(3), Some('*' | '/')),
            _ => false,
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

    fn hash(&mut self) -> Result<(), SyntaxError> {
        let Some((end, inner)) = self.cfg_test_end() else {
            self.at += 1;
            return Ok(());
        };
        // The attribute holds only its tokens, whitespace and comments; the comments can hold the
        // marker.
        while self.at < end {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('/')) => self.line_comment(),
                (Some('/'), Some('*')) => self.block_comment()?,
                (Some('\n'), _) => {
                    self.line += 1;
                    self.at += 1;
                }
                _ => self.at += 1,
            }
        }
        if self.test.is_none() {
            let level = self.delims.len();
            self.test = Some(if inner { TestRegion::Inner { level } } else { TestRegion::item(level, end) });
        }
        Ok(())
    }

    /// A `=>` at the level of a test-only arm: its expression starts after it.
    fn arrow(&mut self) {
        self.at += 2;
        if let Some(TestRegion::Item { level, item: false, .. }) = self.test
            && level == self.delims.len()
        {
            self.test = Some(TestRegion::item(level, self.at));
        }
    }

    /// Whether the code at `start` is a block expression whose first `{` is at the current index:
    /// the block itself, a macro's braces, a labeled block or loop, or a block keyword's expression.
    /// Outer attributes before it are passed over.
    fn starts_block(&self, start: usize) -> bool {
        let at = self.code_start(start);
        at == self.at || self.chars.get(at) == Some(&'\'') || BLOCK_KEYWORDS.contains(&self.word_at(at).as_str()) || self.macro_braces(at)
    }

    /// The index of the code at `start`, past whitespace, comments and outer attributes.
    fn code_start(&self, start: usize) -> usize {
        let mut at = self.skip_space(start);
        while self.chars.get(at) == Some(&'#') {
            let open = self.skip_space(at + 1);
            let Some(end) = self.chars.get(open).filter(|&&c| c == '[').and_then(|_| self.group_end(open)) else {
                break;
            };
            at = self.skip_space(end);
        }
        at
    }

    /// The index after the delimiter group that opens at `at`, read as tokens: delimiters inside
    /// strings, character literals and comments do not count. `None` when it does not close.
    fn group_end(&self, mut at: usize) -> Option<usize> {
        let mut depth = 0_usize;
        loop {
            let c = *self.chars.get(at)?;
            match c {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(at + 1);
                    }
                }
                '/' if matches!(self.chars.get(at + 1), Some('/' | '*')) => {
                    at = self.skip_space(at);
                    continue;
                }
                '"' => {
                    at = self.quoted_end(at + 1, false, 0)?;
                    continue;
                }
                '\'' => {
                    at += match (self.chars.get(at + 1), self.chars.get(at + 2)) {
                        (Some('\\'), _) => self.chars.get(at + 3..)?.iter().position(|&q| q == '\'')? + 4,
                        (Some(_), Some('\'')) => 3,
                        _ => 1,
                    };
                    continue;
                }
                c if is_word(c) => {
                    let word = self.word_at(at);
                    at += word.chars().count();
                    let hashes = self.chars.get(at..).unwrap_or_default().iter().take_while(|&&h| h == '#').count();
                    if matches!(word.as_str(), "r" | "br" | "cr") && self.chars.get(at + hashes) == Some(&'"') {
                        at = self.quoted_end(at + hashes + 1, true, hashes)?;
                    }
                    continue;
                }
                _ => {}
            }
            at += 1;
        }
    }

    /// The index after the closing quote of a string whose text starts at `at`: a raw string closed
    /// by a quote and `hashes` hashes, or a string whose backslashes escape the next character.
    fn quoted_end(&self, mut at: usize, raw: bool, hashes: usize) -> Option<usize> {
        loop {
            match *self.chars.get(at)? {
                '\\' if !raw => at += 2,
                '"' if (1..=hashes).all(|n| self.chars.get(at + n) == Some(&'#')) => return Some(at + 1 + hashes),
                _ => at += 1,
            }
        }
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

    /// Whether the code from `at` to `end` holds only an item's qualifiers: `pub` with its optional
    /// `(..)`, `async`, `const`, `default`, `extern` with its optional ABI string, `safe` and
    /// `unsafe`.
    fn item_qualifiers(&self, mut at: usize, end: usize) -> bool {
        loop {
            at = self.skip_space(at);
            if at >= end {
                return at == end;
            }
            let word = self.word_at(at);
            match word.as_str() {
                "pub" => {
                    at += 3;
                    let open = self.skip_space(at);
                    if open < end && self.chars.get(open) == Some(&'(') {
                        let Some(close) = self.group_end(open) else { return false };
                        at = close;
                    }
                }
                "async" | "const" | "default" | "extern" | "safe" | "unsafe" => at += word.chars().count(),
                "" if self.chars.get(at) == Some(&'"') => {
                    let Some(close) = self.quoted_end(at + 1, false, 0) else { return false };
                    at = close;
                }
                _ => return false,
            }
        }
    }

    /// The word at `at`, with the `r#` of a raw identifier (`r#try`), which is never a keyword.
    fn word_at(&self, at: usize) -> String {
        let raw = self.chars.get(at..at + 2) == Some(&['r', '#'][..]) && self.chars.get(at + 2).is_some_and(|&c| is_word(c) && !c.is_ascii_digit());
        let from = if raw { at + 2 } else { at };
        let word: String = self.chars.get(from..).unwrap_or_default().iter().take_while(|&&c| is_word(c)).collect();
        if raw { format!("r#{word}") } else { word }
    }

    fn open(&mut self, c: char) {
        if let Some(TestRegion::Item { level, start, opened: false, item, pattern: false, angle: 0, params: false, .. }) = self.test
            && level == self.delims.len()
            && c == '{'
        {
            let starts = !item && self.starts_block(start);
            // In an item's header, the braces of a macro (`fn f() -> ty! { u8 } { .. }`) are not its
            // body, and neither are braces in the condition of an `if` and the like.
            let header = if item { self.after_bang } else { self.in_condition(start) };
            if !header && let Some(TestRegion::Item { opened, block, .. }) = &mut self.test {
                *opened = true;
                *block = starts;
            }
        }
        let level = self.delims.len();
        // A macro's delimiters in the header (`struct S where ty!{}: Copy { .. }`) are not the
        // fields; the declaration stays pending across them.
        let fields = match self.declares.last().copied() {
            Some(Declares { level: at, keyword, angle: 0, after_where })
                if at == level && !self.after_bang && (c == '{' || (c == '(' && !after_where)) =>
            {
                self.declares.pop();
                match (keyword, c) {
                    ("enum", '{') => Fields::Variants,
                    ("fn", _) | (_, '{') => Fields::Typed,
                    _ => Fields::Tuple,
                }
            }
            // A variant's `{ .. }` or `( .. )` after its name, not in its discriminant, and not after
            // a path, a method or an operator (`m::f::<u8, u16>::call(..)` after a comma in the
            // discriminant's generic arguments).
            _ if self.after_word
                && !matches!(self.prev_prev, Some(':' | '.' | '=' | '+' | '-' | '/' | '%' | '&' | '|' | '^' | '<' | '>'))
                && self.delims.last().is_some_and(|&(_, _, fields)| fields == Fields::Variants) =>
            {
                match c {
                    '{' => Fields::Typed,
                    '(' => Fields::Tuple,
                    _ => Fields::Untyped,
                }
            }
            _ => Fields::Untyped,
        };
        self.delims.push((c, self.line, fields));
        self.at += 1;
    }

    /// Whether the `{` at the current index is in the condition of the test-only `if`, `while`,
    /// `match` or `for` whose code starts at `start`, not its body: the braces of a macro
    /// (`if ready! { } { .. }`), or a block where an operand starts, after the keyword, an `in`, an
    /// operator or a block keyword (`if { ready } { .. }`, `while !{ done } { .. }`,
    /// `if const { true } { .. }`).
    fn in_condition(&self, start: usize) -> bool {
        if !matches!(self.word_at(self.code_start(start)).as_str(), "if" | "while" | "match" | "for") {
            return false;
        }
        if self.after_bang {
            return true;
        }
        if self.after_word {
            let raw = self.last_word.checked_sub(1).and_then(|i| self.chars.get(i)) == Some(&'#');
            let word = self.word_at(self.last_word);
            return !raw && matches!(word.as_str(), "if" | "while" | "match" | "in" | "const" | "unsafe" | "loop" | "async" | "move");
        }
        matches!(self.prev, Some('!' | '&' | '|' | '=' | '<' | '+' | '-' | '*' | '/' | '%' | '^'))
    }

    fn close(&mut self, c: char) -> Result<(), SyntaxError> {
        let expected = match self.delims.pop() {
            Some(('{', _, _)) => '}',
            Some(('(', _, _)) => ')',
            Some(_) => ']',
            None => return Err(Self::error(self.line, "unmatched closing delimiter")),
        };
        if c != expected {
            return Err(Self::error(self.line, "mismatched closing delimiter"));
        }
        self.at += 1;
        let depth = self.delims.len();
        // `declares` is in ascending order of level, so the ones out of scope are at its end.
        while self.declares.last().is_some_and(|declares| declares.level > depth) {
            self.declares.pop();
        }
        let ends = match self.test {
            Some(TestRegion::Item { level, opened, item, semi, block, .. }) => {
                let block_end = opened && depth == level && c == '}';
                let branch = block_end && !item && block && self.word_at(self.skip_space(self.at)) == "else";
                if block_end && !item && block && self.postfix_follows() {
                    // A method call, field or `?` continues the expression to its `;` or `,`.
                    if let Some(TestRegion::Item { block, .. }) = &mut self.test {
                        *block = false;
                    }
                    false
                } else if branch {
                    // The `else` branch's header (`else if let P { .. } = p`) is read like the first.
                    if let Some(TestRegion::Item { opened, .. }) = &mut self.test {
                        *opened = false;
                    }
                    false
                } else {
                    depth < level || (block_end && if item { !semi } else { block })
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
    /// (`impl<A, B>`, a `where` clause) does not, and neither does one in an expression's generic
    /// arguments (`f::<A, B>`) or in the parameters of a closure that starts it.
    fn item_end(&mut self, comma: bool) {
        // A unit struct (`struct S;`) declares no fields.
        if !comma {
            let level = self.delims.len();
            while self.declares.last().is_some_and(|declares| declares.level >= level) {
                self.declares.pop();
            }
        }
        // A `,` among an enum's variants ends a discriminant.
        if let Some((_, _, fields @ Fields::Discriminant)) = self.delims.last_mut()
            && comma
        {
            *fields = Fields::Variants;
        }
        if let Some(TestRegion::Item { level, opened, item, semi, angle, params, .. }) = self.test
            && level == self.delims.len()
            && if comma { level > 0 && !item && angle == 0 && !params } else { !item || !opened || semi }
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
        let explicit = [Input::rust(Path::new("src/linked")), Input::rust(Path::new("src/file_link.rs"))];
        let found = scan(root, &explicit, &POLICY).unwrap();
        assert_eq!(hits(&found), [("src/file_link.rs".to_string(), 1, "AzureKit")]);
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
    fn overlapping_candidates_are_each_checked() {
        for case in [MatchCase::Exact, MatchCase::AsciiInsensitive] {
            let policy = Policy { case, ..Policy::new(&["A-A"]) };
            assert_eq!(policy.words_in("XA-A-A"), ["A-A"]);
            assert!(policy.words_in("XA-A-AX").is_empty());
        }
    }

    #[test]
    fn a_shebang_line_is_not_read_as_code() {
        let source = "#!/opt/tool(v1/bin/rust-script\nconst S: &str = \"AzureKit\";\n";
        let found = scan_source(Path::new("main.rs"), source, Kind::Rust, &POLICY).unwrap();
        assert_eq!(found.iter().map(|f| (f.line, f.word.as_str())).collect::<Vec<_>>(), [(2, "AzureKit")]);
        assert_eq!(literals("#!/usr/bin/env run-cargo-script\n\"live\""), ["live"]);
        assert_eq!(literals("#![cfg(test)]\nfn a() { \"AzureKit\"; }\n"), Vec::<String>::new());
        assert_eq!(literals("#! /* note */ [cfg(test)]\nfn a() { \"AzureKit\"; }\n"), Vec::<String>::new());
        assert_eq!(literals("\"one\" #!x"), ["one"]);
    }

    #[test]
    fn a_doc_comment_after_a_shebang_does_not_make_it_an_attribute() {
        assert_eq!(literals("#! /*! doc */ [allow(unused)] \"AzureKit\"\n\"live\""), ["live"]);
        assert_eq!(literals("#! /** doc */ [allow(unused)] \"AzureKit\"\n\"live\""), ["live"]);
        assert_eq!(literals("#! /**/ /*** note */ [cfg(test)]\nfn a() { \"AzureKit\"; }\n"), Vec::<String>::new());
    }

    #[test]
    fn test_only_generic_arguments_and_closure_parameters_keep_their_commas() {
        let source = r#"fn arms(x: u8) {
    match x {
        #[cfg(test)]
        0 => pair::<u8, u16>("AzureKit"),
        #[cfg(test)]
        1 => pair::<Vec<u8>, u16>("AzureKit"),
        #[cfg(test)]
        2 => |_: u8, _: u8| "AzureKit",
        #[cfg(test)]
        3 => move |P { a, b }: P, _: u8| "AzureKit",
        #[cfg(test)]
        4 => pair:: /* spaced */ <u8, u16>("AzureKit"),
        _ => pair::<u8, u16>("one"),
    }
    call(#[cfg(test)] pair::<u8, u16>("AzureKit"), "two");
    call(#[cfg(test)] |a, b| a | b, "three");
    call(#[cfg(test)] a < b, "four");
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four"]);
    }

    #[test]
    fn test_only_types_and_qualified_paths_keep_their_generic_commas() {
        let source = r#"struct S {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "one".len() }>,
}
fn arms(x: u8, a: u8, b: u8) {
    match x {
        #[cfg(test)]
        0 => <Pair<u8, Tag<{ "AzureKit".len() }>> as Fixture>::LABEL,
        #[cfg(test)]
        1 => a < b,
        _ => "two",
    }
    call(#[cfg(test)] a < b, "three");
    let _ = S { #[cfg(test)] probe: a < b, label: "four" };
    let _ = S { #[cfg(test)] probe: a < b, live: "six".len() > 0 };
    let _ = S { #[cfg(test)] probe: a < b, live: x("seven") > d };
    let _ = S { #[cfg(test)] probe: a < /* > */ b, live: "eight" };
}
fn typed(#[cfg(test)] p: Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "five".len() }>) {}
struct Bounds<'a> {
    #[cfg(test)]
    field: &'a dyn Foo<A: Send, B: Bar<{ "AzureKit".len() }>>,
    live: Tag<{ "nine".len() }>,
}
enum E {
    V {
        #[cfg(test)]
        field: Pair<u8, Tag<{ "AzureKit".len() }>>,
        live: Tag<{ "ten".len() }>,
    },
    W(#[cfg(test)] Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "eleven".len() }>),
}
fn bounded<F: Fn(u8, u16)>(#[cfg(test)] p: Pair<u8, Tag<{ "AzureKit".len() }>>, f: F) -> Tag<{ "twelve".len() }> {}
struct Tuple(#[cfg(test)] pub Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "thirteen".len() }>);
struct Where<F> where F: Fn(u8, u16) {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "fourteen".len() }>,
}
struct Defaulted<F = fn(u8, u16)> {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "fifteen".len() }>,
}
fn returns(x: u8) -> &'static str {
    match x {
        #[cfg(test)]
        0 => return <Pair<u8, Tag<{ "AzureKit".len() }>> as Fixture>::LABEL,
        _ => "sixteen",
    }
}
"#;
        let expected = [
            "one", "two", "three", "four", "six", "seven", "eight", "five", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen",
            "sixteen",
        ];
        assert_eq!(literals(source), expected);
    }

    #[test]
    fn test_only_parameter_patterns_keep_their_generic_commas() {
        let source = r#"fn mutable(#[cfg(test)] mut p: Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "one".len() }>) {}
fn tuple(#[cfg(test)] (a, b): Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "two".len() }>) {}
fn borrowed(#[cfg(test)] &(a, b): &Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "three".len() }>) {}
fn fields(#[cfg(test)] P { x, .. }: Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "four".len() }>) {}
fn by_ref(#[cfg(test)] ref mut p: Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "five".len() }>) {}
fn paths(#[cfg(test)] m::P(x): Pair<u8, Tag<{ "AzureKit".len() }>>, q: Tag<{ "six".len() }>) {}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six"]);
    }

    #[test]
    fn closing_delimiters_keep_pending_declarations_in_linear_time() {
        // 200,000 pending `fn`s and 200,000 `[]` groups: scanning every pending declaration at
        // each `]` makes 4 * 10^10 comparisons, which takes minutes.
        let count = 200_000;
        let source = format!("m! {{ {}{}}} \"done\"", "fn ".repeat(count), "[] ".repeat(count));
        assert_eq!(literals(&source), ["done"]);
    }

    #[test]
    fn nested_function_pointer_types_keep_the_declaration() {
        let source = r#"struct Nested<T = (fn(),)> {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "one".len() }>,
}
struct Boxed<T = Box<[fn(u8, u16); 2]>>(#[cfg(test)] Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "two".len() }>);
enum Listed<T = [fn(); 1]> {
    V {
        #[cfg(test)]
        field: Pair<u8, Tag<{ "AzureKit".len() }>>,
        live: Tag<{ "three".len() }>,
    },
}
struct Bounded<F> where F: Into<(fn(u8),)> {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "four".len() }>,
}
struct Outer<const N: usize = { struct Inner { #[cfg(test)] field: Pair<u8, Tag<{ "AzureKit".len() }>>, live: Tag<{ "five".len() }> } 0 }> {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "six".len() }>,
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six"]);
    }

    #[test]
    fn macro_braces_in_a_where_clause_are_not_the_body() {
        let source = r#"struct S where ty!{}: Copy {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "one".len() }>,
}
enum E where m::ty! { u8 }: Copy, F: Fn(u8, u16) {
    V {
        #[cfg(test)]
        field: Pair<u8, Tag<{ "AzureKit".len() }>>,
        live: Tag<{ "two".len() }>,
    },
}
union U where ty! {}: Copy {
    #[cfg(test)]
    field: Pair<u8, Tag<{ "AzureKit".len() }>>,
    live: Tag<{ "three".len() }>,
}
fn f() where ty!{}: Copy { let _ = S { #[cfg(test)] probe: a < b, live: "four" }; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four"]);
    }

    #[test]
    fn enum_discriminants_are_expressions() {
        let source = r#"enum E {
    V = f(#[cfg(test)] if 1 < 2 { true } else { false }, live("one") > 0),
    W = m::f(#[cfg(test)] a < b, live("two") > 0),
    X = g(1)(#[cfg(test)] a < b, live("three") > 0),
    T = m::f::<u8, u16>::call(#[cfg(test)] a < b, live("ten") > 0),
    S = <A as B<u8, u16>>::call(#[cfg(test)] a < b, live("eleven") > 0),
    #[doc = "four"]
    Y(#[cfg(test)] Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "five".len() }>),
    Z { #[cfg(test)] field: Pair<u8, Tag<{ "AzureKit".len() }>>, live: Tag<{ "six".len() }> },
}
macro_rules! variants {
    ($a:ident, $b:ident, $c:ident) => {
        enum M {
            $a = f(#[cfg(test)] a < b, live("seven") > 0),
            $b(#[cfg(test)] Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "eight".len() }>),
            $c { #[cfg(test)] field: Pair<u8, Tag<{ "AzureKit".len() }>>, live: Tag<{ "nine".len() }> },
            $(#[$meta])* V(#[cfg(test)] Pair<u8, Tag<{ "AzureKit".len() }>>, Tag<{ "twelve".len() }>),
        }
    };
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "ten", "eleven", "four", "five", "six", "seven", "eight", "nine", "twelve"]);
    }

    #[test]
    fn test_only_cast_types_keep_their_generic_commas() {
        let source = r#"fn arms(x: u8, a: u8, b: u8, ptr: *const u8) {
    match x {
        #[cfg(test)]
        0 => ptr as *const Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        1 => ptr as Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        2 => ptr as &'static dyn Foo<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        3 => ptr as ::std::ptr::NonNull<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        4 => ptr as *mut m::Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        6 => ptr as *const <T as Trait>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        7 => ptr as <T as Trait<u8>>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        8 => ptr as m::Pair<u8, fn() -> u8>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        9 => ptr as Vec::<u8>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        10 => ptr as Pair<{ 1 > 0 }>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        11 => ptr as m::r#Pair<u8>::Assoc<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        12 => ptr as /* type */ Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        13 => f as fn() -> Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        14 => f as unsafe fn(u8, &[u8]) -> &'static Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        15 => f as fn() -> (u8, Pair<u8, u16>),
        #[cfg(test)]
        16 => f as fn() -> m::Pair<u8, Tag<{ "AzureKit".len() }>>,
        #[cfg(test)]
        5 => a < b,
        _ => "one",
    }
    call(#[cfg(test)] (a as u8) < b, "two");
    call(#[cfg(test)] a < b, "three");
    call(#[cfg(test)] a as u8 + a < b, "four");
    call(#[cfg(test)] a as u8 > b && a < b, "five");
    call(#[cfg(test)] a as u8 == b || a < b, "six");
    call(#[cfg(test)] a as u8 - b < b, "seven");
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six", "seven"]);
    }

    #[test]
    fn test_only_borrowed_closures_keep_their_parameter_commas() {
        let source = r#"fn arms(x: u8) {
    match x {
        #[cfg(test)]
        0 => &|_, _| "AzureKit",
        #[cfg(test)]
        1 => &mut move |_, _| "AzureKit",
        2 => &|_, _| "one",
        _ => &|_, _| "two",
    }
}
"#;
        assert_eq!(literals(source), ["one", "two"]);
    }

    #[test]
    fn a_raw_identifier_is_not_read_as_a_keyword() {
        let source = r##"fn f(r#let: bool) {
    #[cfg(test)] if r#let {}
    let _ = "one";
    #[cfg(test)] if r#let { "AzureKit"; }
    let _ = r#"two"#;
}
"##;
        assert_eq!(literals(source), ["one", "two"]);
    }

    #[test]
    fn braces_in_a_test_only_condition_are_not_its_body() {
        let source = r#"fn statements(ready: bool) {
    #[cfg(test)] if ready! { } { let _ = "AzureKit"; }
    let _ = "one";
    #[cfg(test)] if { ready } { "AzureKit"; }
    let _ = "two";
    #[cfg(test)] while !{ ready } { "AzureKit"; }
    let _ = "three";
    #[cfg(test)] if ready { "AzureKit"; } else if { ready } { "AzureKit"; } else { "AzureKit"; }
    let _ = "four";
    #[cfg(test)] match { 1 } { _ => "AzureKit" }
    let _ = "five";
    #[cfg(test)] if ready() { "AzureKit"; }
    let _ = "six";
    #[cfg(test)] for x in { [1] } { "AzureKit"; }
    let _ = "seven";
    #[cfg(test)] if const { true } { "AzureKit"; }
    let _ = "eight";
    #[cfg(test)] while unsafe { ready } { "AzureKit"; }
    let _ = "nine";
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]);
    }

    #[test]
    fn a_raw_identifier_macro_is_read_as_one_word() {
        let source = r#"fn statements() {
    #[cfg(test)] r#try! {}
    let _ = "one";
    #[cfg(test)] r#try! { "AzureKit" }
    let _ = "two";
}
"#;
        assert_eq!(literals(source), ["one", "two"]);
    }

    #[test]
    fn test_only_item_headers_do_not_end_items_early() {
        let source = r#"#[cfg(test)] fn generic<const N: usize>() {}
fn one() { let _ = "one"; }
#[cfg(test)] impl<const N: usize> Probe<N> {}
fn two() { let _ = "two"; }
#[cfg(test)] fn fixture() -> Tag<{ 1 }> { let _ = "AzureKit"; Tag }
fn three() { let _ = "three"; }
#[cfg(test)] fn nested() -> Tag<Pair<{ 1 }, { 2 }>> where Tag<{ 3 }>: Clone { let _ = "AzureKit"; Tag }
#[cfg(test)] const fn k() -> u8 { 1 }
fn four() { let _ = "four"; }
#[cfg(test)] const unsafe fn u() -> u8 { 1 }
fn five() { let _ = "five"; }
#[cfg(test)] const C: fn() -> u8 = k;
fn six() { let _ = "six"; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six"]);
    }

    #[test]
    fn item_keywords_inside_test_only_expressions_do_not_start_items() {
        let source = r#"fn arms(x: u8) {
    match x {
        #[cfg(test)]
        0 => helper::<fn()>("AzureKit"),
        _ => helper::<u8>("one"),
    }
    call(#[cfg(test)] cast::<impl Fn()>("AzureKit"), "two");
}
struct S {
    #[cfg(test)]
    hook: fn() -> u8,
    name: Tag<"three">,
}
#[cfg(test)] pub(crate) unsafe extern "C" fn ffi() { let _ = "AzureKit"; }
fn four() { let _ = "four"; }
#[cfg(test)] fn typed() -> ty! { u8 } { let _ = "AzureKit"; 0 }
fn five() { let _ = "five"; }
#[cfg(test)] fn noted() -> ty! /* a { */ { u8 } { let _ = "AzureKit"; 0 }
fn six() { let _ = "six"; }
#[cfg(test)] fn halt() -> ! { let _ = "AzureKit"; loop {} }
fn seven() { let _ = "seven"; }
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six", "seven"]);
    }

    #[test]
    fn test_only_let_conditions_end_at_their_body() {
        let source = r#"fn statements(p: P, o: Option<u8>) {
    #[cfg(test)] if let P { x: 0 } = p { let _ = "AzureKit"; }
    let _ = "one";
    #[cfg(test)] if let Some(x) = o { "AzureKit"; }
    let _ = "two";
    #[cfg(test)] while let P { x: 1 } = p { "AzureKit"; }
    let _ = "three";
    #[cfg(test)] for P { x } in [p] { "AzureKit"; }
    let _ = "four";
    #[cfg(test)] if true && let P { x: 0..=2 } = p { "AzureKit"; } else { "AzureKit"; }
    let _ = "five";
    #[cfg(test)] if o == Some(1) { "AzureKit"; }
    let _ = "six";
    #[cfg(test)] if o.is_none() { "AzureKit"; } else if let P { x: 0 } = p { "AzureKit"; } else { "AzureKit"; }
    let _ = "seven";
}
"#;
        assert_eq!(literals(source), ["one", "two", "three", "four", "five", "six", "seven"]);
    }

    #[test]
    fn attribute_strings_and_comments_are_read_as_tokens() {
        let source = r#"fn statements() {
    #[cfg(test)] #[allow(unused, reason = "]")] { "AzureKit"; }
    let _ = "one";
    #[cfg(test)] #[doc = r"]"] #[allow(unused, /* ] */)] { "AzureKit"; }
    let _ = "two";
}
"#;
        assert_eq!(literals(source), ["one", "two"]);
        let policy = Policy { allow_marker: Some("name-ok"), ..POLICY };
        let rust = |src: &str| scan_source(Path::new("lib.rs"), src, Kind::Rust, &policy).unwrap().len();
        assert_eq!(rust("const LIVE: &str = \"AzureKit\"; #[cfg(/* name-ok */ test)] const TEST: &str = \"\";"), 0);
        assert_eq!(rust("const LIVE: &str = \"AzureKit\"; #[cfg( // name-ok\n test)] const TEST: &str = \"\";"), 0);
        assert_eq!(rust("const LIVE: &str = \"AzureKit\"; #[cfg(test)] const TEST: &str = \"\"; // other"), 1);
    }

    #[test]
    fn escaped_bytes_are_decoded_as_utf8() {
        let both = Literals { byte_strings: true, c_strings: true };
        let texts = |source: &str| super::literals(source, both).unwrap().into_iter().map(|lit| lit.text).collect::<Vec<_>>();
        let source = r#"c"Caf\xC3\xA9" b"Caf\xc3\xa9" c"Caf\u{e9}" b"Caf\xC3" b"\xFFok""#;
        assert_eq!(texts(source), ["Café", "Café", "Café", "Caf\u{FFFD}", "\u{FFFD}ok"]);
    }

    #[test]
    fn aliases_of_one_file_are_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "\"AzureKit\"");
        let absolute = root.join("src/a.rs");
        let fits = Policy { limits: Limits { max_total_bytes: 10, ..DEFAULT_LIMITS }, ..POLICY };
        let same = [Input::rust(Path::new("src/a.rs")), Input::rust(&absolute), Input::rust(Path::new("./src/a.rs"))];
        let found = scan(root, &same, &fits).unwrap();
        assert_eq!(found.len(), 1, "{found:#?}");
        let mixed = [Input::text(Path::new("src/a.rs")), Input::rust(&absolute)];
        let err = scan(root, &mixed, &POLICY).unwrap_err();
        assert!(matches!(&err, Error::Conflict { path } if path.ends_with("a.rs")), "{err}");
    }

    #[test]
    fn aliases_under_a_relative_base_are_read_once() {
        let absolute = std::env::current_dir().unwrap().join("Cargo.toml");
        let size = std::fs::metadata(&absolute).unwrap().len();
        let fits = Policy { limits: Limits { max_total_bytes: size, ..DEFAULT_LIMITS }, ..POLICY };
        let same = [Input::manifest(Path::new("Cargo.toml")), Input::manifest(&absolute)];
        scan(Path::new("."), &same, &fits).unwrap();
    }

    #[test]
    fn relative_base_stays_fixed_when_enumeration_changes_the_working_directory() {
        const CHILD_ROOT: &str = "CRAFT_NAMES_SCAN_DIRECTORY_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            fn change_directory(_: &Path) -> bool {
                let root = PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
                std::env::set_current_dir(root.join("switched")).unwrap();
                false
            }
            let policy = Policy { skip: change_directory, ..POLICY };
            let found = scan(Path::new("."), &[Input::rust(Path::new("src"))], &policy).unwrap();
            assert_eq!(hits(&found), [("src/a.rs".to_owned(), 1, "AzureKit")]);
            // Canonical on both sides: macOS temp paths run through the /var -> /private/var link.
            let canonical = |p: PathBuf| p.canonicalize().unwrap();
            assert_eq!(canonical(std::env::current_dir().unwrap()), canonical(PathBuf::from(root).join("switched")));
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "original/src/a.rs", "\"AzureKit\"");
        write(dir.path(), "switched/src/a.rs", "\"BlueCore\"");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "names::tests::relative_base_stays_fixed_when_enumeration_changes_the_working_directory", "--nocapture"])
            .env(CHILD_ROOT, dir.path())
            .current_dir(dir.path().join("original"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[cfg(windows)]
    #[test]
    fn aliases_under_a_drive_relative_base_are_read_once() {
        let current = std::env::current_dir().unwrap();
        let Some(Component::Prefix(drive)) = current.components().next() else { panic!("no drive in {}", current.display()) };
        let absolute = current.join("Cargo.toml");
        let size = std::fs::metadata(&absolute).unwrap().len();
        let fits = Policy { limits: Limits { max_total_bytes: size, ..DEFAULT_LIMITS }, ..POLICY };
        let same = [Input::manifest(Path::new("Cargo.toml")), Input::manifest(&absolute)];
        scan(Path::new(drive.as_os_str()), &same, &fits).unwrap();
        // A drive-relative input names the file under the current folder of its drive.
        let mut spelled = drive.as_os_str().to_os_string();
        spelled.push("Cargo.toml");
        let relative = PathBuf::from(spelled);
        for base in [Path::new(drive.as_os_str()), current.as_path()] {
            let same = [Input::manifest(&relative), Input::manifest(&absolute)];
            scan(base, &same, &fits).unwrap();
            let mixed = [Input::text(&relative), Input::manifest(&absolute)];
            let err = scan(base, &mixed, &POLICY).unwrap_err();
            assert!(matches!(&err, Error::Conflict { path } if path.ends_with("Cargo.toml")), "{err}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_relative_rust_folder_is_resolved_before_it_is_listed() {
        let current = std::env::current_dir().unwrap();
        let Some(Component::Prefix(drive)) = current.components().next() else { panic!("no drive in {}", current.display()) };
        let mut spelled = drive.as_os_str().to_os_string();
        spelled.push("src");
        let input = PathBuf::from(spelled);
        let mut files = Vec::new();
        collect_rust(&current, &input, &POLICY, &mut files).unwrap();
        assert!(!files.is_empty());
        for (opened, kind, path) in &files {
            assert_eq!(*kind, Kind::Rust);
            // The file is opened by the path that was listed, and reported by the input's spelling.
            let rest = path.strip_prefix(&input).unwrap();
            assert_eq!(opened, &current.join("src").join(rest), "{}", path.display());
        }
    }

    #[test]
    fn the_total_budget_is_checked_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.rs", "\"one\"");
        write(root, "b.rs", "\"two\"");
        let inputs = [Input::rust(Path::new("a.rs")), Input::rust(Path::new("b.rs"))];
        let exact = Policy { limits: Limits { max_total_bytes: 10, ..DEFAULT_LIMITS }, ..POLICY };
        assert!(scan(root, &inputs, &exact).unwrap().is_empty());
        let short = Policy { limits: Limits { max_total_bytes: 9, ..DEFAULT_LIMITS }, ..POLICY };
        let err = scan(root, &inputs, &short).unwrap_err();
        assert!(matches!(&err, Error::TotalTooLarge { path, limit: 9 } if path.ends_with("b.rs")), "{err}");
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
