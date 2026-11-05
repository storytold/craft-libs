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
    if opts.ignore_uppercase && in_capitals(w) && n <= 6 {
        return None;
    }
    Some(w)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// craft-libs#485's cased-letter rule, shared by every language: a word or part is in
    /// capitals only with a cased letter; a part without letters counts as an abbreviation.
    #[test]
    fn capitals_need_a_cased_letter() {
        for w in ["PKO", "SMS", "ÉTÉ", "ВКП", "B2B"] {
            assert!(in_capitals(w) && capitals_part(w, 6), "{w}");
        }
        for w in ["日本", "שלום", "ไทย", "Pko", "pko", "123", ""] {
            assert!(!in_capitals(w), "{w}");
        }
        assert!(capitals_part("19", 6) && capitals_part("", 6));
        assert!(!capitals_part("日本", 6) && !capitals_part("ไทย", 6) && !capitals_part("ABCDEFG", 6));
    }

    #[test]
    fn words_needing_no_lookup() {
        let d = AffixDict::from_hunspell("", "1\nkot\n", &[]).unwrap();
        let defaults = Options::default();
        assert_eq!(word_to_check(&AffixDict::default(), "kot", &defaults), None, "no dictionary");
        for w in ["", "a", "2026", "PKO", "B2B", "日本語", "--"] {
            assert_eq!(word_to_check(&d, w, &defaults), None, "{w}");
        }
        assert_eq!(word_to_check(&d, "„kot”", &defaults), Some("kot"));
        assert_eq!(word_to_check(&d, "'kot'", &defaults), Some("kot"));
        let strict = Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
        assert_eq!(word_to_check(&d, "PKO", &strict), Some("PKO"));
        assert_eq!(word_to_check(&d, "B2B", &strict), Some("B2B"));
        assert_eq!(word_to_check(&d, "PRZEDSIĘBIORSTWO", &defaults), Some("PRZEDSIĘBIORSTWO"), "only short capitals are acronyms");
    }

    /// craft-libs#487 review: a trailing combining mark stays on the word it marks.
    #[test]
    fn trailing_combining_marks_are_kept() {
        let d = AffixDict::from_hunspell("", "1\nkot\n", &[]).unwrap();
        let defaults = Options::default();
        assert_eq!(word_to_check(&d, "kox\u{338}", &defaults), Some("kox\u{338}"));
        assert_eq!(word_to_check(&d, "(kox\u{338}).", &defaults), Some("kox\u{338}"));
    }
}
