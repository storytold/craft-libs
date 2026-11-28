//! Scanner for `.ai` editing-data streams read from an [`std::io::Read`]: the tokens
//! [`crate::ai_lex::Lexer`] finds in a byte slice, owned, with the data read only as far as each
//! token needs.
//!
//! - The lexer holds the token it is reading, the bytes it looks ahead at to end that token, and
//!   [`LOOKBEHIND`] bytes before it (an image's `XI` operand line), never the whole stream.
//! - An image's samples after `%%BeginData:` are read by the byte count the comment gives (DSC 3.0,
//!   section 5: <https://web.mit.edu/PostScript/Adobe/Documents/5001.DSC_Spec.pdf>), then up to
//!   the `%%EndData` that ends them, by the same rules as [`crate::ai_lex::Lexer`].
//! - A token that needs more than the limit ([`MAX_TOKEN`], or the one given to
//!   [`Lexer::with_limit`]) is an [`Error::TooLarge`]. The limit is checked before each read, so a
//!   count or a section that claims more bytes than the limit allows is refused without holding
//!   them.
//! - [`Lexer::raw`] reads the bytes after the last token as they come, for data that is not tokens:
//!   an `ASCII85Decode` section read through [`crate::ascii85::Decoder`], after which tokens go on.

use std::fmt;
use std::io::{self, BufRead, Read};

use crate::ai_lex::{
    self, DOCUMENT, NOTES, OPERAND_LINE, PLACE, PREVIEW, RELATIVE, alpha_bytes, find, hex_decode, is_delim, is_eol, is_space, number, previous_line,
};

/// The most bytes one token may take by default: 1 GiB, or 256 MiB where `usize` is 32 bits.
pub const MAX_TOKEN: usize = if usize::BITS >= 64 { 1 << 30 } else { 1 << 28 };

/// Bytes held before the token being read: the line of an image's `XI` operands that the
/// samples' alpha channel is looked up in, and the `%_` before its `%%BeginData:` line.
pub const LOOKBEHIND: usize = OPERAND_LINE + 8;

/// Bytes asked of the reader at a time.
const CHUNK: usize = 64 * 1024;

/// A token from a `.ai` editing-data stream: [`crate::ai_lex::Tok`] with its bytes owned.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    /// A literal `(…)` string, escapes undone.
    Str(Vec<u8>),
    /// A hexadecimal `<…>` string.
    Hex(Vec<u8>),
    /// A literal `/name`.
    Name(Vec<u8>),
    /// An operator (any other word, `[` and `]` included).
    Word(Vec<u8>),
    /// A comment line, without its first `%`.
    Comment(Vec<u8>),
    /// An image's samples: what follows `XI` in a `%%BeginData:` section.
    Data(Vec<u8>),
    /// A placed file's preview: what is between `%AI26_BeginPlacedObjectPreview` and its end.
    Preview(Vec<u8>),
    /// A document included whole, or `None` when no `%%EndDocument` line ends it.
    Document(Option<Vec<u8>>),
}

/// A scanned token together with whether it came from a hidden `%_` line or suffix.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    /// Read from a `%_` line.
    pub hidden: bool,
}

impl From<ai_lex::Token<'_>> for Token {
    fn from(t: ai_lex::Token<'_>) -> Self {
        let tok = match t.tok {
            ai_lex::Tok::Num(v) => Tok::Num(v),
            ai_lex::Tok::Str(s) => Tok::Str(s),
            ai_lex::Tok::Hex(s) => Tok::Hex(s),
            ai_lex::Tok::Name(s) => Tok::Name(s.to_vec()),
            ai_lex::Tok::Word(s) => Tok::Word(s.to_vec()),
            ai_lex::Tok::Comment(s) => Tok::Comment(s.to_vec()),
            ai_lex::Tok::Data(s) => Tok::Data(s.to_vec()),
            ai_lex::Tok::Preview(s) => Tok::Preview(s.to_vec()),
            ai_lex::Tok::Document(s) => Tok::Document(s.map(<[u8]>::to_vec)),
        };
        Self { tok, hidden: t.hidden }
    }
}

