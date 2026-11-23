//! Ranked search matching: one query against many texts, accents and locale case folded by a [`Folder`],
//! with the matched ranges of each text for highlighting.

use crate::fold::{Folded, Folder};
use std::ops::Range;

/// How well a text matches, best first: the derived order puts `Whole` before `Prefix` before `WordStart` before
/// `Contains`.
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
}

/// A text that matches the query.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Match {
    /// How well it matches.
    pub rank: Rank,
    /// Every occurrence of the query, as byte ranges of the text, in order.
    pub ranges: Vec<Range<usize>>,
}

/// A query folded once for scoring many texts.
#[derive(Clone, Debug)]
pub struct Matcher {
    folder: Folder,
    query: Folded,
}

impl Matcher {
    /// A matcher for `query` with surrounding space ignored. An empty query matches every text.
    pub fn new(folder: &Folder, query: &str) -> Matcher {
        Matcher { folder: *folder, query: folder.fold(query.trim()) }
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
        let ranges: Vec<Range<usize>> = self.folder.fold(text).matches(&self.query).collect();
        let first = ranges.first()?;
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
}
