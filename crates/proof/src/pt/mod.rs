//! Portuguese proofing.
//!
//! - **Spelling** against the Hunspell `pt_BR` dictionary installed on the system
//!   ([`dictionary`], found by [`crate::hunspell`]: `pt_BR.aff`/`pt_BR.dic` — VERO and its
//!   derivatives — from the OS, a package manager or LibreOffice; it isn't bundled). Without it
//!   Portuguese text gets no spelling marks. Capitalized stems are names (`Brasília`); a sentence
//!   may start with any word and headings may be in capitals. Hyphenated compounds are checked
//!   part by part (`guarda-roupa`, `ex-marido`), and the user dictionary applies as in English.
//! - **Suggestions** tuned for Portuguese ([`suggest`]): missing accents first (`coraçao` →
//!   `coração`, `saude` → `saúde`), then the dictionary's `REP` confusions (`seje` → `seja`), a
//!   missing space, one-letter typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Portuguese messages.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Portuguese Hunspell dictionary.
const NAMES: &[&str] = &["pt_BR", "pt-BR", "pt"];

/// The Portuguese dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Portuguese");

/// The Portuguese dictionary: the installed Hunspell `pt_BR` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Portuguese spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Portuguese dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Portuguese dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Portuguese? Numbers, single letters, short words in capitals
/// (acronyms: `USP`, `PT`), words with digits and words in other scripts are accepted, and so
/// is every word when no Portuguese dictionary is installed.
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
    // The whole word first, as Hunspell does: a hyphenated entry or user word.
    if w.contains('-') {
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
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
    fn user_dictionary_accepts_a_hyphenated_word_whole() {
        test_dict::install();
        let text = "Ele disse que está na Unigranrio-RJ.";
        assert_eq!(crate::check_spelling_in(text, crate::ProofLang::Pt).len(), 1, "Unigranrio isn't in the dictionary");
        // Added as the context menu adds it, hyphen and all (#2082).
        crate::add_word("Unigranrio-RJ");
        assert!(crate::check_spelling_in(text, crate::ProofLang::Pt).is_empty());
    }
}
