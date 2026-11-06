// SPDX-License-Identifier: MPL-2.0
//! Hungarian compatibility checks. Dictionaries are supplied by the hu Language pack,
//! never committed here. Recorded test words retain their MPL provenance in data/hu.
use craft_proof::affix::AffixDict;
use unicode_normalization::UnicodeNormalization;

/// Regression: compound rule final suffix must match the rule.
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

/// Regression: rule compounds only affix the last part and keep exclusions.
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

/// Regression: agrees with recorded hunspell verdicts and suggestions.
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

/// Regression: agrees with recorded native hyphenation.
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
            differences.push((word, actual));
        }
    }
    assert_eq!(count, 857);
    // Keep the native oracle unchanged. Only the two malformed upstream records
    // may differ, and pin their actual unmodified-data result rather than ignoring them.
    assert_eq!(
        differences,
        vec![("masszázszokni", "masz=száz=szok=ni".into()), ("giccsetek", "giccse=tek".into()),],
        "unexpected native hyphenation differences"
    );
    for (word, want) in [("asszony", "asz-szony"), ("könnyen", "köny-nyen"), ("hosszú", "hosz-szú"), ("ko\u{308}nnyen", "ko\u{308}ny-nyen")] {
        assert_eq!(craft_proof::hyphen::hyphenate_word_in(word, &limits, lang), want);
    }
    assert_eq!(craft_proof::hyphen::hyphenate_word_in("ASSZONY", &limits, lang), "ASSZONY");
    assert_eq!(
        craft_proof::hyphen::hyphen_opportunities_including_all_caps("ASSZONY", &limits, lang),
        vec![craft_proof::hyphen::Opportunity { at: 2, suffix: "Z".into() }]
    );
    craft_proof::packs::set_packs_dir(None);
}

/// Regression: review rule compounds preserve shared suggestion exclusions.
#[test]
fn review_rule_compounds_preserve_shared_suggestion_exclusions() {
    for directive in ["NOSUGGEST", "SUBSTANDARD"] {
        let aff = format!("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nFORBIDDENWORD X\n{directive} X\nSFX S Y 1\nSFX S 0 -ok .\n");
        let d = AffixDict::from_hunspell(&aff, "2\n2/A\n6/BSX\n", &[]).unwrap();
        assert!(d.check("26-ok"), "{directive}: ordinary compound checking");
        assert!(!d.can_suggest("26-ok"), "{directive}: the shared exclusion must survive");
    }
}

/// Regression: review keepcase applies only to the first rule compound part.
#[test]
fn review_keepcase_applies_only_to_the_first_rule_compound_part() {
    let aff = "SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nKEEPCASE K\nSFX S Y 1\nSFX S 0 s .\n";
    let d = AffixDict::from_hunspell(aff, "2\nfoo/A\nbar/BKS\n", &[]).unwrap();
    assert!(d.check("Foobars"));
    let first_kept = AffixDict::from_hunspell(aff, "2\nfoo/AK\nbar/BS\n", &[]).unwrap();
    assert!(!first_kept.check("Foobars"));
}

/// Regression: review irrelevant suffix analyses do not hide the rule flag.
#[test]
fn review_irrelevant_suffix_analyses_do_not_hide_the_rule_flag() {
    let count = 280;
    let mut aff = format!("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nSFX S Y {count}\n");
    let mut dic = format!("{}\nab/A\n", count + 1);
    for i in 0..count {
        aff.push_str(&format!("SFX S x{i} s .\n"));
        dic.push_str(&format!("basex{i}/{}\n", if i + 1 == count { "BS" } else { "S" }));
    }
    let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
    assert!(d.check("abbases"));
}

/// Regression: review unreachable tails do not exhaust rule compound analysis.
#[test]
fn review_unreachable_tails_do_not_exhaust_rule_compound_analysis() {
    let count = 800;
    let mut aff = format!("SET UTF-8\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\nSFX S Y 1\nSFX S 0 -s .\nSFX Z Y {count}\n");
    for i in 0..count {
        aff.push_str(&format!("SFX Z z{i} 0 .\n"));
    }
    let first = "a".repeat(35);
    let d = AffixDict::from_hunspell(&aff, &format!("2\n{first}/A\nend/BS\n"), &[]).unwrap();
    assert!(d.check(&format!("{first}end-s")));
}

/// Regression: review other rule flags cannot hide the requested terminal flag.
#[test]
fn review_other_rule_flags_cannot_hide_the_requested_terminal_flag() {
    let count = 280;
    let mut aff =
        format!("SET UTF-8\nFLAG num\nCOMPOUNDMIN 1\nCOMPOUNDRULE 2\nCOMPOUNDRULE (1)(2)\nCOMPOUNDRULE (4)(3)\nSFX 30000 Y {}\n", count + 1);
    let mut dic = format!("{}\nab/1\n", count + 2);
    for i in 0..=count {
        aff.push_str(&format!("SFX 30000 x{i} s .\n"));
        dic.push_str(&format!("basex{i}/30000,{}\n", if i == count { 2 } else { 3 }));
    }
    let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
    assert!(d.check("abbases"));
}
