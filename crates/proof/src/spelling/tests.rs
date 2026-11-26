//! The spelling options added to [`Options`]: capitals of any length and file addresses, each off by
//! default so nothing changes for a caller that doesn't ask.

use super::*;
use crate::{IssueKind, ProofLang, check_text_with, is_correct_in_with, register_dictionary, set_dictionaries_folder, unregister_dictionary};

const AFF: &[u8] = b"SET UTF-8\n";

/// The words flagged as misspelled in `text`, all of it in `lang`, with `grammar` off.
fn flagged(text: &str, lang: ProofLang, opts: &Options) -> Vec<String> {
    let issues = check_text_with(text, &[(0..text.len(), lang)], opts, false, false);
    assert!(issues.iter().all(|i| i.kind == IssueKind::Spelling), "{issues:?}");
    issues.iter().filter_map(|i| text.get(i.start..i.end)).map(str::to_string).collect()
}

/// A paragraph in a language with a registered dictionary, with no folder in front of it.
fn with_dictionaries(pairs: &[(&str, &[u8])], run: impl FnOnce()) {
    crate::hold_dictionary_tests();
    set_dictionaries_folder(None);
    for (tag, dic) in pairs {
        register_dictionary(tag, AFF, dic).unwrap();
    }
    run();
    for (tag, _) in pairs {
        unregister_dictionary(tag);
    }
    set_dictionaries_folder(Some(crate::tests::shared_test_folder()));
}

#[test]
fn the_new_options_are_off_by_default() {
    let o = Options::default();
    assert!(o.ignore_uppercase && o.ignore_numbers && o.ignore_internet, "the three that were always there");
    assert!(!o.ignore_uppercase_any_length && !o.ignore_file_addresses && !o.flag_repeated);
}

/// A whole word in capitals is ignored whatever its length in every language, by Ignore words in
/// UPPERCASE alone (the entry points every check goes through). The six-letter limit that remains is in
/// the parts of a hyphenated word (Indonesian and Russian) and in the per-language functions
/// (`pl::is_correct_with_options` and the others, `pack_lang::is_correct`); with the option those skip
/// capitals of any length too, and a word not in capitals is still checked.
#[test]
fn capitals_of_any_length_are_ignored_by_whole_word_and_by_option_in_the_parts() {
    let (default, long) = (Options::default(), Options { ignore_uppercase_any_length: true, ..Options::default() });
    let off = Options { ignore_uppercase: false, ignore_uppercase_any_length: true, ..Options::default() };
    with_dictionaries(
        &[("qr", b"1\nalpha\n"), ("ru", "1\nслово\n".as_bytes()), ("id", b"1\nrumah\n"), ("pl", "1\nsłowo\n".as_bytes())],
        || {
            for (tag, short, long_word, lower) in [
                ("qr", "NASA", "ORGANISATIONS", "organisations"),
                ("ru", "ВОЗ", "ПРЕДЛОЖЕНИЕ", "предложение"),
                ("id", "NASA", "ORGANISASINYA", "organisasinya"),
                ("pl", "NATO", "ORGANIZACJONALNY", "organizacjonalny"),
            ] {
                let lang = ProofLang::from_tag(Some(tag));
                // Whole words: any length, with or without the option; nothing but the lower-case word is checked.
                for opts in [&default, &long] {
                    assert!(is_correct_in_with(short, lang, opts) && is_correct_in_with(long_word, lang, opts), "{tag}");
                    assert!(!is_correct_in_with(lower, lang, opts), "{tag}: a word not in capitals is checked");
                    assert!(flagged(&format!("{short} {long_word}"), lang, opts).is_empty(), "{tag}");
                }
                assert!(!is_correct_in_with(long_word, lang, &off), "{tag}: the option needs Ignore words in UPPERCASE");
            }
            // The per-language functions stop at six letters, unless asked.
            let (pl, ru, id) = (crate::pl::is_correct_with_options, crate::ru::is_correct_with_options, crate::id::is_correct_with_options);
            for (check, short, long_word, lower) in [
                (pl as fn(&str, &Options) -> bool, "NATO", "ORGANIZACJONALNY", "organizacjonalny"),
                (ru, "ВОЗ", "ПРЕДЛОЖЕНИЕ", "предложение"),
                (id, "NASA", "ORGANISASINYA", "organisasinya"),
            ] {
                assert!(check(short, &default), "{short}: short words in capitals are ignored as before");
                assert!(!check(long_word, &default), "{long_word}: past six letters it is checked, as before");
                assert!(check(long_word, &long), "{long_word}: with the option it is ignored");
                assert!(!check(lower, &long), "{lower}: a word not in capitals is still checked");
                assert!(!check(long_word, &off), "{long_word}: the option needs Ignore words in UPPERCASE");
            }
            let qr = crate::lang::PackLocale::new("qr").unwrap();
            assert!(!crate::pack_lang::is_correct("ORGANISATIONS", qr, &default) && crate::pack_lang::is_correct("ORGANISATIONS", qr, &long));
            assert!(crate::pack_lang::is_correct("NASA", qr, &default) && !crate::pack_lang::is_correct("organisations", qr, &long));
            // The parts of a hyphenated word: Indonesian and Russian look at each.
            let id = ProofLang::from_tag(Some("id"));
            assert!(!is_correct_in_with("rumah-ORGANISASINYA", id, &default) && is_correct_in_with("rumah-ORGANISASINYA", id, &long));
            let ru = ProofLang::from_tag(Some("ru"));
            assert!(!is_correct_in_with("слово-ПРЕДЛОЖЕНИЕ", ru, &default) && is_correct_in_with("слово-ПРЕДЛОЖЕНИЕ", ru, &long));
        },
    );
}

