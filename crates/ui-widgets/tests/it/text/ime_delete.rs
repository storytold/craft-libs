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

/// A delete before marked text moves the caret the text part keeps with it: a composition that then ends without a
/// commit (Escape, an empty pre-edit) leaves the caret where the marked text was, not that many characters further
/// right.
#[trial]
fn a_delete_before_marked_text_moves_the_caret_it_leaves_behind() {
    let text = Rc::new(RefCell::new("abcd".to_owned()));
    let field = Rc::new(std::cell::Cell::new(egui::Id::NULL));
    let (held, seen) = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.set(TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).show(ui).response.id);
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the field");
    fixture.click(&name).settle();
    fixture.key(Key::End, Modifiers::NONE).settle();
    fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    // The input method marks "XY" after the caret's "abc", deletes the "c" before it, then ends with nothing committed.
    fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "XY".to_owned(), active_range_chars: None })]).settle();
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 })]).settle();
    fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None })]).settle();
    let widget = craft_ui::testing::text::widget_id(field.get(), craft_behavior::PartKey::new("text"));
    let caret =
        egui::text_edit::TextEditState::load(fixture.ctx(), widget).and_then(|state| state.cursor.char_range()).map(|range| range.primary.index.0);
    assert_eq!(caret, Some(2), "the caret sits where the marked text was, after \"ab\"");
    fixture.type_text("Z").settle();
    assert_eq!(*text.borrow(), "abZd", "typing then goes in between \"ab\" and \"d\"");
}

/// A delete after the marked text moves the saved selection's own endpoints with it: one that lay beyond the deleted
/// characters loses them, so the next edit replaces the characters the selection covered and not one more.
#[trial]
fn a_delete_after_marked_text_moves_the_selection_endpoints_beyond_it() {
    let text = Rc::new(RefCell::new("abcdef".to_owned()));
    let field = Rc::new(std::cell::Cell::new(egui::Id::NULL));
    let (held, seen) = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.set(TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).show(ui).response.id);
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the field");
    fixture.click(&name).settle();
    let widget = craft_ui::testing::text::widget_id(field.get(), craft_behavior::PartKey::new("text"));
    // The saved selection is "cde" (caret 5, anchor 2), and the input method marks its text at the anchor.
    let mut state = egui::text_edit::TextEditState::load(fixture.ctx(), widget).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(2), egui::text::CCursor::new(5))));
    state.store(fixture.ctx(), widget);
    fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "XY".to_owned(), active_range_chars: None })]).settle();
    // One character is deleted after the marked text: "c" goes, and the caret that lay at 5 belongs at 4.
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 0, after_chars: 1 })]).settle();
    fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None })]).settle();
    fixture.type_text("Z").settle();
    assert_eq!(*text.borrow(), "abZf", "the caret followed the deletion, so Z replaces only the \"de\" the selection still covers");
}

/// A deletion after a selection leaves the selection covering the same characters: the text the input method deletes
/// is the stretch after it, not the selection itself.
#[trial]
fn a_delete_after_a_selection_leaves_the_selection_on_its_characters() {
    let text = Rc::new(RefCell::new("abcdef".to_owned()));
    let field = Rc::new(std::cell::Cell::new(egui::Id::NULL));
    let (held, seen) = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.set(TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).show(ui).response.id);
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the field");
    fixture.click(&name).settle();
    let widget = craft_ui::testing::text::widget_id(field.get(), craft_behavior::PartKey::new("text"));
    // "cde" is selected, and the input method deletes the character after it.
    let mut state = egui::text_edit::TextEditState::load(fixture.ctx(), widget).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(2), egui::text::CCursor::new(5))));
    state.store(fixture.ctx(), widget);
    fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 0, after_chars: 1 })]).settle();
    assert_eq!(*text.borrow(), "abcde", "the character after the selection is the one deleted");
    fixture.type_text("Z").settle();
    assert_eq!(*text.borrow(), "abZ", "the selection still covers \"cde\", so Z replaces it");
}