/// Why the lexer stopped.
#[derive(Debug)]
pub enum Error {
    /// A token needs more bytes than the limit.
    TooLarge { limit: usize },
    /// The reader failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit } => write!(f, "a token of the editing data is larger than {limit} bytes"),
            Self::Io(e) => write!(f, "the editing data could not be read: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TooLarge { .. } => None,
            Self::Io(e) => Some(e),
        }
    }
}

/// What a `%` gave: a token, or nothing to return yet.
enum Step {
    Token(Token),
    Again,
}

/// Scanner for `.ai` editing data read from `R`. Positions count bytes from the start of the
/// stream.
pub struct Lexer<R> {
    reader: R,
    /// The bytes held, from `base` on.
    buf: Vec<u8>,
    base: usize,
    pos: usize,
    /// Where the token being read starts: the limit counts from here, and bytes more than
    /// [`LOOKBEHIND`] before it may be dropped.
    mark: usize,
    /// The current line started with `%_`.
    hidden: bool,
    /// The reader has no more bytes.
    eof: bool,
    /// A token was refused or the reader failed: no more tokens.
    failed: bool,
    max_token: usize,
    chunk: usize,
}

impl<R: Read> Lexer<R> {
    /// A lexer whose tokens take at most [`MAX_TOKEN`] bytes each.
    pub fn new(reader: R) -> Self {
        Self::with_limit(reader, MAX_TOKEN)
    }

    /// A lexer whose tokens take at most `max_token` bytes each, counted from where a token starts
    /// to the last byte looked at to end it.
    pub fn with_limit(reader: R, max_token: usize) -> Self {
        Self { reader, buf: Vec::new(), base: 0, pos: 0, mark: 0, hidden: false, eof: false, failed: false, max_token, chunk: CHUNK }
    }

    /// Where the next token is looked for, in bytes from the start of the stream.
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// The next token; `None` at the end, and after an error.
    pub fn next_token(&mut self) -> Result<Option<Token>, Error> {
        if self.failed {
            return Ok(None);
        }
        let result = self.scan();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// Skip past the next `needle` (the end of data that isn't tokens) → what was skipped, at most
    /// the limit's bytes.
    pub fn skip_past(&mut self, needle: &[u8]) -> Result<Vec<u8>, Error> {
        self.mark = self.pos;
        let from = self.pos;
        let to = match self.find_from(from, needle)? {
            Some(i) => i.saturating_add(needle.len()),
            None => self.end(),
        };
        self.pos = to;
        Ok(self.held(from, to).to_vec())
    }

    /// The bytes after the last token, read as they come; tokens go on after the bytes read
    /// through it.
    pub fn raw(&mut self) -> Raw<'_, R> {
        Raw { lexer: self }
    }

    fn end(&self) -> usize {
        self.base + self.buf.len()
    }

    /// The end of the bytes the token being read may look at: those held, up to the limit. Bytes
    /// [`Raw`] read ahead past the limit are not looked at.
    fn visible_end(&self) -> usize {
        self.end().min(self.mark.saturating_add(self.max_token))
    }

    /// The stream has ended and every byte of it up to `at` is held.
    fn ended_at(&self, at: usize) -> bool {
        self.eof && at >= self.end()
    }

    /// The bytes held from `from` to `to` (empty where none are).
    fn held(&self, from: usize, to: usize) -> &[u8] {
        let to = to.min(self.end());
        match (from.checked_sub(self.base), to.checked_sub(self.base)) {
            (Some(f), Some(t)) => self.buf.get(f..t).unwrap_or_default(),
            _ => &[],
        }
    }

    /// The byte at `at`, when it is held.
    fn at(&self, at: usize) -> Option<u8> {
        at.checked_sub(self.base).and_then(|i| self.buf.get(i)).copied()
    }

    /// The byte at `at`, read when it isn't held yet; `None` past the end of the stream.
    fn byte(&mut self, at: usize) -> Result<Option<u8>, Error> {
        self.fill_to(at.saturating_add(1))?;
        Ok(self.at(at))
    }

