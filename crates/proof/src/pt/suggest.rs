//! Portuguese spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with
//! the words Portuguese runs into their neighbours (`nãoé` → `não é`, `paraela` → `para ela`) and
//! letters to try for a dictionary without a TRY line.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`não é`, `da água`, `para isso`).
const LEADING_WORDS: &[&str] = &[
    "a", "o", "e", "é", "de", "do", "da", "dos", "das", "em", "no", "na", "nos", "nas", "por", "para", "com", "sem", "sob", "sobre", "ao", "aos",
    "à", "às", "que", "se", "não", "como", "mas", "mais", "já", "até", "entre", "contra", "perante", "desde", "após", "ante", "durante", "mediante",
    "segundo", "conforme", "salvo", "fora", "onde", "quando", "bem", "mal", "pois", "logo", "portanto", "porém", "todavia", "contudo", "assim",
    "tão", "tanto", "tudo", "nada", "algo", "cada", "outro", "mesmo", "próprio", "ainda", "tampouco", "quase", "talvez", "nunca", "sempre",
];
/// Short words often run into the one before (`amo-te`, `deu-lhe`; in writing without the
/// hyphen: `amote`, `deulhe`).
const TRAILING_WORDS: &[&str] = &[
    "me", "te", "se", "lhe", "nos", "vos", "lhes", "o", "a", "os", "as", "ele", "ela", "eles", "elas", "isso", "isto", "aquilo", "mim", "ti", "si",
    "eu", "tu", "que", "de", "em", "é",
];

/// What Portuguese adds to its dictionary's tables.
const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "aeiousrntmdlcpvgqbzfhjxçãõáéíóúâêôkw",
    ..Tuning::DEFAULT
};

/// Spelling suggestions for a Portuguese word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_likeliest_fix_comes_first() {
        crate::pt::test_dict::install();
        assert_eq!(suggest("coraçao", 3).first().map(String::as_str), Some("coração"));
        assert_eq!(
            suggest("saude", 3).first().map(String::as_str),
            Some("saúde"),
            "an accent at the first difference beats one further right (saudê is a word too)"
        );
        assert_eq!(suggest("seje", 3).first().map(String::as_str), Some("seja"));
    }
}
