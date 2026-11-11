//! List rows: the rename draft, the row menu and the trailing parts a row reveals.

use craft_behavior::{EditEvent, Mixed, keyed};
use craft_ui::contract::{CollectionEvent, Props as _, ShowCollection as _, request_rename};
use craft_ui::list::{List, ListRow, RowEvent};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const NAMES: [&str; 3] = ["Artboard 1", "Artboard 2", "Artboard 3"];

/// The list's id, which a rename request names.
fn list_id() -> Id {
    Id::NULL.with("rows")
}

/// A list of the three rows whose rename draft is `draft`, and what it reported.
fn rig(draft: fn(&usize) -> Option<String>) -> (Fixture, Rc<RefCell<Vec<CollectionEvent<usize, RowEvent>>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let held = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key).with_text(|key| std::borrow::Cow::Borrowed(NAMES[*key]));
        let list = List::new("Artboards").id(list_id()).rename(true).rename_draft(draft);
        let out = list.show_items(ui, &source, &mut |key, _| ListRow::new(NAMES[*key]));
        held.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    (fixture, seen)
}

/// The text the rename field of `row` opens with.
fn opened_with(draft: fn(&usize) -> Option<String>, row: usize) -> Option<String> {
    let (mut fixture, seen) = rig(draft);
    request_rename(fixture.ctx(), list_id(), &row);
    fixture.settle();
    seen.borrow().iter().find_map(|event| match event {
        CollectionEvent::Rename(EditEvent::Begin { before: Mixed::Same(rename), .. }) => Some(rename.name.clone()),
        _ => None,
    })
}

/// A rename draft replaces the row's text in the field it opens; without one the field holds the row's name.
#[trial]
fn a_rename_draft_opens_the_field_with_it_and_without_one_the_field_holds_the_name() {
    let empty_for_first: fn(&usize) -> Option<String> = |row| (*row == 0).then(String::new);
    assert_eq!(opened_with(empty_for_first, 0).as_deref(), Some(""), "the draft opens the field empty");
    assert_eq!(opened_with(empty_for_first, 1).as_deref(), Some(NAMES[1]), "a row without a draft opens with its name");
}
