//! The start of every Latin-script dictionary language's spelling check (Czech, Croatian,
//! Indonesian, Italian, Polish, Portuguese, Spanish): the words that need no lookup, under the
//! File › Options › Proofing choices. Each language goes on from there with its own rules.

use crate::Options;
use crate::affix::AffixDict;

/// Is `c` a letter of the Latin script (the only one these dictionaries cover)?
fn latin(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// `word` trimmed of quotes and apostrophes, when `d` must say whether it is right; `None` when it
/// is right without a lookup: there is no dictionary (no spelling marks), the word is a single
/// letter or has no letters, it has digits or is a short word in capitals (an acronym) and the
/// options skip those, or it is in another script.
pub(crate) fn word_to_check<'a>(d: &AffixDict, word: &'a str, opts: &Options) -> Option<&'a str> {
    word_to_check_in(d, word, opts).filter(|w| w.chars().any(latin))
}

/// [`word_to_check`] in any script: the start of a Language pack language's check, whose
/// dictionary may be Cyrillic, Greek or any other script.
pub(crate) fn word_to_check_in<'a>(d: &AffixDict, word: &'a str, opts: &Options) -> Option<&'a str> {
    if d.is_empty() {
        return None;
    }
    // A trailing combining mark with no precomposed form (`x\u{338}`) is part of the word.
    let edge = |c: char| !c.is_alphanumeric() && c != '\'' && c != '’';
    let w = word.trim_start_matches(edge).trim_end_matches(|c| edge(c) && !unicode_normalization::char::is_combining_mark(c));
    let w = w.trim_matches(['\'', '’']);
    let n = w.chars().count();
    if n <= 1 || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit())) || !w.chars().any(char::is_alphabetic) {
        return None;
    }
    if opts.ignore_uppercase && in_capitals(w) && n <= capitals_limit(opts) {
        return None;
    }
    Some(w)
}

/// The most letters of a word in capitals Ignore words in UPPERCASE skips in the languages with a
/// dictionary (`NASA`, not `ORGANISATIONS`), unless [`Options::ignore_uppercase_any_length`] is on. (English
/// has always skipped them whatever their length.)
pub(crate) const ABBREVIATION_LETTERS: usize = 6;

/// The longest word in capitals `opts` skips: [`ABBREVIATION_LETTERS`], or any length.
pub(crate) fn capitals_limit(opts: &Options) -> usize {
    if opts.ignore_uppercase_any_length { usize::MAX } else { ABBREVIATION_LETTERS }
}

/// Is `w` in capitals: a cased letter and no lower-case one? Only a word with a cased letter can
/// be: Hebrew, Arabic, Thai, Chinese… words have none, so they are never taken for acronyms.
pub(crate) fn in_capitals(w: &str) -> bool {
    w.chars().any(char::is_uppercase) && w.chars().all(|c| !c.is_lowercase())
}

/// Is `part` of a hyphenated word an abbreviation Ignore words in UPPERCASE skips: at most `max`
/// characters, [`in_capitals`] or without letters (`SMS` in `SMS-ка`, `19` in `COVID-19`)? A
/// part in a caseless script is no abbreviation.
pub(crate) fn capitals_part(part: &str, max: usize) -> bool {
    part.chars().count() <= max && (in_capitals(part) || !part.chars().any(char::is_alphabetic))
}

/// Does a whitespace-delimited token look like a file address: a network path (`\\server\share`), a
/// drive path (`C:\dir`, `C:/dir`), a `file:` address, or a path from the root or the home folder that
/// has another `/` further on (`/usr/lib`, `~/docs/a`)? Brackets and quotes before it are not part of it.
/// Spelling skips its words with [`Options::ignore_file_addresses`].
pub(crate) fn is_file_address(token: &str) -> bool {
    let t = token.trim_start_matches(['(', '[', '{', '<', '"', '\'', '“', '‘', '«']);
    let mut chars = t.chars();
    let (first, second, third) = (chars.next(), chars.next(), chars.next());
    if t.starts_with("\\\\") {
        return true;
    }
    if first.is_some_and(|c| c.is_ascii_alphabetic()) && second == Some(':') && matches!(third, Some('\\' | '/')) {
        return true;
    }
    if t.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:")) {
        return true;
    }
    t.strip_prefix('/').or_else(|| t.strip_prefix("~/")).is_some_and(|rest| rest.contains('/'))
}

#[cfg(test)]
mod tests;
