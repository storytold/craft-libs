//! Integration tests of craft-design-lint: the contract-shape rules (`behavior/*`, `props/*`, `style/*`, `contract/entry`,
//! `contract/shadow`, `contract/widget`, `contract/overlay-flag`).
//!
//! Every rule has a reject fixture that must fail with the rule's id. The accept fixtures hold the nearest legal forms
//! and must produce no finding.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::path::Path;

use craft_design_lint::{Config, StaticScope, analyze_contract, check_with_scope};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/contract-rules");

fn fixture(dir: &str, name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join(dir).join(name)).unwrap()
}

/// Path of a component-family module (layer 5) and of a pattern module (layer 6).
const FAMILY: &str = "crates/x/src/chips/mod.rs";

fn scope() -> StaticScope {
    StaticScope::new().layer("crates/x/src/chips", 5).layer("crates/x/src/patterns", 6).layer("crates/x/src/shell", 7)
}

fn rules_at(src: &str, path: &str, scope: &StaticScope) -> Vec<&'static str> {
    let mut rules: Vec<_> = analyze_contract(src, Path::new(path), scope, false).iter().map(|f| f.rule).collect();
    rules.sort_unstable();
    rules.dedup();
    rules
}

/// Reject fixtures: file and the rules it must trigger (all of them, nothing else).
const REJECT: [(&str, &[&str]); 19] = [
    ("behavior-pipeline-only.rs", &["behavior/pipeline-only"]),
    ("props-reserved-setter.rs", &["props/reserved-setter"]),
    ("props-hand-setter.rs", &["props/hand-setter"]),
    ("props-own-field.rs", &["props/own-field"]),
    ("props-shadow.rs", &["props/shadow"]),
    ("props-point-size.rs", &["props/point-size"]),
    ("style-state-pick.rs", &["contract/pointer", "style/state-pick"]),
    ("style-match-state.rs", &["style/match-state"]),
    ("style-states-field.rs", &["style/states-field"]),
    ("style-size-arm.rs", &["style/size-arm"]),
    ("style-token-read.rs", &["style/token-read"]),
    ("style-instance-setter.rs", &["style/instance-setter"]),
    ("style-to-egui.rs", &["style/to-egui"]),
    ("contract-entry.rs", &["contract/entry"]),
    ("contract-shadow.rs", &["contract/shadow"]),
    ("contract-widget.rs", &["contract/widget"]),
    ("contract-overlay-flag.rs", &["contract/overlay-flag"]),
    ("contract-unsafe-allow.rs", &["contract/unsafe-allow"]),
    ("contract-registration.rs", &["contract/registration"]),
];

#[test]
fn every_reject_fixture_fails_with_exactly_its_rules() {
    for (file, expected) in REJECT {
        let rules = rules_at(&fixture("reject", file), FAMILY, &scope());
        assert_eq!(rules, expected, "{file}");
    }
}

#[test]
fn accept_fixtures_pass() {
    for file in ["behavior-pipeline-only.rs", "props.rs", "style.rs", "entry.rs"] {
        let findings = analyze_contract(&fixture("accept", file), Path::new(FAMILY), &scope(), false);
        assert!(findings.is_empty(), "{file}: {findings:?}");
    }
}

#[test]
fn type_facts_cross_files_of_a_crate() {
    let root = Path::new(FIXTURES).join("tree");
    let scope = scope().layer("src/chips", 5).crate_tree(&root, "fx", "src/lib.rs").unwrap();
    let report = check_with_scope(&Config::new(["src/chips"]), &root, &scope).unwrap();
    let hits: Vec<_> = report.findings.iter().map(|f| (f.path.display().to_string(), f.rule)).collect();
    assert_eq!(
        hits,
        [
            ("src/chips/builder.rs".to_owned(), "contract/shadow"),
            ("src/chips/builder.rs".to_owned(), "props/hand-setter"),
            ("src/chips/builder.rs".to_owned(), "contract/entry"),
            ("src/chips/component.rs".to_owned(), "contract/overlay-flag"),
        ]
    );
}
