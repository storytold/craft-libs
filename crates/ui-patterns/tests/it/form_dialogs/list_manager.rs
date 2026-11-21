//! `ListManager`'s action rules: a name check, a button that never enables, the Cancel label and the section under
//! the list; a name box the caller binds.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::patterns::form_dialogs::{ActionEnabled, ListManager, ListManagerAction, ListManagerEntry};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
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
