//! Gujarati proofing using the installed Hunspell `gu_IN` dictionary (never bundled).
//! Suggestions use its runtime MAP/REP/TRY/KEY tables. Generic grammar checks have Gujarati
//! messages. Hyphenation is by akshara ([`crate::indic`]).

pub mod grammar;
pub(crate) mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::script_checks::Script;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Gujarati Hunspell dictionary.
const NAMES: &[&str] = &["gu_IN", "gu-IN", "gu"];

/// The Gujarati dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Gujarati");

/// The Gujarati dictionary: the installed Hunspell `gu_IN` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Gujarati spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Gujarati dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Gujarati dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Returns `true` when `c` is in the Gujarati Unicode block (U+0A80..=U+0AFF).
pub fn is_gujarati_char(c: char) -> bool {
    ('\u{0A80}'..='\u{0AFF}').contains(&c)
}

/// Is `word` spelled correctly in Gujarati? Numbers, single letters, short words with digits and words in other scripts are accepted, and so
/// is every word when no Gujarati dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words that contain numbers off, words with digits are checked.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    crate::script_checks::is_correct(&SCRIPT, d, word, opts)
}

/// Gujarati for the shared checks: the whole block is kept in words, its letters are everything
/// in it but the digits.
pub(crate) const SCRIPT: Script = Script { in_words: is_gujarati_char, is_letter, is_digit: is_gujarati_digit, variant: None };

fn is_letter(c: char) -> bool {
    is_gujarati_char(c) && !c.is_numeric()
}

/// Is `c` a Gujarati digit (U+0AE6..=U+0AEF)?
fn is_gujarati_digit(c: char) -> bool {
    ('\u{0AE6}'..='\u{0AEF}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_lookup_accepts_gujarati_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-gu-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["gu_IN", "gu-IN", "gu"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "ઘરો"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn gujarati_words_affixes_and_case_markers() {
        test_dict::install();
        let d = dictionary();
        assert!(has_dictionary());
        assert_eq!(d.len(), 15);
        for w in [
            "ઘર",
            "ઘરો",
            "ઘરોને",
            "ઘરોમાં",
            "પુસ્તક",
            "પુસ્તકો",
            "પુસ્તકોમાં",
            "પાણી",
            "શાળા",
            "ભાષા",
            "પરીક્ષણ",
            "પ્રેમ",
            "શહેર",
            "ગામ",
            "મિત્ર",
            "અઘર",
            "શાળા-શહેર",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["ઘરર", "પુસ્તકક", "પાણીઈ", "શાળળા", "zqપ્રતિબંધિત"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// A zero-width joiner or non-joiner in a conjunct (`પુસ્` U+200D `તક`) keeps the word whole,
    /// and the word is found under the dictionary's spelling, which has no joiner.
    #[test]
    fn words_with_joiners_are_one_word_and_spelled() {
        test_dict::install();
        for word in ["પુસ્\u{200D}તક", "પુસ્\u{200C}તકોમાં"] {
            let text = format!("ઘર {word} ગામ");
            let at = text.find(word).unwrap();
            assert!(crate::words(&text).contains(&(at, at + word.len())), "{word:?}");
            assert!(is_correct(word), "{word:?}");
            let issues = crate::check_text(&text, &[(0..text.len(), crate::ProofLang::Gu)]);
            assert!(issues.is_empty(), "{word:?}: {issues:?}");
        }
        // Still a misspelling with a joiner, and a joiner at a word's edge is not in the word.
        assert!(!is_correct("પુસ્\u{200D}તકક"));
        let text = "ઘર\u{200D} ગામ";
        assert!(crate::words(text).contains(&(0, "ઘર".len())));
        // A word with a joiner written twice is a repeated word.
        let text = "પુસ્\u{200D}તક પુસ્\u{200D}તક";
        let issues = crate::check_grammar_in(text, crate::ProofLang::Gu);
        assert!(issues.iter().any(|i| i.start == 0 && i.end == text.len()), "{issues:?}");
        // A word Gujarati repeats on purpose (`જલ્દી જલ્દી`) still is one when typed with a joiner.
        for text in ["જલ્\u{200D}દી જલ્\u{200D}દી", "જલ્\u{200C}દી જલ્\u{200C}દી"] {
            let issues = crate::check_grammar_in(text, crate::ProofLang::Gu);
            assert!(issues.is_empty(), "{text:?}: {issues:?}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected() {
        test_dict::install();
        assert!(!is_correct("zqપ્રતિબંધિત"));
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "શાળા"));
        assert!(install_dictionary("FLAG bogus\n", "1\nઘર\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "2026", "૧૨", "૨૦૨૬", "અ", "English", "日本語"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        let no_ignore = crate::Options { ignore_numbers: false, ..Default::default() };
        assert!(check(&d, "૧૨", &no_ignore));
        assert!(check(&d, "2026", &no_ignore));
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("નવોશબ્દગુજરાતી"));
        crate::add_word("નવોશબ્દગુજરાતી");
        assert!(is_correct("નવોશબ્દગુજરાતી"));
    }

    #[test]
    fn gujarati_dispatch_messages_options_and_mixed_spans() {
        crate::hold_dictionary_tests();
        test_dict::install();
        assert!(crate::has_dictionary(Some("gu-IN")));
        assert!(crate::is_correct_for("ઘરો", Some("guj")));
        let text = "ઘર પુસ્તકક";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Gu)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "પુસ્તકક");
        assert_eq!(issues[0].message, "શક્ય જોડણી ભૂલ");
        assert_eq!(issues[0].lang, crate::ProofLang::Gu);
        let text = "ઘર this is is wrong";
        let issues = crate::check_text(text, &[(0..3, crate::ProofLang::Gu), (4..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
    }
}
