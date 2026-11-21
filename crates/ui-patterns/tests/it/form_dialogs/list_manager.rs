//! `ListManager`'s in-place rename: a commit is a choice, Escape and an entry of another kind are not.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
