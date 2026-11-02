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
    // Only a word with a cased letter can be in capitals: Hebrew, Arabic, Thai… words have none.
    if opts.ignore_uppercase && w.chars().any(char::is_uppercase) && w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return None;
    }
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

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
