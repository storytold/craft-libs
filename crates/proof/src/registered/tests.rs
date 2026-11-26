//! A dictionary handed over as bytes ([`register_dictionary`]): its words are right, the others
//! wrong, it comes after the dictionaries folder and before the packs, the system and the
//! embedded English, it is charged to the budget, and a pair past the caps is refused.

use super::*;
use crate::{EnglishSource, ProofLang, is_correct_in, set_dictionaries_folder};

const AFF: &[u8] = b"SET UTF-8\nTRY esianrtolducgmphbykfvwjzxq\nSFX S Y 1\nSFX S 0 s .\n";

/// Hold the tests that change process-wide state, and start from nothing registered and the
/// folder the other tests leave set.
fn start() {
    crate::hold_dictionary_tests();
    for tag in registered_dictionaries() {
        unregister_dictionary(&tag);
    }
    set_dictionaries_folder(Some(crate::tests::shared_test_folder()));
}

/// Leave nothing of ours behind.
fn finish() {
    for tag in registered_dictionaries() {
        unregister_dictionary(&tag);
    }
    crate::set_budget(crate::DEFAULT_BUDGET);
    crate::packs::set_packs_dir(None);
    set_dictionaries_folder(Some(crate::tests::shared_test_folder()));
}

fn lang(tag: &str) -> ProofLang {
    ProofLang::from_tag(Some(tag))
}

/// An English with a registered pair knows its words and no others, whatever the built-in list or
/// SCOWL knows; another English is left alone; unregistering gives the old source back.
#[test]
fn a_registered_pair_makes_its_words_right_and_others_wrong_in_an_english() {
    start();
    let before = crate::english_source(ProofLang::En);
    assert!(is_correct_in("hello", ProofLang::En));
    register_dictionary("en-US", AFF, b"3\nzzyzx\nquuxer/S\nhello\n").unwrap();
    assert_eq!(registered_dictionaries(), ["en-us"]);
    assert_eq!(crate::english_source(ProofLang::En), Some(EnglishSource::Registered));
    for w in ["zzyzx", "quuxer", "quuxers", "hello"] {
        assert!(is_correct_in(w, ProofLang::En), "{w}");
    }
    for w in ["world", "becuase", "quuxs"] {
        assert!(!is_correct_in(w, ProofLang::En), "{w}");
    }
    assert!(crate::suggest_in("zzyzy", 3, ProofLang::En).contains(&"zzyzx".to_string()));
    // Only American English reads the en-US pair: the others keep their own source.
    assert!(is_correct_in("world", ProofLang::EnGb) && is_correct_in("world", ProofLang::EnCa));
    assert_ne!(crate::english_source(ProofLang::EnGb), Some(EnglishSource::Registered));
    // The tags of an English are the language's: `en` and `en-us` are the same pair, and a tag of
    // another region that spells British is British English's.
    register_dictionary("en", AFF, b"1\nzzyzx\n").unwrap();
    assert_eq!(registered_dictionaries(), ["en-us"]);
    register_dictionary("en-NZ", AFF, b"1\nkiwiword\n").unwrap();
    assert_eq!(registered_dictionaries(), ["en-gb", "en-us"]);
    assert!(is_correct_in("kiwiword", ProofLang::EnGb) && !is_correct_in("world", ProofLang::EnGb));
    assert!(unregister_dictionary("en-US") && !unregister_dictionary("en-US"));
    assert!(unregister_dictionary("en-GB"));
    assert!(is_correct_in("world", ProofLang::En) && is_correct_in("world", ProofLang::EnGb));
    assert_eq!(crate::english_source(ProofLang::En), before);
    finish();
}

