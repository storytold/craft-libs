//! `Label` and `Word`: the text a component shows, beside the words it may come from.

use std::borrow::Cow;
use std::sync::Arc;

pub use craft_i18n::Word;
use craft_i18n::Words;
use craft_theme::variant::Tone;

/// A label: a literal stays borrowed, a built string is shared. Cloning is cheap.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum Label {
    /// No text.
    #[default]
    Empty,
    /// A literal.
    Static(&'static str),
    /// A built string.
    Shared(Arc<str>),
    /// A word from a catalogue.
    Word(Word),
}

impl Label {
    /// The text of a label whose `Word` is not resolved yet reads as its English text.
    pub fn as_str(&self) -> &str {
        match self {
            Label::Empty => "",
            Label::Static(text) => text,
            Label::Shared(text) => text,
            Label::Word(word) => word.english,
        }
    }
}

impl Label {
    /// The label of `text`: a borrowed text stays borrowed.
    pub fn from_cow(text: Cow<'static, str>) -> Self {
        match text {
            Cow::Borrowed(text) => Label::Static(text),
            Cow::Owned(text) => Label::Shared(text.into()),
        }
    }

    /// This label with a `Word` replaced by its text in `words`, resolved once at step 1.
    #[doc(hidden)]
    pub fn resolved(&self, words: &Words) -> Label {
        match self {
            Label::Word(word) => Label::from_cow(words.get(*word)),
            other => other.clone(),
        }
    }
}

impl From<&'static str> for Label {
    fn from(text: &'static str) -> Self {
        Label::Static(text)
    }
}

impl From<String> for Label {
    fn from(text: String) -> Self {
        Label::Shared(text.into())
    }
}

impl From<Arc<str>> for Label {
    fn from(text: Arc<str>) -> Self {
        Label::Shared(text)
    }
}

impl From<Word> for Label {
    fn from(word: Word) -> Self {
        Label::Word(word)
    }
}

impl From<Label> for Cow<'static, str> {
    fn from(label: Label) -> Self {
        match label {
            Label::Empty => Cow::Borrowed(""),
            Label::Static(text) => Cow::Borrowed(text),
            Label::Shared(text) => Cow::Owned(text.to_string()),
            Label::Word(word) => Cow::Borrowed(word.english),
        }
    }
}

/// The text of a `Validity`: a label and an optional tone override. `Warn` defaults to `Tone::Warn`, `Refuse` to
/// `Tone::Bad`; `effective_tone` is the one place that default is decided.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Message {
    /// The text.
    pub text: Label,
    /// The tone, when it is not the one the kind of validity implies.
    pub tone: Option<Tone>,
}

impl Message {
    /// A message with the default tone.
    pub fn new(text: impl Into<Label>) -> Self {
        Message { text: text.into(), tone: None }
    }

    /// The message in `tone`.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// The tone the message shows in: its own, else `Warn` when it belongs to a `Validity::Warn` (`warn`), else `Bad`.
    pub fn effective_tone(&self, warn: bool) -> Tone {
        self.tone.unwrap_or(if warn { Tone::Warn } else { Tone::Bad })
    }
}
