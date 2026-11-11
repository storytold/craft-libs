//! The matches of a search across the pages of a document.

use std::ops::Range;

/// One page's matches, as [`FindPattern::find_page`] finds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageMatches {
    /// The char ranges, in order, at most the search's limit.
    pub ranges: Vec<Range<usize>>,
    /// The page has more matches than the limit allowed.
    pub limit_reached: bool,
}

impl From<Vec<Range<usize>>> for PageMatches {
    fn from(ranges: Vec<Range<usize>>) -> Self {
        PageMatches { ranges, limit_reached: false }
    }
}

/// One match: the page (or story) it is on, by the app's index, and its char range there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMatch {
    /// The page (or story) index the app gave.
    pub page: usize,
    /// Char range of the match on that page.
    pub range: Range<usize>,
}

/// The matches of a search across a document, in document order, and the current one.
///
/// Pages report their text as it becomes available (PdfCraft extracts text in the background):
/// [`set_page`](Self::set_page) replaces one page's matches and keeps the current match where it
/// was. Until the user picks a match or moves the view, the current match follows the page the
/// search started from (Acrobat): a page that arrives late may still supply an earlier match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindMatches {
    matches: Vec<FindMatch>,
    current: Option<usize>,
    /// The page the search started from, while the current match is chosen automatically.
    from: Option<usize>,
    /// The user ended automatic following (picked a match, navigated, scrolled).
    follow_ended: bool,
    /// The pages that had more matches than the limit.
    limited: Vec<usize>,
}

impl FindMatches {
    /// No matches; a new search starts from page `from`.
    pub fn new(from: usize) -> Self {
        FindMatches { matches: Vec::new(), current: None, from: Some(from), follow_ended: false, limited: Vec::new() }
    }

    /// Every match found.
    pub fn matches(&self) -> &[FindMatch] {
        &self.matches
    }

    /// The number of matches.
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// Whether there are no matches.
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// The current match's index, for "3 of 12".
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// The current match, if one is chosen.
    pub fn current_match(&self) -> Option<&FindMatch> {
        self.current.and_then(|i| self.matches.get(i))
    }

    /// Replace page `page`'s matches with `found` (ranges sorted by start; a plain `Vec` of ranges
    /// has not reached the limit, [`FindPattern::find_page`] says). The current match stays on
    /// the same page and range when it is still there; while following, the current match becomes
    /// the first at or after the search's start page. Returns whether the current match changed,
    /// so the app scrolls to it (only while following: a match the user already moved away from
    /// is listed, never revealed).
    // pdfcraft: crates/ui-egui/src/canvas/find.rs:120 `refresh_find_page`, :54 `Find::follow_page`.
    pub fn set_page(&mut self, page: usize, found: impl Into<PageMatches>) -> bool {
        let PageMatches { ranges, limit_reached } = found.into();
        let previous = self.current_match().cloned();
        self.limited.retain(|limited| *limited != page);
        if limit_reached {
            self.limited.push(page);
        }
        self.matches.retain(|m| m.page != page);
        self.matches.extend(ranges.into_iter().map(|range| FindMatch { page, range }));
        self.matches.sort_by_key(|m| (m.page, m.range.start));
        let following = !self.follow_ended && self.from.is_some();
        self.current = match self.from.filter(|_| !self.follow_ended) {
            Some(from) => self.first_at_or_after(from),
            // A match the user picked keeps its identity; without one, the list's first is
            // current without the view moving.
            None => previous.as_ref().and_then(|k| self.matches.iter().position(|m| m == k)).or_else(|| self.first_at_or_after(0)),
        };
        following && self.current_match() != previous.as_ref()
    }

    /// Whether a page had more matches than the limit, so the count is a lower bound.
    pub fn limit_reached(&self) -> bool {
        !self.limited.is_empty()
    }

    /// The first match at or after `page`, or the first match when none follows: a search wraps
    /// round the document.
    // pdfcraft: crates/ui-egui/src/canvas/find.rs:45 `Find::first_at_or_after`.
    pub fn first_at_or_after(&self, page: usize) -> Option<usize> {
        self.matches.iter().position(|m| m.page >= page).or_else(|| (!self.matches.is_empty()).then_some(0))
    }

    /// Next (or with `back` the previous) match, wrapping round; the user's choice, so following
    /// ends. `None` without matches.
    pub fn step(&mut self, back: bool) -> Option<&FindMatch> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(match (self.current, back) {
            (Some(c), false) => (c + 1) % n,
            (Some(c), true) => (c + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        });
        self.stop_following();
        self.current_match()
    }

    /// The user moved the view (page box, thumbnails, outline, scrolling): following ends, and a
    /// page that arrives later no longer moves the current match.
    pub fn stop_following(&mut self) {
        self.from = None;
        self.follow_ended = true;
    }

    /// The user picked match `index` (Search panel row); following ends.
    pub fn pick(&mut self, index: usize) {
        if index < self.matches.len() {
            self.current = Some(index);
        }
        self.stop_following();
    }
}
