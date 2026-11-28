//! Check Spelling's Ignore and restart.

use std::cell::RefCell;
use std::rc::Rc;

use craft_text::ui::spelling::{CheckSpellingDialog, SpellingAction, SpellingItem};
use craft_ui::contract::Role;
use craft_ui::dialog::StackDialog as _;
use craft_ui::testing::{Fixture, trial};
use serde_json::json;

fn items() -> Vec<SpellingItem> {
    vec![
        SpellingItem::new(1, 0..3, "teh", vec!["the".to_owned(), "tea".to_owned()]),
        SpellingItem::new(1, 4..11, "recieve", vec!["receive".to_owned()]),
    ]
}

#[trial]
fn ignore_reports_its_item_and_restart_shows_the_first_item() {
    let dialog = Rc::new(RefCell::new(CheckSpellingDialog::new(items())));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().show(ui);
    });
    fixture.settle();
    let ignore =
        fixture.by_role(Role::BUTTON).into_iter().find(|button| fixture.tree().name_of(button.node) == Some("Ignore")).expect("the Ignore button");
    fixture.click(&ignore).settle();
    assert_eq!(dialog.borrow_mut().take_action(), Some(SpellingAction::Ignore(items()[0].clone())));
    let word = |dialog: &Rc<RefCell<CheckSpellingDialog>>| dialog.borrow().json_fields().expect("fields")["word"].clone();
    assert_eq!(word(&dialog), json!("recieve"), "Ignore moves on");

    dialog.borrow_mut().set_items(items());
    assert_eq!(word(&dialog), json!("recieve"), "set_items keeps the position");
    dialog.borrow_mut().restart(items());
    assert_eq!(word(&dialog), json!("teh"), "restart shows the first item");
    assert_eq!(dialog.borrow().json_fields().expect("fields")["changeTo"], json!("the"), "with its first suggestion");
}

#[trial]
fn restarting_drops_a_change_waiting_on_the_previous_word() {
    let dialog = Rc::new(RefCell::new(CheckSpellingDialog::new(items())));
    let shown = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().show(ui);
    });
    fixture.settle();
    let change = fixture.by_label("Change").expect("the Change button");
    fixture.click(&change).settle();
    let next = SpellingItem::new(2, 0..3, "adn", vec!["and".to_owned()]);
    dialog.borrow_mut().restart(vec![next.clone()]);
    assert!(dialog.borrow_mut().take_action().is_none(), "the previous word's change is discarded");
    fixture.settle();
    let change = fixture.by_label("Change").expect("the restarted Change button");
    fixture.click(&change).settle();
    assert_eq!(dialog.borrow_mut().take_action(), Some(SpellingAction::Change { item: next, to: "and".to_owned() }));
}
