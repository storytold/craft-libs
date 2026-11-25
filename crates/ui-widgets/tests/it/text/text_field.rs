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

/// A field that keeps its text on Escape (`keeps_on_escape`, a live filter's query) commits the typing, lets focus
/// go and leaves the same Escape to its enclosing scope; it reports the focus loss as `text_lost_focus`.
#[trial]
fn a_field_that_keeps_its_text_on_escape_commits_leaves_focus_and_passes_the_escape_on() {
    let text = Rc::new(RefCell::new(String::new()));
    let (passed, lost, id) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(false)), Rc::new(Cell::new(Id::NULL)));
    // What the field reported: the commits and cancels of its events and the passes that reported a submit.
    let (commits, cancels, submits) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
    let (shown, scope_saw, lost_seen, field) = (text.clone(), passed.clone(), lost.clone(), id.clone());
    let (seen_commits, seen_cancels, seen_submits) = (commits.clone(), cancels.clone(), submits.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        Stack::new().show(ui, |ui, body| {
            let out = TextField::new("Filter", Bind::bound(&mut *held)).keeps_on_escape().show(ui);
            field.set(out.response.id);
            lost_seen.set(lost_seen.get() || out.text_lost_focus);
            for event in out.events.iter() {
                match event {
                    craft_behavior::EditEvent::Commit { .. } => seen_commits.set(seen_commits.get() + 1),
                    craft_behavior::EditEvent::Cancel { .. } => seen_cancels.set(seen_cancels.get() + 1),
                    _ => {}
                }
            }
            seen_submits.set(seen_submits.get() + u32::from(out.submitted));
            if body.take_cancel() {
                scope_saw.set(scope_saw.get() + 1);
            }
        });
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("abc");
    fixture.idle(2);
    assert!(!lost.get(), "typing loses no focus");
    let (commits_before, cancels_before) = (commits.get(), cancels.get());
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(*text.borrow(), "abc", "Escape keeps the typed text");
    assert_eq!(commits.get() - commits_before, 1, "the open typing run commits exactly once");
    assert_eq!(cancels.get() - cancels_before, 0, "and nothing is cancelled");
    assert_eq!(submits.get(), 0, "the Escape submits nothing");
    assert_eq!(passed.get(), 1, "the enclosing scope gets the one Escape");
    assert_ne!(fixture.focused(), Some(id.get()), "the field lets focus go");
    assert!(lost.get(), "and reports the focus loss");
}

