//! The word core: the built-in strings of a library, in the language of the app.
//!
//! A [`Word`] names one string: its id, its English text and its context. [`Words`] looks words up in a
//! catalog and shows English where there is no catalog or no row, so a missing translation never fails
//! anything. A [`WordSet`] is a table of words with its English text, written by `craft_ui::words!`.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use crate::{I18n, Lang};

/// One built-in string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Word {
    /// `<module>.<camelCase>`, or `""` for a word looked up by its English text.
    pub id: &'static str,
    /// The text shown when no translation exists.
    pub english: &'static str,
    /// The catalog context of a word with no id (`key` for key caps), or `""`.
    pub context: &'static str,
}

impl Word {
    /// A word with an id; a catalog may translate it by the id or by the English text.
    pub const fn keyed(id: &'static str, english: &'static str) -> Word {
        Word { id, english, context: "" }
    }

    /// A word looked up by its English text alone.
    pub const fn plain(english: &'static str) -> Word {
        Word { id: "", english, context: "" }
    }

    /// A word whose translation is only the row of `context`, English where there is none.
    pub const fn in_context(context: &'static str, english: &'static str) -> Word {
        Word { id: "", english, context }
    }

    /// The id, or the English text of a word with no id.
    pub const fn key(&self) -> &'static str {
        if self.id.is_empty() { self.english } else { self.id }
    }
}

/// A count word: the registered word is the English plural, `one` the English singular. A catalog writes the
/// forms of a locale in an `@plural` row keyed by the two English texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Counted {
    /// The registered word: the English plural.
    pub word: Word,
    /// The English singular.
    pub one: &'static str,
}

impl Counted {
    /// The count word `id` with its English singular and plural.
    pub const fn new(id: &'static str, one: &'static str, other: &'static str) -> Counted {
        Counted { word: Word::keyed(id, other), one }
    }
}

type Lookup = Arc<dyn Fn(Word) -> Option<String> + Send + Sync>;
type Plural = Arc<dyn Fn(&'static str, u64, &'static str, &'static str) -> Option<String> + Send + Sync>;

/// The rows a `Words` scope adds over the words it inherits: a word and the text shown for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WordRows(Vec<(Word, String)>);

impl WordRows {
    /// No rows.
    pub fn new() -> Self {
        WordRows(Vec::new())
    }

    /// Adds the row `word` shows `text`.
    #[must_use]
    pub fn row(mut self, word: Word, text: impl Into<String>) -> Self {
        self.0.push((word, text.into()));
        self
    }

    /// The text of `word`, when a row gives one.
    pub fn text(&self, word: Word) -> Option<&str> {
        self.0.iter().rev().find(|(row, _)| *row == word).map(|(_, text)| text.as_str())
    }

    /// The rows in order.
    pub fn iter(&self) -> impl Iterator<Item = (Word, &str)> {
        self.0.iter().map(|(word, text)| (*word, text.as_str()))
    }
}

/// The lookup of an app: its catalog in its language, or English.
#[derive(Clone, Default)]
pub struct Words {
    lookup: Option<Lookup>,
    plural: Option<Plural>,
    sets: SetCache,
    /// Unique per translator: equal words have equal generations (`generation`).
    generation: u64,
}

/// A generation no `Words` built before has; English (the default) is 0.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The word sets built from one `Words`, by type, so that a component asking for its set every frame builds it
/// once. A clone starts empty: it may get a different lookup.
#[derive(Default)]
struct SetCache(Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>);

impl Clone for SetCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Words {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Words").field("translated", &self.lookup.is_some()).finish()
    }
}

impl Words {
    /// English everywhere.
    pub fn english() -> Words {
        Words::default()
    }

    /// Words translated by `lookup`, which returns `None` where it has no text.
    pub fn new(lookup: impl Fn(Word) -> Option<String> + Send + Sync + 'static) -> Words {
        Words { lookup: Some(Arc::new(lookup)), plural: None, sets: SetCache::default(), generation: next_generation() }
    }