/// A language that reads packs, and one with a slot of its own (Polish), are proofed from the
/// registered pair; text in a language with no pair stays unchecked.
#[test]
fn a_registered_pair_proofs_any_language() {
    start();
    let qr = lang("qr");
    assert!(is_correct_in("anything", qr), "no dictionary: nothing is marked");
    assert!(!crate::has_dictionary(Some("qr")));
    let generation = crate::dictionary_generation();
    register_dictionary("qr", AFF, b"2\nregword/S\nother\n").unwrap();
    assert!(crate::dictionary_generation() > generation, "text is checked again");
    assert!(crate::has_dictionary(Some("qr")) && qr.is_proofed());
    assert!(is_correct_in("regword", qr) && is_correct_in("regwords", qr) && !is_correct_in("anything", qr));
    assert!(crate::suggest_in("regwrd", 3, qr).contains(&"regword".to_string()));
    assert_eq!(crate::pack_dictionary_file(qr), None, "a registered pair has no file");
    assert_eq!(crate::dictionary_load_error(qr), None);
    // A region reads its language's pair, as a file `qr.dic` serves `qr-xx`.
    assert!(is_correct_in("regword", lang("qr-XX")));
    // Polish (a slot of its own), with the folder that holds a Polish test dictionary cleared.
    set_dictionaries_folder(None);
    register_dictionary("pl-PL", AFF, b"1\nzzpl\n").unwrap();
    assert!(is_correct_in("zzpl", ProofLang::Pl) && !is_correct_in("kotek", ProofLang::Pl));
    assert!(crate::pl::has_dictionary());
    assert_eq!(crate::installed_dictionaries().into_iter().find(|(l, _)| *l == ProofLang::Pl).and_then(|(_, p)| p), None);
    // Registering again replaces the pair; removing it leaves the language unchecked.
    register_dictionary("pl-PL", AFF, b"1\nzzplb\n").unwrap();
    assert!(is_correct_in("zzplb", ProofLang::Pl) && !is_correct_in("zzpl", ProofLang::Pl));
    // A pair for `pl` alone ranks after one for `pl-PL`, as `pl.dic` does after `pl_PL.dic`.
    register_dictionary("pl", AFF, b"1\nzzplain\n").unwrap();
    assert!(is_correct_in("zzplb", ProofLang::Pl) && !is_correct_in("zzplain", ProofLang::Pl));
    assert!(unregister_dictionary("pl-PL"));
    assert!(is_correct_in("zzplain", ProofLang::Pl) && !is_correct_in("zzplb", ProofLang::Pl));
    assert!(unregister_dictionary("pl"));
    assert!(unregister_dictionary("qr"));
    assert!(is_correct_in("anything", qr));
    finish();
}

