//! `document::merge_changes` (gap 31).
use super::common;

use common::{Prefs, open_in, read_config, write_config};
use craft_settings::document::{merge_changes, parse};
use craft_settings::{MergeRule, Settings};
use serde::{Deserialize, Serialize};

fn merged(base: &str, ours: &str, theirs: &str) -> String {
    merged_as::<Prefs>(base, ours, theirs)
}

fn merged_as<T: Settings>(base: &str, ours: &str, theirs: &str) -> String {
    let mut theirs = parse(theirs).unwrap();
    merge_changes::<T>(&parse(base).unwrap(), &parse(ours).unwrap(), &mut theirs);
    theirs.to_string()
}

/// The merged file as [`Ruled`] reads it; a list entry the type cannot read fails the read.
fn merged_ruled(base: &str, ours: &str, theirs: &str) -> Ruled {
    toml_edit::de::from_document(parse(&merged_as::<Ruled>(base, ours, theirs)).unwrap()).unwrap()
}

/// How many entries `recent` keeps.
const RECENT_CAP: usize = 4;

/// Settings whose items name their own merge rules, as an app's recent files, AutoCorrect entries and window are.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct Ruled {
    recent: Vec<String>,
    /// `[from, to]` pairs.
    autocorrect: Vec<Vec<String>>,
    window: Window,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct Window {
    x: i64,
    width: i64,
}

impl Settings for Ruled {
    const APP_ID: &'static str = "ruledcraft";
    const SCHEMA_VERSION: u32 = 1;

    fn merge_rule(path: &[&str]) -> MergeRule {
        match path {
            ["recent"] => MergeRule::Recent { cap: RECENT_CAP, key: text },
            ["autocorrect"] => MergeRule::Entries { key: replaced_text },
            ["window"] => MergeRule::Whole,
            _ => MergeRule::ByKey,
        }
    }
}

