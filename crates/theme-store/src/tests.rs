#![cfg(not(target_arch = "wasm32"))]

use craft_ui::testing::trial;
use std::{fs, path::Path, time::Duration};

use craft_theme::{ContrastChoice, ModeChoice, MotionChoice, Selection, Size, TextSize, ThemeId, ThemePair};

use crate::{
    Accepted, Problem, SelectionChange, SelectionError, Store, StoreError, StoreOptions, SuiteDir,
    testing::{FailAt, FailingFs, SAMPLE_THEME_JSON},
};

fn options() -> StoreOptions {
    StoreOptions::new("test 1.0").lock_timeout(Duration::from_millis(100))
}

fn open(dir: &Path) -> Store {
    Store::open(&SuiteDir::at(dir), options())
}

fn id(s: &str) -> ThemeId {
    ThemeId::parse(s).unwrap()
}

/// `theme` in both slots, with the defaults for every field but the mode.
fn sel(theme: &str, mode: ModeChoice) -> Selection {
    Selection {
        theme: ThemePair::same(id(theme)),
        mode,
        contrast: ContrastChoice::System,
        motion: MotionChoice::System,
        size: Size::Compact,
        text_size: TextSize::Small,
    }
}

fn whole(theme: &str, mode: ModeChoice) -> SelectionChange {
    SelectionChange::whole(sel(theme, mode))
}

fn theme(s: &str) -> SelectionChange {
    SelectionChange::themes(ThemePair::same(id(s)))
}

/// A dark-only theme file with `extra` members after the id.
fn theme_json(id: &str, extra: &str) -> String {
    format!(
        r##"{{"schema":"craft-design-theme","schema_version":1,"id":"{id}","name":"{id}"{extra},"variants":"dark",
        "seeds":{{"accent":"#4b8bf5","neutral":"#26262a","status":{{"good":"#3fd6b8","warn":"#ffc14d","bad":"#ff6259","info":"#4b8bf5"}}}}}}"##
    )
}

#[trial]
fn a_newer_selection_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("selection.json");
    let last = Accepted::from_selection(sel("craft.graphite", ModeChoice::Light));
    let mut store = Store::open(&SuiteDir::at(tmp.path()), options().last_accepted(last.clone()));
    let future = r#"{"schema_version":3,"suite":{"theme":"craft.graphite","mode":"sepia"}}"#;
    fs::write(&path, future).unwrap();
    assert!(matches!(store.load_selection(), Err(Problem::Selection(SelectionError::TooNew { schema_version: 3 }))));
    assert!(store.is_read_only());
    assert_eq!(store.accepted(), &last);
    let refused = store.update(&last, whole("craft.graphite", ModeChoice::Dark));
    assert!(matches!(refused, Err(StoreError::Selection(SelectionError::TooNew { .. }))));
    assert!(store.import(SAMPLE_THEME_JSON.as_bytes(), false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), future);
    assert!(!tmp.path().join("library").exists());
}

#[trial]
fn a_damaged_selection_keeps_the_last_state_and_moves_aside_only_when_written() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("selection.json");
    let mut store = open(tmp.path());
    store.import(theme_json("one", "").as_bytes(), false).unwrap();
    let base = store.accepted().clone();
    let base = store.update(&base, whole("one", ModeChoice::Light)).unwrap();
    // Still JSON, so its counters are salvaged.
    let damaged = r#"{"schema_version":2,"revision":40,"library_revision":7,"suite":{"theme":5}}"#;
    fs::write(&path, damaged).unwrap();
    assert!(matches!(store.load_selection(), Err(Problem::Selection(SelectionError::Corrupt(_)))));
    assert_eq!(store.accepted(), &base);

    // Paths that write nothing leave it where it is: a change that matches, InUse, IdExists, a
    // failed write.
    store.update(&base, SelectionChange::mode(ModeChoice::Light)).unwrap();
    assert!(matches!(store.delete(&id("one")), Err(StoreError::InUse(_))));
    assert!(matches!(store.import(theme_json("one", "").as_bytes(), false), Err(StoreError::IdExists(_))));
    // Moved aside, then the write fails creating its temporary file: put back.
    let mut failing = Store::open(&SuiteDir::at(tmp.path()), options().file_system(Box::new(FailingFs(FailAt::Create))));
    assert!(failing.update(&base, SelectionChange::mode(ModeChoice::Dark)).is_err());
    assert!(failing.take_moved_aside().is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
    assert!(store.take_moved_aside().is_empty());

    let next = store.update(&base, SelectionChange::mode(ModeChoice::Dark)).unwrap();
    assert_eq!((next.revision, next.library_revision), (41, 7));
    assert_eq!(next.suite, sel("one", ModeChoice::Dark).into());
    let moved = store.take_moved_aside();
    assert_eq!(moved.len(), 1);
    assert_eq!(fs::read_to_string(&moved[0]).unwrap(), damaged);
    assert!(moved[0].file_name().unwrap().to_str().unwrap().starts_with("selection.broken-"));
}

#[trial]
fn ids_users_cant_create_never_reach_the_file_system() {
    let tmp = tempfile::tempdir().unwrap();
    let library = tmp.path().join("library");
    fs::create_dir_all(&library).unwrap();
    // On Windows this name opens the console device; elsewhere it is a plain file to ignore.
    fs::write(library.join("con.json"), theme_json("con", "")).unwrap();
    let mut store = open(tmp.path());
    let con = id("con");
    assert!(matches!(store.load_theme(&con), Err(StoreError::NotFound(_))));
    assert_eq!(store.theme_stamp(&con).unwrap(), None);
    assert!(matches!(store.export(&con), Err(StoreError::NotFound(_))));
    assert!(matches!(store.delete(&con), Err(StoreError::NotFound(_))));
    assert!(store.theme(&con).is_none());
    // A library theme that doesn't exist is never written as the suite's, in either slot.
    assert!(matches!(store.update(&Accepted::default(), theme("con")), Err(StoreError::NotFound(_))));
    assert!(matches!(store.update(&Accepted::default(), SelectionChange::theme_dark(id("gone"))), Err(StoreError::NotFound(_))));
    assert!(library.join("con.json").exists());
    assert!(!tmp.path().join("selection.json").exists());
}
