//! Gujarati proofing using the installed Hunspell `gu_IN` dictionary (never bundled).
//! Suggestions use its runtime MAP/REP/TRY/KEY tables. Generic grammar checks have Gujarati
//! messages; hyphenation keeps the existing fallback.

pub mod grammar;
pub(crate) mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
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
    if d.is_empty() {
        return true;
    }
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && !is_gujarati_char(c) && !matches!(c, '\'' | '’' | '-'));
    let w = w.trim_matches(['\'', '’', '-']);
    let letter_count = w.chars().filter(|&c| (is_gujarati_char(c) && !c.is_numeric()) || c.is_alphabetic()).count();
    if letter_count <= 1 || (opts.ignore_numbers && w.chars().any(char::is_numeric)) {
        return true;
    }
    // A word in another script (or with no Gujarati characters) is not marked by this dictionary.
    if !w.chars().any(is_gujarati_char) {
        return true;
    }
    if w.contains('-') {
        // The whole word first, as Hunspell does: a hyphenated entry or user word.
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || !p.chars().any(is_gujarati_char) || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
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
