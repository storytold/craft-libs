//! Gujarati suggestions: the shared ranking ([`crate::affix_suggest`]) with the installed
//! dictionary's own MAP/REP/TRY/KEY tables and nothing Gujarati hard-coded.

use crate::affix::AffixDict;
use crate::affix_suggest::Tuning;

/// Spelling suggestions for a Gujarati word, best first, at most `max`.
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
    fn suggestions_are_fast_enough() {
        crate::gu::test_dict::install();
        let _ = suggest("ઘર", 1);
        let t = std::time::Instant::now();
        for word in ["zqખૂબલાંબોશબ્દગુજરાતી", "પુસતક", "પરીક્ષણણ"] {
            let _ = suggest(word, 6);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("3 Gujarati suggestion calls: {ms:.0} ms");
        assert!(ms < 20_000.0, "{ms} ms");
    }

    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        crate::affix::tests::assert_nosuggest_is_honoured(suggest_in);
    }

    #[test]
    fn gujarati_suggestions_use_runtime_tables() {
        crate::gu::test_dict::install();
        for (word, want) in [("શાલા", "શાળા"), ("પાની", "પાણી"), ("પ્રેમમ", "પ્રેમ"), ("પરીકશણ", "પરીક્ષણ")]
        {
            let s = suggest(word, 8);
            assert!(s.contains(&want.into()), "{word}: {s:?}");
        }
    }

    #[test]
    fn suggestions_stay_bounded_and_skip_forbidden_words() {
        crate::gu::test_dict::install();
        for w in ["", "બે શબ્દો", &"શ".repeat(41)] {
            assert!(suggest(w, 4).is_empty());
        }
        assert!(suggest("ઘર", 0).is_empty());
        assert!(suggest_in(&AffixDict::default(), "ઘર", 4).is_empty());
        assert!(suggest("ઘર", 4).len() <= 4);
        assert!(!suggest("ઘર", 4).contains(&"ઘર".into()));
        assert!(!suggest("zqપ્રતિબંધિત", 8).contains(&"zqપ્રતિબંધિત".into()));
    }
}
