//! PostScript token scanner over an in-memory byte slice, tracking the unread whitespace
//! boundary for `currentfile` data reads (`read`, `readstring`, filters, `token`, `eexec`).

use std::fmt;

/// Deepest procedure nesting the scanner reads.
const MAX_NESTING: usize = 256;

/// Most bytes an ASCII85 string literal may decode to.
const MAX_ASCII85: usize = 1 << 28;

/// How an immediately evaluated name (`//name`) is prefixed in [`Token::Exec`], until the
/// interpreter replaces it.
pub const IMMEDIATE: &str = "//";

/// A scanned PostScript token.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Int(i64),
    Real(f64),
    /// A literal name (`/name`).
    Name(String),
    /// An executable name (operator, `[`, `]`, `<<`, `>>`, or `//name` prefixed with [`IMMEDIATE`]).
    Exec(String),
    /// A byte string (`(...)`, `<...>`, or `<~...~>`).
    Str(Vec<u8>),
    /// A procedure (`{ ... }`).
    Proc(Vec<Token>),
}

/// A finite number parsed from a PostScript numeric token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    Int(i64),
    Real(f64),
}

/// An error returned by [`Lexer::next`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LexError {
    Syntax(String),
    Limit(&'static str),
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(what) => write!(f, "syntaxerror: {what}"),
            Self::Limit(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for LexError {}

/// PostScript byte-slice scanner with `currentfile` data boundary tracking.
pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    /// The whitespace character that ended the last token is still unread (data the program
    /// reads from itself starts after it).
    fresh: bool,
    /// The last object read has immediately evaluated names (`//name`) in it, read as executable
    /// names starting with [`IMMEDIATE`] for the interpreter to replace by their values.
    pub immediate: bool,
}

/// Whether `b` is PostScript whitespace.
pub fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

/// Whether `b` is a PostScript delimiter.
pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn syntax<T>(what: &str) -> Result<T, LexError> {
    Err(LexError::Syntax(what.to_string()))
}

impl<'a> Lexer<'a> {
    /// Create a scanner over `src`.
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, fresh: false, immediate: false }
    }

    /// What is left to read.
    pub fn rest(&self) -> &'a [u8] {
        self.src.get(self.pos..).unwrap_or_default()
    }

    /// What is left to read as data: after the whitespace that ended the last token (a CR LF
    /// pair counts as one).
    pub fn data(&mut self) -> &'a [u8] {
        if std::mem::take(&mut self.fresh) {
            match self.peek() {
                Some(b'\r') => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                }
                Some(b) if is_space(b) => self.pos += 1,
                _ => {}
            }
        }
        self.rest()
    }

    /// Skip `n` bytes (the data a filter or `readstring` took).
    pub fn advance(&mut self, n: usize) {
        self.fresh = false;
        self.pos = self.pos.saturating_add(n).min(self.src.len());
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if is_space(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next token at the top level (`None` at the end of the file).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Token>, LexError> {
        self.immediate = false;
        let t = self.token(0)?;
        self.fresh = true;
        Ok(t)
    }

    fn token(&mut self, depth: usize) -> Result<Option<Token>, LexError> {
        self.skip_space();
        let Some(b) = self.peek() else { return Ok(None) };
        self.pos += 1;
        Ok(Some(match b {
            b'(' => Token::Str(self.string()?),
            b'<' => self.angle_string()?,
            b'>' => {
                if self.peek() != Some(b'>') {
                    return syntax(">");
                }
                self.pos += 1;
                Token::Exec(">>".to_string())
            }
            b'[' => Token::Exec("[".to_string()),
            b']' => Token::Exec("]".to_string()),
            b'{' => self.proc(depth)?,
            b'}' | b')' => return syntax(&char::from(b).to_string()),
            b'/' => self.name(),
            _ => {
                self.pos -= 1;
                let w = self.word();
                match number(&w) {
                    Some(Number::Int(i)) => Token::Int(i),
                    Some(Number::Real(r)) => Token::Real(r),
                    None => Token::Exec(w),
                }
            }
        }))
    }

    fn angle_string(&mut self) -> Result<Token, LexError> {
        match self.peek() {
            Some(b'<') => {
                self.pos += 1;
                Ok(Token::Exec("<<".to_string()))
            }
            Some(b'~') => {
                self.pos += 1;
                Ok(Token::Str(self.ascii85()?))
            }
            _ => Ok(Token::Str(self.hex()?)),
        }
    }

    fn proc(&mut self, depth: usize) -> Result<Token, LexError> {
        if depth >= MAX_NESTING {
            return Err(LexError::Limit("procedures nested too deeply"));
        }
        let mut items = vec![];
        loop {
            self.skip_space();
            match self.peek() {
                None => return syntax("{"),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => match self.token(depth + 1)? {
                    Some(o) => items.push(o),
                    None => return syntax("{"),
                },
            }
        }
        Ok(Token::Proc(items))
    }

    fn name(&mut self) -> Token {
        // `//name` stands for the name's value when it is read.
        let immediate = self.peek() == Some(b'/');
        if immediate {
            self.pos += 1;
            self.immediate = true;
        }
        let name = self.word();
        if immediate { Token::Exec(format!("{IMMEDIATE}{name}")) } else { Token::Name(name) }
    }

    /// A regular token's characters.
    fn word(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_space(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(self.src.get(start..self.pos).unwrap_or_default()).into_owned()
    }

    /// A literal string after its `(`.
    fn string(&mut self) -> Result<Vec<u8>, LexError> {
        let mut out = vec![];
        let mut depth = 0usize;
        loop {
            let Some(b) = self.peek() else { return syntax("(") };
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => return Ok(out),
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => self.string_escape(&mut out)?,
                // PLRM §3.8 treats an unescaped CR or CRLF as one LF inside a string.
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
    }

    fn string_escape(&mut self, out: &mut Vec<u8>) -> Result<(), LexError> {
        let Some(e) = self.peek() else { return syntax("(") };
        self.pos += 1;
        match e {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'\r' => {
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                for _ in 0..2 {
                    match self.peek() {
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

    /// A hexadecimal string after its `<`.
    fn hex(&mut self) -> Result<Vec<u8>, LexError> {
        let Some(end) = self.rest().iter().position(|b| *b == b'>') else {
            return syntax("<");
        };
        let body = self.rest().get(..end).unwrap_or_default();
        let Some(out) = hex_decode(body) else {
            return syntax("<");
        };
        self.advance(end + 1);
        Ok(out)
    }

    /// An ASCII85 string after its `<~`.
    fn ascii85(&mut self) -> Result<Vec<u8>, LexError> {
        let Some(end) = find(self.rest(), b"~>") else {
            return syntax("<~");
        };
        let text = String::from_utf8_lossy(self.rest().get(..end + 2).unwrap_or_default()).into_owned();
        self.advance(end + 2);
        let Some(out) = craft_eps::ascii85::decode(text.as_bytes(), MAX_ASCII85) else {
            return syntax("<~");
        };
        Ok(out)
    }
}

/// Where `needle` first starts in `hay`.
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Hexadecimal digits (whitespace ignored, an odd last digit padded with 0); `None` on others.
pub fn hex_decode(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() / 2);
    let mut high: Option<u8> = None;
    for &b in body {
        if is_space(b) {
            continue;
        }
        let v = char::from(b).to_digit(16)? as u8;
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// A number token (`12`, `-3.5`, `.5`, `1e-3`, `16#ff`), or `None`.
pub fn number(w: &str) -> Option<Number> {
    let first = w.bytes().next()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if let Ok(i) = w.parse::<i64>() {
        return Some(Number::Int(i));
    }
    if let Some((base, digits)) = w.split_once('#') {
        let base: u32 = base.parse().ok().filter(|b| (2..=36).contains(b))?;
        // `from_str_radix` accepts a leading sign; a PostScript radix number has digits only.
        if digits.starts_with(['+', '-']) {
            return None;
        }
        return i64::from_str_radix(digits, base).ok().map(Number::Int);
    }
    // Rust reads `inf` and `nan`; PostScript doesn't.
    if w.bytes().any(|b| b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E')) {
        return None;
    }
    let v: f64 = w.parse().ok()?;
    v.is_finite().then_some(Number::Real(v))
}

#[cfg(test)]
mod tests {
    use super::{LexError, Lexer, Number, Token, find, hex_decode, number};

    #[test]
    fn radix_number_with_sign_after_hash_is_not_a_number() {
        assert!(number("16#+ff").is_none());
        assert!(number("16#-ff").is_none());
        assert_eq!(number("16#ff"), Some(Number::Int(255)));
    }

    #[test]
    fn currentfile_data_consumes_single_whitespace_or_crlf() {
        let mut lex = Lexer::new(b"image\r\nDATA");
        assert_eq!(lex.next(), Ok(Some(Token::Exec("image".into()))));
        assert_eq!(lex.rest(), b"\r\nDATA");
        assert_eq!(lex.data(), b"DATA");
        lex.advance(2);
        assert_eq!(lex.rest(), b"TA");
    }

    #[test]
    fn literal_string_line_endings_preserve_currentfile_data() {
        for eol in [b"\n".as_slice(), b"\r", b"\r\n"] {
            let source = [b"(first".as_slice(), eol, b"second\\", eol, b"third\\r\\015)", b"\r\nA\r\nB"].concat();
            let mut lex = Lexer::new(&source);
            assert_eq!(lex.next(), Ok(Some(Token::Str(b"first\nsecondthird\r\r".to_vec()))), "line ending {eol:?}");
            assert_eq!(lex.rest(), b"\r\nA\r\nB");
            assert_eq!(lex.data(), b"A\r\nB");
        }
    }

    #[test]
    fn partial_lines_and_eof() {
        let mut lex = Lexer::new(b"1 2 add");
        assert_eq!(lex.next(), Ok(Some(Token::Int(1))));
        assert_eq!(lex.next(), Ok(Some(Token::Int(2))));
        assert_eq!(lex.next(), Ok(Some(Token::Exec("add".into()))));
        assert_eq!(lex.next(), Ok(None));
        assert_eq!(lex.data(), b"");
    }

    #[test]
    fn immediate_names_and_procedures() {
        let mut lex = Lexer::new(b"{ //foo /bar }");
        assert_eq!(lex.next(), Ok(Some(Token::Proc(vec![Token::Exec("//foo".into()), Token::Name("bar".into())]))));
        assert!(lex.immediate);
    }

    #[test]
    fn find_and_hex_decode() {
        assert_eq!(find(b"abcdef", b"cd"), Some(2));
        assert_eq!(find(b"abcdef", b""), Some(0));
        assert_eq!(find(b"abcdef", b"xy"), None);
        assert_eq!(hex_decode(b"41 4"), Some(vec![0x41, 0x40]));
        assert_eq!(hex_decode(b"4g"), None);
    }

    #[test]
    fn unterminated_string_is_syntax_error() {
        let mut lex = Lexer::new(b"(open");
        assert_eq!(lex.next(), Err(LexError::Syntax("(".into())));
    }

    #[test]
    fn error_does_not_mark_fresh() {
        let mut lex = Lexer::new(b") rest");
        assert_eq!(lex.next(), Err(LexError::Syntax(")".into())));
        assert_eq!(lex.data(), b" rest");
    }
}
