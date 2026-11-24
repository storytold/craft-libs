//! The tokens of an RTF file. Nothing is interpreted here: a token is what the bytes say.

/// Is this an RTF file? Leading white space, then `{\rtf`.
pub fn is_rtf(bytes: &[u8]) -> bool {
    bytes.trim_ascii_start().starts_with(b"{\\rtf")
}

/// One token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    /// `{`
    Open,
    /// `}`
    Close,
    /// A control word: `\par`, `\fs24`, `\li-720`. The parameter is clamped into `i32`.
    Word { name: &'a str, param: Option<i32> },
    /// `\*`: the destination that follows may be ignored by a reader that does not know it.
    Star,
    /// `\'hh`: one byte of text in the current code page.
    Hex(u8),
    /// Any other control symbol, by the byte after the backslash: `\~`, `\-`, `\_`, `\\`,
    /// `\{`, `\}`, and `\` before a line end (which is `\par`).
    Symbol(u8),
    /// The data of `\binN`: the `N` bytes after the word's delimiter, as they are.
    Bin(&'a [u8]),
    /// A run of text bytes in the current code page: no `{`, `}`, `\`, line end or NUL.
    Text(&'a [u8]),
}

/// The tokens of `bytes`, in order. Never fails: a malformed escape is dropped and whatever
/// follows it is read as the structure it is.
///
/// - A control word is at most 32 letters and its parameter at most 10 digits; what follows is
///   text. A `-` starts a parameter only when a digit follows it.
/// - One space after a control word is its delimiter and not text.
/// - Line ends and NUL bytes between tokens are not tokens.
/// - `\'` without two hex digits gives no token: it and the hex digits it does have are
///   skipped, and a `}` or `\` after it is read as itself.
/// - A date word (`\revdttm`, `\revdttmdel`, `\crdate`, `\prdate`) holds a 32-bit pattern that
///   some writers put unsigned: it is read as that pattern, not clamped.
#[derive(Clone, Debug)]
pub struct Tokens<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Tokens<'a> {
    pub fn new(bytes: &'a [u8]) -> Tokens<'a> {
        Tokens { bytes, at: 0 }
    }

    /// The offset of the next token.
    pub fn offset(&self) -> usize {
        self.at
    }

    fn word(&mut self) -> Token<'a> {
        let b = self.bytes;
        let start = self.at + 1;
        let mut j = start;
        while b.get(j).is_some_and(u8::is_ascii_alphabetic) && j - start < 32 {
            j += 1;
        }
        let name = b.get(start..j).and_then(|w| std::str::from_utf8(w).ok()).unwrap_or("");
        let neg = b.get(j) == Some(&b'-');
        let digits = j + usize::from(neg);
        let mut k = digits;
        while b.get(k).is_some_and(u8::is_ascii_digit) && k - digits < 10 {
            k += 1;
        }
        let param = if k > digits {
            // At most 10 digits, so it fits an i64.
            let v = b.get(digits..k).and_then(|d| std::str::from_utf8(d).ok()).and_then(|d| d.parse::<i64>().ok()).unwrap_or(0);
            let v = if neg { -v } else { v };
            let dttm = matches!(name, "revdttm" | "revdttmdel" | "crdate" | "prdate");
            let v = if dttm && v > i64::from(i32::MAX) && v <= i64::from(u32::MAX) { v - (1i64 << 32) } else { v };
            Some(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
        } else {
            k = j;
            None
        };
        if b.get(k) == Some(&b' ') {
            k += 1;
        }
        self.at = k;
        if name == "bin" {
            let n = usize::try_from(param.unwrap_or(0)).unwrap_or(0);
            let end = k.saturating_add(n).min(b.len());
            self.at = end;
            return Token::Bin(b.get(k..end).unwrap_or_default());
        }
        Token::Word { name, param }
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let b = self.bytes;
        loop {
            let c = *b.get(self.at)?;
            match c {
                b'{' | b'}' => {
                    self.at += 1;
                    return Some(if c == b'{' { Token::Open } else { Token::Close });
                }
                b'\r' | b'\n' | 0 => self.at += 1,
                b'\\' => {
                    let n = *b.get(self.at + 1)?;
                    if n.is_ascii_alphabetic() {
                        return Some(self.word());
                    }
                    self.at += 2;
                    match n {
                        b'\'' => {
                            let hex = b.get(self.at..self.at + 2).filter(|h| h.iter().all(u8::is_ascii_hexdigit));
                            if let Some(v) = hex.and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                                self.at += 2;
                                return Some(Token::Hex(v));
                            }
                            // Malformed: skip the hex digits of a truncated pair, nothing else.
                            let mut n = 0;
                            while n < 2 && b.get(self.at).is_some_and(u8::is_ascii_hexdigit) {
                                self.at += 1;
                                n += 1;
                            }
                        }
                        b'*' => return Some(Token::Star),
                        b'\r' => return Some(Token::Symbol(b'\n')),
                        n => return Some(Token::Symbol(n)),
                    }
                }
                _ => {
                    let start = self.at;
                    while b.get(self.at).is_some_and(|c| !matches!(c, b'{' | b'}' | b'\\' | b'\r' | b'\n' | 0)) {
                        self.at += 1;
                    }
                    return Some(Token::Text(b.get(start..self.at).unwrap_or_default()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::{Bin, Close, Hex, Open, Star, Symbol, Text};

    fn w(name: &str, param: Option<i32>) -> Token<'_> {
        Token::Word { name, param }
    }

    fn lex(s: &[u8]) -> Vec<Token<'_>> {
        Tokens::new(s).collect()
    }

    #[test]
    fn groups_words_symbols_and_text() {
        assert_eq!(
            lex(br"{\rtf1\ansi {\*\generator X;}\b Hi\b0  there\par\~\'e9}"),
            vec![
                Open,
                w("rtf", Some(1)),
                w("ansi", None),
                Open,
                Star,
                w("generator", None),
                Text(b"X;"),
                Close,
                w("b", None),
                Text(b"Hi"),
                w("b", Some(0)),
                Text(b" there"),
                w("par", None),
                Symbol(b'~'),
                Hex(0xE9),
                Close
            ]
        );
        assert!(is_rtf(b" \r\n{\\rtf1}") && !is_rtf(b"{rtf1}") && !is_rtf(b""));
    }

    #[test]
    fn a_hyphen_is_a_sign_only_before_a_digit() {
        assert_eq!(
            lex(br"\li-720 a\emdash-roll\tab- x"),
            vec![w("li", Some(-720)), Text(b"a"), w("emdash", None), Text(b"-roll"), w("tab", None), Text(b"- x")]
        );
    }

    #[test]
    fn long_words_and_parameters_are_cut_not_scanned_without_end() {
        let long = format!("\\{}1", "a".repeat(40));
        assert_eq!(lex(long.as_bytes()), vec![w(&"a".repeat(32), None), Text(b"aaaaaaaa1")]);
        assert_eq!(lex(br"\fs123456789012 x"), vec![w("fs", Some(1_234_567_890)), Text(b"12 x")]);
        assert_eq!(lex(br"\fs9999999999\li-9999999999"), vec![w("fs", Some(i32::MAX)), w("li", Some(i32::MIN))]);
    }

    #[test]
    fn dates_keep_their_bit_pattern() {
        assert_eq!(
            lex(br"\revdttm4294967295\crdate2147483648\fs4294967295"),
            vec![w("revdttm", Some(-1)), w("crdate", Some(i32::MIN)), w("fs", Some(i32::MAX))]
        );
    }

    #[test]
    fn malformed_hex_escapes_leave_the_structure_alone() {
        assert_eq!(lex(br"{\b A\'4}B"), vec![Open, w("b", None), Text(b"A"), Close, Text(b"B")]);
        assert_eq!(lex(br"a\'+fb\'\par"), vec![Text(b"a"), Text(b"+fb"), w("par", None)]);
        assert_eq!(lex(br"\'zz\'4"), vec![Text(b"zz")]);
        assert_eq!(lex(b"a\\"), vec![Text(b"a")], "a backslash at the end is dropped");
    }

    #[test]
    fn binary_data_is_not_scanned() {
        assert_eq!(lex(b"{\\pict\\bin5 {\\}\0\\x}"), vec![Open, w("pict", None), Bin(b"{\\}\0\\"), Text(b"x"), Close]);
        assert_eq!(lex(b"\\bin99 ab"), vec![Bin(b"ab")], "cut at the end of the file");
        assert_eq!(lex(b"\\bin-4 ab\\bin"), vec![Bin(b""), Text(b"ab"), Bin(b"")]);
    }

    #[test]
    fn line_ends_and_nul_are_not_text() {
        assert_eq!(lex(b"a\r\nb\0c\\\nd\\\r\ne"), vec![Text(b"a"), Text(b"b"), Text(b"c"), Symbol(b'\n'), Text(b"d"), Symbol(b'\n'), Text(b"e")]);
        assert_eq!(lex(b"\\\xe9x"), vec![Symbol(0xE9), Text(b"x")]);
        let mut t = Tokens::new(b"ab{");
        assert_eq!((t.next(), t.offset()), (Some(Text(b"ab")), 2));
    }
}
