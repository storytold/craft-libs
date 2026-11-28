//! `Options::flag_repeated`: a word written twice in a row is a spelling issue on the second word.

use crate::{
    IssueKind, Options, ProofLang, check_spelling_with, check_text_with, register_dictionary, rules, set_dictionaries_folder, unregister_dictionary,
};

fn on() -> Options {
    Options { flag_repeated: true, ..Options::default() }
}

/// (kind, the text it covers) of each issue of `text` all in `lang`.
fn issues(text: &str, lang: ProofLang, opts: &Options, grammar: bool) -> Vec<(IssueKind, String)> {
    check_text_with(text, &[(0..text.len(), lang)], opts, grammar, false)
        .into_iter()
        .map(|i| (i.kind, text.get(i.start..i.end).unwrap_or("?").to_string()))
        .collect()
}

fn repeated(text: &str, lang: ProofLang) -> Vec<String> {
    issues(text, lang, &on(), false).into_iter().filter(|(k, _)| *k == IssueKind::Repeated).map(|(_, w)| w).collect()
}

/// Off by default: `the the` is no spelling issue, with the option off or with grammar off or on.
#[test]
fn repeated_words_are_not_flagged_by_default() {
    crate::hold_dictionary_tests();
    assert!(issues("The the", ProofLang::En, &Options::default(), false).is_empty());
    // The grammar rule is what the crate has always done: a Grammar issue over both words.
    let g = check_text_with("The the", &[(0..7, ProofLang::En)], &Options::default(), true, false);
    assert_eq!(g.len(), 1, "{g:?}");
    assert_eq!((g[0].kind, g[0].start, g[0].end, g[0].rule), (IssueKind::Grammar, 0, 7, Some(rules::REPEATED_WORD)));
}

/// With the option: one `Repeated` issue on the second word, no rule, no suggestion, the language
/// of the text, a message in it.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn a_repeated_word_is_an_issue_on_the_second_word() {
    crate::hold_dictionary_tests();
    let text = "Say the the word";
    let found = check_text_with(text, &[(0..text.len(), ProofLang::En)], &on(), false, false);
    assert_eq!(found.len(), 1, "{found:?}");
    let i = &found[0];
    assert_eq!((i.kind, i.start, i.end, i.rule, i.lang), (IssueKind::Repeated, 8, 11, None, ProofLang::En));
    assert!(i.suggestions.is_empty() && i.message.contains("Repeated word") && i.message.contains("the"), "{i:?}");
    // The English-only entry points read the option too.
    let spelling = check_spelling_with("Say the the word", &on());
    assert_eq!(spelling.iter().map(|i| (i.kind, i.start, i.end)).collect::<Vec<_>>(), [(IssueKind::Repeated, 8, 11)]);
    assert!(check_spelling_with("Say the the word", &Options::default()).is_empty());
}

/// What counts: case is ignored, any white space between, runs of three give two, words in other
/// scripts and with combining marks; what doesn't: punctuation between, numbers, one word, a hyphen.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn which_words_repeat() {
    crate::hold_dictionary_tests();
    let en = ProofLang::En;
    for (text, want) in [
        ("The the", vec!["the"]),
        ("THE the", vec!["the"]),
        ("the\tthe", vec!["the"]),
        ("the\u{a0}the", vec!["the"]),
        ("the \n the", vec!["the"]),
        ("the the the", vec!["the", "the"]),
        ("the the cat cat", vec!["the", "cat"]),
        ("école ÉCOLE", vec!["ÉCOLE"]),
        ("e\u{301}cole école", vec!["école"]),
        ("mp3 mp3", vec!["mp3"]),
        ("don't don't", vec!["don't"]),
        ("слово слово", vec!["слово"]),
        ("the, the", vec![]),
        ("the. the", vec![]),
        ("the - the", vec![]),
        ("the-the", vec![]),
        ("1 1", vec![]),
        ("2026 2026 12", vec![]),
        ("the", vec![]),
        ("", vec![]),
        ("the them", vec![]),
    ] {
        assert_eq!(repeated(text, en), want, "{text:?}");
    }
}

