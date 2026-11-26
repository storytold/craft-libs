//! Czech proofing using the installed Hunspell `cs_CZ` dictionary (never bundled).
//! Suggestions use its runtime MAP/REP/TRY/KEY tables. Generic grammar checks have Czech
//! messages; hyphenation keeps the existing fallback, without Czech pattern data.

pub mod grammar;
pub(crate) mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of a Czech Hunspell dictionary.
const NAMES: &[&str] = &["cs_CZ", "cs-CZ", "cs"];

/// The Czech dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Czech");

/// The Czech dictionary: the installed Hunspell `cs_CZ` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Czech spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Czech dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is a Czech dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Is `word` spelled correctly in Czech? Numbers, single letters, short words in capitals
/// (acronyms: `ČR`, `EU`), words with digits and words in other scripts are accepted, and so
/// is every word when no Czech dictionary is installed.
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
    if w.contains('-') {
        // The whole word first, as Hunspell does: a hyphenated entry or user word.
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || known(d, p));
    }
    known(d, w)
}

/// Is one word (no hyphens) in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}