    /// Which translator these words are: words with the same generation read every word the same (a clone keeps it),
    /// and words built anew get a new one.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Counts written by `plural(id, n, one, other)`, which returns `None` to fall back to the English forms.
    #[must_use]
    pub fn with_plural(mut self, plural: impl Fn(&'static str, u64, &'static str, &'static str) -> Option<String> + Send + Sync + 'static) -> Words {
        self.plural = Some(Arc::new(plural));
        self.generation = next_generation();
        self
    }

    /// The words of `i18n` in `lang`: [`I18n::tid`] for a word with an id, [`I18n::get`] for one with a context,
    /// [`I18n::t`] for the rest, and [`I18n::trn`] for counts. `None` where the catalog has no row.
    pub fn from_i18n(i18n: Arc<I18n>, lang: Lang) -> Words {
        let words = Arc::clone(&i18n);
        let lookup = move |word: Word| -> Option<String> {
            let text = if !word.id.is_empty() {
                words.tid(lang, word.id, word.english)
            } else if !word.context.is_empty() {
                words.get(lang, word.context, word.english)?
            } else {
                words.t(lang, word.english)
            };
            (text != word.english).then(|| text.to_owned())
        };
        Words::new(lookup).with_plural(move |_, n, one, other| Some(i18n.trn(lang, n, one, other)))
    }

    /// These words with `rows` over them: a row's text wins, every other word reads as before.
    #[must_use]
    pub fn overlay(&self, rows: &WordRows) -> Words {
        let inner = self.clone();
        let rows = rows.clone();
        let mut over = Words::new(move |word| match rows.text(word) {
            Some(text) => Some(text.to_owned()),
            None => inner.lookup.as_ref().and_then(|lookup| lookup(word)),
        });
        over.plural = self.plural.clone();
        over
    }

    /// The text of `word`: never empty. English when no translator is installed (without allocating), when it
    /// has no text, when its text is empty, or when its text lacks a `{placeholder}` of the English (logged once
    /// per word).
    pub fn get(&self, word: Word) -> Cow<'static, str> {
        let Some(lookup) = &self.lookup else {
            return Cow::Borrowed(word.english);
        };
        match lookup(word) {
            Some(text) if text.is_empty() => Cow::Borrowed(word.english),
            Some(text) if !keeps_placeholders(word.english, &text) => {
                warn_once(word, &text);
                Cow::Borrowed(word.english)
            }
            Some(text) => Cow::Owned(text),
            None => Cow::Borrowed(word.english),
        }
    }

    /// The English set `L` translated by these words, built on the first call and shared after it.
    pub fn set<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        let mut cache = self.sets.0.lock().unwrap_or_else(PoisonError::into_inner);
        let shared = cache.entry(TypeId::of::<L>()).or_insert_with(|| Arc::new(L::english().localized_in(self)));
        Arc::clone(shared).downcast::<L>().unwrap_or_else(|_| Arc::new(L::english().localized_in(self)))
    }

    /// `n` things written by `count`: the plural forms of the locale, or the English forms. `digits` is `n` as
    /// the locale writes numbers.
    pub fn counted(&self, count: Counted, n: u64, digits: &str) -> String {
        let plural = self.plural.as_ref().and_then(|plural| plural(count.word.key(), n, count.one, count.word.english));
        if let Some(text) = plural {
            return text;
        }
        let form = match self.get(count.word) {
            Cow::Owned(translated) => translated,
            Cow::Borrowed(english) => (if n == 1 { count.one } else { english }).to_owned(),
        };
        crate::fmt(&form, &[("n", digits)])
    }
}

/// A table of words with its English text: every field is one row. Written by `craft_ui::words!`.
pub trait WordSet: Sized {
    /// The words of the table, in field order.
    const WORDS: &'static [Word];
    /// The count words of the table.
    const COUNTS: &'static [Counted];

    /// Every field in English.
    fn english() -> Self;

    /// Every field replaced by its word's text in `words`.
    #[must_use]
    fn localized_in(self, words: &Words) -> Self;

    /// The text each word of the table shows now.
    fn rows(&self) -> WordRows;
}

/// A template with `{name}` placeholders, filled at use. The only placeholder filler: each value is isolated in
/// right-to-left text, so a Latin file name inside an Arabic sentence keeps its order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WordTemplate(String);

impl WordTemplate {
    /// A template of `text`.
    pub fn new(text: impl Into<String>) -> Self {
        WordTemplate(text.into())
    }

    /// The template text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The text with each `{name}` filled from `args`.
    pub fn fill(&self, args: &[(&str, &str)]) -> String {
        crate::fmt(&self.0, args)
    }
}

/// A count word with the forms its words give.
#[derive(Clone, Debug)]
pub struct Count {
    counted: Counted,
    words: Words,
}

impl Count {
    /// The count `counted` written by `words`.
    pub fn new(counted: Counted, words: &Words) -> Self {
        Count { counted, words: words.clone() }
    }

    /// `n` things; `digits` is `n` as the locale writes numbers.
    pub fn text(&self, n: u64, digits: &str) -> String {
        self.words.counted(self.counted, n, digits)
    }

    /// The registered count word.
    pub fn counted(&self) -> Counted {
        self.counted
    }
}

/// The `{name}` placeholders of `text`.
fn placeholders(text: &str) -> impl Iterator<Item = &str> {
    text.split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}'))
        .map(|(name, _)| name)
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

fn keeps_placeholders(english: &str, translation: &str) -> bool {
    placeholders(english).all(|name| placeholders(translation).any(|found| found == name))
}

fn warn_once(word: Word, translation: &str) {
    static WARNED: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Mutex::default);
    if WARNED.lock().unwrap_or_else(PoisonError::into_inner).insert(word.key()) {
        log::warn!("translation of {:?} ignored: {translation:?} lacks a placeholder of {:?}", word.key(), word.english);
    }
}
