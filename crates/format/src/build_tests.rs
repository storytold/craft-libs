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

#[test]
fn a_step_is_a_declaration_not_a_mention() {
    assert!(!declares_step("//! No fn step() is needed.\n"));
    assert!(!declares_step("//! a note\n// fn step(v) was removed\nconst X: u8 = 1;\n"));
    assert!(declares_step("//! a note\npub fn step (v: &mut Value) -> Result<(), E> { Ok(()) }\n"));
    // a declaration inside a block comment, nested or not, is not one
    assert!(!declares_step("//! Documentation only.\n/*\nfn step() {}\n*/\n"));
    assert!(!declares_step("/* outer /* inner */\nfn step() {}\n*/\nconst X: u8 = 1;\n"));
    assert!(declares_step("/* a note */\nfn step(v: &mut Value) {}\n"));
    // comment markers inside strings and character literals are not comments
    assert!(declares_step("//! A change with a step.\nconst MARKER: &str = \"/*\";\npub fn step(_v: &mut Value) -> Result { Ok(()) }\n"));
    assert!(declares_step("const A: &str = r#\"/* \"quoted\" \"#;\nconst B: &[u8] = br\"/*\";\nfn step(v: &mut Value) {}\n"));
    assert!(declares_step("const Q: char = '\"';\nconst E: char = '\\'';\nfn step(v: &mut Value) { let _: &'static str = \"\"; }\n"));
    assert!(declares_step("const S: &str = \"a \\\" /* b\";\nfn step(v: &mut Value) {}\n"));
    assert!(!declares_step("const S: &str = \"\nfn step(v) \";\n"), "inside a string");
    // a comment between the words is whitespace
    assert!(declares_step("pub fn/* migration */step(v: &mut Value) {}\n"));
    // across lines
    assert!(declares_step("//! A change with a step.\npub fn step\n    (_v: &mut Value) -> Result {\n    Ok(())\n}\n"));
    assert!(declares_step("pub fn\nstep(v: &mut Value) {}\n"));
    assert!(!declares_step("fn stepper(v: &mut Value) {}\nfn prefn step() {}\n"), "other names");
    assert!(declares_step("const X: u8 = 1; // fn step() is below\nfn step(v: &mut Value) {}\n"));
    assert!(declares_step("//! a note\npub(crate) fn  step(v: &mut Value) -> Result<(), E> { Ok(()) }\n"));
    assert!(declares_step("//! a note\n    fn step(v: &mut Value) {}\n"));
    assert!(!declares_step("//! a note\nfn stepper(v: &mut Value) {}\nfn fnstep() {}\n"));
}

#[test]
fn a_documentation_only_change_has_no_step_and_a_missing_version_folder_fails() {
    let dir = scratch("doc-only");
    write(&dir.join("drawing/v4/doc.rs"), "//! No fn step() is needed.\n");
    let code = lists(&dir, &[DRAWING]).unwrap().code;
    assert!(code.contains("Part { name: \"doc\", step: None }"), "{code}");
    // a version folder that can't be listed is an error, not an empty format
    let missing = parts(&dir.join("drawing/v9"), "x", false, &mut String::new(), &mut String::new());
    assert!(missing.is_err());
    assert!(parts(&dir.join("drawing/pending"), "x", true, &mut String::new(), &mut String::new()).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}
