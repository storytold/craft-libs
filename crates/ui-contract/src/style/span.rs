//! `Span`: one styled stretch of a text run.

use std::borrow::Cow;

/// One styled stretch of a run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Span<'a> {
    /// The text.
    pub text: Cow<'a, str>,
    /// Italic.
    pub italic: bool,
    /// Tabular figures.
    pub tabular: bool,
    /// The monospace face.
    pub mono: bool,
    /// The muted text colour.
    pub muted: bool,
    /// A shortcut hint: muted.
    pub shortcut: bool,
    /// A search match highlight.
    pub highlight: bool,
    /// Underlined in the span's colour, `stroke.thin` wide, along the bottom of the line box.
    pub underline: bool,
    /// The role's semibold face, at the role's size.
    pub strong: bool,
}

impl<'a> Span<'a> {
    /// A plain span.
    pub fn new(text: impl Into<Cow<'a, str>>) -> Self {
        Span {
            text: text.into(),
            italic: false,
            tabular: false,
            mono: false,
            muted: false,
            shortcut: false,
            highlight: false,
            underline: false,
            strong: false,
        }
    }

    /// Italic.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Tabular figures.
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }

    /// The monospace face.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The muted colour.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// A shortcut hint.
    pub fn shortcut(mut self) -> Self {
        self.shortcut = true;
        self
    }

    /// A match highlight.
    pub fn highlight(mut self) -> Self {
        self.highlight = true;
        self
    }

    /// Underlined: a link, or the mnemonic letter of a menu title.
    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// The role's semibold face: a short mark that stands out among the row's other text, such as a layer's `fx`.
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }
}
