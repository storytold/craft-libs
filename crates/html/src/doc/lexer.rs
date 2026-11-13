//! The crate's tokenizer ([`crate::Lexer`]) as the document reader takes its tokens: text and
//! attribute values decoded, element and attribute names in lower case.

use crate::{Token, unescape, unescape_attr};

#[derive(Debug)]
pub(super) enum Tok {
    Text(String),
    Start { name: String, attrs: Vec<(String, String)>, self_close: bool },
    End(String),
}

pub(super) struct Lexer<'a> {
    inner: crate::Lexer<'a, 'static>,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(s: &'a str) -> Lexer<'a> {
        Lexer { inner: crate::Lexer::new(s) }
    }

    /// Where the next token starts.
    pub(super) fn pos(&self) -> usize {
        self.inner.pos()
    }

    /// Read again from `pos` (an earlier [`Lexer::pos`]).
    pub(super) fn set_pos(&mut self, pos: usize) {
        self.inner.set_pos(pos);
    }

    /// Skip past the end tag of raw text element `tag`; returns where that end tag starts (the
    /// end of the input when there is none).
    pub(super) fn skip_past_end_tag(&mut self, tag: &str) -> usize {
        let from = self.inner.pos();
        from + self.inner.raw_content(tag).len()
    }

    pub(super) fn next(&mut self) -> Option<Tok> {
        Some(match self.inner.next()? {
            Token::Text(t) => Tok::Text(unescape(t)),
            Token::Start(tag) => Tok::Start {
                attrs: tag.attrs.iter().map(|(n, v)| (n.to_ascii_lowercase(), unescape_attr(v))).collect(),
                name: tag.name,
                self_close: tag.self_close,
            },
            Token::End(name) => Tok::End(name),
        })
    }
}
