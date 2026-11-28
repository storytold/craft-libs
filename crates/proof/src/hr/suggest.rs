//! Croatian spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with
//! the words Croatian runs into their neighbours (`neznam` → `ne znam`), the letter groups of
//! Croatian diacritics and letters to try, for a dictionary without MAP or TRY lines.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`ne znam`, `na primjer`, `da li`).
const LEADING_WORDS: &[&str] = &[
    "a", "ali", "i", "ili", "da", "do", "od", "na", "ne", "ni", "o", "po", "pod", "pri", "sa", "s", "se", "su", "u", "uz", "za", "iz", "je", "će",
    "bi", "bih", "što", "koji", "koja", "koje", "kao", "kad", "dok", "bez", "pred", "nad",
];
const TRAILING_WORDS: &[&str] =
    &["je", "su", "se", "ga", "mi", "ti", "mu", "joj", "ih", "im", "li", "će", "ću", "bi", "bih", "i", "a", "u", "na", "za", "od", "do"];

/// What Croatian adds to its dictionary's tables.
const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "aeiourntsjkvlmdpbgczčćšžđfh",
    map: &["cčć", "dđ", "sš", "zž"],
    ..Tuning::DEFAULT
};

/// Spelling suggestions for a Croatian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}
