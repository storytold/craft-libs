//! `TextField`: the search form's clear part empties the text; a multi-line field grows to `max_lines` or fills its
//! room and scrolls its text inside; a bound caret goes both ways.

use craft_behavior::PartKey;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CLEAR: PartKey = PartKey::new("clear");

/// Half a point: rects compared across passes may round apart by that much.
const SNAP: f32 = 0.5;

/// The room a filling field is shown in.
const ROOM: Vec2 = Vec2::new(320.0, 240.0);

/// Where a field's text is painted, found by a piece of it no label holds: the whole text, scrolled past the field's
/// edges when it is taller.
fn run_with(fixture: &Fixture, piece: &str) -> Rect {
    fixture
        .texts()
        .iter()
        .find(|run| run.text.contains(piece))
        .map(|run| run.rect)
        .unwrap_or_else(|| panic!("the field paints its text holding {piece:?}"))
}

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

/// A two-line field capped at four grows with each typed line past its two up to four, then keeps its height: the
/// text scrolls inside it with the caret's line, the last, in view, and the wheel over it scrolls the text back.
#[trial]
fn a_capped_field_grows_to_its_lines_then_scrolls_with_the_caret_in_view() {
    let text = Rc::new(RefCell::new(String::new()));
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let (shown, seen) = (text.clone(), rect.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.set(TextField::new("Notes", Bind::bound(&mut *shown.borrow_mut())).lines(2).max_lines(4).show(ui).response.rect);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let mut heights = Vec::new();
    for line in 1..=7 {
        if line > 1 {
            fixture.key(Key::Enter, Modifiers::NONE);
        }
        fixture.type_text(&line.to_string());
        fixture.idle(3);
        heights.push(rect.get().height());
    }
    assert_eq!(text.borrow().lines().count(), 7, "Enter breaks lines in a multi-line field: {:?}", text.borrow());
    assert!((heights[1] - heights[0]).abs() <= SNAP, "one and two lines fit the field's two lines: {heights:?}");
    assert!(heights[2] > heights[1] + 1.0 && heights[3] > heights[2] + 1.0, "the field grows with its third and fourth lines: {heights:?}");
    assert!(heights[4..].iter().all(|height| (height - heights[3]).abs() <= SNAP), "past four lines the field keeps its height: {heights:?}");

    let field = rect.get();
    let run = run_with(&fixture, "\n7");
    assert!(run.height() > field.height(), "the text {run:?} is taller than the capped field {field:?}");
    assert!(run.top() < field.top(), "the text scrolls inside the field: {run:?} in {field:?}");
    assert!(run.bottom() <= field.bottom() + SNAP, "the caret's line, the last, stays in view: {run:?} in {field:?}");

    // The text part is the wheel's scroll owner over the field; a wheel up scrolls its text back down.
    fixture.frame(vec![pointer::moved(field.center())]);
    fixture.frame(vec![pointer::wheel_lines(1.0)]);
    fixture.idle(3);
    let back = run_with(&fixture, "\n7");
    assert!(back.top() > run.top() + 1.0, "the wheel scrolls the text inside the field: {run:?} to {back:?}");
    assert!((rect.get().height() - field.height()).abs() <= SNAP, "the wheel leaves the field's height alone");
}

/// A multi-line field sized `Sizing::Fill` takes the room it is offered, with no regard for `max_lines` or its text's
/// height, and its text scrolls inside it with the caret's line in view.
#[trial]
fn a_filling_field_takes_the_proposal_and_ignores_max_lines() {
    const LINES: usize = 30;
    let text = Rc::new(RefCell::new((1..=LINES).map(|line| line.to_string()).collect::<Vec<_>>().join("\n")));
    let (rect, room) = (Rc::new(Cell::new(Rect::NOTHING)), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown, seen, given) = (text.clone(), rect.clone(), room.clone());
    let mut fixture = Fixture::new(move |ui| {
        let at = Rect::from_min_size(ui.max_rect().min, ROOM);
        given.set(at);
        let mut held = shown.borrow_mut();
        Stack::new().fill().show_at(ui, at, |ui, _| {
            let field = TextField::new("Notes", Bind::bound(&mut *held)).lines(2).max_lines(3).sizing(Sizing::Fill);
            seen.set(field.show(ui).response.rect);
        });
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("!");
    fixture.idle(3);
    assert!(text.borrow().ends_with("30!"), "the typing lands at the end: {:?}", text.borrow());

    let (field, room) = (rect.get(), room.get());
    let run = run_with(&fixture, "\n30");
    let line = run.height() / LINES as f32;
    assert!(room.expand(SNAP).contains_rect(field), "the filling field {field:?} stays in its room {room:?}");
    assert!(field.height() > room.height() * 0.75 && field.width() > room.width() * 0.75, "the field takes the room: {field:?} in {room:?}");
    assert!(field.height() > 4.0 * line, "the field is taller than its three `max_lines`: {field:?}, a line {line}");
    assert!(run.height() > field.height(), "the text {run:?} is taller than the room it was given {field:?}");
    assert!(run.top() < field.top(), "the text scrolls inside the field: {run:?} in {field:?}");
    assert!(run.bottom() <= field.bottom() + SNAP, "the caret's line, the last, stays in view: {run:?} in {field:?}");
}

/// A caret the app writes lands at the character boundary at or before its byte offset, collapses the selection and
/// scrolls into view; the user moving the caret writes its byte offset back to the binding.
#[trial]
fn a_bound_caret_moves_the_caret_and_follows_it() {
    // Six lines in a two-line field; "é" takes bytes 1 and 2.
    const TEXT: &str = "aé\nb\nc\nd\ne\nf";
    let text = Rc::new(RefCell::new(TEXT.to_owned()));
    let caret = Rc::new(Cell::new(0usize));
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let (shown, bound, seen) = (text.clone(), caret.clone(), rect.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut at = bound.get();
        let mut held = shown.borrow_mut();
        let field = TextField::new("Notes", Bind::bound(&mut *held)).lines(2).max_lines(2).caret(Bind::bound(&mut at));
        seen.set(field.show(ui).response.rect);
        drop(held);
        bound.set(at);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let top = run_with(&fixture, "\nf");
    assert!(top.top() >= rect.get().top() - SNAP, "focus arriving keeps the bound caret at the start, the first line in view");

    caret.set(TEXT.len());
    fixture.idle(3);
    let (field, end) = (rect.get(), run_with(&fixture, "\nf"));
    assert!(
        end.top() < field.top() && end.bottom() <= field.bottom() + SNAP,
        "an app caret at the end scrolls the last line into view: {end:?} in {field:?}"
    );
    assert_eq!(caret.get(), TEXT.len(), "the caret the app wrote stays");

    // Everything selected; the app's caret, inside "é", collapses the selection at the boundary before it.
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.settle();
    caret.set(2);
    fixture.idle(3);
    assert_eq!(caret.get(), 1, "the caret lands on the character boundary before the offset and is written back");
    let back = run_with(&fixture, "\nf");
    assert!(back.top() >= rect.get().top() - SNAP, "the caret's line, the first, scrolls back into view: {back:?} in {:?}", rect.get());
    fixture.type_text("X");
    fixture.idle(2);
    assert_eq!(*text.borrow(), "aXé\nb\nc\nd\ne\nf", "the typing goes in at the caret and replaces no selection");
    assert_eq!(caret.get(), 2, "the caret after the typing is written back as a byte offset");

    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(caret.get(), 4, "the user's caret move past the two-byte \"é\" writes its byte offset back");
}

/// The clear part empties the field in the pass that takes its click: the field's node reads empty after that one pass,
/// as an agent's click reply comes with it (the control channel answers with the pass that ran the input).
#[trial]
fn the_clear_part_empties_the_field_in_the_pass_of_its_click() {
    use craft_ui::testing::focus::AccessActionKind;
    let id = Rc::new(Cell::new(Id::NULL));
    let field = id.clone();
    // The app's own text, as a search panel's query is.
    let text = Rc::new(RefCell::new(String::from("abc")));
    let held = text.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut query = held.borrow_mut();
        field.set(TextField::search("Search", Bind::bound(&mut *query)).show(ui).response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(id.get()), Some("abc"));
    let clear = fixture.part(id.get(), CLEAR).expect("a search field with text shows the clear part");
    fixture.access(AccessActionKind::Click, &clear);
    assert_eq!(text.borrow().as_str(), "", "the click emptied the text");
    assert_eq!(fixture.tree().value(id.get()), Some(""), "the field reads empty in the pass of the click");
}