/// Without `keeps_on_escape` a plain field keeps its default: Escape over a typing run restores the text from before
/// it and the field takes that Escape; a search field still clears.
#[trial]
fn without_keeps_on_escape_plain_and_search_fields_keep_their_escape() {
    let (plain, search) = (Rc::new(RefCell::new(String::from("old"))), Rc::new(RefCell::new(String::new())));
    let passed = Rc::new(Cell::new(0_u32));
    let (a, b, scope_saw) = (plain.clone(), search.clone(), passed.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (mut first, mut second) = (a.borrow_mut(), b.borrow_mut());
        Stack::new().show(ui, |ui, body| {
            TextField::new("Plain", Bind::bound(&mut *first)).id_salt("plain").select_on_focus().show(ui);
            TextField::search("Search", Bind::bound(&mut *second)).id_salt("search").show(ui);
            if body.take_cancel() {
                scope_saw.set(scope_saw.get() + 1);
            }
        });
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("new");
    fixture.idle(1);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(*plain.borrow(), "old", "a plain field's Escape restores the text from before the run");
    assert_eq!(passed.get(), 0, "and the field takes that Escape");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(1);
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("q");
    fixture.idle(1);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(*search.borrow(), "", "a search field's Escape still clears");
}

/// `text_lost_focus` reports a real focus loss without an edit, and nothing while the field keeps focus.
#[trial]
fn text_lost_focus_reports_a_focus_loss_without_an_edit() {
    let (first, second) = (Rc::new(RefCell::new(String::from("one"))), Rc::new(RefCell::new(String::from("two"))));
    let lost = Rc::new(Cell::new(0_u32));
    let (a, b, seen) = (first.clone(), second.clone(), lost.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = TextField::new("First", Bind::bound(&mut *a.borrow_mut())).id_salt("first").show(ui);
        if out.text_lost_focus {
            seen.set(seen.get() + 1);
        }
        TextField::new("Second", Bind::bound(&mut *b.borrow_mut())).id_salt("second").show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(lost.get(), 0, "a focused field reports no loss");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(lost.get(), 1, "Tab off the unedited field reports one loss");
    assert_eq!(*first.borrow(), "one", "and edits nothing");
}

/// A field that keeps its text on Escape does so only while it can be edited: one that went read-only in the middle of
/// a typing run takes Escape as a plain field does, so the run is cancelled, not committed. (A read-only component
/// refuses every event that carries a value, so the cancel reaches the app as an event and writes nothing back.)
#[trial]
fn a_field_that_went_read_only_mid_run_cancels_its_run_on_escape() {
    let text = Rc::new(RefCell::new(String::from("old")));
    let read_only = Rc::new(Cell::new(false));
    let events = Rc::new(RefCell::new(Vec::new()));
    let id = Rc::new(Cell::new(Id::NULL));
    let (shown, locked, log, field) = (text.clone(), read_only.clone(), events.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let out = TextField::new("Filter", Bind::bound(&mut *held)).keeps_on_escape().select_on_focus().read_only(locked.get()).show(ui);
        field.set(out.response.id);
        log.borrow_mut().extend(out.events.iter().map(|event| match event {
            craft_behavior::EditEvent::Commit { .. } => "commit",
            craft_behavior::EditEvent::Cancel { .. } => "cancel",
            _ => "other",
        }));
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("new");
    fixture.idle(2);
    assert_eq!(*text.borrow(), "new", "the typing reached the binding");
    read_only.set(true);
    fixture.idle(2);
    events.borrow_mut().clear();
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert!(!events.borrow().contains(&"commit"), "the run is not committed: {:?}", events.borrow());
    assert!(events.borrow().contains(&"cancel"), "the run is cancelled: {:?}", events.borrow());
    // The draft is gone: the field shows the host's value again, not the text typed before the run ended.
    *text.borrow_mut() = String::from("host");
    fixture.idle(3);
    assert_eq!(fixture.tree().value(id.get()), Some("host"), "the field shows the host's value");
}

/// A kept Escape that closes the popover the field sits in hands focus back to the popover's opener, as any Escape that
/// closes it does: the field is let go only after the popover's focus restoration has found it holding the focus.
#[trial]
fn a_kept_escape_that_closes_its_popover_returns_focus_to_the_opener() {
    use craft_ui::buttons::Button;
    use craft_ui::popover::Popover;
    use craft_ui::testing::contract::clicked;
    let open = Rc::new(Cell::new(false));
    let text = Rc::new(RefCell::new(String::new()));
    let opener_id = Rc::new(Cell::new(Id::NULL));
    let (shown, held, opener_seen) = (open.clone(), text.clone(), opener_id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Filter options").show(ui);
        opener_seen.set(opener.response.id);
        let mut is_open = shown.get();
        if clicked(&opener) {
            is_open = !is_open;
        }
        Popover::new("Filter", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            let mut query = held.borrow_mut();
            TextField::new("Query", Bind::bound(&mut *query)).keeps_on_escape().show(ui);
        });
        shown.set(is_open);
    });
    fixture.settle();
    let button = fixture.by_label("Filter options").expect("the opener");
    fixture.click(&button);
    fixture.idle(4);
    assert!(open.get() && fixture.by_label("Query").is_some(), "the popover opened on its field");
    fixture.type_text("ab");
    fixture.idle(2);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(6);
    assert_eq!(*text.borrow(), "ab", "the kept Escape keeps the text");
    assert!(!open.get(), "the Escape closed the popover");
    assert_eq!(fixture.focused(), Some(opener_id.get()), "focus returned to the opener");
}
