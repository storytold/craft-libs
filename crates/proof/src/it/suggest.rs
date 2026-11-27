//! Italian spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with the
//! words Italian runs into their neighbours (`perme` → `per me`, `amilano` → `a milano`) and
//! letters to try for a dictionary without a TRY line.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`per me`, `a Milano`, `di Roma`).
const LEADING_WORDS: &[&str] = &[
    "a", "e", "è", "di", "da", "in", "con", "su", "per", "tra", "fra", "che", "chi", "cui", "non", "si", "ci", "vi", "ne", "più", "meno", "molto",
    "tanto", "come", "dove", "quando", "mai", "qui", "qua", "lì", "là", "ogni", "altro", "tutto", "questo", "quello", "bene", "male", "anche",
    "ancora", "sempre", "già", "poi", "ed", "od", "ad", "un", "uno", "una", "il", "lo", "la", "i", "gli", "le", "del", "al", "dal", "nel", "sul",
    "col", "pel", "allo", "dello", "nello", "sullo", "della", "alla", "dalla", "nella", "sulla", "degli", "agli", "dagli", "negli", "sugli", "delle",
    "alle", "dalle", "nelle", "sulle", "dei", "ai", "dai", "nei", "sui", "coi", "dei",
];
/// Short words often run into the one before (enclitic pronouns: `dimmelo` in writing without
/// the doubling, or a dropped space: `perme` → `per me`).
const TRAILING_WORDS: &[&str] = &[
    "mi", "ti", "si", "ci", "vi", "ne", "lo", "la", "li", "le", "gli", "me", "te", "lui", "lei", "noi", "voi", "loro", "che", "cosa", "come", "dove",
    "è",
];

/// What Italian adds to its dictionary's tables.
const TUNING: Tuning =
    Tuning {
        split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS }, alphabet: "aeiortnslcdupmghfbqvàèéìòóùz", ..Tuning::DEFAULT
    };

/// Spelling suggestions for an Italian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}
