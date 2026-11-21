//! Three processes writing the suite folder at once, two updating different fields of the suite
//! selection and one saving over a library theme: every write lands, neither update overwrites
//! the other's field, and another process reads the last save. The test binary runs itself as
//! the three children.
#![cfg(not(target_arch = "wasm32"))]

use craft_ui::testing::trial;
use std::{path::Path, process::Command};

use craft_theme::{ModeChoice, SuiteChoice, ThemeId, ThemePair};
use craft_theme_store::{Accepted, SelectionChange, Store, StoreError, StoreOptions, SuiteDir};

const ROLE: &str = "CRAFT_THEME_STORE_TEST_ROLE";
const DIR: &str = "CRAFT_THEME_STORE_TEST_DIR";
const UPDATES: u64 = 200;
const THEMES: [&str; 2] = ["craft.graphite", "craft.studio"];
const MODES: [ModeChoice; 2] = [ModeChoice::Light, ModeChoice::Dark];
const SAVED: &str = "saved";

fn both(theme: &str) -> SelectionChange {
    SelectionChange::themes(ThemePair::same(ThemeId::parse(theme).unwrap()))
}

/// The library theme the save child writes, at `version`.
fn saved_file(version: u64) -> String {
    format!(
        r##"{{"schema":"craft-design-theme","schema_version":1,"id":"{SAVED}","name":"Saved","version":{version},"variants":"dark",
        "seeds":{{"accent":"#4b8bf5","neutral":"#26262a","status":{{"good":"#3fd6b8","warn":"#ffc14d","bad":"#ff6259","info":"#4b8bf5"}}}}}}"##
    )
}

#[trial]
fn two_processes_keep_every_update() {
    if let (Ok(role), Ok(dir)) = (std::env::var(ROLE), std::env::var(DIR)) {
        child(&role, Path::new(&dir));
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Both children start from an existing selection, so neither takes the no-base path.
    let mut store = Store::open(&SuiteDir::at(tmp.path()), StoreOptions::new("parent"));
    store.update(&Accepted::default(), both("craft.artcraft").then(SelectionChange::mode(ModeChoice::System))).unwrap();
    store.import(saved_file(1).as_bytes(), false).unwrap();

    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = ["theme", "mode", "save"]
        .into_iter()
        .map(|role| {
            Command::new(&exe)
                .args(["--exact", "two_processes::two_processes_keep_every_update", "--test-threads=1", "--nocapture"])
                .env(ROLE, role)
                .env(DIR, tmp.path())
                .spawn()
                .unwrap()
        })
        .collect();
    // Released together, so their updates interleave.
    std::fs::write(tmp.path().join("go"), "").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    store.load_selection().unwrap();
    let last = (UPDATES - 1) as usize % 2;
    let accepted = store.accepted();
    assert_eq!((accepted.revision, accepted.library_revision), (2 + 3 * UPDATES, 1 + UPDATES));
    let theme = Some(ThemeId::parse(THEMES[last]).unwrap());
    assert_eq!(accepted.suite, SuiteChoice { theme_light: theme.clone(), theme_dark: theme, mode: Some(MODES[last]), ..SuiteChoice::default() });
    let saved = store.load_theme(&ThemeId::parse(SAVED).unwrap()).unwrap();
    assert_eq!(&*saved.bytes, saved_file(1 + UPDATES).as_bytes());
}

/// Each update changes one field relative to this child's own last write, which is stale for
/// the other child's field: the merge has to keep the disk value there. Each save goes from the
/// stamp of the child's own last save.
fn child(role: &str, dir: &Path) {
    let mut store = Store::open(&SuiteDir::at(dir), StoreOptions::new(format!("child {role}")));
    store.load_selection().unwrap();
    let saved = ThemeId::parse(SAVED).unwrap();
    let mut stamp = store.theme_stamp(&saved).unwrap().unwrap();
    while !dir.join("go").exists() {
        std::thread::yield_now();
    }
    for i in 0..UPDATES as usize {
        let base = store.accepted().clone();
        loop {
            let written = match role {
                "theme" => store.update(&base, both(THEMES[i % 2])).map(drop),
                "mode" => store.update(&base, SelectionChange::mode(MODES[i % 2])).map(drop),
                _ => store.save_theme(&saved, saved_file(i as u64 + 2).as_bytes(), stamp).map(|s| stamp = s.stamp),
            };
            match written {
                Ok(()) => break,
                Err(StoreError::Busy) => continue,
                Err(e) => panic!("{role} write {i}: {e}"),
            }
        }
    }
}
