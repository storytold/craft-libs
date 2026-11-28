//! The design checks over this repository's mapped crates: structure and egui-ban rules, zero findings.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::path::{Path, PathBuf};

use craft_design_lint::{Config, Metadata, check_workspace};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn mapped_crates_have_no_findings() {
    let root = root();
    let config = Config::load(&root.join("design-lint.toml")).unwrap();
    let findings = check_workspace(&root, &config, &Metadata::run(&root).unwrap()).unwrap();
    let listing = findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
    assert!(findings.is_empty(), "{listing}");
}