/// Only text with proofing tools: a language with none (`Other`, or a pack language with no
/// dictionary) has no issues; the words must be in one language.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn only_words_of_one_language_with_proofing_tools_repeat() {
    crate::hold_dictionary_tests();
    set_dictionaries_folder(None);
    assert!(repeated("la la", ProofLang::Other).is_empty());
    let qs = ProofLang::from_tag(Some("qs"));
    assert!(repeated("la la", qs).is_empty(), "a pack language with no dictionary has no tools");
    register_dictionary("qs", b"SET UTF-8\n", b"1\nla\n").unwrap();
    assert_eq!(repeated("la la", qs), ["la"]);
    unregister_dictionary("qs");
    set_dictionaries_folder(Some(crate::tests::shared_test_folder()));
    // Two words next to each other in different spans are in different languages.
    let text = "the the";
    let split = check_text_with(text, &[(0..3, ProofLang::En), (3..7, ProofLang::EnGb)], &on(), false, false);
    assert!(split.iter().all(|i| i.kind != IssueKind::Repeated), "{split:?}");
    let same = check_text_with(text, &[(0..3, ProofLang::En), (3..7, ProofLang::En)], &on(), false, false);
    assert!(same.iter().any(|i| i.kind == IssueKind::Repeated), "{same:?}");
    // A language with a message of its own says it in that language.
    let pl = check_text_with("kot kot", &[(0..7, ProofLang::Pl)], &on(), false, false);
    let r = pl.iter().find(|i| i.kind == IssueKind::Repeated).expect("Polish repeats");
    assert_eq!((r.start, r.end, r.lang), (4, 7, ProofLang::Pl));
    assert_eq!(r.message, "Powtórzone słowo: „kot”");
}

/// Both the option and the grammar rule on: each repeated word is found once (the option replaces the
/// rule `repeated-word`); the other grammar rules still report.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn the_option_replaces_the_grammar_rule_it_overlaps() {
    crate::hold_dictionary_tests();
    let both = check_text_with("The the  cat", &[(0..12, ProofLang::En)], &on(), true, false);
    let kinds: Vec<_> = both.iter().map(|i| (i.kind, i.rule)).collect();
    assert_eq!(kinds, [(IssueKind::Repeated, None), (IssueKind::Grammar, Some(rules::DOUBLE_SPACE))], "{both:?}");
    // Results come in order of position.
    assert!(both.windows(2).all(|w| w[0].start <= w[1].start), "{both:?}");
}

/// The message is the grammar rule's own, in each language that has one (so a host sees the same
/// words from either).
#[test]
fn the_message_is_the_grammar_rules_message() {
    crate::hold_dictionary_tests();
    for (tag, word) in [
        ("en-US", "the"),
        ("pl", "kot"),
        ("pt-BR", "ele"),
        ("pt-PT", "ele"),
        ("it", "casa"),
        ("hr", "kuća"),
        ("tr", "kitap"),
        ("ru", "дом"),
        ("cs", "dům"),
        ("id", "rumah"),
        ("es-ES", "casa"),
        ("es-MX", "casa"),
        ("fr-FR", "chat"),
    ] {
        let lang = ProofLang::from_tag(Some(tag));
        let text = format!("{word} {word}");
        let rule = crate::check_grammar_in(&text, lang).into_iter().find(|i| i.rule == Some(rules::REPEATED_WORD)).expect(tag);
        assert_eq!(rule.message, super::message(lang, word), "{tag}");
    }
    // A language with no grammar rules says it in English.
    assert_eq!(super::message(ProofLang::from_tag(Some("nl")), "huis"), "Repeated word: \"huis\"");
}

/// Hostile text: any text and any spans give issues inside the text, with no panic.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn any_text_is_checked_without_a_panic() {
    crate::hold_dictionary_tests();
    for text in ["\u{0}\u{0}", "a a\u{0}a a", "\u{301}\u{301} \u{301}\u{301}", "ǅ ǆ", "İ i", "ß SS", &"a ".repeat(5000), &"é".repeat(3000)] {
        for i in check_text_with(text, &[(0..text.len(), ProofLang::En)], &on(), true, false) {
            assert!(i.start <= i.end && i.end <= text.len() && text.is_char_boundary(i.start) && text.is_char_boundary(i.end), "{text:?}: {i:?}");
        }
    }
    assert_eq!(repeated(&"a ".repeat(5000), ProofLang::En).len(), 4999);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
    #[test]
    fn repeated_issues_cover_one_whole_word_inside_the_text(s in "[a-zA-Z0-9 \t.,é]{0,60}") {
        for i in check_text_with(&s, &[(0..s.len(), ProofLang::Other)], &on(), false, false) {
            proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
        }
        for i in check_text_with(&s, &[(0..s.len(), ProofLang::En)], &on(), false, false).into_iter().filter(|i| i.kind == IssueKind::Repeated) {
            proptest::prop_assert!(i.start < i.end && i.end <= s.len() && s.is_char_boundary(i.start) && s.is_char_boundary(i.end));
            let w = &s[i.start..i.end];
            proptest::prop_assert!(w.chars().any(char::is_alphabetic) && !w.contains(char::is_whitespace));
        }
    }
}
