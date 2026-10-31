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
use unicode_segmentation::UnicodeSegmentation;

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
