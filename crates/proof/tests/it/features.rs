//! What a build embeds, by its features: `embed-english` (the SCOWL dictionaries of the Englishes)
//! and `builtin-moby` (the word list built into the crate), both on by default.
//!
//! Without them (`default-features = false`, for a build that fetches its English dictionaries,
//! such as the web's) English has no dictionary of its own until the host registers one or the
//! system has one: nothing is marked, nothing is suggested, and the language reports that it is not
//! proofed. Run them with `cargo test -p craft-proof --no-default-features --test it`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use craft_proof::{ProofLang, is_correct_in, suggest_in};

/// A build with neither feature carries no English data: no SCOWL notice, no SCOWL English other than
/// American, and an English with no source marks nothing, suggests nothing and says what it needs.
/// A registered dictionary then makes it a checked language, and removing it takes that away.
#[cfg(not(any(feature = "embed-english", feature = "builtin-moby")))]
#[test]
fn a_build_without_the_defaults_embeds_no_english_data() {
    use craft_proof::EnglishSource;
    assert!(craft_proof::scowl::notice().is_none(), "no SCOWL notice: SCOWL is not in the build");
    let embedded = craft_proof::packs::embedded_locales();
    assert!(!embedded.iter().any(|l| ["en-gb", "en-ca", "en-au"].contains(l)), "{embedded:?}");

    // No source of its own (a system English dictionary may be installed, which is not the build's).
    let none = |lang: ProofLang| matches!(craft_proof::english_source(lang), Some(EnglishSource::Missing | EnglishSource::Hunspell(_)));
    for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::EnAu] {
        assert!(none(lang), "{lang:?}: {:?}", craft_proof::english_source(lang));
    }
    for (lang, tag, name) in [
        (ProofLang::En, "en-US", "en_US"),
        (ProofLang::EnGb, "en-GB", "en_GB"),
        (ProofLang::EnCa, "en-CA", "en_CA"),
        (ProofLang::EnAu, "en-AU", "en_AU"),
    ] {
        if craft_proof::english_source(lang) != Some(EnglishSource::Missing) {
            continue; // the system has a dictionary of this English
        }
        assert!(!lang.is_proofed() && !craft_proof::has_dictionary(Some(tag)), "{lang:?}");
        assert!(is_correct_in("becuase", lang), "{lang:?}: with no dictionary nothing is marked");
        assert!(is_correct_in("colour", lang) && is_correct_in("color", lang), "{lang:?}: not even the other dialect's spelling");
        assert!(suggest_in("becuase", 5, lang).is_empty(), "{lang:?}");
        assert!(craft_proof::check_spelling_in("becuase", lang).is_empty(), "{lang:?}");
        assert_eq!(craft_proof::missing_dictionary(Some(tag)), Some(name), "the Hunspell name to register or install");
    }

    // A registered dictionary makes American English a checked language.
    craft_proof::register_dictionary("en-US", b"SET UTF-8\n", b"3\nbecause\nthe\ncolor\n").unwrap();
    assert_eq!(craft_proof::english_source(ProofLang::En), Some(EnglishSource::Registered));
    assert!(ProofLang::En.is_proofed() && craft_proof::has_dictionary(Some("en-US")));
    assert!(!is_correct_in("becuase", ProofLang::En) && is_correct_in("because", ProofLang::En));
    assert!(suggest_in("becuase", 5, ProofLang::En).contains(&"because".to_string()));
    assert_eq!(craft_proof::missing_dictionary(Some("en-US")), None);
    assert!(craft_proof::unregister_dictionary("en-US"));
    assert!(none(ProofLang::En));
}

/// With the defaults, English is checked with nothing to read: the built-in list or SCOWL is there,
/// so English is proofed, needs no dictionary and flags a misspelling.
#[cfg(all(feature = "embed-english", feature = "builtin-moby"))]
#[test]
fn the_defaults_check_english_with_nothing_to_read() {
    for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::EnAu] {
        assert!(lang.is_proofed(), "{lang:?}");
        assert!(!is_correct_in("becuase", lang) && is_correct_in("because", lang), "{lang:?}");
        assert!(suggest_in("becuase", 5, lang).contains(&"because".to_string()), "{lang:?}");
    }
    assert_eq!(craft_proof::missing_dictionary(Some("en-US")), None);
    assert_eq!(craft_proof::missing_dictionary(Some("en-GB")), None);
}
