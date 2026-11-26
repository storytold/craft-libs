//! The grammar rule ids: every grammar issue names its rule, the lists are exactly what the rules
//! report, the ids don't change, and the README lists them.

use super::*;
use crate::{Issue, IssueKind, check_grammar_in};

fn lang(tag: &str) -> ProofLang {
    ProofLang::from_tag(Some(tag))
}

/// For each language, a text that makes each of its rules report (one text may make several), the
/// language's tag, and the README's name for it.
fn samples() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        ("en-US", "English", vec!["the the", "a apple", "an cat", "word , x", "a  b", "hello. world"]),
        ("pl", "Polish", vec!["kot kot", "tak ,", "a  b", "Kot śpi. kot wstaje.", "Wiem że przyjdzie.", "Idę w wtorek.", "tak,, nie"]),
        ("pt-BR", "Portuguese", vec!["ele ele", "casa ,", "a  b", "Ele foi. ele voltou.", "sim,, não"]),
        (
            "it",
            "Italian",
            vec!["casa casa", "casa ,", "a  b", "Ciao. casa", "ciao,, casa", "un pò di vino", "qual'è la casa", "E' bello", "un'albero", "un amica"],
        ),
        ("hr", "Croatian", vec!["kuća kuća", "kuća ,", "a  b", "Kuća je. kuća", "da,, ne"]),
        ("tr", "Turkish", vec!["kitap kitap", "ev ,", "a  b", "ali, ve veli", "ve, veli", "Ev güzel. ev büyük."]),
        ("ru", "Russian", vec!["дом дом", "дом ,", "а  б", "Он ушёл. он вернулся.", "Я знаю что он придёт.", "да,, нет"]),
        ("cs", "Czech", vec!["dům dům", "dům ,", "a  b"]),
        ("id", "Indonesian", vec!["rumah rumah", "rumah ,", "a  b"]),
        ("es-ES", "Spanish", vec!["casa casa", "¿ Vienes?", "casa ,", "a  b", "Vienes mañana?"]),
        ("fr-FR", "French", vec!["Bonjour!", "«Bonjour»", "\"bonjour tout le monde\"", "le le chat", "Il part. il revient.", "le arbre"]),
    ]
}

/// Every issue of a grammar check names its rule, one of its language's; and every rule the language
/// lists reports somewhere in the samples (so the lists are not wishes).
#[test]
fn every_grammar_issue_names_a_rule_of_its_language_and_every_listed_rule_reports() {
    for (tag, name, texts) in samples() {
        let lang = lang(tag);
        let listed = grammar_rules(lang);
        assert!(!listed.is_empty(), "{name}");
        let mut seen: Vec<&'static str> = Vec::new();
        for text in texts {
            for issue in check_grammar_in(text, lang) {
                assert_eq!(issue.kind, IssueKind::Grammar, "{name}: {text:?}");
                let rule = issue.rule.unwrap_or_else(|| panic!("{name}: {text:?}: {issue:?} names no rule"));
                assert!(listed.contains(&rule), "{name}: {text:?}: rule {rule:?} is not in {listed:?}");
                if !seen.contains(&rule) {
                    seen.push(rule);
                }
            }
        }
        for rule in listed {
            assert!(seen.contains(rule), "{name}: no sample makes the rule {rule:?} report: {seen:?}");
        }
    }
}