/// The order of the sources: the dictionaries folder first, then a registered pair, then an
/// installed pack. A reload of the folder or the packs does not forget the registered pair.
#[test]
fn a_registered_pair_comes_after_the_folder_and_before_the_packs() {
    start();
    let root = std::env::temp_dir().join(format!("wc-proof-registered-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (folder, packs) = (root.join("folder"), root.join("packs"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::create_dir_all(packs.join("qr")).unwrap();
    std::fs::write(packs.join("qr/pack.toml"), "locale = \"qr\"\nlicence = \"MIT\"\n").unwrap();
    std::fs::write(packs.join("qr/spelling.aff"), AFF).unwrap();
    std::fs::write(packs.join("qr/spelling.dic"), "1\npackword\n").unwrap();
    crate::packs::set_packs_dir(Some(packs));
    let qr = lang("qr");
    assert!(is_correct_in("packword", qr) && !is_correct_in("regword", qr));
    register_dictionary("qr", AFF, b"1\nregword\n").unwrap();
    assert!(is_correct_in("regword", qr) && !is_correct_in("packword", qr), "a registered pair is before a pack");
    std::fs::write(folder.join("qr.aff"), AFF).unwrap();
    std::fs::write(folder.join("qr.dic"), "1\nfolderword\n").unwrap();
    set_dictionaries_folder(Some(folder.clone()));
    assert!(is_correct_in("folderword", qr) && !is_correct_in("regword", qr), "the folder is before a registered pair");
    std::fs::remove_file(folder.join("qr.dic")).unwrap();
    std::fs::remove_file(folder.join("qr.aff")).unwrap();
    set_dictionaries_folder(Some(folder));
    assert!(is_correct_in("regword", qr), "a reload keeps the registered pair");
    crate::packs::reload();
    assert!(is_correct_in("regword", qr));
    finish();
    let _ = std::fs::remove_dir_all(&root);
}

/// A pair is charged `estimate(aff + dic)` to the budget until it is replaced or removed, and the
/// dictionaries loaded from packs give way to it: with the budget at the registered pair and one
/// pack dictionary, a second pack's load evicts the first.
#[test]
fn a_registered_pair_is_charged_to_the_budget() {
    start();
    assert_eq!(bytes(), 0);
    register_dictionary("qr", AFF, b"1\nregword\n").unwrap();
    let one = bytes();
    assert_eq!(one, (AFF.len() as u64 + b"1\nregword\n".len() as u64) * crate::pack_lang::BYTES_PER_FILE_BYTE);
    register_dictionary("qs", AFF, b"1\nsecond\n").unwrap();
    let two = bytes();
    assert!(two > one);
    register_dictionary("qs", AFF, b"3\nsecond\nthird\nfourth\n").unwrap();
    assert!(bytes() > two, "a replaced pair is charged again, not added");
    assert_eq!(registered_dictionaries(), ["qr", "qs"]);
    assert!(unregister_dictionary("qs"));
    assert_eq!(bytes(), one);

    // Two pack languages, each estimated at `pack` bytes; room for the registered pair and one.
    let root = std::env::temp_dir().join(format!("wc-proof-registered-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for l in ["qa", "qb"] {
        std::fs::create_dir_all(root.join(l)).unwrap();
        std::fs::write(root.join(l).join("pack.toml"), format!("locale = \"{l}\"\nlicence = \"MIT\"\n")).unwrap();
        std::fs::write(root.join(l).join("spelling.aff"), AFF).unwrap();
        std::fs::write(root.join(l).join("spelling.dic"), format!("1\nword{l}\n")).unwrap();
    }
    crate::packs::set_packs_dir(Some(root.clone()));
    let pack = (AFF.len() as u64 + b"1\nwordqa\n".len() as u64) * crate::pack_lang::BYTES_PER_FILE_BYTE;
    crate::set_budget(one + pack);
    let loads = crate::pack_lang::loads();
    assert!(is_correct_in("wordqa", lang("qa")));
    assert!(is_correct_in("wordqb", lang("qb")));
    assert!(is_correct_in("wordqa", lang("qa")));
    assert_eq!(crate::pack_lang::loads() - loads, 3, "the second pack evicted the first: the pair holds the rest of the room");
    assert!(unregister_dictionary("qr"));
    assert_eq!(bytes(), 0);
    finish();
    let _ = std::fs::remove_dir_all(&root);
}

/// A pair past the caps or that can't be read is refused, with the reason in words, and nothing is
/// registered.
#[test]
fn a_pair_past_the_caps_is_refused_with_a_reason() {
    start();
    let refused = |tag: &str, aff: &[u8], dic: &[u8]| register_dictionary(tag, aff, dic).unwrap_err();
    // Not a language.
    for tag in ["x-none", "und", "", "zxx", "1x", "not a tag"] {
        let e = refused(tag, AFF, b"1\nword\n");
        assert_eq!(e, RegisterError::NoLanguage, "{tag:?}");
        assert!(e.to_string().contains("language"), "{e}");
    }
    // A file past the per-file cap, whichever of the two.
    let big = vec![b' '; MAX_FILE_BYTES + 1];
    let e = refused("qr", &big, b"1\nword\n");
    assert_eq!(e, RegisterError::TooLarge { file: "aff", bytes: MAX_FILE_BYTES + 1, limit: MAX_FILE_BYTES });
    assert!(e.to_string().contains(".aff") && e.to_string().contains("too large"), "{e}");
    let e = refused("qr", AFF, &big);
    assert!(matches!(e, RegisterError::TooLarge { file: "dic", .. }), "{e:?}");
    // A pair that can't be decoded, or that holds no words.
    let e = refused("qr", b"SET SHIFT_JIS\n", b"1\nword\n");
    assert!(matches!(&e, RegisterError::Unreadable(r) if r.contains("encoding")), "{e:?}");
    let e = refused("qr", AFF, b"1\n\xff\xfe\n");
    assert!(matches!(&e, RegisterError::Unreadable(r) if r.contains("UTF-8")), "{e:?}");
    for empty in [&b""[..], b"0\n", b"  \n\n"] {
        let e = refused("qr", AFF, empty);
        assert!(matches!(&e, RegisterError::Unreadable(r) if r.contains("no words")), "{e:?}");
    }
    // Past the budget: the estimate of the pair is more than the room that is left.
    crate::set_budget(100);
    let e = refused("qr", AFF, b"1\nword\n");
    let needed = (AFF.len() as u64 + b"1\nword\n".len() as u64) * crate::pack_lang::BYTES_PER_FILE_BYTE;
    assert_eq!(e, RegisterError::OverBudget { needed, budget: 100 });
    assert!(e.to_string().contains("budget"), "{e}");
    crate::set_budget(crate::DEFAULT_BUDGET);
    // The budget is checked before the bytes are decoded: a file that isn't text is over the budget,
    // not unreadable, when it could not fit anyway.
    crate::set_budget(1 << 20);
    let junk = vec![0xff; 1 << 20];
    let e = refused("qr", AFF, &junk);
    assert!(matches!(e, RegisterError::OverBudget { budget, .. } if budget == 1 << 20), "{e:?}");
    crate::set_budget(crate::DEFAULT_BUDGET);
    // None of these registered anything.
    assert!(registered_dictionaries().is_empty());
    assert_eq!(bytes(), 0);
    assert!(is_correct_in("anything", lang("qr")));
    finish();
}

/// At most [`MAX_REGISTERED`] languages; replacing one at the cap is allowed, a new one is not.
#[test]
fn only_so_many_languages_can_be_registered() {
    start();
    let tags: Vec<String> = (0..MAX_REGISTERED).map(|i| format!("z{}{}", (b'a' + (i / 26) as u8) as char, (b'a' + (i % 26) as u8) as char)).collect();
    for t in &tags {
        register_dictionary(t, AFF, b"1\nword\n").unwrap();
    }
    assert_eq!(registered_dictionaries().len(), MAX_REGISTERED);
    let e = register_dictionary("qr", AFF, b"1\nword\n").unwrap_err();
    assert_eq!(e, RegisterError::TooMany { limit: MAX_REGISTERED });
    assert!(e.to_string().contains(&MAX_REGISTERED.to_string()), "{e}");
    register_dictionary(&tags[3], AFF, b"1\nreplaced\n").unwrap();
    assert!(is_correct_in("replaced", lang(&tags[3])) && !is_correct_in("word", lang(&tags[3])));
    assert!(unregister_dictionary(&tags[0]));
    register_dictionary("qr", AFF, b"1\nword\n").unwrap();
    finish();
}

/// A pair that decodes but doesn't parse is registered (it is read on first use) and then reported
/// like any unreadable dictionary: the language falls back to what it had, and says why.
#[test]
fn a_pair_that_does_not_parse_is_reported_on_use_and_never_panics() {
    start();
    register_dictionary("en-GB", b"SET UTF-8\nFLAG BOGUS\n", b"1\nword\n").unwrap();
    assert_ne!(crate::english_source(ProofLang::EnGb), Some(EnglishSource::Registered));
    let why = crate::dictionary_load_error(ProofLang::EnGb).unwrap_or_default();
    assert!(why.contains("FLAG BOGUS"), "{why}");
    assert!(is_correct_in("world", ProofLang::EnGb), "English is still checked by the next source");
    register_dictionary("qr", b"SET UTF-8\nFLAG BOGUS\n", b"1\nword\n").unwrap();
    assert!(is_correct_in("anything", lang("qr")), "no readable dictionary: nothing is marked");
    assert!(crate::dictionary_load_error(lang("qr")).unwrap_or_default().contains("FLAG BOGUS"));
    finish();
}