    /// Read until the bytes up to `to` are held, or the stream has ended. An error when the bytes
    /// up to `to` that the stream has go past the token's limit, whether they are held already or
    /// not.
    fn fill_to(&mut self, to: usize) -> Result<(), Error> {
        let limit = self.mark.saturating_add(self.max_token);
        loop {
            if self.end() >= to || self.eof {
                if to.min(self.end()) > limit {
                    return Err(Error::TooLarge { limit: self.max_token });
                }
                return Ok(());
            }
            let room = limit.saturating_sub(self.end());
            if room == 0 {
                return Err(Error::TooLarge { limit: self.max_token });
            }
            self.read_more(room).map_err(Error::Io)?;
        }
    }

    /// One read of at most `room` bytes, after dropping the bytes no token looks at again.
    fn read_more(&mut self, room: usize) -> io::Result<()> {
        let drop = self.mark.min(self.pos).saturating_sub(LOOKBEHIND).saturating_sub(self.base);
        if drop >= self.chunk {
            self.buf.drain(..drop.min(self.buf.len()));
            self.base += drop;
        }
        let old = self.buf.len();
        self.buf.resize(old + self.chunk.min(room), 0);
        loop {
            let Some(dst) = self.buf.get_mut(old..) else { return Ok(()) };
            match self.reader.read(dst) {
                Ok(n) => {
                    self.buf.truncate(old + n);
                    self.eof = n == 0;
                    return Ok(());
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.buf.truncate(old);
                    return Err(e);
                }
            }
        }
    }

    /// The first position from `from` whose byte `stop` accepts, or the end of the stream.
    fn scan_until(&mut self, from: usize, stop: impl Fn(u8) -> bool) -> Result<usize, Error> {
        let mut at = from;
        loop {
            let seen = self.visible_end();
            if let Some(i) = self.held(at, seen).iter().position(|b| stop(*b)) {
                return Ok(at + i);
            }
            at = at.max(seen);
            if self.ended_at(at) {
                return Ok(at);
            }
            self.fill_to(at.saturating_add(1))?;
        }
    }

    /// Where the next `needle` from `from` starts, or `None` when the stream ends first.
    fn find_from(&mut self, from: usize, needle: &[u8]) -> Result<Option<usize>, Error> {
        let mut at = from;
        loop {
            let seen = self.visible_end();
            if let Some(i) = find(self.held(at, seen), needle) {
                return Ok(Some(at + i));
            }
            if self.ended_at(seen) {
                return Ok(None);
            }
            at = at.max(seen.saturating_sub(needle.len().saturating_sub(1)));
            self.fill_to(seen.saturating_add(1))?;
        }
    }

    fn at_line_start(&self) -> bool {
        self.pos == 0 || self.at(self.pos - 1).is_some_and(is_eol)
    }

    /// Move to the end of the current line (before its line end).
    fn line(&mut self) -> Result<(), Error> {
        self.pos = self.scan_until(self.pos, is_eol)?;
        Ok(())
    }

    /// Move past the current line and its end (`\r`, `\n` or `\r\n`) → where the line's text
    /// started and ended.
    fn whole_line(&mut self) -> Result<(usize, usize), Error> {
        let start = self.pos;
        self.line()?;
        let end = self.pos;
        if self.byte(self.pos)? == Some(b'\r') {
            self.pos += 1;
        }
        if self.byte(self.pos)? == Some(b'\n') {
            self.pos += 1;
        }
        Ok((start, end))
    }

    fn scan(&mut self) -> Result<Option<Token>, Error> {
        loop {
            self.mark = self.pos;
            let Some(b) = self.byte(self.pos)? else { return Ok(None) };
            if is_eol(b) {
                self.pos += 1;
                self.hidden = false;
                continue;
            }
            if is_space(b) {
                self.pos += 1;
                continue;
            }
            if b == b'%' {
                match self.percent()? {
                    Step::Token(t) => return Ok(Some(t)),
                    Step::Again => continue,
                }
            }
            return self.regular(b).map(Some);
        }
    }

