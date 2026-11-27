//! The open list of a Select is one gesture: the rows the keys reach are the value until Escape puts it back or Tab
//! chooses it, and typing on the closed field opens the list on the first match. The keys the list takes are its own;
//! a command or app shortcut it leaves runs and closes it.

use craft_host::{AppInfo, Host};
use craft_ui::buttons::Button;
use craft_ui::contract::overlay::close_popups;
use craft_ui::contract::{Bind, CommandSet, Entry, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::Commands as _;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, text};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Event, Key, Modifiers, Ui, Vec2};
use std::cell::Cell;
use std::sync::Arc;

/// A select over One to Four.
fn select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::new(3u8, "Three"), Entry::new(4u8, "Four")];
    Select::new("Pick", data.bind("value", 1u8), &entries).show(ui).into()
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

/// The list that Escape closed is closed for its field's keys while it still fades out: the Enter that follows at once
/// is the field's own and opens the list again, rather than choosing a row of the list that is leaving.
#[trial]
fn enter_on_the_field_right_after_escape_opens_the_list_again_during_its_exit_fade() {
    let mut fixture = focused();
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.key(Key::Escape, Modifiers::NONE);
    assert_eq!(value(&mut fixture), 1, "Escape puts back the value from before the list opened");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(list_open(&mut fixture), "Enter on the focused field opens the list again before the old one has faded out");
    assert_eq!(value(&mut fixture), 1, "and picks no row of the list that was leaving");
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

/// A select over a Latin entry, the two Turkic capital i's and a Greek one.
fn turkic(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "Kitap"), Entry::new(2u8, "IRMAK"), Entry::new(3u8, "İSTANBUL"), Entry::new(4u8, "Ωμέγα")];
    Select::new("Pick", data.bind("value", 1u8), &entries).show(ui).into()
}

/// The value after typing `typed` on the closed, focused select holding `held`, with Azerbaijani as the host's
/// interface language.
fn typed_in_azerbaijani(held: u8, typed: &str) -> u8 {
    let (i18n, _) = craft_i18n::I18n::from_owned([("az".to_owned(), String::new(), String::new())], &["az"]);
    let lang = i18n.from_code("az").expect("Azerbaijani is offered");
    let host = Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).lang(lang).build();
    let mut fixture = Fixture::story(&Story::new("select_turkic", turkic));
    fixture.host(Arc::new(host));
    *fixture.data().value("value", 1u8) = held;
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    fixture.type_text(typed);
    fixture.settle();
    assert!(list_open(&mut fixture), "`{typed}` matches an entry and opens the list");
    value(&mut fixture)
}

/// Type-ahead folds the typed letters and the labels in the interface language: in Azerbaijani the dotted and dotless
/// i find both capitals, a decomposed dotted i finds İSTANBUL, and a Greek label matches its letters in either case.
#[trial]
fn type_ahead_folds_case_in_the_interface_language() {
    assert_eq!(typed_in_azerbaijani(1, "ır"), 2, "`ır` finds IRMAK");
    assert_eq!(typed_in_azerbaijani(3, "ı"), 2, "`ı` after İSTANBUL wraps round to IRMAK");
    assert_eq!(typed_in_azerbaijani(2, "i"), 3, "`i` after IRMAK finds İSTANBUL");
    assert_eq!(typed_in_azerbaijani(1, "is"), 3, "`is` finds İSTANBUL, whose capital lowercases to i and a combining dot");
    assert_eq!(typed_in_azerbaijani(1, "i\u{307}s"), 3, "a typed i with a combining dot finds İSTANBUL");
    assert_eq!(typed_in_azerbaijani(1, "ωμ"), 4, "a Greek prefix finds its label");
    assert_eq!(typed_in_azerbaijani(1, "ΩΜ"), 4, "in capitals too");
}

/// A select over two groups under headings: Shapes (One, Two) and Lines (Three).
fn grouped(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::heading("Shapes"), Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::heading("Lines"), Entry::new(3u8, "Three")];
    Select::new("Pick", data.bind("value", 1u8), &entries).show(ui).into()
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

/// The name of an edit event's step.
fn step_name<V>(event: &craft_behavior::EditEvent<V>) -> &'static str {
    use craft_behavior::EditEvent;
    match event {
        EditEvent::Begin { .. } => "begin",
        EditEvent::Update { .. } => "update",
        EditEvent::Commit { .. } => "commit",
        EditEvent::Cancel { .. } => "cancel",
        EditEvent::Set { .. } => "set",
        _ => "other",
    }
}

