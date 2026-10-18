#![cfg(feature = "build")]

//! The build half: what is read from `contributors/`, and that the Rust it writes compiles against
//! this crate's types (`data/sample.rs` is that output, checked in and included here).
//! `CRAFT_CREDITS_BLESS=1 cargo test -p craft-credits --features build` rewrites it.

use std::path::{Path, PathBuf};

use craft_credits::build::{self, Person};
use craft_credits::{NameMode, SortKey};

mod sample {
    include!("data/sample.rs");
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

#[test]
fn the_written_rust_is_the_checked_in_sample() {
    let src = build::render(&build::load(&data("sample")));
    let golden = data("sample.rs");
    if std::env::var_os("CRAFT_CREDITS_BLESS").is_some() {
        std::fs::write(&golden, &src).unwrap();
    }
    assert_eq!(std::fs::read_to_string(golden).unwrap().replace("\r\n", "\n"), src);
}

#[test]
fn names_come_only_from_a_verified_entry() {
    let c = build::load(&data("sample"));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(c.total_commits, 120);
    let who = |login: &str| c.contributors.iter().find(|p| p.login == login).cloned().unwrap();
    // Given names win over the synced GitHub name, trimmed.
    assert_eq!((who("ada").display_name.as_deref(), who("ada").real_name.as_deref()), (Some("Ada"), Some("Ada Lovelace")));
    // An entry the JSON doesn't mark as verified gives no names.
    assert_eq!(who("unverified"), Person { display_name: None, real_name: None, ..who("unverified") });
    assert_eq!(who("synced").display_name.as_deref(), Some("Synced Name"));
    // Blank names (and a sync with no GitHub name) give none.
    assert_eq!((who("blank").display_name, who("blank").real_name), (None, None));
    assert_eq!(who("pr-only").first_commit, "");
    // Rows without a login are left out.
    assert_eq!(c.contributors.len(), 5);
    assert_eq!(c.models.len(), 1);
}

#[test]
fn the_included_tables_work_with_the_crate() {
    assert_eq!(sample::TOTAL_COMMITS, 120);
    let by_name: Vec<_> = craft_credits::sorted(sample::CONTRIBUTORS, NameMode::DisplayName, SortKey::Name, true)
        .iter()
        .map(|c| c.name(NameMode::DisplayName))
        .collect();
    assert_eq!(by_name, ["Ada", "@blank", "@pr-only", "Synced Name", "@unverified"]);
    assert_eq!(sample::MODELS[0].model, "Claude Opus");
}

#[test]
fn broken_or_missing_files_give_empty_tables_and_warnings() {
    for dir in [data("broken"), data("no-such-folder")] {
        let c = build::load(&dir);
        assert_eq!((c.total_commits, c.contributors.len(), c.models.len()), (0, 0, 0), "{}", dir.display());
        assert_eq!(c.warnings.len(), 2, "{:?}", c.warnings);
        // Cargo shows only the first line of a warning, so each is one line, fallback included.
        assert!(c.warnings.iter().all(|w| !w.contains(['\n', '\r'])), "{:?}", c.warnings);
        assert!(c.warnings[0].contains("contributors.json") && c.warnings[0].ends_with("the About window shows no contributors"));
        assert!(c.warnings[1].contains("people.toml") && c.warnings[1].ends_with("contributors are shown by username only"));
        // Still valid Rust.
        assert!(build::render(&c).contains("pub static CONTRIBUTORS: &[::craft_credits::Contributor] = &[\n];"));
    }
}

#[test]
fn a_broken_people_file_still_credits_everyone_by_username() {
    let dir = std::env::temp_dir().join(format!("craft-credits-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(data("sample/contributors.json"), dir.join("contributors.json")).unwrap();
    std::fs::copy(data("broken/people.toml"), dir.join("people.toml")).unwrap();
    let c = build::load(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(c.contributors.len(), 5);
    assert!(c.contributors.iter().all(|p| p.display_name.is_none() && p.real_name.is_none()));
    assert_eq!(c.warnings.len(), 1);
}
