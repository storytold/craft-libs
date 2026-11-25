//! An input method's delete-surrounding edits reach a framework text field: the reported range around the caret is
//! deleted, in characters or in bytes of UTF-8 text rounded to whole characters.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Event, ImeEvent, Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// A focused field holding `start`, its caret `left` characters before the end.
fn field(start: &str, left: usize) -> (Fixture, Rc<RefCell<String>>) {
    let text = Rc::new(RefCell::new(start.to_owned()));
    let held = text.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).show(ui);
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the field");
    fixture.click(&name).settle();
    fixture.key(Key::End, Modifiers::NONE).settle();
    for _ in 0..left {
        fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    }
    (fixture, text)
}

#[trial]
fn the_input_methods_byte_range_before_the_caret_is_deleted() {
    let (mut fixture, text) = field("abc", 0);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 2, after_bytes: 0 })]).settle();
    assert_eq!(*text.borrow(), "a", "the two bytes before the caret");
    fixture.type_text("Z").settle();
    assert_eq!(*text.borrow(), "aZ", "the caret stays where the deleted text began");
}

#[trial]
fn a_byte_range_deletes_whole_characters_only() {
    let (mut fixture, text) = field("aé🙂z", 1);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 4, after_bytes: 0 })]).settle();
    assert_eq!(*text.borrow(), "aéz", "the four bytes of the emoji before the caret");
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 })]).settle();
    assert_eq!(*text.borrow(), "aéz", "a range that ends inside \"é\" deletes nothing of it");
}

#[trial]
fn the_input_methods_characters_after_the_caret_are_deleted() {
    let (mut fixture, text) = field("abcd", 3);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 2 })]).settle();
    assert_eq!(*text.borrow(), "d", "one character before the caret and two after it");
}

/// A range wider than the text around the caret deletes what there is, rather than underflowing its count of
/// characters before it.
#[trial]
fn a_character_range_wider_than_the_text_deletes_what_there_is() {
    let (mut fixture, text) = field("ab", 0);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 5, after_chars: 0 })]).settle();
    assert_eq!(*text.borrow(), "", "five characters asked of two bytes before the caret");
    let (mut fixture, text) = field("ab", 2);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 0, after_chars: 5 })]).settle();
    assert_eq!(*text.borrow(), "", "and five asked after it");
}
