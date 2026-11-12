//! The table's model through the dialog: which rows are listed and which count as conflicts.

use craft_keymap::{Binding, CommandRef, Platform, Policy};
use craft_ui::testing::trial;

use super::fixture::{self, Rig};

#[trial]
fn hidden_entries_are_not_listed_but_still_conflict() {
    let mut options = fixture::options();
    options.visible = Some(|b| b.command.id != "file.export");
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), options, None);
    assert!(!rig.has("Export…: ⌘E"), "a hidden command is not listed");
    rig.shared.borrow_mut().km.assign(&CommandRef::from("file.save"), Binding::parse("Cmd+E").expect("a chord"), Policy::Keep).expect("assigns");
    rig.pass();
    rig.choose("Filter:", "Conflicts");
    assert!(rig.has("Save: ⌘S") && rig.has("Save: ⌘E"), "the hidden command still clashes with Save");
}

/// Every row that can take another shortcut shows its add button after its chips, in a narrow dialog too: the chips
/// give way to it.
#[trial]
fn every_row_that_can_take_a_shortcut_has_its_add_button() {
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), fixture::options(), None);
    rig.fixture.room(egui::vec2(640.0, 600.0));
    rig.fixture.settle();
    for label in ["Save", "Save As…", "Export…", "Bring to Front", "Loop"] {
        let add = format!("Add a shortcut to {label}");
        assert!(rig.has(&add), "{label:?} has no add button; buttons {:?}", rig.names_with(craft_ui::contract::Role::BUTTON, "Add a shortcut"));
    }
}
