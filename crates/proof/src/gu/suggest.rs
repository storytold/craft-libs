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
    fn suggestion_results_stay_bounded_on_complex_inputs() {
        crate::gu::test_dict::install();
        for word in ["zqખૂબલાંબોશબ્દગુજરાતી", "પુસતક", "પરીક્ષણણ"] {
            let res = suggest(word, 6);
            assert!(res.len() <= 6, "{word}: got {} suggestions", res.len());
            assert!(!res.contains(&"zqપ્રતિબંધિત".to_string()));
            for s in &res {
                assert!(!s.is_empty());
                assert!(super::super::is_correct(s), "suggested word {s} should be valid");
            }
        }
    }

    /// A NOSUGGEST stem stays correct spelling, but the Gujarati collector never offers it or any
    /// prefixed, suffixed, REP-reached or recased form of it; ordinary stems are still offered.
    #[test]
    fn nosuggest_stems_and_forms_are_never_offered() {
        let aff = "SET UTF-8\nNOSUGGEST N\nFORBIDDENWORD F\nTRY abeklmnostxy\nMAP 1\nMAP ae\nKEY ak\nREP 2\nREP xy toka\nREP zz normal\nPFX X Y 1\nPFX X 0 pre .\nSFX A Y 1\nSFX A 0 ble/B .\nSFX B Y 1\nSFX B 0 s .\n";
        let d = AffixDict::from_hunspell(aff, "4\ntoka/NXA\nnormal/XA\nTkname/N\nbanned/F\n", &[]).unwrap();
        for word in d.forms("toka").into_iter().chain(["TOKA".into(), "Toka".into(), "Tkname".into(), "TKNAME".into()]) {
            assert!(d.check(&word), "NOSUGGEST is still correct: {word}");
            assert!(!d.can_suggest(&word), "NOSUGGEST form: {word}");
        }
        for input in ["tokax", "tokablesx", "pretokax", "pretokablesx", "toka", "TOKA", "tkname", "teka", "xy", "normaltoka", "toak", "taak"] {
            let got = suggest_in(&d, input, 32);
            assert!(got.iter().all(|w| w.split(' ').all(|part| d.can_suggest(part))), "{input}: {got:?}");
            assert!(got.iter().all(|w| !w.to_lowercase().contains("toka") && w.to_lowercase() != "tkname"), "{input}: {got:?}");
        }
        assert!(suggest_in(&d, "norrmal", 8).contains(&"normal".into()));
        assert!(suggest_in(&d, "zz", 8).contains(&"normal".into()));
        // Filtering happens before display casing: an ordinary mixed-case stem keeps its all-caps
        // suggestion, with and without a NOSUGGEST declaration.
        for aff in ["", "NOSUGGEST N\n"] {
            let ordinary = AffixDict::from_hunspell(aff, "2\nmixCase\nhidden/N\n", &[]).unwrap();
            assert!(suggest_in(&ordinary, "MIXCASX", 8).contains(&"MIXCASE".into()));
        }

        // The same contract in Gujarati script, with Gujarati affixes and REP pairs.
        let aff = "SET UTF-8\nNOSUGGEST N\nTRY ગુપ્તસામાન્યઅોં\nREP 2\nREP ગુપત ગુપ્ત\nREP સામાનય સામાન્ય\nPFX X Y 1\nPFX X 0 અ .\nSFX A Y 1\nSFX A 0 ો/B .\nSFX B Y 1\nSFX B 0 માં .\n";
        let d = AffixDict::from_hunspell(aff, "2\nગુપ્ત/NXA\nસામાન્ય/XA\n", &[]).unwrap();
        let hidden = d.forms("ગુપ્ત");
        assert!(hidden.contains(&"અગુપ્તો".into()) && hidden.contains(&"ગુપ્તોમાં".into()), "{hidden:?}");
        for word in hidden.iter().chain(&["અગુપ્તોમાં".to_string()]) {
            assert!(d.check(word), "NOSUGGEST is still correct: {word}");
            assert!(!d.can_suggest(word), "NOSUGGEST form: {word}");
        }
        for input in ["ગુપત", "અગુપત", "ગુપતો", "અગુપતોમાં", "ગુપ્તત", "અગુપ્તોમા"]
        {
            let got = suggest_in(&d, input, 32);
            assert!(got.iter().all(|w| w.split(' ').all(|part| d.can_suggest(part))), "{input}: {got:?}");
            assert!(got.iter().all(|w| !w.contains("ગુપ્ત")), "{input}: {got:?}");
        }
        assert!(suggest_in(&d, "સામાનય", 8).contains(&"સામાન્ય".into()));
        assert!(suggest_in(&d, "અસામાનયો", 8).contains(&"અસામાન્યો".into()));
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
