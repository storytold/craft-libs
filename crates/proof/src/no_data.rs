//! A build with no Language data embedded (no `CRAFT_I18N_DIR`): every public entry point still
//! works and never panics. English spelling still marks misspellings, against an English
//! Hunspell dictionary installed on the system or else the Moby word list built into the crate;
//! the dialect variants and hyphenation that need the data are off, and grammar (code, not data)
//! still works.

use crate::hyphen::{self, Limits, Source};
use crate::{EnglishSource, Options, ProofLang, dict, en_variants, patterns};

const LANGS: [ProofLang; 18] = [
    ProofLang::En,
    ProofLang::EnGb,
    ProofLang::EnCa,
    ProofLang::Pl,
    ProofLang::Bg,
    ProofLang::Pt,
    ProofLang::It,
    ProofLang::Hr,
    ProofLang::Tr,
    ProofLang::Ru,
    ProofLang::Cs,
    ProofLang::Id,
    ProofLang::Es,
    ProofLang::EsMx,
    ProofLang::Es419,
    ProofLang::EsUy,
    ProofLang::De,
    ProofLang::Other,
];

/// Words and text that exercise each entry point, hostile ones included.
const SAMPLES: [&str; 10] = [
    "",
    "teh colour of hyphenation",
    "Ünïcödé — ‘quotes’ don't",
    "a a  apple .Next sentence",
    "https://example.org a@b.org",
    "zażółć gęślą jaźń",
    "Привет, мир — кто-то",
    "¿Qué tal? más",
    "\u{0}\u{FFFF}\u{10FFFF}",
    "supercalifragilisticexpialidociousxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
];

#[test]
fn nothing_is_embedded() {
    assert!(crate::data::EN_US_DIC.is_empty());
    assert!(crate::data::EN_US_SPELLING.is_empty() && crate::data::EN_GB_SPELLING.is_empty() && crate::data::EN_CA_SPELLING.is_empty());
    assert!(crate::scowl::notice().is_none());
    for l in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
        assert!(crate::scowl::dict(l).is_none() && crate::scowl::load_error(l).is_none());
    }
    assert!(dict::Dictionary::en_us().is_empty());
    for p in [patterns::Patterns::en_us(), patterns::Patterns::pl(), patterns::Patterns::pt(), patterns::Patterns::it(), patterns::Patterns::de()] {
        assert!(p.is_empty());
        assert_eq!(p.exception_count(), 0);
    }
}

/// R6.4, English never goes unchecked: without the embedded word list, English is checked against
/// an installed English Hunspell dictionary or, without one, the Moby word list built into the
/// crate. Either way a misspelling is marked and common words are right. (Before, everything was right.)
#[test]
fn english_spelling_still_marks_misspellings() {
    crate::hold_dictionary_tests();
    for l in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
        assert!(matches!(crate::english_source(l), Some(EnglishSource::Hunspell(_) | EnglishSource::CoreList)), "{l:?}");
        assert!(!crate::is_correct_in("becuase", l) && !crate::is_correct_in("teh", l), "{l:?}");
        assert!(crate::suggest_in("teh", 5, l).contains(&"the".to_string()), "{l:?}");
        assert!(crate::suggest_in("becuase", 5, l).contains(&"because".to_string()), "{l:?}");
        for w in ["the", "The", "house", "houses", "because", "running", "people", "beautiful", "don't"] {
            assert!(crate::is_correct_in(w, l), "{w} {l:?}");
        }
        assert_eq!(crate::check_spelling_in("Becuase the house of the people", l).len(), 1, "{l:?}");
    }
    assert!(
        !crate::is_correct("becuase") && !crate::is_correct_with("becuase", &Options::default()) && !crate::is_correct_for("becuase", Some("en-GB"))
    );
    assert!(crate::suggest("becuase", 5).contains(&"because".to_string()));
    assert_eq!(crate::check_spelling("Becuase house").len(), 1);
    // The dialect variants are Language data: none without it.
    for w in ["colour", "color"] {
        assert_eq!(en_variants::preferred(w, ProofLang::En), None);
        assert!(!en_variants::is_foreign(w, ProofLang::EnGb));
    }
    // Text in a language without proofing is never checked with English.
    assert!(crate::check_spelling_in("Teh hous zxqv", ProofLang::Other).is_empty());
    assert!(crate::is_correct_for("teh", Some("fr-FR")));
}

