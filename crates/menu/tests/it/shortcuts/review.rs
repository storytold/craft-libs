//! The import review: its choices re-plan the import and reach the app, and it draws only the lines in view.

use std::collections::BTreeMap;

use craft_keymap::import::kys;
use craft_keymap::{CommandRef, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Overrides, Platform};
use craft_menu::shortcuts::Outcome;
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{Rig, keymap, options};

const HEAD: &str = "<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/Mine=Mine\">";
const TAIL: &str = "</photoshop-keyboard-shortcuts>";

fn command(name: &str, keys: &str) -> String {
    format!("<command name=\"{name}\"><shortcut>{keys}</shortcut></command>")
}

fn set(commands: &[String]) -> ForeignSet {
    kys::read(&format!("{HEAD}{}{TAIL}", commands.concat())).expect("a set")
}

/// The command each foreign entry was matched to and the code each was given.
type Remembered = (BTreeMap<String, Option<CommandRef>>, BTreeMap<String, Option<String>>);

/// What the dialog last asked the app to keep.
fn answers(rig: &Rig) -> Option<Remembered> {
    rig.outcomes().into_iter().rev().find_map(|outcome| match outcome {
        Outcome::Remembered { targets, codes, .. } => Some((targets, codes)),
        _ => None,
    })
}

/// A set that matches Save, and two entries that match nothing.
fn review_rig() -> Rig {
    let mut km = keymap(Platform::Mac);
    let mut overrides = Overrides::new();
    overrides.insert("file.save", vec!["Cmd+Shift+S".into()]);
    km.set_overrides(overrides);
    let mut rig = Rig::new(km, options(), None);
    rig.review(set(&[command("Save", "Cmd+S"), command("Zorp", "Cmd+Q"), command("Blorp", "Cmd+J")]), ImportOptions::default());
    rig
}

/// Only the lines in view are drawn: a review of several hundred entries holds a few include boxes, not one per entry.
#[trial]
fn a_large_review_draws_only_the_rows_in_view() {
    let commands: Vec<String> = (0..400).map(|i| command(&format!("Entry {i}"), "Cmd+K")).collect();
    let mut rig = Rig::new(keymap(Platform::Mac), options(), None);
    rig.review(set(&commands), ImportOptions::default());
    let drawn = rig.names_with(Role::CHECK_BOX, "").len();
    assert!(drawn > 0, "the lines in view carry include boxes");
    assert!(drawn < 100, "{drawn} include boxes drawn for 400 entries");
}

/// Start, include and a picked target re-plan the import; a pick and No equivalent reach the app.
#[trial]
fn review_choices_replan_the_import() {
    let mut rig = review_rig();
    assert!(rig.has("Will change (1)"), "Save's own shortcut differs from the current one");

    // The start changes what the import is applied over, not which entries it takes, as in the production app.
    rig.click("Defaults");
    assert!(rig.has("Will change (1)"), "from the defaults Save still takes the set's key");
    rig.click("Current shortcuts");
    assert!(rig.has("Will change (1)"));

    rig.click("Include: Save");
    assert!(!rig.has("Will change (1)"), "a left-out entry is not imported");
    assert!(rig.has("Skipped (1)"), "and is listed as skipped");
    rig.click("Include: Save");
    assert!(rig.has("Will change (1)"));

    rig.choose("Command for Zorp", "Export…");
    assert!(rig.has("Will change (2)"), "the picked command takes Zorp's key");
    let (targets, _) = answers(&rig).expect("the pick is reported");
    assert_eq!(targets.get("zorp"), Some(&Some(CommandRef::from("file.export"))));

    rig.choose("Command for Blorp", "No equivalent");
    let (targets, _) = answers(&rig).expect("the answer is reported");
    assert_eq!(targets.get("blorp"), Some(&None), "No equivalent is kept as an answer");
    assert!(rig.has("Will change (2)"), "and imports nothing");
}

/// A key code both entries leave undecided is answered by recording a key or by No equivalent.
#[trial]
fn an_undecided_key_code_is_answered_by_recording_or_no_equivalent() {
    let coded = |name: &str| ForeignEntry::new(None, [name]).with_key(ForeignKey::code(None, "931", Some(931), None));
    let mut rig = Rig::new(keymap(Platform::Mac), options(), None);
    rig.review(ForeignSet::new(vec![coded("Save"), coded("Export")]), ImportOptions::default());
    assert!(rig.has("key:931 · Save, Export"), "the code is listed once with its entries");
    assert!(rig.has("Unreadable (2)"));

    rig.click("Record a key: key:931");
    rig.press(Modifiers::SHIFT, Key::ShiftLeft);
    assert!(rig.has("Unreadable (2)"), "a modifier press waits for the key");
    assert!(answers(&rig).is_none());

    rig.press(Modifiers::NONE, Key::K);
    assert!(!rig.has("Unreadable (2)"), "the entries are readable once the code is answered");
    let (_, codes) = answers(&rig).expect("the answer is reported");
    assert_eq!(codes.get("key:931"), Some(&Some("K".to_owned())));

    rig.click("Record a key: key:931");
    rig.press(Modifiers::NONE, Key::Escape);
    assert!(rig.is_open(), "Escape stops the recording, not the dialog");
    assert!(!rig.shared.borrow().state.is_recording());
    let (_, codes) = answers(&rig).expect("the answer is kept");
    assert_eq!(codes.get("key:931"), Some(&Some("K".to_owned())));

    rig.click("No equivalent");
    let (_, codes) = answers(&rig).expect("the answer is reported");
    assert_eq!(codes.get("key:931"), Some(&None));
}
