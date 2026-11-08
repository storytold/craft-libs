//! Line-break opportunities for text layout: where a composer may end a line, as byte offsets into
//! the paragraph. Font-independent; measuring and choosing among the breaks stays in the composer.
//!
//! [`line_breaks`] starts from the Unicode line-breaking algorithm (UAX #14, through
//! [`unicode_linebreak`]) and adjusts it:
//! - **Kinsoku** ([`Kinsoku`]): the Japanese and Chinese line-start and line-end rules (JLREQ,
//!   CLREQ). Closing brackets, commas, full stops, small kana and the like don't start a line;
//!   opening brackets don't end one. The rules apply only next to East Asian characters, so text
//!   without them gets UAX #14's breaks unchanged.
//! - **Korean words** ([`Options::korean_words`]): Korean set word by word, breaking at spaces.
//! - **Thai, Lao, Khmer and Burmese**: UAX #14 leaves these scripts (class SA) to a dictionary and
//!   never breaks inside a run of them. With the language feature (`thai`, `lao`, `khmer`,
//!   `burmese`) the run is split into words by ICU4X's segmenter: its dictionaries by default, its
//!   much smaller LSTM models with the `lstm` feature.
//!
//! [`Options::for_language`] picks the options for a paragraph's language. When no opportunity
//! leaves a line short enough, [`emergency_breaks`] gives the places to break a word anyway:
//! between grapheme clusters, never inside one.
//!
//! Navigation across extended grapheme clusters (UAX #29) for cursor movement, selection, and
//! atomic cluster mutations is provided by [`prev_grapheme_offset`], [`next_grapheme_offset`],
//! and [`grapheme_bounds`].
//!
//! Composers that build their own opportunity lists can filter them with [`kinsoku_allows`].

#[cfg(any(feature = "thai", feature = "lao", feature = "khmer", feature = "burmese"))]
mod complex;
mod kinsoku;
#[cfg(test)]
mod tests;

use std::ops::Range;

use unicode_linebreak::{BreakClass, BreakOpportunity, break_property};

pub use kinsoku::kinsoku_allows;
use kinsoku::{is_east_asian, soft_line_start};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Which kinsoku (line-start and line-end) rules apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kinsoku {
    /// No kinsoku: a line may also break between any two East Asian characters, punctuation
    /// included.
    None,
    /// Closing brackets, commas, full stops, iteration marks, the prolonged sound mark, small kana
    /// and the like don't start a line; opening brackets don't end one.
    #[default]
    Hard,
    /// As `Hard`, except that 々, the prolonged sound mark ー and small kana may start a line
    /// (JLREQ's relaxed rules, Appendix C.3).
    Soft,
}

/// How [`line_breaks`] adjusts UAX #14.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Options {
    pub kinsoku: Kinsoku,
    /// Set Korean word by word (KLREQ): no break between two Hangul letters or Han ideographs
    /// (hanja), only at spaces and punctuation. For Korean-language text only, since Han
    /// ideographs count as Korean letters. Off, Korean breaks between syllables as UAX #14 does.
    pub korean_words: bool,
}

impl Options {
    /// The options for text in `language`, a BCP 47 tag such as `ja` or `ko-KR` (`None`: unknown).
    /// Korean is set word by word; every other language, and an unknown one, gets the defaults.
    /// Thai, Lao, Khmer and Burmese words are found by script, whatever the language.
    pub fn for_language(language: Option<&str>) -> Self {
        let primary = language.and_then(|tag| tag.split(['-', '_']).next()).unwrap_or_default();
        Self { korean_words: primary.eq_ignore_ascii_case("ko"), ..Self::default() }
    }
}

/// A place where a line may end: the next line starts at byte `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Break {
    pub offset: usize,
    /// The line must end here (after a newline or another hard break, and at the end of the text).
    pub mandatory: bool,
}

/// Line-break opportunities in `text`, in order. The last one is the end of the text (mandatory);
/// empty text has none.
pub fn line_breaks(text: &str, options: &Options) -> Vec<Break> {
    #[cfg(any(feature = "thai", feature = "lao", feature = "khmer", feature = "burmese"))]
    let words = complex::word_breaks(text, complex::MODEL);
    #[cfg(not(any(feature = "thai", feature = "lao", feature = "khmer", feature = "burmese")))]
    let words = Vec::new();
    breaks_with_words(text, options, words)
}

