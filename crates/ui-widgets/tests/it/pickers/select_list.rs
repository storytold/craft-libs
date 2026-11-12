//! The open list of a Select is one gesture: the rows the keys reach are the value until Escape puts it back or Tab
//! chooses it, and typing on the closed field opens the list on the first match. The keys the list takes are its own;
//! a command or app shortcut it leaves runs and closes it.

use craft_ui::buttons::Button;
use craft_ui::contract::overlay::close_popups;
use craft_ui::contract::{Bind, CommandSet, Entry, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::Commands as _;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, text};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Event, Key, Modifiers, Ui};
use std::cell::Cell;

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

/// A select over two groups under headings: Shapes (One, Two) and Lines (Three).
fn grouped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::heading("Shapes"), Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::heading("Lines"), Entry::new(3u8, "Three")];
    Select::new("Pick", Bind::bound(data.value("value", 1u8)), &entries).show(ui).into()
}

#[trial]
fn a_heading_in_the_list_takes_no_pick_and_the_keys_and_type_ahead_pass_over_it() {
    let mut fixture = Fixture::story(&Story::new("select_grouped", grouped));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    let heading = fixture.by_label("Lines").expect("the heading shows in the open list");
    fixture.click(&heading);
    fixture.settle();
    assert!(list_open(&mut fixture), "a click on a heading leaves the list open");
    assert_eq!(value(&mut fixture), 1, "and picks nothing");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value(&mut fixture), 3, "Down from the last row of a group lands on the first row of the next");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    fixture.type_text("l");
    fixture.settle();
    assert!(!list_open(&mut fixture), "a heading's caption is no type-ahead match");
    assert_eq!(value(&mut fixture), 1, "and the value stays");
}

/// The select in an app that reads its own shortcuts from egui's input before the select runs, as PhotoCraft does: the
/// keys it reads are noted in `app_read`, and its Cmd+1 shortcut closes the open popups as it runs.
fn select_in_app(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pressed: Vec<(Key, Modifiers)> = ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    if pressed.iter().any(|(key, modifiers)| *key == Key::Num1 && modifiers.command) {
        close_popups(ui.ctx());
    }
    data.note("app_read", Vec::<Key>::new()).extend(pressed.iter().map(|(key, _)| *key));
    select(ui, data)
}

/// The app's select, focused, with its list open.
fn open_in_app() -> Fixture {
    let mut fixture = Fixture::story(&Story::new("select_in_app", select_in_app));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    fixture
}

fn app_read(fixture: &mut Fixture, key: Key) -> bool {
    fixture.data().note("app_read", Vec::<Key>::new()).contains(&key)
}

#[trial]
fn a_letter_typed_into_the_open_list_is_its_own_and_reaches_no_app_shortcut() {
    let mut fixture = open_in_app();
    fixture.frame(vec![key_press(Key::T, Modifiers::NONE), text("t")]);
    fixture.frame(vec![key_release(Key::T, Modifiers::NONE)]);
    fixture.settle();
    assert_eq!(value(&mut fixture), 2, "the list's type-ahead takes the letter");
    assert!(list_open(&mut fixture), "and the list stays open");
    assert!(!app_read(&mut fixture, Key::T), "the app reads no press of the letter");
}

#[trial]
fn an_app_shortcut_runs_with_the_list_open_and_closes_it_through_the_runtime() {
    let mut fixture = open_in_app();
    fixture.key(Key::Num1, Modifiers::COMMAND);
    fixture.settle();
    assert!(app_read(&mut fixture, Key::Num1), "the chord the list leaves reaches the app");
    assert!(!list_open(&mut fixture), "the app's shortcut closes the list");
    assert_eq!(value(&mut fixture), 1, "and the value stays");
}

/// The commands of the scope around the select.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "selectList", scope = "trial.selectList")]
#[non_exhaustive]
pub enum SelectListCommand {
    #[command(label = "Mark", default = "Cmd+1")]
    Mark,
}

/// The select in a stack whose command scope binds Cmd+1; the runs of the command are counted in `marks`.
fn select_in_scope(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let marks = Cell::new(0u32);
    let out = Stack::new().command_scope(<SelectListCommand as CommandSet>::SCOPE).show(ui, |ui, cx| {
        while cx.take_command::<SelectListCommand>().is_some() {
            marks.set(marks.get() + 1);
        }
        select(ui, data);
    });
    *data.value("marks", 0u32) += marks.get();
    out.into()
}

#[trial]
fn a_key_command_runs_with_the_list_open_and_closes_it() {
    let mut fixture = Fixture::story(&Story::new("select_in_scope", select_in_scope));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    fixture.key(Key::Num1, Modifiers::COMMAND);
    fixture.settle();
    assert_eq!(*fixture.data().value("marks", 0u32), 1, "the command the key binds runs");
    assert!(!list_open(&mut fixture), "and closes the list");
    assert_eq!(value(&mut fixture), 1, "and the value stays");
}
