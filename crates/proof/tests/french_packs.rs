//! French uses ordinary Language packs and keeps each regional locale at the public API.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use craft_proof::affix;
use craft_proof::{IssueKind, Options, ProofLang, check_text_with, is_correct_in_with, suggest_in};
use std::path::{Path, PathBuf};

#[path = "../src/fr/test_dict.rs"]
mod fixture;

struct Tools(PathBuf);
impl Drop for Tools {
    fn drop(&mut self) {
        craft_proof::set_dictionaries_folder(None);
        craft_proof::packs::set_packs_dir(None);
        craft_proof::remove_word("zxqrechargefr");
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pair(dir: &Path, name: &str, aff: &str, dic: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
    std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
}

#[test]
fn french_packs_regions_reload_and_original_ranges() {
    assert!(!fixture::dictionary().is_empty());
    let root = std::env::temp_dir().join(format!("craft-fr-packs-{}", std::process::id()));
    let _tools = Tools(root.clone());
    let pack = root.join("packs/fr");
    pair(&pack, "spelling", fixture::AFF, fixture::DIC);
    std::fs::write(pack.join("pack.toml"), "locale = 'fr'\nname = 'Français'\nlicence = 'MIT'\n").unwrap();
    std::fs::write(pack.join("hyphenation.pat"), "ma1ison\n").unwrap();
    let folder = root.join("dictionaries");
    std::fs::create_dir_all(&folder).unwrap();
    craft_proof::packs::set_packs_dir(Some(root.join("packs")));
    craft_proof::set_dictionaries_folder(Some(folder.clone()));
    let defaults = Options::default();
    let strict = Options { ignore_uppercase: false, ignore_numbers: false, ..defaults };
    check_locales(defaults, strict);
    check_typography(strict);
    let fr = ProofLang::from_tag(Some("fr-FR"));
    let ca = ProofLang::from_tag(Some("fr-CA"));
    check_user_word(fr, ca, strict);
    check_overrides(folder, fr, ca, strict);
}

fn check_locales(defaults: Options, strict: Options) {
    for tag in ["fr", "fr-FR", "fr-CA", "fr-BE", "fr-CA-u-nu-latn", "fr-u-ca-gregory"] {
        let lang = ProofLang::from_tag(Some(tag));
        assert!(matches!(lang, ProofLang::Pack(_)), "{tag}");
        assert!(craft_proof::has_dictionary(Some(tag)), "{tag}");
        assert!(craft_proof::pack_dictionary_file(lang).unwrap().ends_with("fr/spelling.dic"), "{tag}");
        for word in ["école", "e\u{301}cole", "l'arbre", "l’arbre", "maison-école"] {
            assert!(is_correct_in_with(word, lang, &strict), "{tag}: {word}");
        }
        assert!(!is_correct_in_with("maison--école", lang, &strict));
        assert!(!is_correct_in_with("l'arbres", lang, &strict));
        assert!(is_correct_in_with("SNCF", lang, &defaults));
        assert!(!is_correct_in_with("SNCF", lang, &strict));
        assert!(is_correct_in_with("mp3x", lang, &defaults));
        assert!(!is_correct_in_with("mp3x", lang, &strict));
        assert!(suggest_in("ecole", 6, lang).iter().any(|s| s == "école"));
        let text = "🙂 maison\u{338} école";
        let issues = check_text_with(text, &[(0..text.len(), lang)], &strict, false, false);
        assert_eq!(issues.len(), 1, "{tag}: {issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "maison\u{338}");
        assert_eq!(issues[0].lang, lang);
        assert_eq!(issues[0].kind, IssueKind::Spelling);
        assert_eq!(issues[0].message, "Faute d’orthographe possible");
    }
}

fn check_typography(strict: Options) {
    let fr = ProofLang::from_tag(Some("fr-FR"));
    let ca = ProofLang::from_tag(Some("fr-CA"));
    for lang in [fr, ca] {
        assert_eq!(craft_proof::hyphen::hyphen_points_in("maison", &craft_proof::hyphen::Limits::default(), lang), [2]);
    }
    for tag in ["fr-FR", "fr-BE", "fr-u-ca-gregory", "fr-FR-u-rg-cazzzz"] {
        let issues = craft_proof::check_grammar_in("Bonjour!", ProofLang::from_tag(Some(tag)));
        assert_eq!(issues.len(), 1, "{tag}: {issues:?}");
        assert_eq!(issues[0].suggestions, ["\u{202f}!"]);
    }
    assert!(craft_proof::check_grammar_in("Bonjour!", ca).is_empty());
    assert!(craft_proof::check_grammar_in("Bonjour!", ProofLang::from_tag(Some("fr-Latn-CA"))).is_empty());
    assert!(craft_proof::check_grammar_in("Bonjour!", ProofLang::from_tag(Some("nl-NL"))).is_empty());
    assert!(craft_proof::fr::grammar::check_in("Bonjour!", ProofLang::En).is_empty());
    let text = "Bonjour! Salut!";
    let issues = check_text_with(text, &[(0..8, fr), (8..text.len(), ca)], &strict, true, false);
    let spacing: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Grammar && i.message.contains("fine insécable")).collect();
    assert_eq!(spacing.len(), 1, "{issues:?}");
    assert_eq!(spacing[0].lang, fr);
}

fn check_user_word(fr: ProofLang, ca: ProofLang, strict: Options) {
    craft_proof::add_word("zxqrechargefr");
    for lang in [fr, ca] {
        assert!(is_correct_in_with("l’zxqrechargefr", lang, &strict));
    }
    craft_proof::remove_word("zxqrechargefr");
    assert!(!is_correct_in_with("l’zxqrechargefr", ca, &strict));
}

fn check_overrides(folder: PathBuf, fr: ProofLang, ca: ProofLang, strict: Options) {
    // Regional overrides keep the generic pack fallback and the shared reload generation.
    pair(&folder, "fr_FR", "SET UTF-8\n", "1\nhexagonefixture\n");
    pair(&folder, "fr_CA", "SET UTF-8\n", "1\nquebecfixture\n");
    let generation = craft_proof::dictionary_generation();
    craft_proof::set_dictionaries_folder(Some(folder.clone()));
    assert!(craft_proof::dictionary_generation() > generation);
    assert!(is_correct_in_with("hexagonefixture", fr, &strict));
    assert!(!is_correct_in_with("hexagonefixture", ca, &strict));
    assert!(is_correct_in_with("quebecfixture", ca, &strict));
    assert!(!is_correct_in_with("quebecfixture", fr, &strict));
    assert!(craft_proof::pack_dictionary_file(ca).unwrap().ends_with("fr_CA.dic"));
    std::fs::remove_file(folder.join("fr_CA.dic")).unwrap();
    std::fs::remove_file(folder.join("fr_CA.aff")).unwrap();
    craft_proof::set_dictionaries_folder(Some(folder));
    assert!(is_correct_in_with("école", ca, &strict), "removed regional override falls back to fr pack");
    assert!(craft_proof::pack_dictionary_file(ca).unwrap().ends_with("fr/spelling.dic"));
}
