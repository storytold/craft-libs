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
