//! Indonesian suggestions: the shared ranking ([`crate::affix_suggest`]: case, MAP, REP, missing
//! spaces, KEY, TRY edits and a stem scan); every table comes from the installed `id_ID`
//! dictionary, so nothing Indonesian is hard-coded here.

use crate::affix::AffixDict;

/// Spelling suggestions for an Indonesian word, best first, at most `max`, in the word's case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &crate::affix_suggest::Tuning::DEFAULT)
}
