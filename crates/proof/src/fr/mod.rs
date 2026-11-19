//! French proofing through the shared Language pack cache.
//!
//! A French locale remains [`ProofLang::Pack`]: dictionary selection, installation, reloads,
//! memory limits and hyphenation use the same paths as other packs. This module adds French
//! elisions, accent-oriented suggestions and conservative typography, including Quebec spacing.
//! No dictionary or hyphenation data is bundled here.

pub mod grammar;
mod suggest;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod test_dict;

use crate::affix::AffixDict;
use crate::{Options, ProofLang};

pub use suggest::suggest_in;

/// Words after whose elision a user-dictionary entry is accepted. Ordinary words and their
/// elisions are always decided by the dictionary (`l'arbre`, but not `l'arbres`).
const ELISIONS: &[&str] = &["l", "d", "j", "m", "t", "s", "n", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu", "presqu", "quelqu"];

/// Check French spelling in the installed France/generic pack, with the default options.
/// Regional callers should use [`crate::is_correct_in`] with their full locale.
pub fn is_correct(word: &str) -> bool {
    crate::is_correct_in(word, ProofLang::from_tag(Some("fr-FR")))
}

/// French suggestions from the installed France/generic pack, best first, at most `max`.
/// Regional callers should use [`crate::suggest_in`] with their full locale.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    crate::suggest_in(word, max, ProofLang::from_tag(Some("fr-FR")))
}

/// Check against a supplied dictionary, without changing any installed pack or cache.
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, &crate::composed(word), &Options::default())
}

pub(crate) fn check(d: &AffixDict, word: &str, opts: &Options) -> bool {
    let Some(w) = crate::spelling::word_to_check(d, word, opts) else { return true };
    // A regional dictionary may store the curly apostrophe without ICONV. Honor that entry
    // before the French ASCII-apostrophe fallback.
    if known(d, w) {
        return true;
    }
    let original = w;
    let w = w.replace('’', "'");
    if known(d, &w) {
        return true;
    }
    if let Some((head, tail)) = w.split_once('\'')
        && ELISIONS.contains(&head.to_lowercase().as_str())
        && !tail.is_empty()
        && !tail.contains('\'')
        && crate::in_user_dictionary(&tail.to_lowercase())
    {
        return true;
    }
    original.contains('-') && original.split('-').all(|p| !p.is_empty() && (p.chars().count() <= 1 || known(d, p) || known(d, &p.replace('’', "'"))))
}

fn known(d: &AffixDict, word: &str) -> bool {
    d.check(word) || crate::in_user_dictionary(&word.to_lowercase())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn original_french_words_and_elisions() {
        let d = test_dict::dictionary();
        for word in [
            "maison",
            "maisons",
            "école",
            "écoles",
            "grand",
            "grande",
            "grands",
            "grandes",
            "aimer",
            "aime",
            "aimons",
            "aiment",
            "suis",
            "est",
            "sommes",
            "sont",
            "a",
            "avons",
            "ont",
            "Paris",
            "Montréal",
            "œuvre",
            "sœur",
            "aujourd'hui",
            "aujourd’hui",
            "peut-être",
            "arc-en-ciel",
            "km",
            "Maison",
            "MAISON",
            "SNCF",
            "2026",
            "très",
            "rivière",
            "l'arbre",
            "l’arbre",
            "L'arbre",
            "d'enfants",
            "l'homme",
            "qu'elle",
            "qu’il",
            "j'aime",
            "j’ai",
            "d'avoir",
            "l'école",
            "jusqu'à",
            "maison-école",
            "est-il",
            "a-t-il",
        ] {
            assert!(is_correct_with(&d, word), "{word}");
        }
        for word in [
            "maisonn",
            "ecole",
            "arbrre",
            "beaucoups",
            "aimmer",
            "soeur",
            "Km",
            "grandd",
            "aim",
            "l'hasard",
            "l'chat",
            "l'arbres",
            "l'arbrre",
            "qu'maison",
            "maison-ecole",
            "maison--école",
        ] {
            assert!(!is_correct_with(&d, word), "{word}");
        }
        assert!(is_correct_with(&d, "e\u{301}cole"));
        assert!(!is_correct_with(&d, "maison\u{338}"), "a trailing mark is part of the word");
    }

    #[test]
    fn curly_apostrophes_in_compounds_keep_exact_dictionary_parts() {
        let d = AffixDict::from_hunspell("SET UTF-8\n", "4\naujourd’hui\nécole\ncafé\nl'école\n", &[]).unwrap();
        for word in ["aujourd’hui-école", "aujourd’hui-e\u{301}cole", "café-aujourd’hui", "l’école-aujourd’hui"] {
            assert!(is_correct_with(&d, word), "{word}");
        }
        for word in ["aujourd’hui-écolle", "aujourd’hui--école", "aujourd'hui-école"] {
            assert!(!is_correct_with(&d, word), "{word}");
        }
    }

    #[test]
    fn explicit_curly_apostrophe_without_iconv() {
        let d = AffixDict::from_hunspell("SET UTF-8\n", "1\naujourd’hui\n", &[]).unwrap();
        assert!(is_correct_with(&d, "aujourd’hui"));
        assert!(!is_correct_with(&d, "aujourd’huui"));
    }

    #[test]
    fn user_word_after_an_elision() {
        let d = test_dict::dictionary();
        let word = "zxqmotadaptfr";
        assert!(!is_correct_with(&d, word) && !is_correct_with(&d, &format!("l'{word}")));
        crate::add_word(word);
        assert!(is_correct_with(&d, word) && is_correct_with(&d, &format!("l'{word}")) && is_correct_with(&d, &format!("d’{word}")));
        assert!(!is_correct_with(&d, &format!("chat'{word}")));
        crate::remove_word(word);
    }

    #[test]
    fn options_and_missing_dictionary() {
        let d = test_dict::dictionary();
        let defaults = Options::default();
        let strict = Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
        assert!(check(&d, "SNCF", &defaults) && check(&d, "mp3x", &defaults));
        assert!(!check(&d, "SNCF", &strict) && !check(&d, "mp3x", &strict));
        assert!(check(&d, "MAISON", &strict) && check(&d, "2026", &strict));
        for word in ["école", "l'arbre", "xqzv", "SNCF", ""] {
            assert!(check(&AffixDict::default(), word, &strict));
        }
    }
}
