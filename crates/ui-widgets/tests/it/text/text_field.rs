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

#[trial]
fn select_all_after_typing_selects_the_complete_new_draft() {
    let text = Rc::new(RefCell::new("4".to_owned()));
    let request = Rc::new(Cell::new(false));
    let (shown, select) = (text.clone(), request.clone());
    let mut fixture = Fixture::new(move |ui| {
        TextField::new("Name", Bind::bound(&mut *shown.borrow_mut())).select_on_focus().select_all(select.replace(false)).show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    *text.borrow_mut() = "iv".to_owned();
    request.set(true);
    fixture.type_text("2");
    assert_eq!(*text.borrow(), "2", "typing replaces the replacement value");
    fixture.type_text("x").settle();
    assert_eq!(*text.borrow(), "2x", "the next input appends to the new draft");
}

/// A letter typed into a text field is the field's own: the field gets the text, the app reads no press of it, and
/// `focus_holds` names the letter, and Space, before they are typed.
#[trial]
fn a_letter_typed_into_a_field_is_its_own() {
    let text = Rc::new(RefCell::new(String::new()));
    let app_read = Rc::new(RefCell::new(Vec::new()));
    let (shown, read) = (text.clone(), app_read.clone());
    let mut fixture = Fixture::new(move |ui| {
        read.borrow_mut().extend(ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        }));
        TextField::new("Name", Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let holds = |key| craft_ui::runtime::keys::focus_holds(fixture.ctx(), craft_keymap::Chord::plain(key));
    assert!(holds(craft_keymap::Key::T) && holds(craft_keymap::Key::Space), "the field names a letter and Space as its own");
    fixture.frame(vec![craft_ui::testing::input::key_press(Key::T, Modifiers::NONE), craft_ui::testing::input::text("t")]);
    fixture.frame(vec![craft_ui::testing::focus::key_release(Key::T, Modifiers::NONE)]);
    fixture.idle(2);
    assert_eq!(*text.borrow(), "t", "the field gets the text");
    assert!(!app_read.borrow().contains(&Key::T), "and the app reads no press of it");
}

/// A field a click opens with a `Take` request, sent in that one pass, takes the typed keys at once: the press on
/// the opening button does not take the focus back. A field shown later with a `WhenNone` request leaves the focus
/// where it is.
#[trial]
fn a_one_shot_focus_request_takes_the_keys_and_when_none_leaves_a_held_focus() {
    use craft_ui::buttons::Button;
    use craft_ui::runtime::focus::FocusRequest;
    let (inline, search) = (Rc::new(RefCell::new(String::new())), Rc::new(RefCell::new(String::new())));
    let (open, show_search) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let (typed, found, opened, searching) = (inline.clone(), search.clone(), open.clone(), show_search.clone());
    let search_sent = Rc::new(Cell::new(false));
    let mut fixture = Fixture::new(move |ui| {
        let clicked = Button::new("Edit").show(ui).clicks > 0;
        if opened.get() || clicked {
            let mut typed = typed.borrow_mut();
            let field = TextField::new("Inline", Bind::bound(&mut *typed)).id_salt("inline");
            let field = if clicked && !opened.get() { field.request_focus(FocusRequest::Take) } else { field };
            field.show(ui);
            opened.set(true);
        }
        if searching.get() {
            let mut found = found.borrow_mut();
            let field = TextField::new("Search", Bind::bound(&mut *found)).id_salt("search");
            let field = if search_sent.replace(true) { field } else { field.request_focus(FocusRequest::WhenNone) };
            field.show(ui);
        }
    });
    fixture.settle();
    let edit = fixture.by_label("Edit").expect("the button shows");
    fixture.click(&edit).settle();
    assert!(open.get(), "the click opened the field");
    fixture.type_text("abc");
    fixture.idle(2);
    assert_eq!(*inline.borrow(), "abc", "the opened field takes the keys at once");

    show_search.set(true);
    fixture.settle();
    fixture.type_text("d");
    fixture.idle(2);
    assert_eq!(*inline.borrow(), "abcd", "the focused field keeps the keys");
    assert_eq!(*search.borrow(), "", "a `WhenNone` request takes no focus a field holds");
}
