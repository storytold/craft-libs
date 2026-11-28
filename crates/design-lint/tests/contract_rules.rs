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
const REJECT: [(&str, &[&str]); 20] = [
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
    ("contract-global-state.rs", &["contract/no-global-state"]),
];

#[test]
fn every_reject_fixture_fails_with_exactly_its_rules() {
    for (file, expected) in REJECT {
        let rules = rules_at(&fixture("reject", file), FAMILY, &scope());
        assert_eq!(rules, expected, "{file}");
    }
}

#[test]
fn unsafe_allow_is_accepted_only_in_the_ffi_crates() {
    let src = fixture("reject", "contract-unsafe-allow.rs");
    for path in ["crates/win32/src/lib.rs", "crates/ui-bench/src/alloc.rs"] {
        assert!(rules_at(&src, path, &scope()).is_empty(), "{path}");
    }
    assert_eq!(rules_at(&src, "crates/font-subset/src/lib.rs", &scope()), ["contract/unsafe-allow"]);
}

#[test]
fn accept_fixtures_pass() {
    for file in ["behavior-pipeline-only.rs", "props.rs", "style.rs", "entry.rs", "global-state.rs"] {
        let findings = analyze_contract(&fixture("accept", file), Path::new(FAMILY), &scope(), false);
        assert!(findings.is_empty(), "{file}: {findings:?}");
    }
}

#[test]
fn type_facts_cross_files_of_a_crate() {
    let root = Path::new(FIXTURES).join("tree");
    let scope = scope().layer("src/chips", 5).crate_tree(&root, "fx", "src/lib.rs").unwrap();
    let report = check_with_scope(&Config::new(["src/chips"]), &root, &scope).unwrap();
    let hits: Vec<_> = report.findings.iter().map(|f| (f.path.display().to_string().replace('\\', "/"), f.rule)).collect();
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

#[test]
fn global_state_counts_every_form_and_honours_the_allowlist() {
    let src = fixture("reject", "contract-global-state.rs");
    let found = analyze_contract(&src, Path::new(FAMILY), &scope(), false);
    // static, static mut, static OnceLock, thread_local!, impl MemoProps, a static in a fn, `use std::fs`, `std::fs::read`,
    // and the two derive-only forms: impl PlainData and a direct `__split_for_bind`.
    assert_eq!(found.iter().filter(|f| f.rule == "contract/no-global-state").count(), 10, "{found:?}");
    // An entry names one item; the rest of the file is still flagged. The three derive-only forms have no entry.
    let mut allowed = StaticScope::new();
    for what in ["COUNTER", "RAW", "CACHE", "LOCAL", "INNER", "std::fs"] {
        allowed = allowed.allow(&format!("contract/no-global-state:{what}"), "crates/x/src/chips");
    }
    let left = analyze_contract(&src, Path::new(FAMILY), &allowed, false);
    assert_eq!(left.len(), 3, "{left:?}");
    let some = StaticScope::new().allow("contract/no-global-state:COUNTER", "crates/x/src/chips");
    let left = analyze_contract(&src, Path::new(FAMILY), &some, false);
    assert_eq!(left.len(), 9, "a second static in an allowlisted file is flagged: {left:?}");
    for skipped in ["crates/x/build.rs", "crates/x/src/bin/tool.rs"] {
        assert!(rules_at(&src, skipped, &scope()).is_empty(), "{skipped}");
    }
}
