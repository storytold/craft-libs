//! `ListManager`'s action rules: a name check, a button that never enables, the Cancel label and the section under
//! the list; a name box the caller binds; and its in-place rename: a commit is a choice, Escape and an entry of another
//! kind are not.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::dialog::{DialogHost, DialogResult};
use craft_ui::display::Text;
use craft_ui::patterns::form_dialogs::{
    ActionEnabled, HostedListManager, ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords,
};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A name is valid while it is not empty and holds no digit.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| c.is_ascii_digit())
}

/// The manager under test: Add (`OnNewName`), Pin (`OnValidName`), Lock (`Never`, with its reason) and a Close button.
fn fixture() -> Fixture {
    Fixture::new(|ui| {
        let actions = vec![
            ListManagerAction::new("add", "Add", ActionEnabled::OnNewName).keeps_open(),
            ListManagerAction::new("pin", "Pin", ActionEnabled::OnValidName).keeps_open(),
            ListManagerAction::new("lock", "Lock", ActionEnabled::Never).reason("Locking is not available.").keeps_open(),
        ];
        let entries = vec![ListManagerEntry::new("a", "Alpha")];
        let _ = ListManager::new("Marks", "Name:", entries, actions)
            .name_check(plain_name)
            .cancel_label("Close")
            .under_list(|ui| {
                Text::new("Under the list").show(ui);
            })
            .show(ui);
    })
}

fn enabled(fixture: &Fixture, label: &str) -> bool {
    let button = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
    !fixture.tree().is_disabled_key(button.node)
}

// `OnValidName` follows the name check even for a name an entry has, where `OnNewName` refuses it; `Never` stays off
// with its reason; `cancel_label` replaces Cancel; the `under_list` content is above the description.
#[trial]
fn a_valid_name_action_follows_the_name_check_and_a_never_action_stays_off() {
    let mut fixture = fixture();
    fixture.idle(10);
    assert!(!enabled(&fixture, "Pin"), "an empty name is not valid");
    assert!(!enabled(&fixture, "Lock"), "a Never action is off with an empty name");

    let name = fixture.by_label("Name:").expect("the name box");
    fixture.click(&name).idle(2);
    fixture.type_text("Beta").idle(4);
    assert!(enabled(&fixture, "Pin"), "a name the check accepts enables the action");
    assert!(enabled(&fixture, "Add"), "a new name enables Add");
    assert!(!enabled(&fixture, "Lock"), "a Never action is off whatever is typed");
    let lock = fixture.by_label("Lock").expect("the Lock button");
    assert_eq!(fixture.tree().description_of(lock.node), Some("Locking is not available."), "the reason is the disabled button's tooltip");

    fixture.key(egui::Key::A, egui::Modifiers::COMMAND).idle(2);
    fixture.type_text("Alpha").idle(4);
    assert!(enabled(&fixture, "Pin"), "an existing name passes the check too");
    assert!(!enabled(&fixture, "Add"), "OnNewName refuses an existing name");
    let under = fixture.by_label("Under the list").expect("the section under the list");
    let description = fixture.by_label("Description").expect("the named entry's description line");
    assert!(under.rect.min.y < description.rect.min.y, "the section stands above the description");

    fixture.key(egui::Key::A, egui::Modifiers::COMMAND).idle(2);
    fixture.type_text("Beta2").idle(4);
    assert!(!enabled(&fixture, "Pin"), "a name the check refuses disables the action");

    assert!(fixture.by_label("Close").is_some(), "cancel_label names the second button");
    assert!(fixture.by_label("Cancel").is_none(), "Cancel is replaced");
}

// A bound name box is the caller's string both ways: a caller write shows in the box on the next pass and the buttons
// follow it, and what the user types lands in the caller's string.
#[trial]
fn a_bound_name_box_shows_the_callers_writes_and_takes_the_typed_name() {
    let name = Rc::new(RefCell::new(String::new()));
    let held = Rc::clone(&name);
    let mut fixture = Fixture::new(move |ui| {
        let actions = vec![ListManagerAction::new("add", "Add", ActionEnabled::OnNewName).keeps_open()];
        let entries = vec![ListManagerEntry::new("a", "Alpha")];
        let mut name = held.borrow_mut();
        let _ = ListManager::new("Marks", "Name:", entries, actions).name_box(Bind::bound(&mut *name)).show(ui);
    });
    fixture.idle(10);
    *name.borrow_mut() = "Gamma".to_owned();
    fixture.idle(4);
    let field = fixture.by_label("Name:").expect("the name box");
    assert_eq!(fixture.tree().value_of(field.node), Some("Gamma"), "the caller's write shows in the box");
    assert!(enabled(&fixture, "Add"), "the buttons follow the caller's name");

    fixture.click(&field).idle(2);
    fixture.key(egui::Key::A, egui::Modifiers::COMMAND).idle(2);
    fixture.type_text("Alpha").idle(4);
    assert_eq!(*name.borrow(), "Alpha", "the typed name is in the caller's string");
    assert!(!enabled(&fixture, "Add"), "an existing name disables Add");
}

