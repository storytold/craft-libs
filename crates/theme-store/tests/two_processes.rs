//! Two processes updating different fields of the suite selection at once: every update lands
//! and neither overwrites the other's field. The test binary runs itself as the two children.
#![cfg(not(target_arch = "wasm32"))]

use std::{path::Path, process::Command};

use craft_theme::{Mode, Selection, ThemeId};
use craft_theme_store::{Accepted, SelectionChange, Store, StoreError, StoreOptions, SuiteDir};

const ROLE: &str = "CRAFT_THEME_STORE_TEST_ROLE";
const DIR: &str = "CRAFT_THEME_STORE_TEST_DIR";
const UPDATES: u64 = 200;
const THEMES: [&str; 2] = ["craft.graphite", "craft.high-contrast"];
const MODES: [Mode; 2] = [Mode::Light, Mode::Dark];

#[test]
fn two_processes_keep_every_update() {
    if let (Ok(role), Ok(dir)) = (std::env::var(ROLE), std::env::var(DIR)) {
        child(&role, Path::new(&dir));
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Both children start from an existing selection, so neither takes the no-base path.
    let mut store = Store::open(&SuiteDir::at(tmp.path()), StoreOptions::new("parent"));
    store
        .update(&Accepted::default(), SelectionChange::whole(Selection { theme: ThemeId::parse("craft.artcraft").unwrap(), mode: Mode::System }))
        .unwrap();

    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = ["theme", "mode"]
        .into_iter()
        .map(|role| {
            Command::new(&exe)
                .args(["--exact", "two_processes_keep_every_update", "--test-threads=1", "--nocapture"])
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
    assert_eq!(accepted.revision, 1 + 2 * UPDATES);
    assert_eq!(accepted.suite, craft_theme_store::SuiteChoice::from(Selection { theme: ThemeId::parse(THEMES[last]).unwrap(), mode: MODES[last] }));
}

/// Each update changes one field relative to this child's own last write, which is stale for
/// the other child's field: the merge has to keep the disk value there.
fn child(role: &str, dir: &Path) {
    let mut store = Store::open(&SuiteDir::at(dir), StoreOptions::new(format!("child {role}")));
    store.load_selection().unwrap();
    while !dir.join("go").exists() {
        std::thread::yield_now();
    }
    for i in 0..UPDATES as usize {
        let base = store.accepted().clone();
        let mine = match role {
            "theme" => SelectionChange::theme(ThemeId::parse(THEMES[i % 2]).unwrap()),
            _ => SelectionChange::mode(MODES[i % 2]),
        };
        loop {
            match store.update(&base, mine.clone()) {
                Ok(_) => break,
                Err(StoreError::Busy) => continue,
                Err(e) => panic!("{role} update {i}: {e}"),
            }
        }
    }
}
