//! Ranked search matching: one query against many texts, accents and locale case folded by a [`Folder`],
//! with the matched ranges of each text for highlighting.

use crate::fold::{Folded, Folder};
use std::ops::Range;

/// How well a text matches, best first: the derived order puts `Whole` before `Prefix` before `WordStart` before
/// `Contains` before `AllWords`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Rank {
    /// The text is the query.
    Whole,
    /// The text starts with the query.
    Prefix,
    /// The query occurs at the start of a word of the text.
    WordStart,
    /// The query occurs inside a word.
    Contains,
    /// Every word of a multi-word query occurs in the text, in any order (right-to-left locales only).
    AllWords,
}

/// A text that matches the query.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Match {
    /// How well it matches.
    pub rank: Rank,
    /// Every occurrence of the query, as byte ranges of the text, in order; for `AllWords`, the first occurrence of
    /// each word, sorted by start.
    pub ranges: Vec<Range<usize>>,
}

/// A query folded once for scoring many texts.
#[derive(Clone, Debug)]
pub struct Matcher {
    folder: Folder,
    query: Folded,
    /// The folded words of a query of two or more words in a right-to-left locale, where a line typed or pasted
    /// across directions often holds its words in another order; empty otherwise, so a left-to-right ranking is
    /// the whole-run one.
    words: Vec<Folded>,
}

impl Matcher {
    /// A matcher for `query` with surrounding space ignored. An empty query matches every text.
    pub fn new(folder: &Folder, query: &str) -> Matcher {
        let query = query.trim();
        let words: Vec<Folded> = if folder.rtl() {
            query.split_whitespace().map(|word| folder.fold(word)).filter(|word| !word.as_str().is_empty()).collect()
        } else {
            Vec::new()
        };
        Matcher { folder: *folder, query: folder.fold(query), words: if words.len() > 1 { words } else { Vec::new() } }
    }

    /// Whether the query is empty.
    pub fn is_empty(&self) -> bool {
        self.query.as_str().is_empty()
    }

    /// How `text` matches, or `None` when it doesn't. An empty query matches every text as `Whole`, with no ranges.
    pub fn score(&self, text: &str) -> Option<Match> {
        if self.is_empty() {
            return Some(Match { rank: Rank::Whole, ranges: Vec::new() });
        }
        let folded = self.folder.fold(text);
        let ranges: Vec<Range<usize>> = folded.matches(&self.query).collect();
        let Some(first) = ranges.first().cloned() else {
            return self.all_words(&folded);
        };
        let rank = if first.start == 0 && first.end == text.len() {
            Rank::Whole
        } else if first.start == 0 {
            Rank::Prefix
        } else if ranges.iter().any(|range| text[..range.start].chars().next_back().is_some_and(|before| !before.is_alphanumeric())) {
            Rank::WordStart
        } else {
            Rank::Contains
        };
        Some(Match { rank, ranges })
    }

    /// `AllWords` when every word of the query occurs in `folded`, with each word's first occurrence as source ranges.
    fn all_words(&self, folded: &Folded) -> Option<Match> {
        if self.words.is_empty() {
            return None;
        }
        let mut ranges: Vec<Range<usize>> = self.words.iter().map(|word| folded.matches(word).next()).collect::<Option<_>>()?;
        ranges.sort_by_key(|range| range.start);
        ranges.dedup();
        Some(Match { rank: Rank::AllWords, ranges })
    }
}
