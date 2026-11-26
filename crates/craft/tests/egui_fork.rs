//! Integration tests of craft-craft: egui fork.

use craft::egui_fork::{GIT, PACKAGES, REV};

fn workspace_file(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn constants_equal_the_workspace_patch() {
    let manifest: toml::Table = workspace_file("Cargo.toml").parse().unwrap();
    let patch = manifest["patch"]["crates-io"].as_table().unwrap();
    let mut from_fork = Vec::new();
    for (name, entry) in patch {
        if entry.get("git").and_then(|g| g.as_str()) == Some(GIT) {
            assert_eq!(entry["rev"].as_str(), Some(REV), "[patch.crates-io] {name} is at another rev than craft::egui_fork::REV");
            from_fork.push(name.as_str());
        }
    }
    assert_eq!(from_fork, PACKAGES, "craft::egui_fork::PACKAGES differs from the packages [patch.crates-io] takes from GIT");
}
