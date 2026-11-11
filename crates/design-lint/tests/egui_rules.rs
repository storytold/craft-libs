//! Integration tests of craft-design-lint: the egui-ban and API-shape rules.
//!
//! Each rule has a reject fixture that must fail with the rule's id; rules with an "allowed in" location
//! also pass when the same source sits in that location.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::path::Path;

use craft_design_lint::{StaticScope, analyze_contract};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/egui-rules");
const ALLOWED_DIR: &str = "src/runtime/egui";

fn fixture(dir: &str, name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join(dir).join(name)).unwrap()
}

/// The scope that allows every egui call and target check in `runtime/egui`, and lists the crate as egui-linked.
fn scope() -> StaticScope {
    ["contract", "test/kit"]
        .into_iter()
        .fold(StaticScope::new(), |s, k| s.allow(k, ALLOWED_DIR))
        .allow("contract/platform:target", ALLOWED_DIR)
        .allow("contract/motion:clock", ALLOWED_DIR)
        .egui_linked("crates/x")
        .harness_free("crates/x/tests/it")
        .closed("Politeness")
}

fn rules_at(src: &str, path: &str, scope: &StaticScope) -> Vec<&'static str> {
    let mut rules: Vec<_> = analyze_contract(src, Path::new(path), scope, true).iter().map(|f| f.rule).collect();
    rules.dedup();
    rules
}

/// Reject fixtures: file, the rule it must trigger, the path it is checked at.
const REJECT: [(&str, &str, &str); 21] = [
    ("contract-egui-import.rs", "contract/egui-import", "src/ui/a.rs"),
    ("contract-allocate.rs", "contract/allocate", "src/ui/a.rs"),
    ("contract-egui-widget.rs", "contract/egui-widget", "src/ui/a.rs"),
    ("contract-access.rs", "contract/access", "src/ui/a.rs"),
    ("contract-layers.rs", "contract/layers", "src/ui/a.rs"),
    ("contract-layers-swallow.rs", "contract/layers", "src/ui/a.rs"),
    ("contract-memory.rs", "contract/memory", "src/ui/a.rs"),
    ("contract-keys.rs", "contract/keys", "src/ui/a.rs"),
    ("contract-pointer.rs", "contract/pointer", "src/ui/a.rs"),
    ("contract-motion.rs", "contract/motion", "src/ui/a.rs"),
    ("contract-motion-clock.rs", "contract/motion", "src/ui/a.rs"),
    ("contract-containers.rs", "contract/containers", "src/ui/a.rs"),
    ("contract-containers-scroll.rs", "contract/containers", "src/ui/a.rs"),
    ("contract-platform.rs", "contract/platform", "src/ui/a.rs"),
    ("test-layout.rs", "test/layout", "crates/x/tests/other.rs"),
    ("test-kit.rs", "test/kit", "crates/x/src/lib.rs"),
    ("test-harness.rs", "test/harness", "crates/x/tests/it/main.rs"),
    ("test-cfg-test.rs", "test/cfg-test", "crates/x/src/lib.rs"),
    ("test-literal-size.rs", "test/literal-size", "crates/x/tests/it/main.rs"),
    ("test-ignored.rs", "test/ignored", "src/ui/a.rs"),
    ("test-ignored-trial.rs", "test/ignored", "src/ui/a.rs"),
];

#[test]
fn every_reject_fixture_fails_with_its_rule() {
    for (file, rule, path) in REJECT {
        let rules = rules_at(&fixture("reject", file), path, &scope());
        assert!(rules.contains(&rule), "{file}: expected {rule}, got {rules:?}");
    }
    let rules = rules_at(&fixture("reject", "api-non-exhaustive.rs"), "src/lib.rs", &scope());
    assert_eq!(rules, ["api/non-exhaustive"]);
}

#[test]
fn allowed_location_passes_the_same_source() {
    let allowed_only = REJECT
        .iter()
        .filter(|(file, _, _)| !file.starts_with("test-") && !matches!(*file, "contract-layers-swallow.rs" | "contract-containers-scroll.rs"));
    for (file, rule, _) in allowed_only {
        let rules = rules_at(&fixture("reject", file), &format!("{ALLOWED_DIR}/a.rs"), &scope());
        assert!(!rules.contains(rule), "{file}: {rule} fired in the allowed location");
    }
    let rules = rules_at(&fixture("reject", "test-kit.rs"), &format!("{ALLOWED_DIR}/a.rs"), &scope());
    assert!(rules.is_empty(), "{rules:?}");
}

#[test]
fn family_modules_neither_import_runtime_egui_nor_build_egui_text() {
    let src = fixture("reject", "family-raw-egui.rs");
    let layered = StaticScope::new().layer("crates/x/src/text", 5).layer("crates/x/src/shell", 7);
    let findings = analyze_contract(&src, Path::new("crates/x/src/text/mod.rs"), &layered, false);
    let hits: Vec<_> = findings.iter().map(|f| (f.rule, f.line)).collect();
    assert_eq!(
        hits,
        [
            ("contract/runtime-egui", 1),
            ("contract/text-edit", 1),
            ("contract/egui-widget", 5),
            ("contract/egui-widget", 7),
            ("contract/egui-widget", 8),
            ("contract/runtime-egui", 9),
            ("contract/text-edit", 9),
        ]
    );
    let shell = analyze_contract(&src, Path::new("crates/x/src/shell/app.rs"), &layered, false);
    assert!(shell.iter().all(|f| f.rule != "contract/runtime-egui"), "layer 7 hosts the adapter");
}
