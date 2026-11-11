//! `TextField`: the search form's clear part empties the text.

use craft_behavior::PartKey;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CLEAR: PartKey = PartKey::new("clear");

/// Clearing a search field that has an open typing run empties it for good: the clear part goes, and leaving the
/// field commits nothing back.
#[trial]
fn clearing_a_search_field_with_a_draft_leaves_it_empty_after_blur() {
    let (text, other) = (Rc::new(RefCell::new(String::new())), Rc::new(RefCell::new(String::new())));
    let id = Rc::new(Cell::new(Id::NULL));
    let (shown, beside, field) = (text.clone(), other.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        field.set(TextField::search("Search", Bind::bound(&mut *shown.borrow_mut())).show(ui).response.id);
        TextField::new("Other", Bind::bound(&mut *beside.borrow_mut())).id_salt("other").show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("abc");
    fixture.idle(2);
    assert_eq!(*text.borrow(), "abc");
    let clear = fixture.part(id.get(), CLEAR).expect("a search field with text shows the clear part");
    fixture.click(&clear);
    fixture.idle(3);
    assert_eq!(*text.borrow(), "");
    assert!(fixture.part(id.get(), CLEAR).is_none(), "an empty search field shows no clear part");
    assert!(fixture.texts().iter().all(|run| !run.text.contains("abc")), "the field shows the cleared text");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(*text.borrow(), "", "leaving the field commits nothing back");
}
