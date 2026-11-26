//! Polish proofing.
//!
//! - **Spelling** against the SJP.PL Hunspell dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pl_PL.aff`/`pl_PL.dic` from the OS, a
//!   package manager or LibreOffice; it isn't bundled). Without it Polish text gets no spelling
//!   marks. Capitalized stems are names (`Warszawa`); a sentence may start with any word and
//!   headings may be in capitals. Hyphenated compounds are checked part by part
//!   (`biało-czerwony`), foreign names inflected with an apostrophe are accepted
//!   (`Kennedy'ego`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Polish ([`suggest()`]): missing diacritics first (`zolw` → `żółw`), then
//!   the common spelling confusions (`ż`/`rz`, `u`/`ó`, `h`/`ch`…), a missing space (`napewno` →
//!   `na pewno`), one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Polish messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Polish Hunspell dictionary.
const NAMES: &[&str] = &["pl_PL", "pl-PL", "pl"];

/// The Polish dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Polish");

/// The Polish dictionary: the installed Hunspell `pl_PL` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Polish spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Polish dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Polish dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Endings Polish adds after an apostrophe to foreign names whose last letter isn't pronounced
/// (`Kennedy'ego`, `Wilde'a`, `Shakespeare'owi`).
const APOSTROPHE_ENDINGS: &[&str] = &["a", "ach", "ami", "e", "ego", "em", "emu", "i", "m", "om", "owi", "owie", "u", "y", "ów", "ą", "ę"];

/// Is `word` spelled correctly in Polish? Numbers, single letters, short words in capitals
/// (acronyms: `PKO`, `ZUS`), words with digits and words in other scripts are accepted, and so
/// is every word when no Polish dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] with the File › Options › Proofing choices: with Ignore words in UPPERCASE off,
/// short words in capitals are checked against the dictionary too, and with Ignore words that
/// contain numbers off, so are words with digits.
pub fn is_correct_with_options(word: &str, opts: &crate::Options) -> bool {
    check(&dictionary(), word, opts)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &AffixDict, word: &str) -> bool {
    check(d, word, &crate::Options::default())
}

/// [`is_correct_with_options`] against a given dictionary.
fn check(d: &AffixDict, word: &str, opts: &crate::Options) -> bool {
    let Some(w) = crate::spelling::word_to_check(d, word, opts) else {
        return true;
    };
    let w = w.replace('’', "'");
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, &w)
}

/// Is one word (no hyphens) in the dictionary, the user dictionary, or a foreign name with a
/// Polish ending after an apostrophe?
fn known(d: &AffixDict, w: &str) -> bool {
    if d.check(w) || crate::in_user_dictionary(&w.to_lowercase()) {
        return true;
    }
    if let Some((stem, ending)) = w.split_once('\'') {
        return stem.chars().next().is_some_and(char::is_uppercase) && stem.chars().count() >= 2 && APOSTROPHE_ENDINGS.contains(&ending);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polish_words_are_correct() {
        test_dict::install();
        for w in [
            "źdźbło",
            "gżegżółka",
            "chrząszcz",
            "pięćdziesięciu",
            "rzeka",
            "którzy",
            "żółw",
            "Zażółć",
            "gęślą",
            "jaźń",
            "konstytucja",
            "konstytucyjnego",
            "niebieskiego",
            "nieładnie",
            "przyszliśmy",
            "zrobilibyśmy",
            "Warszawa",
            "Warszawie",
            "WARSZAWA",
            "Kot",
            "biało-czerwony",
            "polsko-niemieckiej",
            "Kennedy'ego",
            "Shakespeare’a",
            "PKO",
            "2026",
            "e-mail",
            "w",
            "się",
        ] {
            assert!(is_correct(w), "{w}");
        }
    }
}
