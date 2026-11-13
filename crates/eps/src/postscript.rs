//! A bounded-token PostScript scanner for EPS programs.
//!
//! This module only turns source bytes into values. It does not execute operators or interpret
//! graphics state. That boundary lets an application keep its own drawing and font policies while
//! sharing the format-level lexical rules.

/// Maximum nesting depth for executable procedures in a source token.
pub const MAX_PROCEDURE_DEPTH: usize = 100;

/// A lexical PostScript value. Names are left unevaluated for the caller's interpreter.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A number accepted by the PostScript source reader.
    Number(f64),
    /// An executable name, including immediately evaluated `//name`.
    Name(String),
    /// A literal name such as `/name`.
    LiteralName(String),
    /// A parenthesized or hexadecimal byte string.
    String(Vec<u8>),
    /// The encoded bytes between `<~` and `~>` (including delimiters), not decoded data.
    Ascii85(Vec<u8>),
    /// A procedure whose nested values have already been tokenized.
    Procedure(Vec<Token>),
    /// A closing brace not consumed as part of a procedure, or another malformed delimiter.
    Null,
}

/// PostScript source bytes and the current byte offset.
pub struct Scanner<'a> {
    /// Source bytes. Public so an interpreter can implement `currentfile` operators.
    pub data: &'a [u8],
    /// Current byte offset in [`Scanner::data`].
    pub pos: usize,
}