/// [`line_breaks`], given the word breaks inside Thai, Lao, Khmer and Burmese runs (sorted).
fn breaks_with_words(text: &str, options: &Options, words: Vec<usize>) -> Vec<Break> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut uax = unicode_linebreak::linebreaks(text).peekable();
    let mut words = words.into_iter().peekable();
    let mut out = Vec::new();
    // The last character before the boundary that isn't a combining mark: kinsoku looks at the
    // character a mark belongs to.
    let mut base: Option<char> = None;
    let mut after_mark = false;
    for (i, c) in text.char_indices() {
        if i > 0 {
            // Both lists are sorted by offset: take the entry for `i` from each, if it has one.
            let uax_here = uax.next_if(|&(o, _)| o == i).map(|(_, op)| op);
            let word = words.next_if(|&o| o == i).is_some();
            let allowed = uax_here.is_some() || word;
            let brk = if uax_here == Some(BreakOpportunity::Mandatory) {
                Some(true)
            } else {
                adjust(allowed, base, after_mark, c, options).then_some(false)
            };
            if let Some(mandatory) = brk {
                out.push(Break { offset: i, mandatory });
            }
        }
        after_mark = is_mark(c);
        if !after_mark {
            base = Some(c);
        }
    }
    out.push(Break { offset: text.len(), mandatory: true });
    out
}

/// Emergency breaks for `text[range]`, a piece between two [`line_breaks`] that is too long for a
/// line: byte offsets into `text`, strictly inside `range` and in order, between extended grapheme
/// clusters (UAX #29). An emoji ZWJ sequence, a flag or a letter with its marks stays whole. Empty
/// if `range` isn't on char boundaries of `text`.
pub fn emergency_breaks(text: &str, range: Range<usize>) -> Vec<usize> {
    let Some(piece) = text.get(range.clone()) else { return Vec::new() };
    piece.grapheme_indices(true).skip(1).map(|(o, _)| range.start + o).collect()
}

/// Returns the start byte offset of the extended grapheme cluster preceding `cursor`.
///
/// If `cursor` is at an extended grapheme cluster boundary, this returns the start
/// of the preceding cluster (or `0` if `cursor == 0`). If `cursor` falls strictly
/// inside a cluster (for example, between a base vowel and a combining mark, or between
/// components of an emoji sequence), it snaps back to the start of that enclosing cluster.
///
/// # Architecture Contract (Tier 2: Text Core)
/// - **WHY:** Cursor navigation (`Arrow Left`, `Backspace`) in document editors must operate on
///   Extended Grapheme Cluster boundaries (UAX #29). Naive byte or `char` decrements peel
///   combining marks off decomposed vowels (e.g. stripping `\u{0302}` from `e\u{0323}\u{0302}`),
///   corrupting NFD sequences and leaving invalid orphan marks.
/// - **DANGER:** Unaligned cursor offsets (e.g. from mouse clicks inside multi-byte codepoints
///   or between marks) can panic downstream string slicing or break document invariants.
/// - **CONTRACT:**
///   1. `cursor <= text.len() && text.is_char_boundary(cursor)` is expected; out-of-bounds inputs
///      are clamped defensively to `text.len()`.
///   2. Always returns a valid UTF-8 char boundary in range `0..=cursor`.
///   3. Zero panics; zero allocations.
pub fn prev_grapheme_offset(text: &str, cursor: usize) -> usize {
    let mut c = cursor.min(text.len());
    while !text.is_char_boundary(c) {
        c = c.saturating_sub(1);
    }
    if c == 0 || text.is_empty() {
        return 0;
    }
    let mut gc = GraphemeCursor::new(c, text.len(), true);
    match gc.prev_boundary(text, 0) {
        Ok(Some(boundary)) => boundary,
        _ => 0,
    }
}

/// Returns the end byte offset of the extended grapheme cluster following `cursor`.
///
/// If `cursor` is at an extended grapheme cluster boundary, this returns the end
/// of the subsequent cluster (or `text.len()` if `cursor >= text.len()`). If `cursor`
/// falls strictly inside a cluster (for example, between a base vowel and a combining
/// mark, or between components of an emoji sequence), it snaps forward to the end of
/// that enclosing cluster.
///
/// # Architecture Contract (Tier 2: Text Core)
/// - **WHY:** Forward cursor navigation (`Arrow Right`, `Delete`) must treat extended grapheme
///   clusters as indivisible atomic units (UAX #29), ensuring isomorphic mutation behavior
///   between precomposed (NFC) and decomposed (NFD) text.
/// - **DANGER:** Stepping forward by single codepoints splits emoji ZWJ chains, flag pairs, or
///   base letters from their diacritics.
/// - **CONTRACT:**
///   1. `cursor <= text.len() && text.is_char_boundary(cursor)` is expected; out-of-bounds inputs
///      return `text.len()`.
///   2. Always returns a valid UTF-8 char boundary in range `cursor.min(text.len())..=text.len()`.
///   3. Zero panics; zero allocations.
pub fn next_grapheme_offset(text: &str, cursor: usize) -> usize {
    let mut c = cursor.min(text.len());
    while !text.is_char_boundary(c) {
        c = c.saturating_sub(1);
    }
    if c >= text.len() {
        return text.len();
    }
    let mut gc = GraphemeCursor::new(c, text.len(), true);
    match gc.next_boundary(text, 0) {
        Ok(Some(boundary)) => boundary,
        _ => text.len(),
    }
}

