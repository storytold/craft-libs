//! The hex field: uppercase digits without the `#`, which its row shows, a typed colour applies before
//! Enter, one digit per channel reads and writes web colours, and a draft that is no colour says why.

use craft_theme::Rgba;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::HexField;
use egui::{Event, Key, Modifiers};
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

/// A paste that is a colour replaces the field's digits wherever the caret is, with or without its `#` and spaces; a
/// three-digit paste is the shorthand and applies on commit; a paste that is no colour is inserted as text.
#[trial]
fn a_pasted_colour_replaces_the_digits_with_or_without_its_hash() {
    let start = Rgba::rgb(0xff, 0x88, 0x00);
    let paste = |fixture: &mut Fixture, pasted: &str| {
        fixture.key(Key::End, Modifiers::NONE);
        fixture.frame(vec![Event::Paste(pasted.into())]).settle();
    };
    for pasted in ["336699", "#336699", "  #336699 "] {
        let (mut fixture, value) = focused(start, 2);
        paste(&mut fixture, pasted);
        assert_eq!(*value.borrow(), Rgba::rgb(0x33, 0x66, 0x99), "{pasted:?} replaces the digits with the caret at the end");
    }

    let (mut fixture, value) = focused(start, 2);
    paste(&mut fixture, "0f0");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), Rgba::rgb(0x00, 0xff, 0x00), "three digits read as 00FF00");

    let (mut fixture, value) = focused(start, 2);
    paste(&mut fixture, "12");
    assert_eq!(text(&fixture), "FF880012", "a paste that is no colour is inserted at the caret");
    assert_eq!(*value.borrow(), start, "and the colour stays");
}

/// A content-sized hex field is as wide as its digits in the monospace face it draws them in, not in the body face:
/// all six digits of #336699 and the caret past the last of them lie inside the field's text rect, and none is cut off.
#[trial]
fn a_content_sized_hex_field_shows_all_six_digits() {
    let mut fixture = Fixture::new(|ui| {
        Row::new().show(ui, |ui, _| {
            HexField::hex("Hex", Bind::given(Rgba::rgb(0x33, 0x66, 0x99))).sizing(Sizing::Content).show(ui);
        });
    });
    fixture.settle();
    let digits = fixture.texts().iter().find(|run| run.text == "336699").cloned().unwrap_or_else(|| panic!("the field shows its digits"));
    // The run's clip is egui's own: its editor clips the text to the laid-out text one point outset (the room of a
    // two-point caret centred on the text's end), cut to the field's text part. So the clip shows that no digit is
    // cut, but its end edge follows the text, not the field, and cannot prove the field's width.
    let clip = digits.clip;
    assert!(
        digits.rect.min.x >= clip.min.x && digits.rect.max.x <= clip.max.x,
        "the digits {:?} are not cut by the clip they are painted under {clip:?}",
        digits.rect
    );
    // The field's box proves it: its text part is the box less the field's padding, the same on both sides, and the
    // digits start at the part's start edge. The field measures the whole caret width past the last digit
    // (`MeasureCx::field_text`), at least the half of it egui paints past the text's end.
    let field = fixture.by_label("Hex").unwrap_or_else(|| panic!("the hex field")).rect;
    let text_end = field.max.x - (digits.rect.min.x - field.min.x);
    let caret = fixture.ctx().global_style().visuals.text_cursor.stroke.width;
    assert!(
        digits.rect.max.x + caret <= text_end + 0.01,
        "the digits {:?} and the caret past them lie inside the field's text part, which ends at {text_end} in the field {field:?}",
        digits.rect
    );
}