impl<'a> Scanner<'a> {
    /// Start scanning at a byte offset, clamped to the source length.
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos: pos.min(data.len()) }
    }

    /// The next token; `{ … }` is returned as one procedure token.
    pub fn token(&mut self) -> Option<Token> {
        self.token_at_depth(0)
    }

    fn token_at_depth(&mut self, depth: usize) -> Option<Token> {
        self.skip_ws();
        let c = *self.data.get(self.pos)?;
        Some(match c {
            b'{' => {
                self.pos += 1;
                let mut body = Vec::new();
                loop {
                    self.skip_ws();
                    match self.data.get(self.pos) {
                        None => break,
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        _ if depth >= MAX_PROCEDURE_DEPTH => break,
                        _ => match self.token_at_depth(depth + 1) {
                            Some(t) => body.push(t),
                            None => break,
                        },
                    }
                }
                Token::Procedure(body)
            }
            b'}' => {
                self.pos += 1;
                Token::Null
            }
            b'/' => {
                self.pos += 1;
                let immediately_evaluated = self.data.get(self.pos) == Some(&b'/');
                if immediately_evaluated {
                    self.pos += 1;
                }
                let name = String::from_utf8_lossy(self.word()).into_owned();
                if immediately_evaluated { Token::Name(name) } else { Token::LiteralName(name) }
            }
            b'(' => Token::String(self.parenthesized()),
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Token::Name("<<".to_owned())
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Token::Name(">>".to_owned())
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'~') => self.ascii85(),
            b'<' => Token::String(self.hex_string()),
            b'[' | b']' => {
                self.pos += 1;
                Token::Name(char::from(c).to_string())
            }
            b')' | b'>' => {
                self.pos += 1;
                Token::Null
            }
            _ => {
                let word = self.word();
                if word.is_empty() {
                    self.pos += 1;
                    return Some(Token::Null);
                }
                match parse_number(word) {
                    Some(n) => Token::Number(n),
                    None => {
                        let name = String::from_utf8_lossy(word).into_owned();
                        if let Some((radix, digits)) = name.split_once('#') {
                            let radix = radix.parse::<u32>().ok().filter(|r| (2..=36).contains(r));
                            if let Some(number) = radix.and_then(|r| i64::from_str_radix(digits, r).ok()) {
                                return Some(Token::Number(number as f64));
                            }
                        }
                        Token::Name(name)
                    }
                }
            }
        })
    }

    /// Step over inline image data that follows an `image` operator reading `currentfile`.
    pub fn skip_image_data(&mut self) {
        self.skip_ws();
        let rest = self.data.get(self.pos..).unwrap_or_default();
        if rest.starts_with(b"<~") || self.data.get(self.pos).is_some_and(|c| (b'!'..=b'u').contains(c) && !c.is_ascii_hexdigit()) {
            self.pos = match rest.windows(2).position(|w| w == b"~>") {
                Some(p) => self.pos.saturating_add(p).saturating_add(2).min(self.data.len()),
                None => self.data.len(),
            };
            return;
        }
        while self.pos < self.data.len() && (self.data[self.pos].is_ascii_hexdigit() || is_white(self.data[self.pos])) {
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'>') {
            self.pos += 1;
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            if is_white(c) {
                self.pos += 1;
            } else if c == b'%' {
                let start = self.pos;
                while self.pos < self.data.len() && !matches!(self.data[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
                let line = &self.data[start..self.pos];
                let end_marker: Option<&[u8]> = if line.starts_with(b"%%BeginData:") {
                    Some(b"%%EndData")
                } else if line.starts_with(b"%%BeginBinary:") {
                    Some(b"%%EndBinary")
                } else if line.starts_with(b"%%BeginPreview") {
                    Some(b"%%EndPreview")
                } else {
                    None
                };
                if let Some(marker) = end_marker {
                    self.pos = match self.data.get(self.pos..).unwrap_or_default().windows(marker.len()).position(|w| w == marker) {
                        Some(offset) => self.pos.saturating_add(offset).min(self.data.len()),
                        None => self.data.len(),
                    };
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &[u8] {
        let start = self.pos;
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            if is_white(c) || matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') {
                break;
            }
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    fn parenthesized(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut nesting = 1usize;
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            match c {
                b'\\' => {
                    let e = self.data.get(self.pos).copied().unwrap_or(b'\\');
                    self.pos = self.pos.saturating_add(1).min(self.data.len());
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'0'..=b'7' => {
                            let mut value = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(*d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        b'\n' | b'\r' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    nesting = nesting.saturating_add(1);
                    out.push(c);
                }
                b')' => {
                    nesting = nesting.saturating_sub(1);
                    if nesting == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn hex_string(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut hex = Vec::new();
        while self.pos < self.data.len() && self.data[self.pos] != b'>' {
            if self.data[self.pos].is_ascii_hexdigit() {
                hex.push(self.data[self.pos]);
            }
            self.pos += 1;
        }
        if self.data.get(self.pos) == Some(&b'>') {
            self.pos += 1;
        }
        if hex.len() % 2 == 1 {
            hex.push(b'0');
        }
        hex.chunks(2).filter_map(|pair| Some((hex_value(*pair.first()?) << 4) | hex_value(*pair.get(1)?))).collect()
    }

    fn ascii85(&mut self) -> Token {
        let start = self.pos;
        self.pos = self.pos.saturating_add(2).min(self.data.len());
        while self.pos + 1 < self.data.len() && !(self.data[self.pos] == b'~' && self.data[self.pos + 1] == b'>') {
            self.pos += 1;
        }
        self.pos = self.pos.saturating_add(2).min(self.data.len());
        Token::Ascii85(self.data[start..self.pos].to_vec())
    }
}

fn is_white(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

fn hex_value(c: u8) -> u8 {
    (c as char).to_digit(16).unwrap_or(0) as u8
}

fn parse_number(word: &[u8]) -> Option<f64> {
    if word.is_empty() || !word.iter().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.')) || !word.iter().any(u8::is_ascii_digit) {
        return None;
    }
    let text = std::str::from_utf8(word).ok()?;
    text.parse::<f64>()
        .ok()
        .or_else(|| text.trim_start_matches(['+', '-']).parse::<f64>().ok().map(|value| if text.starts_with('-') { -value } else { value }))
}

#[cfg(test)]
mod tests {
    use super::{Scanner, Token};

    fn scan(source: &[u8]) -> Vec<Token> {
        let mut scanner = Scanner::new(source, 0);
        let mut tokens = Vec::new();
        while let Some(token) = scanner.token() {
            tokens.push(token);
        }
        tokens
    }

    #[test]
    fn scans_names_numbers_strings_and_procedures() {
        assert_eq!(
            scan(b"/x //y 16#ff { 2 (a\\n(b)) }"),
            vec![
                Token::LiteralName("x".into()),
                Token::Name("y".into()),
                Token::Number(255.0),
                Token::Procedure(vec![Token::Number(2.0), Token::String(b"a\n(b)".to_vec())]),
            ]
        );
    }

    #[test]
    fn skips_dsc_binary_data_and_preserves_inline_ascii85() {
        assert_eq!(scan(b"%%BeginBinary: 4\nxxxx%%EndBinary\n<~z~>"), vec![Token::Ascii85(b"<~z~>".to_vec())]);
    }

    #[test]
    fn malformed_delimiters_terminate_without_advancing_past_source() {
        let mut scanner = Scanner::new(b"<deadbeef", 0);
        assert_eq!(scanner.token(), Some(Token::String(vec![0xde, 0xad, 0xbe, 0xef])));
        assert_eq!(scanner.pos, scanner.data.len());
    }
}