/// Returns the `(start, end)` byte offsets of the extended grapheme cluster enclosing `cursor`.
///
/// For any `cursor` within `0..text.len()`, returns the boundaries `(start, end)` of the
/// cluster containing `cursor` (where `start <= cursor < end`). If `cursor >= text.len()`,
/// returns the boundaries of the last cluster in `text` (or `(0, 0)` if `text` is empty).
///
/// # Architecture Contract (Tier 2: Text Core)
/// - **WHY:** High-level editor actions (such as double-click word/character selection and
///   block cursor rendering) need the full enclosing grapheme cluster boundaries. Calling
///   `prev_grapheme_offset` at cluster start moves to the *preceding* cluster, requiring
///   downstream consumers to implement fragile off-by-one compensation without this primitive.
/// - **DANGER:** In multi-byte or decomposed clusters (e.g. Vietnamese NFD or emoji ZWJ),
///   imprecise cursor placement within combining marks must never return partial slices.
/// - **CONTRACT:**
///   1. Returns `(0, 0)` for empty text.
///   2. Always returns valid UTF-8 char boundaries with `0 <= start <= cursor.min(text.len()) <= end <= text.len()`.
///   3. Zero panics; zero allocations.
pub fn grapheme_bounds(text: &str, cursor: usize) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }
    let mut c = cursor.min(text.len());
    while !text.is_char_boundary(c) {
        c = c.saturating_sub(1);
    }
    if c >= text.len() {
        return (prev_grapheme_offset(text, text.len()), text.len());
    }
    let mut gc = GraphemeCursor::new(c, text.len(), true);
    match gc.is_boundary(text, 0) {
        Ok(true) => {
            let end = match gc.next_boundary(text, 0) {
                Ok(Some(end)) => end,
                _ => text.len(),
            };
            (c, end)
        }
        _ => {
            let start = match gc.prev_boundary(text, 0) {
                Ok(Some(start)) => start,
                _ => 0,
            };
            let mut gc_end = GraphemeCursor::new(start, text.len(), true);
            let end = match gc_end.next_boundary(text, 0) {
                Ok(Some(end)) => end,
                _ => text.len(),
            };
            (start, end)
        }
    }
}

/// Whether a line may break between `before` and `after`, given whether UAX #14 (or the word
/// segmenter) allows it. `after_mark`: the character right before the boundary is a combining
/// mark, joiner or variation selector that belongs to `before`.
fn adjust(allowed: bool, before: Option<char>, after_mark: bool, after: char, options: &Options) -> bool {
    let Some(before) = before else { return allowed };
    // The East Asian breaks UAX #14 lacks go only between two base characters: never inside a
    // cluster, so not after a joiner or before a mark.
    let east_asian_pair = !after_mark && is_east_asian(before) && is_east_asian(after) && !is_mark(after);
    let allowed = match options.kinsoku {
        Kinsoku::None => allowed || east_asian_pair,
        // UAX #14 keeps small kana, ー and 々 (classes CJ and NS) off the line start.
        Kinsoku::Soft => (allowed || (east_asian_pair && soft_line_start(after))) && kinsoku_allows(before, Some(after), Kinsoku::Soft),
        Kinsoku::Hard => allowed && kinsoku_allows(before, Some(after), Kinsoku::Hard),
    };
    allowed && !(options.korean_words && is_korean_letter(before) && is_korean_letter(after))
}

/// A combining mark, variation selector or joiner (UAX #14 classes CM and ZWJ): it belongs to the
/// character before it.
fn is_mark(c: char) -> bool {
    matches!(break_property(c as u32), BreakClass::CombiningMark | BreakClass::ZeroWidthJoiner)
}

/// Hangul in every form, and Han ideographs (read as hanja).
fn is_korean_letter(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x3131..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xFFA0..=0xFFDC
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}
