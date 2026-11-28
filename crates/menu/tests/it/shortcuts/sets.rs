//! The set row: Cancel with sets, a delete prompt that outlives its set, export and import.

use craft_keymap::{Overrides, Profiles};
use craft_menu::shortcuts::Outcome;
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{Rig, builtins, keymap, options};

fn profiles_with(names: &[&str]) -> Profiles {
    let mut profiles = Profiles::default();
    for name in names {
        profiles.save_as(name, Overrides::new(), &builtins()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
    profiles
}

fn new_set(rig: &mut Rig, name: &str) {
    rig.action("New Set…");
    let field = rig.named("Name:", Role::TEXT_INPUT);
    rig.fixture.click(&field);
    rig.fixture.type_text(name);
    rig.fixture.idle(2);
    rig.click("Create");
}

/// Cancel restores the keymap and the sets as they were at open, also after a set was made and deleted.
#[trial]
fn cancel_restores_the_keymap_and_the_sets() {
    let before = profiles_with(&["Mine"]);
    let mut rig = Rig::mac_with_sets(before.clone());
    rig.click("Save: ⌘S");
    rig.press(Modifiers::NONE, Key::F2);
    new_set(&mut rig, "Other");
    assert_eq!(rig.active_set().as_deref(), Some("Other"));
    assert_eq!(rig.bindings("file.save"), ["F2"], "based on the current shortcuts");
    rig.click("Delete");
    assert!(rig.has("Delete the set “Other”?"));
    rig.click("Cancel");
    rig.click("Cancel");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Cancel)));
    let shared = rig.shared.borrow();
    assert_eq!(shared.sets.as_ref().map(|(p, _)| p), Some(&before));
    assert!(shared.km.overrides().is_empty());
}

/// A delete prompt that outlives its set's activity (the app made another set active meanwhile) deletes the set it
/// names and keeps the edits of the active set.
#[trial]
fn a_stale_delete_prompt_neither_reloads_nor_drops_edits() {
    let mut rig = Rig::mac_with_sets(profiles_with(&["Mine", "Other"]));
    rig.click("Save: ⌘S");
    rig.press(Modifiers::NONE, Key::F2);
    rig.click("Delete");
    assert!(rig.has("Delete the set “Other”?"));
    {
        let mut shared = rig.shared.borrow_mut();
        let (profiles, _) = shared.sets.as_mut().expect("the rig has sets");
        profiles.select("Mine", &builtins()).expect("Mine exists");
    }
    rig.fixture.idle(2);
    rig.answer("Delete");
    let shared = rig.shared.borrow();
    let profiles = shared.sets.as_ref().map(|(profiles, _)| profiles).expect("the rig has sets");
    assert!(!profiles.sets().contains_key("Other"));
    assert_eq!(profiles.active(), "Mine");
    drop(shared);
    assert_eq!(rig.bindings("file.save"), ["F2"], "the active set keeps its edits");
}

/// A set exported from one dialog imports into another as a new set with the same shortcuts; a file that is not a set
/// is refused; without sets the import replaces the keymap's overrides as one undo step.
#[trial]
fn set_export_import_round_trip() {
    let mut rig = Rig::mac_with_sets(Profiles::default());
    new_set(&mut rig, "Name");
    rig.click("Save: ⌘S");
    rig.press(Modifiers::NONE, Key::F2);
    rig.click("Export…");
    let (name, json) = rig
        .outcomes()
        .into_iter()
        .find_map(|o| if let Outcome::Export { name, json, .. } = o { Some((name, json)) } else { None })
        .unwrap_or_else(|| panic!("Outcome::Export"));
    assert_eq!(name, "Name");
    let edited = rig.shared.borrow().km.overrides().clone();
    let profiles = rig.shared.borrow().sets.as_ref().map(|(p, _)| p.clone());

    let mut fresh = Rig::new(keymap(craft_keymap::Platform::Mac), options(), profiles);
    assert_eq!(fresh.import_json(&json), Ok("Name 2".into()));
    assert_eq!(fresh.active_set().as_deref(), Some("Name 2"));
    assert_eq!(fresh.shared.borrow().km.overrides(), &edited);
    assert!(fresh.import_json("{}").is_err());
    assert!(fresh.has("This is not a shortcut set file") || fresh.has("This is not a craft-keymap file"));

    let mut plain = Rig::mac();
    assert!(plain.import_json(&json).is_ok());
    assert_eq!(plain.shared.borrow().km.overrides(), &edited);
    plain.click("Undo");
    assert!(plain.shared.borrow().km.overrides().is_empty());
}

/// Switching away from a set with unsaved edits asks to save them first, and the set row waits while the question is
/// open: Cancel keeps the set and its edits, Don't Save switches and leaves the set as stored, Save stores the edits
/// and switches. An unmodified set switches with no question.
#[trial]
fn switching_away_from_a_modified_set_asks_to_save_first() {
    let mut profiles = profiles_with(&["Mine", "Other"]);
    profiles.select("Mine", &builtins()).expect("Mine exists");
    let mut rig = Rig::mac_with_sets(profiles);
    let stored = |rig: &Rig| {
        let shared = rig.shared.borrow();
        shared.sets.as_ref().and_then(|(profiles, _)| profiles.sets().get("Mine").cloned()).expect("Mine is stored")
    };
    let question = "Save changes to “Mine”?";
    let edit = |rig: &mut Rig| {
        rig.click("Save: ⌘S");
        rig.press(Modifiers::NONE, Key::F2);
        assert_eq!(rig.bindings("file.save"), ["F2"]);
    };

    edit(&mut rig);
    rig.choose("Set:", "Other");
    assert!(rig.has(question), "a switch away from unsaved edits asks first");
    let select = rig.control("Set:", Role::COMBO_BOX);
    assert!(rig.fixture.tree().is_disabled_key(select.node), "the set row waits while the question is open");
    rig.click("Cancel");
    assert!(rig.is_open() && !rig.has(question), "Cancel answers the question and keeps the dialog");
    assert_eq!(rig.active_set().as_deref(), Some("Mine"));
    assert_eq!(rig.bindings("file.save"), ["F2"], "Cancel keeps the edits");

    rig.choose("Set:", "Other");
    rig.answer("Don't Save");
    assert_eq!(rig.active_set().as_deref(), Some("Other"));
    assert!(stored(&rig).is_empty(), "Don't Save leaves Mine as stored");

    rig.choose("Set:", "Mine");
    assert!(!rig.has("Save changes to “Other”?"), "an unmodified set switches with no question");
    assert_eq!(rig.active_set().as_deref(), Some("Mine"));
    edit(&mut rig);
    rig.choose("Set:", "Other");
    rig.answer("Save");
    assert_eq!(rig.active_set().as_deref(), Some("Other"));
    assert!(!stored(&rig).is_empty(), "Save stores the edits in Mine");
    assert_ne!(rig.bindings("file.save"), ["F2"], "Other has its own shortcuts");
}