/// The choices a list manager reported and whether it closed.
type Answers = (Rc<RefCell<Vec<ListManagerChoice>>>, Rc<Cell<bool>>);

/// A list manager whose first action is `first`, with every choice and close it reports.
fn enter_fixture(first: ListManagerAction) -> (Fixture, Answers) {
    let choices = Rc::new(RefCell::new(Vec::new()));
    let closed = Rc::new(Cell::new(false));
    let (seen, ended) = (Rc::clone(&choices), Rc::clone(&closed));
    let fixture = Fixture::new(move |ui| {
        if ended.get() {
            return;
        }
        let actions = vec![first.clone(), ListManagerAction::new("rename", "Rename", ActionEnabled::Always).keeps_open()];
        let entries = vec![ListManagerEntry::new("a", "Alpha")];
        let out = ListManager::new("Workspaces", "Name:", entries, actions).opened(Bind::given(true)).show(ui);
        ended.set(out.closed.is_some());
        seen.borrow_mut().extend(out.choice);
    });
    (fixture, (choices, closed))
}

// Enter in the name box with no row picked runs the first action while it is enabled, as a click on it does, and
// leaves the dialog open while it is disabled; with a row picked it still answers once.
#[trial]
fn enter_in_the_name_box_runs_the_first_action_only_while_it_is_enabled() {
    let ok = ListManagerAction::new("ok", "OK", ActionEnabled::Always);
    let (mut fixture, (choices, closed)) = enter_fixture(ok.clone());
    fixture.room(Vec2::new(900.0, 700.0)).idle(10);
    let name = fixture.by_label("Name:").expect("the name box");
    fixture.click(&name).idle(2);
    fixture.type_text("Beta").idle(4);
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    assert!(closed.get(), "Enter closes the dialog as OK would");
    {
        let seen = choices.borrow();
        let [choice] = seen.as_slice() else { panic!("one choice for Enter, got {seen:?}") };
        assert_eq!((choice.action.as_str(), choice.entry.as_deref(), choice.name.as_str()), ("ok", None, "Beta"));
    }

    let (mut fixture, (choices, closed)) = enter_fixture(ok.clone());
    fixture.room(Vec2::new(900.0, 700.0)).idle(10);
    let alpha = fixture.by_label("Alpha").expect("the list shows Alpha");
    fixture.click(&alpha).idle(4);
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    assert!(closed.get(), "Enter on a picked row closes the dialog");
    assert_eq!(choices.borrow().len(), 1, "Enter on a picked row answers once: {:?}", choices.borrow());

    let (mut fixture, (choices, closed)) = enter_fixture(ok);
    fixture.room(Vec2::new(900.0, 700.0)).idle(10);
    let alpha = fixture.by_label("Alpha").expect("the list shows Alpha");
    fixture.click(&alpha).idle(4);
    let name = fixture.by_label("Name:").expect("the name box");
    fixture.click(&name).idle(2);
    fixture.key(Key::A, Modifiers::COMMAND).idle(2);
    fixture.type_text("Beta").idle(4);
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    assert!(closed.get(), "Enter after retyping the name closes the dialog");
    assert_eq!(choices.borrow().first().map(|choice| choice.name.as_str()), Some("Beta"), "Enter answers with the typed name");

    let (mut fixture, (choices, closed)) = enter_fixture(ListManagerAction::run(&ListManagerWords::default()));
    fixture.room(Vec2::new(900.0, 700.0)).idle(10);
    let name = fixture.by_label("Name:").expect("the name box");
    fixture.click(&name).idle(2);
    fixture.type_text("Beta").idle(4);
    assert!(!enabled(&fixture, "Run"), "a new name leaves an entry action off");
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    assert!(!closed.get(), "Enter does not run a disabled first action");
    assert!(choices.borrow().is_empty(), "no choice for a disabled first action: {:?}", choices.borrow());
}

