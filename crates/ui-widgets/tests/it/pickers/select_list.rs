//! The open list of a Select is one gesture: the rows the keys reach are the value until Escape puts it back or Tab
//! chooses it, and typing on the closed field opens the list on the first match.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, Role, Show as _};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Ui};

/// A select over One to Four.
fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::new(3u8, "Three"), Entry::new(4u8, "Four")];
    Select::new("Pick", Bind::bound(data.value("value", 1u8)), &entries).show(ui).into()
}

/// The select with a button after it, which Tab reaches next.
fn select_then_button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let out = select(ui, data);
    Button::new("Next").show(ui);
    out
}

/// A fixture with the select focused and holding One.
fn focused() -> Fixture {
    let mut fixture = Fixture::story(&Story::new("select_list", select));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    fixture
}

fn value(fixture: &mut Fixture) -> u8 {
    *fixture.data().value("value", 1u8)
}

fn list_open(fixture: &mut Fixture) -> bool {
    !fixture.by_role(Role::LIST_BOX).is_empty()
}

#[trial]
fn keys_in_the_open_list_move_the_value_and_escape_puts_it_back() {
    let mut fixture = focused();
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value(&mut fixture), 2, "the row Down reaches is the value at once");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(!list_open(&mut fixture), "Escape closes the list");
    assert_eq!(value(&mut fixture), 1, "and puts back the value from before the list opened");
}

#[trial]
fn typing_on_the_closed_select_opens_the_list_on_the_first_match() {
    let mut fixture = focused();
    fixture.type_text("t");
    fixture.settle();
    assert!(list_open(&mut fixture), "a typed letter opens the list");
    assert_eq!(value(&mut fixture), 2, "on the first entry it begins, after the held one");
}

#[trial]
fn tab_in_the_open_list_chooses_the_row_the_keys_reached_and_moves_on() {
    let mut fixture = Fixture::story(&Story::new("select_then_button", select_then_button));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let on_select = fixture.focused_stop();
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    assert!(!list_open(&mut fixture), "Tab closes the list");
    assert_eq!(value(&mut fixture), 2, "and keeps the row the keys reached");
    assert_ne!(fixture.focused_stop(), on_select, "and the focus moved on");
}
