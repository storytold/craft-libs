//! Italian proofing.
//!
//! - **Spelling** against the Hunspell `it_IT` dictionary installed on the system (found by
//!   [`crate::hunspell`]: `it_IT.aff`/`it_IT.dic` from the OS, a package manager or
//!   LibreOffice; it isn't bundled — the upstream file is GPL-licensed). Without it Italian
//!   text gets no spelling marks. Elided forms check the word after the apostrophe
//!   (`l'amico`, `un'idea`, `c'è`, `dov'è`, `senz'altro`); truncations keep their apostrophe
//!   (`po'`, `mo'`); hyphenated compounds are checked part by part; the user dictionary applies
//!   as in English. Forms a grammar rule explains better (`pò`, `qual'è`) are left to it.
//! - **Suggestions** tuned for Italian ([`suggest`]): missing accents first (`caffe` → `caffè`,
//!   `perche` → `perché`), then the dictionary's `REP` confusions, a missing space, one-letter
//!   typos, and a dictionary scan for anything further off.
//! - **Grammar** ([`grammar`]): cheap, reliable rules with Italian messages, which work with
//!   no dictionary installed.

pub mod grammar;
mod suggest;
#[cfg(test)]
pub(crate) mod test_dict;

use std::sync::Arc;

use crate::affix::AffixDict;
use crate::slot::DictSlot;

pub use suggest::suggest;

/// File names of an Italian Hunspell dictionary (Italy first, then Switzerland).
const NAMES: &[&str] = &["it_IT", "it_CH", "it-IT", "it"];

/// The Italian dictionary, found on first use (see [`DictSlot`]).
pub(crate) static DICT: DictSlot<AffixDict> = DictSlot::new(NAMES, "Italian");

/// The Italian dictionary: the installed Hunspell `it_IT` (read on first use), one given with
/// [`install_dictionary`], or an empty one when none is available (Italian spelling is off
/// then; see [`has_dictionary`]).
pub fn dictionary() -> Arc<AffixDict> {
    DICT.get()
}

/// Use this Italian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    DICT.install(aff, dic)
}

/// Is an Italian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    DICT.has_dictionary()
}

/// Proclitics (and verb clippings) that elide before a vowel or `h`: the word after the
/// apostrophe is what gets checked (`l'amico`, `un'idea`, `c'è`, `m'hai`, `dov'è`, `anch'io`).
const PROCLITICS: &[&str] = &[
    "l", "un", "una", "uno", "all", "dall", "dell", "nell", "sull", "coll", "pell", "c", "m", "t", "s", "v", "d", "j", "n", "ch", "gl", "quest",
    "quell", "bell", "buon", "sant", "grand", "poc", "nessun", "ciascun", "qualcun", "alcun", "dov", "cos", "com", "quand", "anch", "senz", "tutt",
    "nient", "mezz", "sott", "tant", "quant", "dev",
];

/// Truncated words written with an apostrophe (`un po'`, `a mo' di`, `be'`): [`crate::words`]
/// trims the apostrophe, so spelling sees the bare word; [`grammar`] checks the apostrophe.
const TRUNCATIONS: &[&str] = &["po", "mo", "be"];

/// Misspellings a grammar rule reports with a clearer message and fix (`pò` → `po'`, `qual'è`
/// → `qual è`), so spelling leaves them alone rather than marking the word twice.
fn left_to_grammar(lower: &str) -> bool {
    lower == "pò" || grammar::qual_elided(lower).is_some()
}

/// Is `word` spelled correctly in Italian? Numbers, single letters, short words in capitals
/// (acronyms: `RAI`, `TG`), words with digits and words in other scripts are accepted, and so
/// is every word when no Italian dictionary is installed.
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
    let lower = w.to_lowercase();
    if TRUNCATIONS.contains(&lower.as_str()) || left_to_grammar(&lower) {
        return true;
    }
    // The whole word first, as Hunspell does: a hyphenated entry or user word.
    if w.contains('-') {
        return known(d, w) || w.split('-').all(|p| p.chars().count() <= 1 || known_elided(d, p));
    }
    known_elided(d, w)
}

/// [`known`] for a word that may start with an elided proclitic (`l'amico`, `l'ex`): the word after
/// the apostrophe is what gets checked.
fn known_elided(d: &AffixDict, w: &str) -> bool {
    if let Some((head, tail)) = w.split_once(['\'', '’'])
        && PROCLITICS.contains(&head.to_lowercase().as_str())
        && !tail.is_empty()
    {
        return known(d, tail);
    }
    known(d, w)
}

/// Is one word (no hyphens, no apostrophe) in the dictionary or the user dictionary?
fn known(d: &AffixDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// craft-libs#496 review: an elision inside a hyphenated word (`l'ex-amico`) is read per part.
    #[test]
    fn an_elision_in_a_hyphenated_part_is_read() {
        let d = AffixDict::from_hunspell("SET UTF-8\n", "2\nex\namico\n", &[]).unwrap();
        for w in ["l'ex-amico", "dell’ex-amico", "ex-amico"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        assert!(!is_correct_with(&d, "l'exx-amico"));
    }
}