/// A recent entry by its text.
fn text(v: &toml_edit::Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

/// An AutoCorrect pair by the text it replaces.
fn replaced_text(v: &toml_edit::Value) -> Option<String> {
    v.as_array()?.get(0)?.as_str().map(str::to_owned)
}

fn pairs(list: &[[&str; 2]]) -> Vec<Vec<String>> {
    list.iter().map(|pair| pair.iter().map(|s| (*s).to_owned()).collect()).collect()
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

// Two instances open a file without the section they then each change one key of: each keeps the other's key, as a
// section the file lacked holds its defaults, not this instance's change.
#[test]
fn a_section_the_file_lacked_merges_only_the_keys_each_instance_changed() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n");
    let (mut first, _) = open_in(tmp.path());
    let (mut second, _) = open_in(tmp.path());
    first.update(|p| p.general.language = "fr".into());
    second.update(|p| p.general.autosave_minutes = 9);
    first.flush().unwrap();
    second.flush().unwrap();
    let (reopened, _) = open_in(tmp.path());
    let general = &reopened.get().general;
    assert_eq!((general.language.as_str(), general.autosave_minutes), ("fr", 9), "{}", read_config(tmp.path()));
}

/// A file holding `recent = <list>`.
fn recent(list: &[&str]) -> String {
    format!("schema_version = 1\nrecent = {list:?}\n")
}

/// A file holding `autocorrect = <pairs>`.
fn autocorrect(list: &[[&str; 2]]) -> String {
    format!("schema_version = 1\nautocorrect = {list:?}\n")
}

// Two instances share a recent list: neither may lose the other's entries, and what this one opened or used again
// leads.
#[test]
fn a_recent_list_moves_used_entries_forward_and_honours_removals_and_the_cap() {
    // Here `c` was used again, `d` opened and `a` removed; the other instance used `b` again and opened `e`.
    let out = merged_ruled(&recent(&["a", "b", "c"]), &recent(&["d", "c", "b"]), &recent(&["e", "b", "a", "c"]));
    assert_eq!(out.recent, ["d", "c", "e", "b"]);

    // Each instance opened one file into a full list: both new entries lead, cut to the cap.
    let out = merged_ruled(&recent(&["a", "b", "c", "d"]), &recent(&["x", "a", "b", "c"]), &recent(&["y", "a", "b", "c"]));
    assert_eq!(out.recent, ["x", "y", "a", "b"]);
    assert_eq!(out.recent.len(), RECENT_CAP);

    // Clearing the list here removes what this instance had loaded, not what the other one added since.
    let out = merged_ruled(&recent(&["a", "b"]), &recent(&[]), &recent(&["c", "a", "b"]));
    assert_eq!(out.recent, ["c"]);
}

// Two instances edit one list of user entries: every addition survives, every removal holds, a change made here wins,
// and an added entry lands after the entry it was added behind, not at the end.
#[test]
fn entries_from_both_instances_are_kept_with_removals_and_anchored_insertion() {
    // Here: `teh` changed, `adn` added after it, `hte` removed. There: `teh2` added, `recieve` removed.
    let base = autocorrect(&[["teh", "the"], ["hte", "the"], ["recieve", "receive"]]);
    let ours = autocorrect(&[["teh", "THE"], ["adn", "and"], ["recieve", "receive"]]);
    let theirs = autocorrect(&[["teh", "the"], ["hte", "the"], ["teh2", "the"]]);
    let out = merged_ruled(&base, &ours, &theirs);
    assert_eq!(out.autocorrect, pairs(&[["teh", "THE"], ["adn", "and"], ["teh2", "the"]]));

    // Entries added on each side keep their place among the shared ones: `zz` before everything, `ww` between `teh`
    // and `hte`, `yy` after `hte`.
    let base = autocorrect(&[["teh", "the"], ["hte", "the"]]);
    let ours = autocorrect(&[["zz", "z"], ["teh", "the"], ["hte", "the"], ["yy", "y"]]);
    let theirs = autocorrect(&[["teh", "the"], ["ww", "w"], ["hte", "the"]]);
    let out = merged_ruled(&base, &ours, &theirs);
    assert_eq!(out.autocorrect, pairs(&[["zz", "z"], ["teh", "the"], ["ww", "w"], ["hte", "the"], ["yy", "y"]]));
}

// A window moved here and resized there must come out as one instance's window, never this one's place with the
// other's size, which neither instance ever held.
#[test]
fn a_whole_rule_table_is_replaced_not_mixed() {
    let base = "schema_version = 1\n[window]\nx = 0\nwidth = 800\n";
    let ours = "schema_version = 1\n[window]\nx = 300\nwidth = 800\n";
    let theirs = "schema_version = 1\n[window]\nx = 0\nwidth = 1000\nextra = 1\n";
    let text = merged_as::<Ruled>(base, ours, theirs);
    let out = parse(&text).unwrap();
    assert_eq!((out["window"]["x"].as_integer(), out["window"]["width"].as_integer()), (Some(300), Some(800)), "{text}");
    assert!(out["window"].get("extra").is_none(), "no key of theirs survives: {text}");

    // Left alone here while another setting changed, the other instance's window stays whole, keys and all.
    let ours = "schema_version = 1\nrecent = [\"a\"]\n[window]\nx = 0\nwidth = 800\n";
    let text = merged_as::<Ruled>(base, ours, theirs);
    let out = parse(&text).unwrap();
    assert_eq!((out["window"]["x"].as_integer(), out["window"]["width"].as_integer()), (Some(0), Some(1000)), "{text}");
    assert_eq!(out["window"]["extra"].as_integer(), Some(1), "{text}");
}

// A hand edit or an older build can leave entries the type cannot read; carried into the merge they would make the
// whole list read as its default in every later session.
#[test]
fn a_bad_list_entry_is_dropped_not_carried() {
    let out = merged_ruled(&recent(&[]), &recent(&["x"]), "schema_version = 1\nrecent = [{ path = \"y\" }, \"z\", 7]\n");
    assert_eq!(out.recent, ["x", "z"]);

    // A list with no entry that has a key carries nothing, and a value that is not a list gives way to ours.
    let out = merged_ruled(&recent(&[]), &recent(&["x"]), "schema_version = 1\nrecent = [1, 2]\n");
    assert_eq!(out.recent, ["x"]);
    let out = merged_ruled(&recent(&[]), &recent(&["x"]), "schema_version = 1\nrecent = 3\n");
    assert_eq!(out.recent, ["x"]);

    let out = merged_ruled(
        &autocorrect(&[]),
        &autocorrect(&[["teh", "the"]]),
        "schema_version = 1\nautocorrect = [[\"ok\", \"fine\"], \"bare\", [3, 4]]\n",
    );
    assert_eq!(out.autocorrect, pairs(&[["teh", "the"], ["ok", "fine"]]));
}
