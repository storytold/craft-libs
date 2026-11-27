//! The Customize Keyboard layout: each change is applied at once and Escape closes without rolling back.

use craft_keymap::{Binding, CommandRef, Platform};
use craft_menu::shortcuts::{Area, Layout, Outcome};
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::Rig;

/// Records `key` with the command modifier in the New Shortcut field.
fn record(rig: &mut Rig, key: Key) {
    rig.click("New shortcut: None");
    rig.press(Modifiers::COMMAND, key);
}

/// The bindings of Bring to Front, compared as bindings rather than as their text.
fn keys(rig: &Rig) -> Vec<Binding> {
    rig.shared.borrow().km.bindings(&CommandRef::from("object.front")).to_vec()
}

/// Production applies each change at once and has no Cancel: an Escape that rolled back would silently lose them.
#[trial]
fn command_lists_assign_remove_and_close_apply_each_change_at_once() {
    let mut rig = Rig::tuned(Platform::Mac, |options| options.layout = Layout::CommandLists);
    rig.click("Bring to Front");
    record(&mut rig, Key::E);
    let snapshot = {
        let shared = rig.shared.borrow();
        shared.state.snapshot(&shared.km)
    }
    .expect("the dialog has shown");
    assert_eq!(snapshot.selected, Some(CommandRef::from("object.front")));
    assert_eq!(snapshot.holders, [CommandRef::from("file.export")], "the chord's holder is reported");
    assert!(snapshot.pressed.is_some(), "the field holds the chord");
    assert!(rig.bindings("object.front").is_empty());

    rig.click("Assign");
    assert_eq!(rig.bindings("object.front").len(), 1, "Assign gives the command the chord");
    assert!(rig.bindings("file.export").is_empty(), "the chord is taken from its holder");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Changed)), "Assign reports Changed");

    record(&mut rig, Key::J);
    rig.click("Assign");
    assert_eq!(rig.bindings("object.front").len(), 2);

    let both = keys(&rig);
    let changes = rig.outcomes().iter().filter(|o| matches!(o, Outcome::Changed)).count();
    rig.click("⌘E");
    rig.click("Remove");
    let left = keys(&rig);
    assert_eq!(left, [both[1].clone()], "Remove takes away the picked key only");
    let after = rig.outcomes().iter().filter(|o| matches!(o, Outcome::Changed)).count();
    assert!(after > changes, "Remove reports Changed");

    rig.press(Modifiers::NONE, Key::Escape);
    assert!(!rig.is_open(), "Escape closes the dialog");
    assert!(rig.has_outcome(|o| matches!(o, Outcome::Ok)), "Escape closes as OK");
    assert!(!rig.has_outcome(|o| matches!(o, Outcome::Cancel)), "there is no Cancel");
    assert_eq!(keys(&rig), left, "the keymap keeps the edits");
    assert!(rig.bindings("file.export").is_empty(), "the taken chord is not given back");
}

/// A long Categories list must not take the room the Commands list needs: in a dialog of ordinary height a command row
/// still shows above the footer.
#[trial]
fn commands_keep_room_beside_a_long_categories_list() {
    let rig = Rig::tuned(Platform::Mac, |options| {
        options.layout = Layout::CommandLists;
        options.own_areas = true;
        options.size = Some(egui::vec2(640.0, 460.0));
        options.areas = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L"].map(|name| Area::new(name, name, |_| true)).to_vec();
    });
    let row = rig.find("Bring to Front").center();
    let footer = rig.find("Reset All…").center();
    assert!(row.y < footer.y, "the command row ({row:?}) sits above the footer ({footer:?})");
    // The capped list scrolls: the arrow keys still reach its last entry.
    let mut rig = rig;
    rig.click("A");
    for _ in 0..11 {
        rig.press(Modifiers::NONE, Key::ArrowDown);
    }
    let area = {
        let shared = rig.shared.borrow();
        shared.state.snapshot(&shared.km).expect("the dialog has shown").area
    };
    assert_eq!(area, "L", "the last Categories entry is reachable");
}
