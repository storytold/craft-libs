//! The hex field: uppercase digits behind a fixed `#` that is not part of the text, a typed colour applies before
//! Enter, one digit per channel reads and writes web colours, and a draft that is no colour says why.

use craft_theme::Rgba;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::testing::pointer::moved;
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

const NOT_HEX: &str = "Not a hex color";

fn reasons(fixture: &Fixture) -> usize {
    fixture.texts().iter().filter(|run| run.text == NOT_HEX).count()
}

/// A draft that is no colour says why: a field row shows the reason as its status line, a field no row holds shows
/// it as its own tooltip, and either way assistive technology reads it as the field's description. A colour clears
/// it.
#[trial]
fn a_draft_that_is_no_colour_says_why_in_its_row_or_its_own_tooltip() {
    for in_row in [true, false] {
        let value = Rc::new(RefCell::new(Rgba::rgb(0x33, 0x66, 0x99)));
        let shown = value.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut value = shown.borrow_mut();
            if in_row {
                Field::new("Hex color").show(ui, |ui, _| HexField::hex("Hex color", Bind::bound(&mut *value)).show(ui));
            } else {
                HexField::hex("Hex color", Bind::bound(&mut *value)).show(ui);
            }
        });
        fixture.settle();
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.type_text("12");
        fixture.settle();
        let field = fixture.by_label("Hex color").expect("the hex field");
        assert_eq!(fixture.tree().description_of(field.node), Some(NOT_HEX), "in a row: {in_row}");
        assert_eq!(reasons(&fixture), usize::from(in_row), "only a row shows the reason before any rest; in a row: {in_row}");
        fixture.frame(vec![moved(field.center())]);
        fixture.advance(DELAY);
        fixture.settle();
        assert_eq!(reasons(&fixture), 1, "a rest on the field shows the reason once; in a row: {in_row}");

        fixture.key(Key::A, Modifiers::COMMAND);
        fixture.type_text("336699");
        fixture.settle();
        let field = fixture.by_label("Hex color").expect("the hex field");
        assert_eq!(fixture.tree().description_of(field.node), None, "a colour clears the reason; in a row: {in_row}");
        assert_eq!(reasons(&fixture), 0, "in a row: {in_row}");
    }
}