/// The ids are a contract: these lists never lose or rename an id, they only grow.
#[test]
fn the_rule_ids_are_stable() {
    let expected: &[(&str, &[&str])] = &[
        ("en", &["repeated-word", "a-an", "space-before-punctuation", "double-space", "sentence-capital"]),
        ("en-GB", &["repeated-word", "a-an", "space-before-punctuation", "double-space", "sentence-capital"]),
        ("en-AU", &["repeated-word", "a-an", "space-before-punctuation", "double-space", "sentence-capital"]),
        ("en-CA", &["repeated-word", "a-an", "space-before-punctuation", "double-space", "sentence-capital"]),
        ("pl", &["repeated-word", "space-before-punctuation", "double-space", "sentence-capital", "missing-comma", "we-ze", "doubled-punctuation"]),
        ("pt", &["repeated-word", "space-before-punctuation", "double-space", "sentence-capital", "doubled-punctuation"]),
        ("pt-PT", &["repeated-word", "space-before-punctuation", "double-space", "sentence-capital", "doubled-punctuation"]),
        (
            "it",
            &[
                "repeated-word",
                "space-before-punctuation",
                "double-space",
                "sentence-capital",
                "doubled-punctuation",
                "po-apostrophe",
                "qual-e",
                "e-apostrophe",
                "un-apostrophe-masculine",
                "un-apostrophe-feminine",
            ],
        ),
        ("hr", &["repeated-word", "space-before-punctuation", "double-space", "sentence-capital", "doubled-punctuation"]),
        (
            "tr",
            &["repeated-word", "space-before-punctuation", "double-space", "comma-before-conjunction", "comma-after-conjunction", "sentence-capital"],
        ),
        ("ru", &["repeated-word", "space-before-punctuation", "double-space", "sentence-capital", "missing-comma", "doubled-punctuation"]),
        ("cs", &["repeated-word", "space-before-punctuation", "double-space"]),
        ("id", &["repeated-word", "space-before-punctuation", "double-space"]),
        ("es", &["repeated-word", "space-after-opening-mark", "space-before-punctuation", "double-space", "opening-marks"]),
        ("es-MX", &["repeated-word", "space-after-opening-mark", "space-before-punctuation", "double-space", "opening-marks"]),
        ("es-419", &["repeated-word", "space-after-opening-mark", "space-before-punctuation", "double-space", "opening-marks"]),
        ("es-UY", &["repeated-word", "space-after-opening-mark", "space-before-punctuation", "double-space", "opening-marks"]),
        ("fr-FR", &["high-punctuation", "guillemets", "straight-quotes", "repeated-word", "sentence-capital", "elision"]),
        ("fr-CA", &["high-punctuation", "guillemets", "straight-quotes", "repeated-word", "sentence-capital", "elision"]),
    ];
    for (tag, ids) in expected {
        assert_eq!(grammar_rules(lang(tag)), *ids, "{tag}");
    }
    // No grammar rules: nothing to list, and nothing reported.
    for tag in ["de", "de-CH", "bg", "nl", "ja", "x-none", "zh-Hans"] {
        assert!(grammar_rules(lang(tag)).is_empty(), "{tag}");
        assert!(check_grammar_in("the the  a apple , hello. world", lang(tag)).is_empty(), "{tag}");
    }
}

/// A spelling issue names no rule, and a grammar issue made by a regional language keeps its rule.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn spelling_issues_have_no_rule_and_regions_keep_the_rules() {
    let text = "the the  a apple";
    for tag in ["en-US", "en-GB", "en-AU", "en-CA"] {
        let issues = check_grammar_in(text, lang(tag));
        let rules: Vec<_> = issues.iter().map(|i| i.rule).collect();
        assert!(rules.contains(&Some(REPEATED_WORD)) && rules.contains(&Some(DOUBLE_SPACE)) && rules.contains(&Some(A_AN)), "{tag}: {rules:?}");
    }
    let spelling: Vec<Issue> = crate::check_spelling_in("teh cat", ProofLang::En);
    assert!(!spelling.is_empty() && spelling.iter().all(|i| i.kind == IssueKind::Spelling && i.rule.is_none()), "{spelling:?}");
    // Through the whole-paragraph check, grammar issues keep their rule and spelling ones have none.
    let t = "teh cat  sat";
    let all = crate::check_text(t, &[(0..t.len(), ProofLang::En)]);
    assert!(all.iter().any(|i| i.kind == IssueKind::Spelling && i.rule.is_none()), "{all:?}");
    assert!(all.iter().any(|i| i.kind == IssueKind::Grammar && i.rule == Some(DOUBLE_SPACE)), "{all:?}");
}

/// The README lists each language's rule ids, the same as [`grammar_rules`], in its order.
#[test]
fn the_readme_lists_the_rule_ids_of_each_language() {
    let readme = include_str!("../../README.md");
    let section = readme.split("## Grammar rule ids").nth(1).expect("the README has a `## Grammar rule ids` section");
    let section = section.split("\n## ").next().unwrap_or(section);
    let row = |name: &str| -> Vec<String> {
        let line = section.lines().find(|l| l.starts_with(&format!("| {name}"))).unwrap_or_else(|| panic!("no README row for {name}"));
        let ids = line.rsplit('|').nth(1).unwrap_or_default();
        ids.split('`').skip(1).step_by(2).map(str::to_string).collect()
    };
    for (name, tag) in [
        ("English", "en-US"),
        ("Polish", "pl"),
        ("Portuguese", "pt-BR"),
        ("Italian", "it"),
        ("Croatian", "hr"),
        ("Turkish", "tr"),
        ("Russian", "ru"),
        ("Czech", "cs"),
        ("Indonesian", "id"),
        ("Spanish", "es-ES"),
        ("French", "fr-FR"),
    ] {
        assert_eq!(row(name), grammar_rules(lang(tag)), "the README row of {name}");
    }
}
