//! French tuning of the shared, bounded affix suggestion engine.

use crate::affix::AffixDict;
use crate::affix_suggest::{Split, Tuning};

/// Short words often run into the next one (`et la`, `de Paris`, `pour moi`).
const LEADING_WORDS: &[&str] = &[
    "à", "a", "au", "aux", "de", "du", "des", "le", "la", "les", "un", "une", "et", "ou", "où", "en", "y", "ce", "se", "ne", "je", "tu", "il", "on",
    "nous", "vous", "ils", "elle", "par", "pour", "sur", "sous", "dans", "avec", "sans", "chez", "vers", "entre", "très", "plus", "moins", "bien",
    "pas", "que", "qui", "mais", "donc", "car", "si", "son", "sa", "ses", "mon", "ma", "mes", "ton", "ta", "tes", "est", "tout", "tous", "toute",
    "comme", "quand", "puis", "encore", "déjà", "jamais", "toujours",
];
/// Short words often run into the one before (`dis-moi` without the hyphen, `là-bas`, or a
/// dropped space: `pourmoi` → `pour moi`).
const TRAILING_WORDS: &[&str] =
    &["moi", "toi", "lui", "elle", "nous", "vous", "eux", "le", "la", "les", "en", "y", "là", "ci", "il", "on", "ils", "elles", "est", "et"];

const TUNING: Tuning = Tuning {
    split: Split::Near { leading: LEADING_WORDS, trailing: TRAILING_WORDS },
    alphabet: "esaitnrulodcpmévqfbghjàxèêyçôîâûùœ",
    ..Tuning::DEFAULT
};

/// French suggestions from a supplied dictionary; keeps the writer's apostrophe and case.
/// Dictionary OCONV remains active for every other output conversion.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    let word = crate::composed(word);
    let curly = word.contains('’');
    let apostrophe = if curly { '’' } else { '\'' };
    let mut out = Vec::new();
    for text in crate::affix_suggest::suggest_in(d, &word, max, &TUNING) {
        let styled: String = text.chars().map(|c| if c == '\'' || c == '’' { apostrophe } else { c }).collect();
        // A pack may spell only one apostrophe and have no ICONV. Keep its valid output
        // rather than turn it back into the spelling mistake the caller is correcting.
        let text = if styled.split(' ').all(|p| d.can_suggest_cased(p, p)) { styled } else { text };
        if text != word && !out.contains(&text) {
            out.push(text);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn french_accents_confusions_splits_and_elisions() {
        let d = super::super::test_dict::dictionary();
        for (typo, expected) in [
            ("ecole", "école"),
            ("riviere", "rivière"),
            ("soeur", "sœur"),
            ("maisonn", "maison"),
            ("pourmoi", "pour moi"),
            ("l'arbrre", "l'arbre"),
            ("l’arbrre", "l’arbre"),
        ] {
            let got = suggest_in(&d, typo, 8);
            assert!(got.iter().any(|s| s == expected), "{typo}: {got:?}");
        }
        assert!(suggest_in(&d, "e\u{301}cole", 4).iter().all(|s| s != "école"));
        assert!(suggest_in(&d, "ecole", 0).is_empty());
        assert!(suggest_in(&d, &"a".repeat(41), 4).is_empty());
    }

    #[test]
    fn curly_dictionary_without_iconv_remains_suggestible() {
        let d = AffixDict::from_hunspell("SET UTF-8\n", "1\naujourd’hui\n", &[]).unwrap();
        assert!(suggest_in(&d, "aujourd’huui", 4).iter().any(|s| s == "aujourd’hui"));
        assert!(suggest_in(&d, "aujourd'hui", 4).iter().any(|s| s == "aujourd’hui"));
    }

    #[test]
    fn output_conversion_preserves_typed_apostrophe() {
        let d = AffixDict::from_hunspell("SET UTF-8\nICONV 1\nICONV ’ '\nOCONV 1\nOCONV ' ’\n", "1\nl'arbre\n", &[]).unwrap();
        assert!(suggest_in(&d, "l'arbrre", 4).iter().any(|s| s == "l'arbre"));
        assert!(suggest_in(&d, "l’arbrre", 4).iter().any(|s| s == "l’arbre"));
    }
}
