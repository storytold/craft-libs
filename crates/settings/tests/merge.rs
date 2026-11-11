//! `document::merge_changes` (gap 31).
use super::common;

use common::Prefs;
use craft_settings::document::{merge_changes, parse};

fn merged(base: &str, ours: &str, theirs: &str) -> String {
    let mut theirs = parse(theirs).unwrap();
    merge_changes::<Prefs>(&parse(base).unwrap(), &parse(ours).unwrap(), &mut theirs);
    theirs.to_string()
}

// A key only the other instance changed keeps its value; a key only we changed is written; when
// both changed one key, ours wins.
#[test]
fn only_our_changes_are_written() {
    let base = "schema_version = 2\n[general]\nlanguage = \"en\"\nautosave_minutes = 5\n[appearance]\nui_scale = 1.0\n";
    let ours = "schema_version = 2\n[general]\nlanguage = \"fr\"\nautosave_minutes = 5\n[appearance]\nui_scale = 1.0\n";
    let theirs = "schema_version = 2\n[general]\nlanguage = \"de\" # theirs\nautosave_minutes = 9\n[appearance]\nui_scale = 2.0\n[other]\nx = 1\n";
    let out = parse(&merged(base, ours, theirs)).unwrap();
    assert_eq!(out["general"]["language"].as_str(), Some("fr"), "both changed: ours wins");
    assert_eq!(out["general"]["autosave_minutes"].as_integer(), Some(9), "only theirs changed");
    assert_eq!(out["appearance"]["ui_scale"].as_float(), Some(2.0), "only theirs changed");
    assert_eq!(out["other"]["x"].as_integer(), Some(1), "unknown section kept");
    assert!(out.to_string().contains("language = \"fr\" # theirs"), "comment of the replaced line kept");

    let ours = "schema_version = 2\n[general]\nlanguage = \"en\"\nautosave_minutes = 5\ntheme_hint = \"x\"\n[appearance]\nui_scale = 1.0\n";
    let out = parse(&merged(base, ours, theirs)).unwrap();
    assert_eq!(out["general"]["theme_hint"].as_str(), Some("x"), "a key only we added is written");
    assert_eq!(out["general"]["language"].as_str(), Some("de"));
}

// A disk file at a newer schema migrates our base first, so our migration doesn't overwrite the
// other instance's choice.
#[test]
fn stale_migration_is_not_written_over() {
    // We loaded a version 1 file (`dark = true`) and migrated it in memory; another instance
    // already wrote version 2 and chose the light theme.
    let base = "schema_version = 1\n[appearance]\ndark = true\n";
    let ours = "schema_version = 2\n[appearance]\ntheme = \"dark\"\nui_scale = 1.5\n";
    let theirs = "schema_version = 2\n[appearance]\ntheme = \"light\"\n";
    let out = parse(&merged(base, ours, theirs)).unwrap();
    assert_eq!(out["appearance"]["theme"].as_str(), Some("light"), "their choice stays");
    assert_eq!(out["appearance"]["ui_scale"].as_float(), Some(1.5), "our own change is written");
}
