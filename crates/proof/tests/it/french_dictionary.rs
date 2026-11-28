//! Explicit manual real-dictionary gate. Verify the pinned runtime inputs as documented in
//! crates/proof/README.md before running with `--ignored`. Missing data is a failure,
//! never a silently successful test that checked nothing.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use craft_proof::{IssueKind, Options, ProofLang, fr};

#[test]
#[ignore = "requires a real French dictionary in WORDCRAFT_DICTIONARIES"]
fn real_french_dictionary_spelling_suggestions_and_dispatch() {
    let directory = std::env::var_os("WORDCRAFT_DICTIONARIES").expect("set WORDCRAFT_DICTIONARIES to the test dictionary folder");
    craft_proof::set_dictionaries_folder(Some(std::path::PathBuf::from(&directory)));
    let french = ProofLang::from_tag(Some("fr-FR"));
    let installed = craft_proof::pack_dictionary_file(french).expect("the real French dictionary must be installed");
    assert_eq!(installed.parent(), Some(std::path::Path::new(&directory)), "do not silently use another system dictionary");
    let aff = installed.with_extension("aff");
    let dictionary = craft_proof::hunspell::load(&aff, &installed).expect("the real French dictionary loads");
    assert!(dictionary.len() > 50_000, "expected Dicollecte, not the synthetic fixture: {}", dictionary.len());
    for word in [
        "maison",
        "maisons",
        "Paris",
        "mangeons",
        "finissaient",
        "étaient",
        "l'arbre",
        "l’arbre",
        "qu'elles",
        "école",
        "œuvre",
        "aujourd'hui",
        "chevaux",
        "heureuse",
        "va-t-il",
    ] {
        assert!(fr::is_correct(word), "correct French word: {word}");
    }
    for (typo, expected) in [("maisonn", "maison"), ("ecole", "école"), ("l’ecole", "l’école"), ("documant", "document")] {
        assert!(!fr::is_correct(typo), "misspelling: {typo}");
        let suggestions = fr::suggest(typo, 8);
        assert!(suggestions.iter().any(|s| s == expected), "{typo} -> {suggestions:?}, expected {expected}");
    }
    let text = "Une maisonn près de l’école.";
    for lang in [ProofLang::from_tag(Some("fr-FR")), ProofLang::from_tag(Some("fr-CA"))] {
        let issues = craft_proof::check_text(text, &[(0..text.len(), lang)]);
        assert_eq!(issues.len(), 1, "{lang:?}: {issues:?}");
        let issue = &issues[0];
        assert_eq!(&text[issue.start..issue.end], "maisonn");
        assert_eq!(issue.lang, lang);
        assert_eq!(issue.message, "Faute d’orthographe possible");
    }
    check_combining_marks_and_compounds(&dictionary);
    assert!(craft_proof::dictionary_load_error(ProofLang::from_tag(Some("fr-FR"))).is_none());
}

fn check_combining_marks_and_compounds(dictionary: &craft_proof::affix::AffixDict) {
    let spelling = |text: &str, spans: &[(std::ops::Range<usize>, ProofLang)]| {
        craft_proof::check_text_with(text, spans, &Options::default(), false, false)
            .into_iter()
            .filter(|i| i.kind == IssueKind::Spelling)
            .map(|i| (i.start, i.end, i.lang))
            .collect::<Vec<_>>()
    };
    let mut failures = Vec::new();
    for lang in [ProofLang::from_tag(Some("fr-FR")), ProofLang::from_tag(Some("fr-CA"))] {
        for text in
            ["école", "e\u{301}cole", "l’école", "l’e\u{301}cole", "café", "cafe\u{301}", "porte-monnaie", "va-t-il", "a-t-il", "maison-école"]
        {
            assert!(
                fr::is_correct_with(dictionary, text) || ["va-t-il", "a-t-il", "maison-école"].contains(&text),
                "real dictionary recognizes {text:?}"
            );
            let issues = spelling(text, &[(0..text.len(), lang)]);
            if !issues.is_empty() {
                failures.push(format!("correct {lang:?} {text:?}: {issues:?}"));
            }
        }
        for text in ["porte--monnaie", "va--t-il", "maison--école", "e\u{301}colllle", "cafffe\u{301}"] {
            assert!(!fr::is_correct_with(dictionary, text), "the explicit dictionary entry must not already accept {text:?}");
            let issues = spelling(text, &[(0..text.len(), lang)]);
            let expected = vec![(0, text.len(), lang)];
            if issues != expected {
                failures.push(format!("typo {lang:?} {text:?}: {issues:?}, wanted {expected:?}"));
            }
        }
        let text = "e\u{301}cole cafe\u{301} xyzxyz porte--monnaie";
        let foreign = text.find("xyzxyz").unwrap();
        let compound = text.find("porte").unwrap();
        let spans = [(0..foreign, lang), (foreign..compound, ProofLang::Other), (compound..text.len(), lang)];
        let issues = spelling(text, &spans);
        if issues != [(compound, text.len(), lang)] {
            failures.push(format!("mixed {lang:?}: {issues:?}"));
        }
        assert!(spelling(text, &[(0..text.len(), ProofLang::Other)]).is_empty());
        // An English word remains in its own dictionary next to the original French bytes.
        let text = "e\u{301}cole hello cafe\u{301}";
        let en = text.find("hello").unwrap();
        let spans = [(0..en, lang), (en..en + 5, ProofLang::En), (en + 5..text.len(), lang)];
        let issues = spelling(text, &spans);
        if !issues.is_empty() {
            failures.push(format!("English boundary {lang:?}: {issues:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