#[trial]
fn a_committed_rename_reports_entry_and_name_and_escape_keeps_it() {
    let rows = Rc::new(RefCell::new(vec![
        ListManagerEntry::new("a", "Alpha").kind(1),
        ListManagerEntry::new("b", "Bravo"),
        ListManagerEntry::new("c", "Gamma").kind(1),
    ]));
    let choices = Rc::new(RefCell::new(Vec::<ListManagerChoice>::new()));
    let closed = Rc::new(Cell::new(false));
    let (held, seen, ended) = (Rc::clone(&rows), Rc::clone(&choices), Rc::clone(&closed));
    let mut fixture = Fixture::new(move |ui| {
        let entries = held.borrow().clone();
        let actions = vec![ListManagerAction::run(&Env::of(ui).words::<ListManagerWords>())];
        let out = ListManager::new("Macros", "Macro name:", entries, actions).renames(1).opened(Bind::given(true)).show(ui);
        ended.set(ended.get() || out.closed.is_some());
        if let Some(choice) = out.choice {
            if choice.is_rename() {
                for entry in held.borrow_mut().iter_mut().filter(|entry| Some(&entry.id) == choice.entry.as_ref()) {
                    entry.name.clone_from(&choice.name);
                }
            }
            seen.borrow_mut().push(choice);
        }
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let renames = || choices.borrow().iter().filter(|choice| choice.is_rename()).count();

    let alpha = fixture.by_label("Alpha").expect("the list shows Alpha");
    fixture.click(&alpha).idle(4);
    fixture.key(Key::F2, Modifiers::NONE).idle(4);
    fixture.key(Key::A, Modifiers::COMMAND).idle(2);
    fixture.type_text("  Zed ").idle(2);
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    {
        let seen = choices.borrow();
        let [choice] = seen.as_slice() else { panic!("one choice for the commit, got {seen:?}") };
        assert!(choice.is_rename(), "the commit is a rename");
        assert_eq!(choice.entry.as_deref(), Some("a"), "it names the renamed entry");
        assert_eq!(choice.name, "Zed", "it carries the trimmed name");
    }
    assert!(!closed.get(), "the dialog stays open");

    let zed = fixture.by_label("Zed").expect("the row shows the new name");
    fixture.click(&zed).idle(4);
    fixture.key(Key::F2, Modifiers::NONE).idle(4);
    fixture.type_text("Q").idle(2);
    fixture.key(Key::Escape, Modifiers::NONE).idle(6);
    assert_eq!(renames(), 1, "Escape reports no rename");
    assert!(!closed.get(), "Escape ends the editor and keeps the dialog");
    assert_eq!(rows.borrow()[0].name, "Zed", "the entry keeps its name");

    let bravo = fixture.by_label("Bravo").expect("the list shows Bravo");
    fixture.click(&bravo).idle(4);
    fixture.key(Key::F2, Modifiers::NONE).idle(4);
    fixture.type_text("X").idle(2);
    fixture.key(Key::Enter, Modifiers::NONE).idle(6);
    assert_eq!(renames(), 1, "an entry of another kind does not rename");
}

// On the dialog host an action that keeps the list open reaches the host once through `take_choice`, new entries show
// in the open dialog, and an action that closes it is the dialog's answer.
#[trial]
fn a_hosted_list_manager_hands_over_kept_open_choices_and_answers_with_the_closing_one() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 700.0));
    let words = ListManagerWords::default();
    let actions = vec![ListManagerAction::run(&words), ListManagerAction::delete(&words)];
    let entries = vec![ListManagerEntry::new("a", "Alpha"), ListManagerEntry::new("b", "Beta")];
    host.borrow_mut().open(HostedListManager::new("macros", "Macros", "Macro name:", actions).entries(entries));
    fixture.idle(6);
    let press = |fixture: &mut Fixture, label: &str| {
        let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
        fixture.click(&target).idle(4);
    };
    let choice = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<HostedListManager>().and_then(HostedListManager::take_choice);

    press(&mut fixture, "Beta");
    press(&mut fixture, "Delete");
    let deleted = choice(&host).expect("Delete hands its choice to the host");
    assert_eq!((deleted.action.as_str(), deleted.entry.as_deref()), ("delete", Some("b")));
    assert_eq!(choice(&host), None, "the choice is taken once");
    assert!(host.borrow().is_open::<HostedListManager>(), "Delete keeps the list open");

    host.borrow_mut().get_mut::<HostedListManager>().expect("open").set_entries(vec![ListManagerEntry::new("a", "Alpha")]);
    fixture.idle(4);
    assert!(fixture.by_label("Beta").is_none(), "the open list shows the new entries");

    press(&mut fixture, "Alpha");
    press(&mut fixture, "Run");
    let answered = host.borrow_mut().take::<HostedListManager>();
    assert!(matches!(&answered, Some(DialogResult::Ok(run)) if run.action == "run" && run.entry.as_deref() == Some("a")), "{answered:?}");
}
