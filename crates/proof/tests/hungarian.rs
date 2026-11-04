// SPDX-License-Identifier: MPL-2.0
//! Hungarian compatibility checks. Dictionaries are supplied by the hu Language pack,
//! never committed here. Recorded test words retain their MPL provenance in data/hu.
use craft_proof::affix::AffixDict;
use unicode_normalization::UnicodeNormalization;

#[test]
fn compound_rule_final_suffix_must_match_the_rule() {
    let aff = "SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nCOMPOUNDFORBIDFLAG F\nFORBIDDENWORD X\nSFX S Y 1\nSFX S 0 -ok .\nSFX T Y 1\nSFX T 0 -bad/F .\n";
    let d = AffixDict::from_hunspell(aff, "5\n2/A\n6/BST\n7/S\n8/BSX\n6/CS\n", &[]).unwrap();
    for w in ["26-ok", "6-ok", "26-bad", "28-ok"] {
        assert!(d.check(w), "{w}");
    }
    for w in ["27-ok", "226-ok", "26-no", "8-ok"] {
        assert!(!d.check(w), "{w}");
    }
}

#[test]
fn rule_compounds_only_affix_the_last_part_and_keep_exclusions() {
    let aff = "SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE A*B\nFORBIDDENWORD X\nSFX S Y 1\nSFX S 0 -ok .\n";
    let d = AffixDict::from_hunspell(aff, "4\n2/AS\n6/BS\n7/BXS\n26-bad/X\n", &[]).unwrap();
    assert!(d.check("226-ok"));
    // Like the recorded 28-ok case above, the terminal suffix is allowed in
    // rule compounds even when that stem's standalone form is forbidden.
    assert!(d.check("27-ok"));
    for w in ["2-ok6", "27", "7-ok", "26-bad", "226-no"] {
        assert!(!d.check(w), "{w}");
    }
    let banned = AffixDict::from_hunspell(aff, "3\n2/AS\n6/BS\n226-ok/X\n", &[]).unwrap();
    assert!(!banned.check("226-ok"), "an explicit whole-word ban still wins");
}

#[test]
#[ignore = "requires CRAFT_HU_TEST_PACKS pointing to the verified hu Language pack"]
fn agrees_with_recorded_hunspell_verdicts_and_suggestions() {
    let packs = std::path::PathBuf::from(std::env::var("CRAFT_HU_TEST_PACKS").expect("test pack directory"));
    craft_proof::packs::set_packs_dir(Some(packs));
    let lang = craft_proof::ProofLang::from_tag(Some("hu"));
    assert!(craft_proof::has_dictionary(Some("hu")), "hu pack was not loaded");
    let opts = craft_proof::Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
    let mut mismatches = Vec::new();
    let mut total = 0;
    for text in [include_str!("data/hu/main.tsv"), include_str!("data/hu/focused.tsv"), include_str!("data/hu/extended.tsv")] {
        for line in text.lines() {
            let (word, verdict) = line.split_once('\t').unwrap();
            total += 1;
            let composed: String = word.nfc().collect();
            // The recorded CLI rejects decomposed accents; the application accepts
            // the corresponding composed word. Pin that expectation to the oracle,
            // not to another call to the implementation being tested.
            let expected = if composed != word {
                let record = [include_str!("data/hu/main.tsv"), include_str!("data/hu/focused.tsv"), include_str!("data/hu/extended.tsv")]
                    .into_iter()
                    .flat_map(str::lines)
                    .filter_map(|l| l.split_once('\t'))
                    .find(|(w, _)| *w == composed);
                assert_eq!(record.map(|(_, v)| v), Some("1"));
                true
            } else {
                verdict == "1"
            };
            if craft_proof::is_correct_in_with(word, lang, &opts) != expected {
                mismatches.push(word);
            }
        }
    }
    assert_eq!(total, 550);
    assert!(mismatches.is_empty(), "Hunspell differences: {mismatches:?}");
    for line in include_str!("data/hu/suggestions.tsv").lines() {
        let (typo, expected) = line.split_once('\t').unwrap();
        let got = craft_proof::suggest_in(typo, 8, lang);
        assert!(got.iter().any(|s| s == expected), "{typo}: {got:?}");
    }
    craft_proof::packs::set_packs_dir(None);
}

#[test]
#[ignore = "requires CRAFT_HU_TEST_PACKS pointing to the verified hu Language pack"]
fn agrees_with_recorded_native_hyphenation() {
    let packs = std::path::PathBuf::from(std::env::var("CRAFT_HU_TEST_PACKS").expect("test pack directory"));
    craft_proof::packs::set_packs_dir(Some(packs));
    let lang = craft_proof::ProofLang::from_tag(Some("hu"));
    let limits = craft_proof::hyphen::Limits { min_word: 2, ..Default::default() };
    let mut count = 0;
    let mut differences = Vec::new();
    for line in include_str!("data/hu/hyphen-native.tsv").lines() {
        let (word, expected) = line.split_once('\t').unwrap();
        count += 1;
        let actual = craft_proof::hyphen::hyphenate_word_in(word, &limits, lang).replace('-', "=");
        if actual != expected {
            differences.push(format!("{word}: {actual}, expected {expected}"));
        }
    }
    assert_eq!(count, 857);
    assert!(differences.is_empty(), "{} differences: {differences:?}", differences.len());
    for (word, want) in
        [("asszony", "asz-szony"), ("könnyen", "köny-nyen"), ("hosszú", "hosz-szú"), ("ASSZONY", "ASZ-SZONY"), ("ko\u{308}nnyen", "ko\u{308}ny-nyen")]
    {
        assert_eq!(craft_proof::hyphen::hyphenate_word_in(word, &limits, lang), want);
    }
    craft_proof::packs::set_packs_dir(None);
}
