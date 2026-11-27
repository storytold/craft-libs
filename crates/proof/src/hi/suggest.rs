//! Hindi suggestions: the shared ranking ([`crate::affix_suggest`]) with the installed
//! dictionary's own MAP/REP/TRY/KEY tables and nothing Hindi hard-coded.

use crate::affix::AffixDict;
use crate::affix_suggest::Tuning;

/// Spelling suggestions for a Hindi word, best first, at most `max`.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &Tuning::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hindi_suggestions_use_runtime_tables() {
        crate::hi::test_dict::install();
        // MAP (ि/ी, ं/ँ), REP (कश → क्ष) and a doubled letter.
        for (word, want) in [("पानि", "पानी"), ("गांव", "गाँव"), ("परीकशण", "परीक्षण"), ("प्रेमम", "प्रेम"), ("किताबो", "किताबों")]
        {
            let s = suggest(word, 8);
            assert!(s.contains(&want.into()), "{word}: {s:?}");
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::hi::test_dict::install();
        for w in ["", "दो शब्द", &"क".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("घर", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "घर", 4).is_empty());
        for word in ["zqबहुतलंबाहिंदीशब्द", "किताबब", "परीक्षणण"] {
            let res = suggest(word, 6);
            assert!(res.len() <= 6, "{word}: got {} suggestions", res.len());
            assert!(!res.contains(&"zqप्रतिबंधित".to_string()), "{word}: {res:?}");
            assert!(res.iter().all(|s| !s.is_empty() && crate::hi::is_correct(s)), "{word}: {res:?}");
        }
    }

    /// A NOSUGGEST stem stays correct spelling, but the Hindi collector never offers it or any
    /// prefixed, suffixed or REP-reached form of it; ordinary stems are still offered.
    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        let aff = "SET UTF-8\nNOSUGGEST N\nTRY गुप्तसामान्यअताएँ\nREP 2\nREP गुपत गुप्त\nREP सामानय सामान्य\nPFX X Y 1\nPFX X 0 अ .\nSFX A Y 1\nSFX A 0 ता/B .\nSFX B Y 1\nSFX B 0 एँ .\n";
        let d = AffixDict::from_hunspell(aff, "2\nगुप्त/NXA\nसामान्य/XA\n", &[]).unwrap();
        let hidden = d.forms("गुप्त");
        assert!(hidden.contains(&"अगुप्त".into()) && hidden.contains(&"गुप्तताएँ".into()), "{hidden:?}");
        for word in hidden.iter().chain(&["अगुप्तताएँ".to_string()]) {
            assert!(d.check(word), "NOSUGGEST is still correct: {word}");
            assert!(!d.can_suggest(word), "NOSUGGEST form: {word}");
        }
        for input in ["गुपत", "अगुपत", "गुपतता", "अगुपतताएँ", "गुप्तत", "अगुप्तताए"]
        {
            let got = suggest_in(&d, input, 32);
            assert!(got.iter().all(|w| w.split(' ').all(|part| d.can_suggest(part))), "{input}: {got:?}");
            assert!(got.iter().all(|w| !w.contains("गुप्त")), "{input}: {got:?}");
        }
        assert!(suggest_in(&d, "सामानय", 8).contains(&"सामान्य".into()));
        assert!(suggest_in(&d, "असामानयता", 8).contains(&"असामान्यता".into()));
    }
}