    /// At a `%`: a pseudo-comment, a section, a comment line, or a comment later in a line.
    fn percent(&mut self) -> Result<Step, Error> {
        let start = self.at_line_start();
        self.fill_to(self.pos.saturating_add(3))?;
        let rest = self.held(self.pos, self.pos + 3);
        // A pseudo-comment: `%_` at the line's start, or after operands.
        if rest.starts_with(b"%_") && (start || rest.get(2).is_some_and(u8::is_ascii_alphabetic)) {
            self.pos += 2;
            self.hidden = true;
            return Ok(Step::Again);
        }
        // Right after `%_` a section still starts (a hidden image's data).
        let prefixed = self.hidden && self.pos >= 2 && self.held(self.pos - 2, self.pos) == b"%_";
        let line_start = self.pos;
        self.line()?;
        if (start || prefixed)
            && let Some(t) = self.special(line_start)?
        {
            return Ok(Step::Token(Token { hidden: prefixed, ..t }));
        }
        let line = self.held(line_start, self.pos);
        let text = line.get(1..).unwrap_or_default();
        if start {
            return Ok(Step::Token(Token { tok: Tok::Comment(text.to_vec()), hidden: false }));
        }
        // A hidden placed file's markers, a hidden path's note, and a line of hex digits (a hidden
        // blend's record).
        if prefixed && PLACE.iter().chain(&NOTES).chain(&RELATIVE).any(|m| line.starts_with(m)) {
            return Ok(Step::Token(Token { tok: Tok::Comment(text.to_vec()), hidden: true }));
        }
        let digits = text.trim_ascii_end();
        if prefixed && !digits.is_empty() && digits.iter().all(u8::is_ascii_hexdigit) {
            return Ok(Step::Token(Token { tok: Tok::Comment(text.to_vec()), hidden: true }));
        }
        // A comment later in a line.
        Ok(Step::Again)
    }

    /// A token that isn't a comment, starting with `b` at the position.
    fn regular(&mut self, b: u8) -> Result<Token, Error> {
        let hidden = self.hidden;
        self.pos += 1;
        let tok = match b {
            b'(' => Tok::Str(self.string()?),
            b'<' => {
                let from = self.pos;
                let end = self.scan_until(from, |b| b == b'>')?;
                self.pos = if self.at(end).is_some() { end + 1 } else { end };
                // Malformed digits read as an empty string.
                Tok::Hex(hex_decode(self.held(from, end)).unwrap_or_default())
            }
            b'[' | b']' | b'{' | b'}' | b')' | b'>' => Tok::Word(vec![b]),
            b'/' => Tok::Name(self.word()?),
            _ => {
                self.pos -= 1;
                let w = self.word()?;
                match number(&w) {
                    Some(v) => Tok::Num(v),
                    None => Tok::Word(w),
                }
            }
        };
        Ok(Token { tok, hidden })
    }

    /// A regular token's characters.
    fn word(&mut self) -> Result<Vec<u8>, Error> {
        let start = self.pos;
        self.pos = self.scan_until(start, |b| is_space(b) || is_delim(b))?;
        Ok(self.held(start, self.pos).to_vec())
    }

