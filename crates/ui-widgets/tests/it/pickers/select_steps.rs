//! A Select steps over its stepped entries only. A value on an entry that is not stepped has no place among them: it
//! offers both steps, and a step forward lands on the first stop and a step back on the last.

use craft_ui::contract::{Bind, Entry, Role, Show as _};
use craft_ui::pickers::Select;
use craft_ui::testing::access::Declared;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Ui};

/// Entries 1 to 4; 2 and 4 are reached from the open list only.
fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two").stepped(false), Entry::new(3u8, "Three"), Entry::new(4u8, "Four").stepped(false)];
    Select::new("Pick", Bind::bound(data.value("value", 4u8)), &entries).show(ui).into()
}

/// The value after `key` on the focused, closed field holding `held`, and whether it offered Increment and Decrement.
fn step(held: u8, key: Key) -> (u8, (bool, bool)) {
    let mut fixture = Fixture::story(&Story::new("select_steps", select));
    *fixture.data().value("value", 4u8) = held;
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let node = fixture.by_role(Role::COMBO_BOX).first().expect("the field").node;
    let tree = fixture.tree();
    let offered = (tree.declares_key(node, Declared::Increment), tree.declares_key(node, Declared::Decrement));
    fixture.key(key, Modifiers::NONE);
    fixture.settle();
    let value = *fixture.data().value("value", 4u8);
    (value, offered)
}

#[trial]
fn a_select_steps_over_its_stepped_entries_and_from_an_unstepped_one_to_an_end() {
    assert_eq!(step(1, Key::ArrowDown), (3, (true, false)), "the first stop skips the unstepped Two and offers no step back");
    assert_eq!(step(4, Key::ArrowDown), (1, (true, true)), "from an unstepped entry, forward lands on the first stop");
    assert_eq!(step(4, Key::ArrowUp), (3, (true, true)), "from an unstepped entry, back lands on the last stop");
}

#[trial]
fn the_closed_face_shows_an_entrys_closed_label_and_the_accessible_value_keeps_its_label() {
    let mut fixture = Fixture::new(|ui| {
        let entries = [Entry::new(1u8, "1 Dots").closed_label("Dots"), Entry::new(2u8, "2 Lines").closed_label("Lines")];
        Select::new("Leader", Bind::given(1u8), &entries).show(ui);
    });
    fixture.settle();
    let texts: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    assert!(texts.iter().any(|text| text == "Dots"), "the face shows the closed label: {texts:?}");
    assert!(!texts.iter().any(|text| text == "1 Dots"), "and not the list text");
    let node = fixture.by_role(Role::COMBO_BOX).first().expect("the field").node;
    assert_eq!(fixture.tree().value_of(node), Some("1 Dots"), "the accessible value is the full label");
}
