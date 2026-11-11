//! Polish spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with the
//! words Polish runs into their neighbours (`napewno` → `na pewno`, `kochamcię` → `kocham cię`),
//! letters to try for a dictionary without a TRY line, and final devoicing (`chlep` → `chleb`).

use crate::affix::AffixDict;
use crate::affix_suggest::{COST_ADD_DIACRITIC, COST_MISTYPE, Split, Tuning};

/// Short words often run into the next one (`na pewno`, `w ogóle`, `nie wiem`, `przede
/// wszystkim`).
const LEADING_WORDS: &[&str] = &[
    "a", "i", "o", "u", "w", "z", "we", "ze", "na", "po", "do", "od", "za", "co", "to", "nie", "jak", "tak", "przy", "bez", "pod", "nad", "dla",
    "że", "by", "czy", "już", "ani", "przed", "przez", "przede", "ku", "aż", "bo", "ten", "ta", "te", "tym", "tej", "też", "jest", "ode", "beze",
    "nade",
];
/// Short words often run into the one before (`kocham cię`, `daj mi`, `bałem się`).
const TRAILING_WORDS: &[&str] =
    &["się", "cię", "mi", "mu", "ci", "go", "ją", "je", "nam", "wam", "im", "by", "że", "no", "mnie", "tobie", "jej", "ich"];

/// What Polish adds to its dictionary's tables.
const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "aioeznrwcysptkmdłuljągbhęśćóżfńźvqx",
    mistype_cost,
    ..Tuning::DEFAULT
};

/// Final devoicing makes a voiced consonant at the end of a word sound voiceless, so writing it
/// voiceless (`chlep` for `chleb`) is a likelier slip than any other typo.
fn mistype_cost(chars: &[char], i: usize, c: char) -> u32 {
    let last = i + 1 == chars.len();
    if last && matches!((chars.get(i), c), (Some('p'), 'b') | (Some('t'), 'd') | (Some('k'), 'g')) { COST_ADD_DIACRITIC - 1 } else { COST_MISTYPE }
}

/// Spelling suggestions for a Polish word, best first, at most `max`, in the word's letter case.
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
    fn voiced_consonant_suggestion_ranks_first() {
        crate::pl::test_dict::install();
        let result = suggest("chlep", 6);
        assert_eq!(result.first().map(String::as_str), Some("chleb"), "{result:?}");
    }
}