    /// A literal string after its `(` (an unterminated one ends with the stream).
    fn string(&mut self) -> Result<Vec<u8>, Error> {
        let mut out = vec![];
        let mut depth = 0usize;
        while let Some(b) = self.byte(self.pos)? {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.byte(self.pos)? else { break };
                    self.pos += 1;
                    self.escape(e, &mut out)?;
                }
                _ => out.push(b),
            }
        }
        Ok(out)
    }

    /// The character an escape `\e` stands for, pushed to `out`.
    fn escape(&mut self, e: u8, out: &mut Vec<u8>) -> Result<(), Error> {
        match e {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'\r' => {
                if self.byte(self.pos)? == Some(b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                for _ in 0..2 {
                    match self.byte(self.pos)? {
                        Some(d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                out.push((v & 0xff) as u8);
            }
            other => out.push(other),
        }
        Ok(())
    }

    /// The token the comment line from `line_start` to the position stands for when it starts a
    /// section read differently: image data, a placed file's preview and an included document.
    fn special(&mut self, line_start: usize) -> Result<Option<Token>, Error> {
        let line = self.held(line_start, self.pos);
        if line.starts_with(PREVIEW.0) {
            return self.preview().map(Some);
        }
        if line.starts_with(DOCUMENT.0) {
            let tok = Tok::Document(self.document()?);
            return Ok(Some(Token { tok, hidden: false }));
        }
        let count = line
            .strip_prefix(b"%%BeginData:")
            .and_then(|n| std::str::from_utf8(n).ok())
            .and_then(|n| n.split_whitespace().next())
            .and_then(|n| n.parse::<usize>().ok());
        match count {
            Some(n) => self.data(line_start, n),
            None => Ok(None),
        }
    }

    /// After `%AI26_BeginPlacedObjectPreview`: what is up to its end marker, the position moved
    /// past the marker's line.
    fn preview(&mut self) -> Result<Token, Error> {
        let from = self.pos;
        let after = match self.find_from(from, PREVIEW.1)? {
            Some(i) => i,
            None => self.end(),
        };
        self.pos = after;
        self.line()?;
        Ok(Token { tok: Tok::Preview(self.held(from, after).to_vec()), hidden: false })
    }

    /// After a `%%BeginDocument:` line: the document up to the start of the `%%EndDocument` line
    /// that ends it, the position moved past that line, by the rules of
    /// [`crate::ai_lex::Lexer`]. `None` when no line ends it: the position moves to the end.
    fn document(&mut self) -> Result<Option<Vec<u8>>, Error> {
        self.whole_line()?;
        self.hidden = false;
        let from = self.pos;
        let mut depth = 0usize;
        while self.byte(self.pos)?.is_some() {
            let (start, end) = self.whole_line()?;
            // A hidden object's document is on `%_` lines.
            let line = self.held(start, end);
            let line = line.strip_prefix(b"%_").unwrap_or(line);
            if line.starts_with(DOCUMENT.1) {
                if depth == 0 {
                    return Ok(Some(self.held(from, start).to_vec()));
                }
                depth -= 1;
            } else if line.starts_with(DOCUMENT.0) {
                depth = depth.saturating_add(1);
            } else if let Some(n) = line.strip_prefix(b"%%BeginData:").or_else(|| line.strip_prefix(b"%%BeginBinary:")) {
                let mut words = std::str::from_utf8(n).unwrap_or_default().split_whitespace();
                let count = words.next().and_then(|c| c.parse::<usize>().ok());
                let lines = words.nth(1).is_some_and(|unit| unit.eq_ignore_ascii_case("lines"));
                if let Some(count) = count {
                    self.skip_counted(count, lines)?;
                }
            }
        }
        Ok(None)
    }

    /// Skip what a `%%BeginData:` or `%%BeginBinary:` comment counts: `count` lines, or bytes.
    fn skip_counted(&mut self, count: usize, lines: bool) -> Result<(), Error> {
        if lines {
            for _ in 0..count {
                if self.byte(self.pos)?.is_none() {
                    break;
                }
                self.whole_line()?;
            }
        } else {
            let to = self.pos.saturating_add(count);
            self.fill_to(to)?;
            self.pos = to.min(self.end());
        }
        Ok(())
    }

    /// After a `%%BeginData: n` line (from `line_start`): an image's samples, read by the count,
    /// then up to the `%%EndData` that ends them, or `None` when no `XI` follows.
    fn data(&mut self, line_start: usize, n: usize) -> Result<Option<Token>, Error> {
        // The count starts after the comment's line end character (a following `\n` counts).
        let from = self.pos.saturating_add(1);
        self.fill_to(from)?;
        if from > self.end() {
            return Ok(None);
        }
        let mut skip = self.scan_until(from, |b| !is_space(b))? - from;
        self.fill_to(from + skip + 4)?;
        let region = self.held(from + skip, from + skip + 4);
        // A hidden object's section is on `%_` lines up to its samples.
        let hidden = region.starts_with(b"%_XI");
        if hidden {
            skip += 2;
        } else if !region.starts_with(b"XI") {
            return Ok(None);
        }
        let mut data = from + skip + 2;
        self.fill_to(data + 2)?;
        match (self.at(data), self.at(data + 1)) {
            (Some(b'\r'), Some(b'\n')) => data += 2,
            (Some(b'\r' | b'\n'), _) => data += 1,
            _ => {}
        }
        // The samples the count gives are read as they come: a count past the limit is refused
        // when the bytes held reach the limit, and a count past the end of the stream stops there.
        self.fill_to(from.saturating_add(n))?;
        let declared = from.saturating_add(n).min(self.end()).max(data);
        // The alpha channel the line before gives the image follows the counted samples.
        let line_start = if line_start >= 2 && self.held(line_start - 2, line_start) == b"%_" { line_start - 2 } else { line_start };
        let alpha = line_start.checked_sub(self.base).and_then(|at| previous_line(&self.buf, at)).map_or(0, alpha_bytes);
        let start = if alpha > 0 { declared.saturating_add(alpha).saturating_sub(2) } else { declared.saturating_sub(4) }.max(data);
        let stop = declared.saturating_add(n.saturating_mul(2)).saturating_add(64).max(start.saturating_add(64));
        let end = self.end_of_samples(start, stop)?.unwrap_or(declared);
        self.pos = end;
        let samples = self.held(data, end);
        let samples = if hidden { samples.strip_suffix(b"%_").unwrap_or(samples) } else { samples };
        Ok(Some(Token { tok: Tok::Data(samples.to_vec()), hidden: false }))
    }

    /// Where the `%%EndData` that ends an image's samples starts, looked for from `start` to
    /// `stop` as [`crate::ai_lex::Lexer`] looks for it in that window: the first one that `XH`
    /// follows on the next line, else the first one that a line end or the window's end follows.
    /// The bytes are read only as far as the first one that `XH` follows, or to `stop` when none
    /// does; each marker is judged once the bytes after it that decide are held.
    fn end_of_samples(&mut self, start: usize, stop: usize) -> Result<Option<usize>, Error> {
        let mut at = start;
        let mut first = None;
        loop {
            let have = self.visible_end().min(stop);
            let complete = have == stop || self.ended_at(have);
            while at < have {
                let Some(i) = find(self.held(at, have), END_DATA).map(|i| at + i) else {
                    // A marker cut short at the end is looked for again with the bytes after it.
                    at = at.max(have.saturating_sub(END_DATA.len() - 1));
                    break;
                };
                let after = i + END_DATA.len();
                // A line end, `%_` and `XH` after the marker decide.
                if after + 6 > have && !complete {
                    at = i;
                    break;
                }
                match ends_samples(self.held(after, have)) {
                    Some(true) => return Ok(Some(i)),
                    Some(false) => first = first.or(Some(i)),
                    None => {}
                }
                at = i + 1;
            }
            if complete {
                return Ok(first);
            }
            self.fill_to(have.saturating_add(1))?;
        }
    }
}

/// The marker that ends an image's samples.
const END_DATA: &[u8] = b"%%EndData";

/// What the bytes after a `%%EndData` (to the window's end) make of it, as the slice lexer judges:
/// `Some(true)` when `XH` (or `%_XH`) is on the next line, `Some(false)` when only a line end or
/// the window's end follows, `None` when other text follows on its line.
fn ends_samples(rest: &[u8]) -> Option<bool> {
    if !rest.first().is_none_or(|b| is_eol(*b)) {
        return None;
    }
    let next = rest.strip_prefix(b"\r").unwrap_or(rest);
    let next = next.strip_prefix(b"\n").unwrap_or(next);
    let next = next.strip_prefix(b"%_").unwrap_or(next);
    Some(next.starts_with(b"XH"))
}

impl<R: Read> Iterator for Lexer<R> {
    type Item = Result<Token, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token().transpose()
    }
}

/// The bytes after a [`Lexer`]'s last token, read as they come ([`Lexer::raw`]).
pub struct Raw<'a, R> {
    lexer: &'a mut Lexer<R>,
}

impl<R: Read> BufRead for Raw<'_, R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let l = &mut *self.lexer;
        l.mark = l.pos;
        if l.pos >= l.end() && !l.eof {
            l.read_more(l.chunk)?;
        }
        Ok(l.held(l.pos, l.end()))
    }

    fn consume(&mut self, n: usize) {
        let l = &mut *self.lexer;
        l.pos = l.pos.saturating_add(n).min(l.end());
    }
}

impl<R: Read> Read for Raw<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let held = self.fill_buf()?;
        let n = held.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..n), held.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.consume(n);
        Ok(n)
    }
}

#[cfg(test)]
mod tests;