/// The wheel over the field steps the pick with the list open too, one entry per line a turn carries, and takes over
/// the row the keys previewed: the run commits once when the wheel stops, with nothing put back.
#[trial]
fn the_wheel_steps_the_open_select_a_line_an_entry_and_commits_when_it_stops() {
    use craft_behavior::GESTURE_GAP;
    use craft_ui::testing::pointer::{moved, wheel_lines};
    use std::cell::RefCell;
    use std::rc::Rc;
    let value = Rc::new(RefCell::new(1u8));
    let steps: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let (held, seen) = (value.clone(), steps.clone());
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::new(3u8, "Three"), Entry::new(4u8, "Four"), Entry::new(5u8, "Five")];
        let mut guard = held.borrow_mut();
        let out = Select::new("Pick", Bind::bound(&mut *guard), &entries).show(ui);
        seen.borrow_mut().extend(out.events.iter().map(step_name));
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    let face = fixture.by_role(Role::COMBO_BOX).first().expect("the field").rect;
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    assert_eq!(*value.borrow(), 2, "the keys preview Two");
    fixture.frame(vec![moved(face.center())]);
    fixture.frame(vec![wheel_lines(-2.0)]);
    assert_eq!(*value.borrow(), 4, "a turn of two lines down steps two entries on from the previewed one");
    assert!(!steps.borrow().contains(&"commit"), "nothing commits while the wheel turns: {:?}", steps.borrow());
    fixture.advance(GESTURE_GAP * 2.0).idle(3);
    let steps = steps.borrow();
    assert_eq!(steps.iter().filter(|step| **step == "commit").count(), 1, "the wheel's end commits the run once: {steps:?}");
    assert!(!steps.contains(&"cancel"), "and puts nothing back: {steps:?}");
    assert_eq!(*value.borrow(), 4);
}

/// The entry a long select holds.
const HELD: u8 = 40;

/// A select over sixty entries, holding `HELD`.
fn long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries: Vec<Entry<u8>> = (0..60u8).map(|at| Entry::new(at, format!("Item {at}"))).collect();
    Select::new("Pick", data.bind("value", HELD), &entries).show(ui).into()
}

#[trial]
fn a_long_list_in_a_short_window_opens_on_the_roomier_side_with_the_pick_in_view() {
    let mut fixture = Fixture::story(&Story::new("select_long", long));
    fixture.room(Vec2::new(400.0, 240.0)).settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    let window = fixture.ctx().content_rect();
    let list = fixture.by_role(Role::LIST_BOX).first().expect("Alt+Down opens the list").rect;
    assert!(window.contains_rect(list), "the list {list:?} stays inside the window {window:?}");
    let held = fixture.by_label(&format!("Item {HELD}")).expect("the held entry shows").rect;
    assert!(list.expand(0.5).contains_rect(held), "the held entry {held:?} is in view in the list {list:?}");
    fixture.key(Key::Space, Modifiers::NONE);
    fixture.settle();
    assert!(!list_open(&mut fixture), "Space on the lead picks and closes the list");
    assert_eq!(*fixture.data().value("value", HELD), HELD, "the lead the list opened on is the held entry");
}

/// The pointer over a row of an open list moves the lead only: the held value stays the selected option in every
/// frame, and Enter then chooses the row the pointer is on.
#[trial]
fn hovering_a_row_moves_the_lead_and_leaves_the_selected_option_held() {
    use craft_ui::testing::pointer::moved;
    let mut fixture = focused();
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    assert!(list_open(&mut fixture), "Alt+Down opens the list");
    let options = fixture.by_role(Role::LIST_BOX_OPTION);
    assert_eq!(options.len(), 4, "the list shows its four options");
    let held = options[0].node;
    let under_pointer = options[2].center();
    for step in 0..3 {
        fixture.frame(if step == 0 { vec![moved(under_pointer)] } else { Vec::new() });
        let selected: Vec<_> =
            fixture.by_role(Role::LIST_BOX_OPTION).into_iter().filter(|row| fixture.tree().selected_of(row.node) == Some(true)).collect();
        assert!(selected.len() == 1 && selected[0].node == held, "frame {step}: the held row stays the selected option");
    }
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value(&mut fixture), 3, "the lead followed the pointer, so Enter chooses the row under it");
}

/// One to Four with a separator between Two and Three.
fn separated(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two"), Entry::separator(), Entry::new(3u8, "Three"), Entry::new(4u8, "Four")];
    Select::new("Pick", data.bind("value", 1u8), &entries).show(ui).into()
}

/// A separator draws a rule between the rows around it, lower than a row, and the keys pass over it (FW-319: it drew
/// blank room).
#[trial]
fn a_separator_in_the_list_draws_a_rule_and_the_keys_pass_over_it() {
    let mut fixture = Fixture::story(&Story::new("select_separated", separated));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
    fixture.settle();
    let two = fixture.by_label("Two").expect("the Two row").rect;
    let three = fixture.by_label("Three").expect("the Three row").rect;
    let gap = three.top() - two.bottom();
    assert!(gap > 0.0 && gap < two.height(), "the separator is lower than a row: {gap} between rows {} high", two.height());
    let rule = fixture.shape_bounds().iter().any(|shape| {
        shape.top() >= two.bottom() - 0.5 && shape.bottom() <= three.top() + 0.5 && shape.width() > two.width() / 2.0 && shape.height() < gap
    });
    assert!(rule, "a rule across the list between Two and Three");
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value(&mut fixture), 3, "Down from Two lands on Three");
}
