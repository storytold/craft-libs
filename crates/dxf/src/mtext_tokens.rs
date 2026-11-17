//! Streaming lexical tokens for MTEXT inline markup.
//!
//! Apply [`crate::mtext::unescape_codes`] first when Unicode/MIF escapes should become literal
//! characters. Tokens borrow their source and scanning allocates no memory. The consumer owns
//! formatting, line layout, paragraph policy and how unknown commands are displayed.

/// How a stacked-text argument finds its closing semicolon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackMode {
    /// The first semicolon ends the stack, as plain-text extraction historically did.
    Plain,
    /// A backslash quotes the next character, as the structured layout parser does.
    Structured,
}
/// One lexical unit. Unknown commands retain their letter so the caller chooses compatibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token<'a> {
    Char(char),
    OpenGroup,
    CloseGroup,
    Control(char),
    Argument(char, &'a str),
    Stack(&'a str),
    Unknown(char),
}
/// A bounded-memory walk of UTF-8 MTEXT markup.
pub struct Tokens<'a> {
    source: &'a str,
    offset: usize,
    stack_mode: StackMode,
}
impl<'a> Tokens<'a> {
    pub fn new(source: &'a str, stack_mode: StackMode) -> Self {
        Self { source, offset: 0, stack_mode }
    }
    fn next_char(&mut self) -> Option<char> {
        let c = self.source.get(self.offset..)?.chars().next()?;
        self.offset += c.len_utf8();
        Some(c)
    }
    fn argument(&mut self, structured: bool) -> &'a str {
        let start = self.offset;
        let mut end = start;
        while let Some(c) = self.next_char() {
            if c == ';' {
                return self.source.get(start..end).unwrap_or_default();
            }
            if structured && c == '\\' {
                let _ = self.next_char();
            }
            end = self.offset;
        }
        self.source.get(start..end).unwrap_or_default()
    }
}
impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let c = self.next_char()?;
        Some(match c {
            '{' => Token::OpenGroup,
            '}' => Token::CloseGroup,
            '\\' => {
                let code = self.next_char()?;
                match code {
                    'S' => Token::Stack(self.argument(self.stack_mode == StackMode::Structured)),
                    c if crate::mtext::takes_arg(c) => Token::Argument(c, self.argument(false)),
                    c @ ('P' | 'N' | '~' | '\\' | '{' | '}' | 'L' | 'l' | 'O' | 'o' | 'K' | 'k') => Token::Control(c),
                    c => Token::Unknown(c),
                }
            }
            c => Token::Char(c),
        })
    }
}
/// Split a structured stack into top, bottom and separator, unquoting escaped characters.
pub fn split_stack(raw: &str) -> (String, String, Option<char>) {
    let (mut top, mut bottom, mut separator) = (String::new(), String::new(), None);
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(c) = chars.next() {
                if separator.is_none() {
                    top.push(c);
                } else {
                    bottom.push(c);
                }
            }
        } else if separator.is_none() && matches!(c, '/' | '#' | '^') {
            separator = Some(c);
        } else if separator.is_none() {
            top.push(c);
        } else {
            bottom.push(c);
        }
    }
    (top, bottom, separator)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_borrow_text_and_preserve_unknown_commands() {
        let got: Vec<_> = Tokens::new(r"A\Z\P{\H2x;x}\S1/2;", StackMode::Structured).collect();
        assert_eq!(
            got,
            [
                Token::Char('A'),
                Token::Unknown('Z'),
                Token::Control('P'),
                Token::OpenGroup,
                Token::Argument('H', "2x"),
                Token::Char('x'),
                Token::CloseGroup,
                Token::Stack("1/2")
            ]
        );
        assert_eq!(split_stack("1/2"), ("1".into(), "2".into(), Some('/')));
    }
    #[test]
    fn stacked_text_uses_the_requested_terminator_rules() {
        let text = r"\Sa\;b/c;X";
        assert_eq!(Tokens::new(text, StackMode::Plain).next(), Some(Token::Stack(r"a\")));
        assert_eq!(Tokens::new(text, StackMode::Structured).next(), Some(Token::Stack(r"a\;b/c")));
        assert_eq!(split_stack(r"a\;b/c"), ("a;b".into(), "c".into(), Some('/')));
    }
    #[test]
    fn missing_terminator_and_trailing_backslash_are_bounded() {
        assert_eq!(Tokens::new("\\H2x", StackMode::Structured).collect::<Vec<_>>(), [Token::Argument('H', "2x")]);
        assert_eq!(Tokens::new("a\\", StackMode::Structured).collect::<Vec<_>>(), [Token::Char('a')]);
    }
}
