//! Integration tests of craft-design-lint: clippy template.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::path::{Path, PathBuf};

use craft_design_lint::{CLIPPY_TEMPLATE, ClippyDrift, DriftKind, Error, check_clippy, clippy_drift, clippy_shadows, merge_clippy};

fn origin() -> &'static Path {
    Path::new("clippy.toml")
}

/// Every `path` string in the template.
fn paths() -> Vec<String> {
    CLIPPY_TEMPLATE.lines().filter_map(|l| l.split_once("path = \"")?.1.split_once('"').map(|(p, _)| p.to_owned())).collect()
}

#[test]
fn check_clippy_detects_drift() {
    // The full block passes, with extra app entries and unrelated keys.
    let full = format!("allow-unwrap-in-tests = true\n{CLIPPY_TEMPLATE}");
    assert_eq!(clippy_drift(origin(), &full).unwrap(), []);
    let with_extra = merge_clippy(origin(), "disallowed-types = [{ path = \"my::Widget\", reason = \"mine\" }]\n").unwrap();
    assert_eq!(clippy_drift(origin(), &with_extra).unwrap(), []);

    // A missing entry yields exactly one finding naming it.
    let without: String = CLIPPY_TEMPLATE.lines().filter(|l| !l.contains("\"egui::Ui::button\"")).collect::<Vec<_>>().join("\n");
    let drift = clippy_drift(origin(), &without).unwrap();
    assert_eq!(drift, [ClippyDrift { key: "disallowed-methods", path: "egui::Ui::button".into(), kind: DriftKind::Missing }]);
    assert!(drift[0].to_string().contains("`egui::Ui::button`"));

    // A reworded reason is drift too.
    let reworded = CLIPPY_TEMPLATE.replace("use craft_ui::primitives::Divider", "use something else");
    let kinds: Vec<_> = clippy_drift(origin(), &reworded).unwrap().iter().map(|d| d.kind).collect();
    assert_eq!(kinds, [DriftKind::Differs, DriftKind::Differs]);

    // No file at all: everything is missing.
    let dir = tempdir("drift-none");
    let Err(Error::ClippyDrift(all)) = check_clippy(&dir) else { panic!("expected drift") };
    assert_eq!(all.len(), paths().len());
}

fn tempdir(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("craft-design-lint-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn another_clippy_toml_shadows_the_root_one() {
    let write = |root: &Path, rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    let root = tempdir("shadow");
    write(&root, "clippy.toml", CLIPPY_TEMPLATE);
    write(&root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    write(&root, "crates/a/Cargo.toml", "[package]\nname = \"a\"\n");
    write(&root, "crates/b/Cargo.toml", "[package]\nname = \"b\"\n");
    let file = root.join("clippy.toml");
    assert_eq!(clippy_shadows(&file, None).unwrap(), []);
    check_clippy(&root).unwrap();

    // A member crate's own clippy.toml, or a parent directory's, hides the root one from that crate.
    write(&root, "crates/a/clippy.toml", "");
    let shadows = clippy_shadows(&file, None).unwrap();
    assert_eq!(shadows.iter().map(|s| s.path.clone()).collect::<Vec<_>>(), [root.join("crates/a")]);
    assert!(shadows[0].to_string().contains("error[clippy/shadowed]"), "{}", shadows[0]);
    assert!(matches!(check_clippy(&root), Err(Error::ClippyShadowed(s)) if s.len() == 1));
    std::fs::remove_file(root.join("crates/a/clippy.toml")).unwrap();
    write(&root, "crates/.clippy.toml", "");
    assert_eq!(clippy_shadows(&file, None).unwrap().len(), 1);
    std::fs::remove_file(root.join("crates/.clippy.toml")).unwrap();

    // A clippy.toml in a directory without a package below it, in a nested workspace, or under target/ is not read for the root.
    write(&root, "docs/clippy.toml", "");
    write(&root, "vendor/Cargo.toml", "[workspace]\n");
    write(&root, "vendor/x/Cargo.toml", "[package]\nname = \"x\"\n");
    write(&root, "vendor/x/clippy.toml", "");
    write(&root, "target/y/Cargo.toml", "[package]\nname = \"y\"\n");
    write(&root, "target/y/clippy.toml", "");
    assert_eq!(clippy_shadows(&file, None).unwrap(), []);

    // CLIPPY_CONF_DIR: this directory (or a child without its own file) is fine; elsewhere is not.
    assert_eq!(clippy_shadows(&file, Some(root.as_os_str())).unwrap(), []);
    assert_eq!(clippy_shadows(&file, Some(root.join("crates/a").as_os_str())).unwrap(), []);
    let other = tempdir("shadow-other");
    write(&other, "clippy.toml", "");
    let shadows = clippy_shadows(&file, Some(other.as_os_str())).unwrap();
    assert_eq!(shadows.len(), 1);
    assert!(shadows[0].reason.contains("CLIPPY_CONF_DIR"), "{}", shadows[0].reason);
    let none = tempdir("shadow-none");
    assert_eq!(clippy_shadows(&file, Some(none.as_os_str())).unwrap().len(), 1);
}
