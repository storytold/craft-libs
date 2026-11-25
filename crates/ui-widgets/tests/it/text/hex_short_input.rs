//! The hex field's short input (`HexCodec::short_input`), the reference image editor's Color Picker `#` field: fewer
//! than six digits pad on the left with zeros, three still read as the shorthand, more than six read as `FFFFFF` with
//! the typed number noted as held, and fields that do not opt in keep their reading (photocraft#16739).

use craft_behavior::Codec as _;
use craft_theme::Rgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{HexCodec, HexField};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

const START: Rgba = Rgba([0xff, 0x80, 0x00, 0x40]);

fn read(codec: HexCodec, text: &str) -> Option<Rgba> {
    codec.parse(text, &START).ok()
}

#[trial]
fn short_input_pads_with_zeros_and_holds_long_input_to_ffffff() {
    let codec = HexCodec::default().short_input(true);
    assert_eq!(read(codec, "12"), Some(Rgba([0x00, 0x00, 0x12, 0x40])), "two digits pad on the left");
    assert_eq!(read(codec, "1"), Some(Rgba([0x00, 0x00, 0x01, 0x40])), "one digit pads on the left");
    assert_eq!(read(codec, "1234"), Some(Rgba([0x00, 0x12, 0x34, 0x40])), "four digits pad on the left");
    assert_eq!(read(codec, "12345"), Some(Rgba([0x01, 0x23, 0x45, 0x40])), "five digits pad on the left");
    assert_eq!(read(codec, "abc"), Some(Rgba([0xaa, 0xbb, 0xcc, 0x40])), "three digits are still the shorthand");
    assert_eq!(read(codec, "#ff00ff"), Some(Rgba([0xff, 0x00, 0xff, 0x40])), "six digits and a leading # read as today");
    assert_eq!(read(codec, "zzz"), None, "a character that is no digit is no colour");
    assert_eq!(read(codec, ""), None, "nothing typed is no colour");
    assert_eq!(read(codec, "#"), None, "a lone # is no colour");

    let (value, note) = codec.parse_tagged("1234567", &START).expect("seven digits read");
    assert_eq!(value, Rgba([0xff, 0xff, 0xff, 0x40]), "seven digits read as the closest value, keeping the alpha");
    assert_eq!(note.held, Some(f64::from(0x0123_4567)), "the note carries the number typed");
    let (value, note) = codec.parse_tagged(&"f".repeat(1024), &START).expect("a run of digits reads");
    assert_eq!(value, Rgba([0xff, 0xff, 0xff, 0x40]));
    assert!(note.held.is_some(), "an absurdly long number is held too, without a panic");
    let (_, note) = codec.parse_tagged("123456", &START).expect("six digits read");
    assert_eq!(note.held, None, "six digits are held to nothing");

    assert!(codec.applies_as_typed("123456"), "six digits apply as typed");
    for text in ["1", "12", "abc", "1234", "12345", "1234567"] {
        assert!(!codec.applies_as_typed(text), "{text} is the start of a colour or past one: it applies on commit");
    }
}

/// Fields that do not opt in, and the forms short input does not cover (alpha, one digit per channel), read as before.
#[trial]
fn fields_without_short_input_keep_their_reading() {
    let plain = HexCodec::default();
    assert_eq!(read(plain, "12"), None);
    assert_eq!(read(plain, "1234567"), None);
    assert_eq!(read(plain, "abc"), Some(Rgba([0xaa, 0xbb, 0xcc, 0x40])));
    let alpha = HexCodec::default().alpha(true).short_input(true);
    assert_eq!(read(alpha, "12"), None, "an alpha field ignores short input");
    assert_eq!(read(alpha, "11223344"), Some(Rgba([0x11, 0x22, 0x33, 0x44])));
    let web = HexCodec::default().digits(1).short_input(true);
    assert_eq!(read(web, "12"), None, "a one-digit field ignores short input");
    assert_eq!(read(web, "f80"), Some(Rgba([0xff, 0x88, 0x00, 0x40])));
}

/// In a field, a short or long commit applies on Enter and the held number reaches `held_into` in that pass only.
#[trial]
fn a_field_with_short_input_commits_padded_and_held_values() {
    let commit = |text: &str| {
        let value = Rc::new(RefCell::new(Rgba::rgb(0xff, 0x80, 0x00)));
        let held = Rc::new(RefCell::new(Vec::new()));
        let (shown, seen) = (value.clone(), held.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut out = None;
            HexField::hex("Hex color", Bind::bound(&mut *shown.borrow_mut())).short_input(true).held_into(&mut out).show(ui);
            if let Some(number) = out {
                seen.borrow_mut().push(number);
            }
        });
        fixture.settle();
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.type_text(text);
        fixture.settle();
        let typing = *value.borrow();
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
        let committed = *value.borrow();
        let held = held.borrow().clone();
        (typing, committed, held)
    };
    let (typing, committed, held) = commit("12");
    assert_eq!(typing, Rgba::rgb(0xff, 0x80, 0x00), "a short draft waits for the commit");
    assert_eq!(committed, Rgba::rgb(0x00, 0x00, 0x12), "12 commits as 000012");
    assert!(held.is_empty(), "a padded value is no held one");
    let (_, committed, held) = commit("1234567");
    assert_eq!(committed, Rgba::rgb(0xff, 0xff, 0xff), "seven digits commit as FFFFFF");
    assert_eq!(held, [f64::from(0x0123_4567)], "the held number is reported once");
    let (_, committed, held) = commit("336699");
    assert_eq!(committed, Rgba::rgb(0x33, 0x66, 0x99), "six digits as today");
    assert!(held.is_empty());
}
