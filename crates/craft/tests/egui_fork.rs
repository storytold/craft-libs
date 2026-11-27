//! Integration tests of craft-craft: egui fork.

use craft::egui_fork::{GIT, PACKAGES, REV, WINIT_GIT, WINIT_REV, check_lockfile};

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

#[test]
fn winit_constants_equal_the_workspace_patch() {
    let manifest: toml::Table = workspace_file("Cargo.toml").parse().unwrap();
    let winit = &manifest["patch"]["crates-io"]["winit"];
    assert_eq!(
        (winit["git"].as_str(), winit["rev"].as_str()),
        (Some(WINIT_GIT), Some(WINIT_REV)),
        "[patch.crates-io] winit differs from craft::egui_fork"
    );
}

#[test]
fn a_lockfile_with_crates_io_winit_is_refused() {
    let egui = format!("[[package]]\nname = \"egui\"\nversion = \"0.1.0\"\nsource = \"git+{GIT}?rev={REV}#{REV}\"\n");
    let winit = |source: &str| format!("{egui}\n[[package]]\nname = \"winit\"\nversion = \"0.30.0\"\nsource = \"{source}\"\n");
    assert!(check_lockfile(&winit(&format!("git+{WINIT_GIT}?rev={WINIT_REV}#{WINIT_REV}"))).is_ok());
    let err = check_lockfile(&winit("registry+https://github.com/rust-lang/crates.io-index")).expect_err("crates.io winit is refused");
    assert!(err.contains("winit 0.30.0"), "{err}");
}
