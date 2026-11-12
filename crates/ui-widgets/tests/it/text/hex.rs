//! The hex field: uppercase digits behind a fixed `#` that is not part of the text, a typed colour applies before
//! Enter, and one digit per channel reads and writes web colours.

use craft_theme::Rgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::HexField;
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

fn focused(start: Rgba, digits: usize) -> (Fixture, Rc<RefCell<Rgba>>) {
    let value = Rc::new(RefCell::new(start));
    let shown = value.clone();
    let mut fixture = Fixture::new(move |ui| {
        HexField::hex("Hex color", Bind::bound(&mut *shown.borrow_mut())).digits(digits).show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    (fixture, value)
}

fn text(fixture: &Fixture) -> String {
    let field = fixture.by_label("Hex color").expect("the hex field");
    fixture.tree().value_of(field.node).unwrap_or_default().to_owned()
}

#[trial]
fn a_typed_colour_applies_before_enter_behind_a_fixed_hash() {
    let (mut fixture, value) = focused(Rgba::rgb(0xff, 0x88, 0x00), 2);
    assert_eq!(text(&fixture), "FF8800", "uppercase digits; the # is not in the text");
    fixture.type_text("336699");
    assert_eq!(*value.borrow(), Rgba::rgb(0x33, 0x66, 0x99), "six digits are a colour and apply as typed");
    fixture.key(Key::A, Modifiers::COMMAND).key(Key::Backspace, Modifiers::NONE);
    assert_eq!(text(&fixture), "", "deleting everything leaves no #");

    let (mut web, value) = focused(Rgba::rgb(0x33, 0x66, 0x99), 1);
    assert_eq!(text(&web), "369");
    web.type_text("F80");
    assert_eq!(*value.borrow(), Rgba::rgb(0xff, 0x88, 0x00), "one digit per channel doubles each");
}