/// English ignores words in capitals of any length with Ignore words in UPPERCASE alone: the option
/// changes nothing there.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn english_ignores_long_capitals_with_or_without_the_option() {
    crate::hold_dictionary_tests();
    for any in [false, true] {
        let opts = Options { ignore_uppercase_any_length: any, ..Options::default() };
        assert!(is_correct_in_with("ORGANISATIONSZZ", ProofLang::En, &opts), "{any}");
        assert!(!is_correct_in_with("organisationszz", ProofLang::En, &opts), "{any}");
    }
}

/// File addresses: a UNC path, a drive path, a `file:` address and a path from the root or the home
/// folder are skipped with the option, whatever the web option says; a word in a token that only looks
/// a little like one is not.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn file_addresses_are_skipped_only_when_asked() {
    crate::hold_dictionary_tests();
    let (default, on) = (Options::default(), Options { ignore_file_addresses: true, ..Options::default() });
    let addresses = [
        "\\\\server\\teh",
        "C:\\temp\\teh",
        "c:/temp/teh",
        "file:///usr/teh",
        "FILE:teh",
        "/usr/teh/x",
        "~/docs/teh",
        "(\\\\server\\teh)",
        "\"C:\\teh\"",
    ];
    // Without the web option too, so `file:///` (which holds `://`) is only the file option's to skip.
    let bare = Options { ignore_internet: false, ..default };
    for a in addresses {
        assert!(flagged(a, ProofLang::En, &bare).contains(&"teh".to_string()), "{a}: flagged without the option");
        assert!(flagged(a, ProofLang::En, &on).is_empty(), "{a}: skipped with it");
        let no_web = Options { ignore_internet: false, ..on };
        assert!(flagged(a, ProofLang::En, &no_web).is_empty(), "{a}: whatever the web option says");
        // Inside a sentence, the address alone is skipped.
        assert_eq!(flagged(&format!("Open {a} and teh cat"), ProofLang::En, &on), ["teh"], "{a}");
    }
    // Not addresses: one slash, a slash that doesn't start the token, a drive with no separator, a
    // lone letter and colon, a tilde alone.
    for not in ["/teh", "x/teh", "and/or/teh", "C:teh", "~teh", "a:teh", "~/teh", "\\teh"] {
        assert!(flagged(not, ProofLang::En, &on).contains(&"teh".to_string()), "{not}: still checked with the option");
    }
    // The web option stays as it was: web and mail addresses follow `ignore_internet` alone.
    let web = "https://example.org/teh and me@example.org/teh";
    assert!(flagged(web, ProofLang::En, &default).is_empty());
    assert!(flagged(web, ProofLang::En, &on).is_empty());
    let off = Options { ignore_internet: false, ..on };
    assert!(flagged(web, ProofLang::En, &off).contains(&"teh".to_string()), "file addresses don't cover a web address");
}

#[test]
fn file_address_shapes() {
    for yes in ["\\\\a", "C:\\", "z:/", "file:x", "File:", "/a/b", "//a", "~/a/b", "([{<\"'/a/b", "«/a/b"] {
        assert!(is_file_address(yes), "{yes}");
    }
    for no in ["", "/", "/a", "~", "~/", "~/a", "a/b", "C:", "C:x", "1:\\a", "\\a", "fil:x", "https://x/y", "a@b/c"] {
        assert!(!is_file_address(no), "{no}");
    }
}

/// The README's example: a file address is skipped and a repeated word is found, together.
#[test]
#[cfg_attr(not(feature = "builtin-moby"), ignore = "needs an English dictionary: the built-in word list (the builtin-moby feature) or the system's")]
fn the_options_work_together() {
    crate::hold_dictionary_tests();
    let opts = Options { ignore_file_addresses: true, flag_repeated: true, ..Options::default() };
    let text = "Open \\\\server\\teh and the the plan";
    let issues = check_text_with(text, &[(0..text.len(), ProofLang::En)], &opts, false, false);
    assert_eq!(issues.iter().map(|i| (i.kind, text.get(i.start..i.end).unwrap_or_default())).collect::<Vec<_>>(), [(IssueKind::Repeated, "the")]);
    assert_eq!(issues.first().map(|i| i.start), text.rfind("the"));
}
