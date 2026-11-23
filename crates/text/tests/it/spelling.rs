//! `CheckSpellingDialog::current`: the misspelling the dialog is showing, so the app can highlight
//! it while the dialog walks the list (photocraft #14188). The list and the button actions stay as
//! they are around it.

use craft_text::ui::spelling::{CheckSpellingDialog, SpellingAction, SpellingItem};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// Two misspellings in two text objects.
fn items() -> Vec<SpellingItem> {
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
    let (_, dialog) = shown(CheckSpellingDialog::new(items()));
    assert_eq!(dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())), Some((7, 4..7, "teh".to_owned())));
    assert!(CheckSpellingDialog::new(Vec::new()).current().is_none(), "an empty list shows no word");
}

#[trial]
fn ignore_moves_current_to_the_next_word_and_past_the_end_is_none() {
    let (mut fixture, dialog) = shown(CheckSpellingDialog::new(items()));
    press(&mut fixture, "Ignore");
    assert_eq!(dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())), Some((9, 20..27, "recieve".to_owned())));
    press(&mut fixture, "Ignore");
    assert_eq!(dialog.borrow().current(), None, "the walk has passed the end of the list");
    // The list and the actions are untouched around this reader: Ignore changes neither.
    assert_eq!(dialog.borrow().items().len(), 2, "Ignore keeps the listed words");
    assert!(dialog.borrow_mut().take_action().is_none(), "Ignore asks the app for nothing");
}

#[trial]
fn after_an_action_the_kept_position_makes_current_the_next_misspelling() {
    let (mut fixture, dialog) = shown(CheckSpellingDialog::new(items()));
    // Change on the first word: the app replaces it and lists again without it, and `set_items` keeps the position.
    press(&mut fixture, "Change");
    let action = dialog.borrow_mut().take_action();
    assert!(
        matches!(action, Some(SpellingAction::Change { ref item, .. }) if item.word == "teh" && item.object == 7),
        "Change hands the app the first item: {action:?}"
    );
    dialog.borrow_mut().set_items(vec![items().remove(1)]);
    assert_eq!(
        dialog.borrow().current().map(|item| (item.object, item.range.clone(), item.word.clone())),
        Some((9, 20..27, "recieve".to_owned())),
        "the same position is now the next misspelling"
    );
}
