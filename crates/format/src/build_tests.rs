#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

/// A fresh, empty folder for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-format-build-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The error of a run that must fail.
fn refused(dir: &Path) -> String {
    match lists(dir, &[DRAWING]) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

const DRAWING: Family = Family { folder: "drawing", base: 3, list_prefix: "DRAWING_" };

#[test]
fn lists_folded_and_pending_changes_with_their_modules() {
    let dir = scratch("ok");
    write(&dir.join("drawing/v4/add-layers.rs"), "//! Layers.\npub fn step(v: &mut Value) -> Result<(), E> { Ok(()) }\n");
    write(&dir.join("drawing/v4/notes.txt"), "not a change");
    write(&dir.join("drawing/v5/rename.rs"), "\n//! A rename with no step.\n");
    write(&dir.join("drawing/pending/next-1.rs"), "//! Pending.\nfn step(v: &mut Value) {}\n");
    std::fs::create_dir_all(dir.join("drawing/v6x")).unwrap();
    write(&dir.join("drawing/v7"), "a file, not a version folder");
    let lists = lists(&dir, &[DRAWING]).unwrap();
    assert_eq!(lists.watch, vec![dir.join("drawing")]);
    let code = &lists.code;
    assert!(code.contains("const DRAWING_FOLDED: &[(u32, &[Part])] = &["), "{code}");
    assert!(code.contains("    (4, &[\n        Part { name: \"add-layers\", step: Some(drawing_v4_add_layers::step) },\n    ]),"), "{code}");
    assert!(code.contains("Part { name: \"rename\", step: None }"), "{code}");
    assert!(
        code.contains("const DRAWING_PENDING: &[Part] = &[\n        Part { name: \"next-1\", step: Some(drawing_pending_next_1::step) },"),
        "{code}"
    );
    assert!(code.contains("mod drawing_v5_rename;"), "{code}");
    assert!(!code.contains("notes"), "{code}");
    assert!(code.find("const DRAWING_PENDING").unwrap() < code.find("#[path").unwrap(), "modules follow the lists");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_family_has_empty_lists() {
    let dir = scratch("empty");
    std::fs::create_dir_all(dir.join("drawing")).unwrap();
    let code = lists(&dir, &[DRAWING]).unwrap().code;
    assert!(code.contains("const DRAWING_FOLDED: &[(u32, &[Part])] = &[\n];"), "{code}");
    assert!(code.contains("const DRAWING_PENDING: &[Part] = &[\n];"), "{code}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refuses_gaps_bad_names_missing_notes_and_missing_folders() {
    let dir = scratch("bad");
    assert!(refused(&dir).contains("drawing"));
    write(&dir.join("drawing/v5/a.rs"), "//! a");
    assert!(refused(&dir).contains("expected v4/ next, found v5/"));
    std::fs::rename(dir.join("drawing/v5"), dir.join("drawing/v4")).unwrap();
    write(&dir.join("drawing/v4/Bad_Name.rs"), "//! b");
    assert!(refused(&dir).contains("lowercase letters"));
    std::fs::remove_file(dir.join("drawing/v4/Bad_Name.rs")).unwrap();
    write(&dir.join("drawing/pending/no-note.rs"), "pub fn step() {}");
    assert!(refused(&dir).contains("`//!` note"));
    let _ = std::fs::remove_dir_all(&dir);
}
