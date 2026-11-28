//! Check Spelling's Ignore and restart, and `CheckSpellingDialog::current`: the misspelling the dialog is showing,
//! so the app can highlight it while the dialog walks the list (photocraft #14188). The list and the button actions
//! stay as they are around it.

use std::cell::RefCell;
use std::rc::Rc;

use craft_text::ui::spelling::{CheckSpellingDialog, SpellingAction, SpellingItem};
use craft_ui::contract::Role;
use craft_ui::dialog::StackDialog as _;
use craft_ui::testing::{Fixture, trial};
use serde_json::json;

/// Two misspellings in two text objects.
fn two_documents() -> Vec<SpellingItem> {
    vec![
        SpellingItem::new(7, 4..7, "teh", vec!["the".to_owned(), "tea".to_owned()]),
        SpellingItem::new(9, 20..27, "recieve", vec!["receive".to_owned()]),
    ]
}

/// A fixture showing `dialog`, and the dialog itself for reading `current` between the frames.
fn shown(dialog: CheckSpellingDialog) -> (Fixture, Rc<RefCell<CheckSpellingDialog>>) {
    let dialog = Rc::new(RefCell::new(dialog));
    let shared = dialog.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = shared.borrow_mut().show(ui);
    });
    fixture.idle(30);
    (fixture, dialog)
}

fn press(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown"));
    fixture.click(&target).idle(4);
}

#[trial]
fn current_is_the_word_the_dialog_opens_on_and_none_when_nothing_is_left() {
    let (_, dialog) = shown(CheckSpellingDialog::new(two_documents()));
    assert_eq!(dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())), Some((7, 4..7, "teh".to_owned())));
    assert!(CheckSpellingDialog::new(Vec::new()).current().is_none(), "an empty list shows no word");
}

#[trial]
fn ignore_moves_current_to_the_next_word_and_past_the_end_is_none() {
    let (mut fixture, dialog) = shown(CheckSpellingDialog::new(two_documents()));
    press(&mut fixture, "Ignore");
    assert_eq!(dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())), Some((9, 20..27, "recieve".to_owned())));
    // Ignore also reports the word it moved past (next's action rule for the button).
    let action = dialog.borrow_mut().take_action();
    assert!(
        matches!(action, Some(SpellingAction::Ignore(ref item)) if item.word == "teh" && item.object == 7),
        "Ignore reports the word it moved past: {action:?}"
    );
    press(&mut fixture, "Ignore");
    assert_eq!(dialog.borrow().current(), None, "the walk has passed the end of the list");
    // The list is untouched around this reader: Ignore keeps the words listed.
    assert_eq!(dialog.borrow().items().len(), 2, "Ignore keeps the listed words");
}

#[trial]
fn after_an_action_the_kept_position_makes_current_the_next_misspelling() {
    let (mut fixture, dialog) = shown(CheckSpellingDialog::new(two_documents()));
    // Change on the first word: the app replaces it and lists again without it, and `set_items` keeps the position.
    press(&mut fixture, "Change");
    let action = dialog.borrow_mut().take_action();
    assert!(
        matches!(action, Some(SpellingAction::Change { ref item, .. }) if item.word == "teh" && item.object == 7),
        "Change hands the app the first item: {action:?}"
    );
    dialog.borrow_mut().set_items(vec![two_documents().remove(1)]);
    assert_eq!(
        dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())),
        Some((9, 20..27, "recieve".to_owned())),
        "the same position is now the next misspelling"
    );
}

/// Two misspellings in one text object.
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
