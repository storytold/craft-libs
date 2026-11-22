//! Rust source as tokens: comments and layout are gone, so a change to either changes no version.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Ident(String),
    /// A string literal's contents as written.
    Text(String),
    /// A raw string literal's contents: `r"\n"` is two characters, `"\n"` one, so the two never compare equal.
    Raw(String),
    /// A number or character literal, a lifetime.
    Literal(String),
    Punct(char),
}

impl Token {
    pub fn is(&self, word: &str) -> bool {
        matches!(self, Token::Ident(i) if i == word)
    }

    pub fn is_punct(&self, c: char) -> bool {
        matches!(self, Token::Punct(p) if *p == c)
    }

    pub fn ident(&self) -> Option<&str> {
        match self {
            Token::Ident(i) => Some(i),
            _ => None,
        }
    }

    /// The token as it goes into a digest.
    pub fn write(&self, out: &mut String) {
        match self {
            Token::Ident(s) | Token::Literal(s) => out.push_str(s),
            Token::Text(s) => {
                out.push('"');
                out.push_str(s);
                out.push('"');
            }
            Token::Raw(s) => {
                out.push_str("r\"");
                out.push_str(s);
                out.push('"');
            }
            Token::Punct(c) => out.push(*c),
        }
        out.push(' ');
    }
}

struct Cursor<'a> {
    chars: &'a [char],
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek(0).filter(|c| keep(*c)) {
            out.push(c);
            self.at += 1;
        }
        out
    }

    fn skip_block_comment(&mut self) {
        self.at += 2;
        let mut depth = 1;
        while depth > 0 && self.at < self.chars.len() {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('*')) => {
                    depth += 1;
                    self.at += 2;
                }
                (Some('*'), Some('/')) => {
                    depth -= 1;
                    self.at += 2;
                }
                _ => self.at += 1,
            }
        }
    }

    /// A `"` string with escapes; the cursor is on the opening quote.
    fn string(&mut self) -> String {
        self.at += 1;
        let mut out = String::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '"' => break,
                '\\' => {
                    out.push(c);
                    out.extend(self.peek(0));
                    self.at += 1;
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// A raw string, if one starts here (`r"`, `r#"`, with the cursor on the `r`).
    fn raw_string(&mut self) -> Option<String> {
        let mut hashes = 0;
        while self.peek(1 + hashes) == Some('#') {
            hashes += 1;
        }
        if self.peek(1 + hashes) != Some('"') {
            return None;
        }
        self.at += 2 + hashes;
        let mut out = String::new();
        while let Some(c) = self.peek(0) {
            if c == '"' && (0..hashes).all(|k| self.peek(1 + k) == Some('#')) {
                self.at += 1 + hashes;
                break;
            }
            out.push(c);
            self.at += 1;
        }
        Some(out)
    }

    /// A character literal or a lifetime; the cursor is on the `'`.
    fn quote(&mut self) -> String {
        let closes = match self.peek(1) {
            Some('\\') => true,
            Some(_) => self.peek(2) == Some('\''),
            None => false,
        };
        let mut out = String::from("'");
        self.at += 1;
        if closes {
            while let Some(c) = self.peek(0) {
                self.at += 1;
                out.push(c);
                if c == '\\' {
                    out.extend(self.peek(0));
                    self.at += 1;
                } else if c == '\'' {
                    break;
                }
            }
        } else {
            out.push_str(&self.take_while(is_ident));
        }
        out
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `source` as tokens.
pub fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut cur = Cursor { chars: &chars, at: 0 };
    let mut out = Vec::new();
    while let Some(c) = cur.peek(0) {
        match (c, cur.peek(1)) {
            (_, _) if c.is_whitespace() => cur.at += 1,
            ('/', Some('/')) => {
                cur.take_while(|c| c != '\n');
            }
            ('/', Some('*')) => cur.skip_block_comment(),
            ('"', _) => out.push(Token::Text(cur.string())),
            ('\'', _) => out.push(Token::Literal(cur.quote())),
            ('b', Some('"')) => {
                cur.at += 1;
                out.push(Token::Text(cur.string()));
            }
            ('b', Some('\'')) => {
                cur.at += 1;
                out.push(Token::Literal(cur.quote()));
            }
            ('r' | 'b', Some('"' | '#' | 'r')) => {
                let at = cur.at;
                if c == 'b' && cur.peek(1) == Some('r') {
                    cur.at += 1;
                }
                match cur.raw_string() {
                    Some(text) => out.push(Token::Raw(text)),
                    None => {
                        cur.at = at;
                        out.push(Token::Ident(cur.take_while(is_ident)));
                    }
                }
            }
            _ if c.is_ascii_digit() => {
                // a `.` belongs to the number only when a digit follows: `0..n` and `x.0.foo` stay apart
                let mut text = cur.take_while(is_ident);
                if cur.peek(0) == Some('.') && cur.peek(1).is_some_and(|d| d.is_ascii_digit()) {
                    cur.at += 1;
                    text.push('.');
                    text.push_str(&cur.take_while(is_ident));
                }
                out.push(Token::Literal(text));
            }
            _ if is_ident(c) => out.push(Token::Ident(cur.take_while(is_ident))),
            _ => {
                out.push(Token::Punct(c));
                cur.at += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idents(s: &str) -> Vec<String> {
        tokenize(s).iter().filter_map(|t| t.ident().map(str::to_string)).collect()
    }

    #[test]
    fn comments_and_layout_do_not_count() {
        assert_eq!(tokenize("fn  a( ) /* x /* y */ z */ { // c\n 1 }"), tokenize("fn a(){1}"));
    }

    #[test]
    fn strings_do_not_hide_or_invent_code() {
        assert_eq!(idents(r#"a("b // c"); d"#), ["a", "d"]);
        assert_eq!(idents(r##"a(r#"b " c"#); d(br"e"); f(b"g\""); h"##), ["a", "d", "f", "h"]);
    }

    #[test]
    fn a_raw_string_is_not_its_escaped_spelling() {
        assert_ne!(tokenize(r#""\n""#), tokenize(r##"r"\n""##));
        assert_eq!(tokenize(r##"r"a""##), tokenize(r###"r#"a"#"###));
    }

    #[test]
    fn a_change_inside_a_string_is_a_change() {
        assert_ne!(tokenize(r#"a("x")"#), tokenize(r#"a("y")"#));
    }

    #[test]
    fn lifetimes_chars_and_numbers() {
        assert_eq!(idents("fn f<'a>(x: &'a str) -> char { 'x' }"), ["fn", "f", "x", "str", "char"]);
        assert_eq!(idents(r"let q = '\''; let y = b'\n'; z"), ["let", "q", "let", "y", "z"]);
        assert_eq!(idents("0..n; x.0.foo; 1.5e3f32"), ["n", "x", "foo"]);
    }
}