#[test]
fn grammar_still_works() {
    assert!(!crate::check_grammar("This is is wrong.").is_empty());
}

#[test]
fn hyphenation_is_off_where_it_needs_data() {
    let lim = Limits::default();
    for (w, l) in [
        ("hyphenation", ProofLang::En),
        ("hyphenation", ProofLang::Cs),
        ("przedsiębiorstwo", ProofLang::Pl),
        ("computador", ProofLang::Pt),
        ("calcolatore", ProofLang::It),
        ("Silbentrennung", ProofLang::De),
    ] {
        assert_eq!(hyphen::word_breaks_in(w, l), (vec![], Source::None), "{w} {l:?}");
        assert!(hyphen::hyphen_points_in(w, &lim, l).is_empty(), "{w} {l:?}");
        assert_eq!(hyphen::hyphenate_word_in(w, &lim, l), w);
    }
    assert!(hyphen::hyphen_points("hyphenation", &lim).is_empty());
    assert_eq!(hyphen::hyphenate_word("hyphenation", &lim), "hyphenation");
    assert_eq!(hyphen::word_breaks("hyphenation").1, Source::None);
    // Rules are code, not data: Bulgarian still hyphenates.
    assert!(!hyphen::word_breaks_in("книжарница", ProofLang::Bg).0.is_empty());
}

/// Every public entry point, in every language, on every sample: no panic.
#[test]
fn every_public_function_runs_without_data() {
    // English results depend on the dictionaries other tests install.
    crate::hold_dictionary_tests();
    let lim = Limits { min_word: 0, after_first: 0, before_last: 0, capitalized: true };
    let opts = Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
    for text in SAMPLES {
        let len = text.len();
        let _ = crate::words(text);
        let _ = crate::check_spelling(text);
        let _ = crate::check_spelling_with(text, &opts);
        let _ = crate::check_grammar(text);
        let _ = crate::is_correct(text);
        let _ = crate::is_correct_with(text, &opts);
        let _ = crate::suggest(text, 5);
        let _ = crate::custom_word(text);
        let _ = crate::has_dictionary(Some(text));
        let _ = crate::missing_dictionary(Some(text));
        let _ = crate::unreadable_dictionary(Some(text));
        let _ = crate::is_correct_for(text, Some(text));
        let _ = hyphen::word_breaks(text);
        let _ = hyphen::hyphen_points(text, &lim);
        let _ = hyphen::hyphenate_word(text, &lim);
        let spans: Vec<(std::ops::Range<usize>, ProofLang)> = LANGS.iter().map(|&l| (0..len, l)).collect();
        let _ = crate::check_text(text, &spans);
        let _ = crate::check_text_with(text, &spans, &opts, true, true);
        // Out-of-range and reversed spans are hostile input too.
        #[allow(clippy::reversed_empty_ranges)]
        let _ = crate::check_text(text, &[(len + 5..len + 9, ProofLang::En), (3..1, ProofLang::Pl)]);
        for l in LANGS {
            let _ = crate::check_spelling_in(text, l);
            let _ = crate::check_grammar_in(text, l);
            let _ = crate::is_correct_in(text, l);
            let _ = crate::is_correct_in_with(text, l, &opts);
            let _ = crate::suggest_in(text, 5, l);
            let _ = crate::dictionary_load_error(l);
            let _ = crate::english_source(l);
            let _ = hyphen::word_breaks_in(text, l);
            let _ = hyphen::hyphen_points_in(text, &lim, l);
            let _ = hyphen::hyphenate_word_in(text, &lim, l);
            let _ = en_variants::preferred(text, l);
            let _ = en_variants::is_foreign(text, l);
            let _ = en_variants::known_variant(text, l, |_| true);
        }
    }
    let _ = crate::installed_dictionaries();
    let _ = crate::dictionary_generation();
    let _ = crate::user_dictionary();
    let _ = crate::has_dictionary(None);
    let _ = crate::missing_dictionary(None);
    let _ = crate::unreadable_dictionary(None);
}
