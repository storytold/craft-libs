//! Hindi proofing using the installed Hunspell `hi_IN` dictionary (never bundled).
//! Suggestions use its runtime MAP/REP/TRY/KEY tables. Spelling and the generic checks are the
//! ones Gujarati shares, with Devanagari's letters and digits and Hindi messages. Hyphenation is
//! by akshara ([`crate::indic`]).

pub mod grammar;
pub(crate) mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Hindi Hunspell dictionary.
const NAMES: &[&str] = &["hi_IN", "hi-IN", "hi"];

/// The Hindi dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Hindi");

/// The Hindi dictionary: the installed Hunspell `hi_IN` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Hindi spelling is off then;
/// see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Hindi dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8),
/// for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Hindi dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Hindi? Numbers (ASCII or Devanagari digits), single letters,
/// short words with digits and words in other scripts are accepted, and so is every word when no
/// Hindi dictionary is installed. A nukta letter matches the dictionary however either spells it
/// (`ज़`, U+095B, or `ज` and the nukta).
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words that contain
/// numbers off, words with digits are checked.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    crate::script_checks::is_correct(&crate::devanagari::SCRIPT, d, word, opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_lookup_accepts_hindi_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hi-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["hi_IN", "hi-IN", "hi"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = crate::hunspell::load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "किताबें"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn hindi_words_affixes_and_misspellings() {
        test_dict::install();
        assert!(has_dictionary());
        for w in [
            "घर",
            "घरों",
            "किताब",
            "किताबें",
            "किताबों",
            "कमरा",
            "कमरे",
            "कमरों",
            "बच्चे",
            "बच्चों",
            "गाँव",
            "गाँवों",
            "सफल",
            "असफल",
            "परीक्षण",
            "घर-शहर",
            "“किताब”",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["घरर", "किताबब", "पानीई", "कमरो", "zqप्रतिबंधित"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = AffixDict::default();
        assert!(is_correct_with(&empty, "किताबब"));
        assert!(install_dictionary("FLAG bogus\n", "1\nघर\n").is_err());
        let d = AffixDict::from_hunspell(test_dict::AFF, test_dict::DIC, &[]).unwrap();
        for w in ["", "2026", "१२", "२०२६", "क", "।", "English", "日本語", "ગુજરાતી"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        let no_ignore = crate::Options { ignore_numbers: false, ..Default::default() };
        assert!(check(&d, "१२", &no_ignore));
        assert!(check(&d, "2026", &no_ignore));
        assert!(!check(&d, "घरर१२", &no_ignore));
        assert!(check(&d, "घरर१२", &crate::Options::default()));
    }

    /// Composing a word (as every check does) splits a precomposed nukta letter into consonant
    /// and nukta, so a dictionary that lists either spelling still knows the word.
    #[test]
    fn nukta_letters_match_either_spelling() {
        crate::hold_dictionary_tests();
        // ज़मीन listed precomposed (U+095B), सड़क as ड and the nukta.
        install_dictionary("SET UTF-8\n", "2\n\u{095B}मीन\nसड\u{093C}क\n").unwrap();
        for w in ["\u{095B}मीन", "ज\u{093C}मीन", "स\u{095C}क", "सड\u{093C}क"] {
            assert!(crate::is_correct_in(w, crate::ProofLang::Hi), "{w:?}");
        }
        assert!(!crate::is_correct_in("ज\u{093C}मीनन", crate::ProofLang::Hi));
        test_dict::install();
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("नयाशब्दहिंदी"));
        crate::add_word("नयाशब्दहिंदी");
        assert!(is_correct("नयाशब्दहिंदी"));
    }

    #[test]
    fn hindi_dispatch_messages_options_and_mixed_spans() {
        crate::hold_dictionary_tests();
        test_dict::install();
        assert!(crate::has_dictionary(Some("hi-IN")));
        assert!(crate::is_correct_for("किताबें", Some("hin")));
        // The danda ends the sentence and is no part of the word before it.
        let text = "बच्चे किताबब घर।";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Hi)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "किताबब");
        assert_eq!(issues[0].message, "संभावित वर्तनी त्रुटि");
        assert_eq!(issues[0].lang, crate::ProofLang::Hi);
        let text = "घर this is is wrong";
        let issues = crate::check_text(text, &[(0..6, crate::ProofLang::Hi), (7..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
    }
}
