//! Integration tests of craft-design-lint: the structure checks.
//!
//! Each directory under `tests/fixtures/structure` is a small repository with a `metadata.json` (the
//! `cargo metadata --no-deps` output), sources, and `design/` files. `expect` lists the rule ids the
//! run must report, one per line; the accept fixture has none. A rule that stops firing fails its fixture.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::collections::BTreeSet;
use std::path::Path;

use craft_design_lint::{Metadata, check_structure};

fn run(case: &str) -> (BTreeSet<String>, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/structure").join(case);
    let json = std::fs::read_to_string(root.join("metadata.json")).unwrap();
    let findings = check_structure(&root, &Metadata::parse(&root, &json).unwrap(), &[]).unwrap();
    let listing = findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
    (findings.iter().map(|f| f.rule.to_owned()).collect(), listing)
}

fn expected(case: &str) -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/structure").join(case);
    std::fs::read_to_string(root.join("expect")).unwrap().lines().map(str::to_owned).collect()
}

#[test]
fn every_fixture_reports_exactly_its_expected_rules() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/structure");
    let mut cases: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    cases.sort();
    assert!(cases.iter().any(|c| c == "accept"));
    let mut failures = Vec::new();
    for case in &cases {
        let (got, listing) = run(case);
        if got != expected(case) {
            failures.push(format!("{case}: expected {:?}, got {got:?}\n{listing}", expected(case)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
