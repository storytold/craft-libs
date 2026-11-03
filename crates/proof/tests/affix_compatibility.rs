//! Frozen public output from the pre-French shared affix engine at upstream
//! e6c5a701ddff5be16e266028168c70658aae39d9, using original synthetic fixtures.
//! Do not regenerate expectations from the new implementation: they establish the baseline.
//! Legacy WCAFF1 bytes remain frozen, but are explicitly refused by the WCAFF2 engine.

use craft_proof::affix::{AffixDict, capitalize};
use std::fmt::Write;

#[path = "fixtures/affix_compatibility_cases.rs"]
mod fixtures;

fn snapshot(case: &fixtures::Case, dict: &AffixDict) -> String {
    let mut out = format!("{}\nlen={} rules={}\n", case.name, dict.len(), dict.rule_count());
    writeln!(out, "try={:?} key={:?} map={:?} rep={:?} comments={:?}", dict.try_chars(), dict.key(), dict.map(), dict.rep(), dict.comments())
        .unwrap();
    let mut stems: Vec<_> = dict.stems().collect();
    stems.sort_unstable();
    for stem in &stems {
        let mut forms = dict.forms(stem);
        forms.sort_unstable();
        forms.dedup();
        writeln!(out, "forms {stem:?}: {forms:?}").unwrap();
    }
    let mut probes: Vec<String> = case.probes.iter().map(|word| (*word).to_string()).collect();
    for stem in stems {
        probes.extend(dict.forms(stem));
    }
    let originals = probes.clone();
    for word in originals {
        probes.extend([word.to_uppercase(), word.to_lowercase(), capitalize(&word)]);
    }
    probes.sort_unstable();
    probes.dedup();
    for word in probes {
        writeln!(
            out,
            "{word:?}: contains={} check={} suggest={} raw_suggest={}",
            dict.contains(&word),
            dict.check(&word),
            dict.can_suggest(&word),
            dict.contains_suggestible(&word)
        )
        .unwrap();
    }
    out
}

#[test]
fn non_french_affix_outputs_match_pre_french_baseline() {
    let expected = include_str!("fixtures/affix_compatibility.expected");
    let mut actual = String::new();
    for case in fixtures::cases() {
        let dict = AffixDict::from_hunspell(&case.aff, &case.dic, &["original compatibility fixture"]).unwrap();
        let result = snapshot(&case, &dict);
        let restored = AffixDict::from_bytes(&dict.to_bytes().unwrap()).unwrap();
        assert_eq!(snapshot(&case, &restored), result, "current compact round trip: {}", case.name);
        actual.push_str(&result);
    }
    // craft-libs 55332dd predates this adaptation and supports a prefix plus a two-suffix
    // chain (native test affix::tests::a_prefix_with_a_suffix_chain). The old Indonesian
    // fixture's bermain + kan + lah therefore became a word, in these three cases only.
    // Keep the frozen historical file untouched and assert every explicit transition once.
    let mut expected = expected.to_string();
    let mut transitions = 0;
    for word in ["BERMAINKANLAH", "Bermainkanlah", "bermainkanlah"] {
        let before = format!("{word:?}: contains=false check=false suggest=false raw_suggest=false\n");
        let after = if word == "bermainkanlah" {
            format!("{word:?}: contains=true check=true suggest=true raw_suggest=true\n")
        } else {
            format!("{word:?}: contains=false check=true suggest=true raw_suggest=false\n")
        };
        assert_eq!(expected.matches(&before).count(), 1, "one historical observable: {word}");
        expected = expected.replacen(&before, &after, 1);
        transitions += 1;
    }
    assert_eq!(transitions, 3, "the documented upstream exception has exactly three observables");
    assert_eq!(actual, expected, "shared engine output changed beyond the explicit upstream chain exception");
}

#[test]
fn legacy_compact_data_is_refused_without_reinterpreting_homonyms() {
    let encoded = include_str!("fixtures/affix_compatibility.compact");
    let cases = fixtures::cases();
    let lines: Vec<_> = encoded.lines().collect();
    assert_eq!(lines.len(), cases.len());
    for (case, hex) in cases.iter().zip(lines) {
        assert_eq!(hex.len() % 2, 0);
        let bytes: Vec<_> =
            hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
        let raw = miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap();
        assert!(raw.starts_with(b"WCAFF1\n"), "fixture must contain WCAFF1 data: {}", case.name);
        assert!(AffixDict::from_bytes(&bytes).is_err(), "WCAFF1 must be regenerated as WCAFF2: {}", case.name);
    }
}
