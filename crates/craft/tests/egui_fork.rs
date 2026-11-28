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
    let winit = &patch["winit"];
    assert_eq!(winit["git"].as_str(), Some(WINIT_GIT), "[patch.crates-io] winit is from another repository than craft::egui_fork::WINIT_GIT");
    assert_eq!(winit["rev"].as_str(), Some(WINIT_REV), "[patch.crates-io] winit is at another rev than craft::egui_fork::WINIT_REV");
}

#[test]
fn the_workspace_lockfile_passes_the_check() {
    check_lockfile(&workspace_file("Cargo.lock")).unwrap();
}

fn lock_with_winit(source: &str) -> String {
    format!(
        "version = 4\n\n[[package]]\nname = \"egui\"\nversion = \"0.33.0\"\nsource = \"git+{GIT}?rev={REV}#{REV}\"\n\n\
         [[package]]\nname = \"winit\"\nversion = \"0.30.12\"\nsource = \"{source}\"\n"
    )
}

#[test]
fn winit_from_the_pinned_fork_passes() {
    check_lockfile(&lock_with_winit(&format!("git+{WINIT_GIT}?rev={WINIT_REV}#{WINIT_REV}"))).unwrap();
}

#[test]
fn winit_at_another_rev_or_from_crates_io_fails() {
    let other = "0000000000000000000000000000000000000000";
    for source in [format!("git+{WINIT_GIT}?rev={other}#{other}"), "registry+https://github.com/rust-lang/crates.io-index".to_string()] {
        let err = check_lockfile(&lock_with_winit(&source)).unwrap_err();
        assert!(err.contains(&format!("  winit 0.30.12: {source}")), "{err}");
    }
}
