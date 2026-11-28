//! `AltTextField`: the box and the check are named, typing commits the description, and the check marks every
//! target decorative, which disables the box without clearing its text.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{AltText, AltTextField};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

const PROMPT: &str = "Describe this object";

fn rig(start: AltText) -> (Fixture, Rc<RefCell<AltText>>) {
    let value = Rc::new(RefCell::new(start));
    let shown = value.clone();
    let mut fixture = Fixture::new(move |ui| {
        let _ = AltTextField::new(PROMPT, Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    (fixture, value)
}

/// Whether the description box is disabled in the access tree.
fn box_disabled(fixture: &Fixture) -> bool {
    let boxes = fixture.by_role(Role::MULTILINE_TEXT_INPUT);
    let node = boxes.first().expect("the description box is a multi-line text input").node;
    fixture.tree().is_disabled_key(node)
}

#[trial]
fn marking_decorative_disables_the_description_and_keeps_its_text() {
    let (mut fixture, value) = rig(AltText::new(""));
    assert!(fixture.by_label(PROMPT).is_some(), "the field is named by its prompt");
    let check = fixture.by_role(Role::CHECK_BOX).remove(0);
    assert!(!box_disabled(&fixture), "the box edits while the object is not decorative");

    let description = fixture.by_role(Role::MULTILINE_TEXT_INPUT).remove(0);
    fixture.click(&description);
    fixture.type_text("A green circle");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(value.borrow().text, Mixed::Same("A green circle".to_owned()), "typing commits the description");

    fixture.click(&check);
    fixture.idle(2);
    assert_eq!(value.borrow().decorative, Mixed::Same(true), "the check marks the object decorative");
    assert_eq!(value.borrow().text, Mixed::Same("A green circle".to_owned()), "the description keeps its text");
    assert!(box_disabled(&fixture), "the box is disabled while every target is decorative");

    let (mut fixture, value) = rig(AltText::parts(Mixed::Same("Kept".to_owned()), Mixed::Mixed));
    assert!(!box_disabled(&fixture), "the box edits while only some targets are decorative");
    let check = fixture.by_role(Role::CHECK_BOX).remove(0);
    fixture.click(&check);
    fixture.idle(2);
    assert_eq!(value.borrow().decorative, Mixed::Same(true), "a mixed check sets every target decorative");
    assert_eq!(value.borrow().text, Mixed::Same("Kept".to_owned()));
    assert!(box_disabled(&fixture));
}

/// Focus that moves from the description box to the check, another part of the same component, is the box losing its
/// key focus: the field reports it once, in the pass it happens, and not while the focus stays in the box.
#[trial]
fn focus_moving_to_the_check_reports_the_box_losing_it_once() {
    let value = Rc::new(RefCell::new(AltText::new("")));
    let lost = Rc::new(std::cell::Cell::new(0_usize));
    let (shown, seen) = (value.clone(), lost.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = AltTextField::new(PROMPT, Bind::bound(&mut *shown.borrow_mut())).show(ui);
        seen.set(seen.get() + usize::from(out.text_lost_focus));
    });
    fixture.settle();
    let description = fixture.by_role(Role::MULTILINE_TEXT_INPUT).remove(0);
    fixture.click(&description);
    fixture.idle(3);
    assert_eq!(lost.get(), 0, "the box keeps its focus while it is typed in");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(lost.get(), 1, "Tab to the check reports the loss once");
}
