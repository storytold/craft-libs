//! A derived number (an automatic leading) shows in parentheses until it is edited, and an edit starts from the plain number.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// The value the automatic setting works out.
const COMPUTED: f64 = 87.6;

/// A derived field is read out in parentheses with its unit; focusing it shows the plain number to edit, and a
/// committed number is the field's value.
#[trial]
fn a_derived_number_shows_in_parentheses_and_edits_from_the_plain_number() {
    let value = Rc::new(Cell::new(Some(COMPUTED)));
    let id = Rc::new(Cell::new(egui::Id::NULL));
    let (shown, seen) = (value.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut current = shown.get();
        let out = NumberField::new("Leading", Bind::bound(&mut current)).id_salt("leading").unit_kind(UnitKind::Points).derived(true).show(ui);
        seen.set(out.response.id);
        shown.set(current);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(id.get()), Some("(87.6 pt)"), "an idle derived field reads out its value in parentheses");
    assert_eq!(fixture.tree().numeric(id.get()).and_then(|numeric| numeric.value), Some(COMPUTED), "its numeric value is the plain number");

    let field = fixture.by_label("Leading").expect("the Leading number field");
    fixture.click(&field).settle();
    assert_eq!(fixture.tree().value(id.get()), Some("87.6"), "an edit starts from the plain number");

    fixture.key(Key::A, Modifiers::COMMAND).type_text("12").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(12.0), "the committed number is the field's value");
    assert_eq!(fixture.tree().numeric(id.get()).and_then(|numeric| numeric.value), Some(12.0), "the field reports the committed number");
}

/// A derived reading that is wider than the field's text area with its padding, "(12.5 pt)" in a one-cell field, takes
/// room from the padding and shows whole inside the field, and the field is as wide as the same field showing the plain
/// number with its unit. Guards against sizing the text area by the plain number, which cut the closing parenthesis.
#[trial]
fn a_derived_number_shows_whole_inside_its_field() {
    let boxes = Rc::new(Cell::new([egui::Rect::NOTHING; 2]));
    let seen = boxes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut derived = Some(12.5);
        let mut plain = Some(12.5);
        let at = NumberField::new("Leading", Bind::bound(&mut derived)).unit_kind(UnitKind::Points).derived(true).show(ui).response.rect;
        let beside = NumberField::new("Size", Bind::bound(&mut plain)).unit_kind(UnitKind::Points).show(ui).response.rect;
        seen.set([at, beside]);
    });
    fixture.settle();
    fixture.idle(3);
    let [field, plain] = boxes.get();
    let text = fixture.texts().iter().find(|text| text.text == "(12.5 pt)").expect("the derived reading is painted").clone();
    assert!(field.contains_rect(text.rect), "the reading {:?} lies inside the field {field:?}", text.rect);
    assert!(text.clip.contains_rect(text.rect), "the reading {:?} is not cut by its clip {:?}", text.rect, text.clip);
    assert_eq!(field.width(), plain.width(), "the derived field is as wide as the same field with the plain number");
}

/// A blank value is no number to derive: the field shows it as it would without `derived`, never as empty
/// parentheses, and an edit from the blank still starts from nothing.
#[trial]
fn a_blank_derived_number_shows_no_parentheses() {
    let value = Rc::new(Cell::new(None::<f64>));
    let id = Rc::new(Cell::new(egui::Id::NULL));
    let (shown, seen) = (value.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut current = shown.get();
        let out = NumberField::new("Leading", Bind::bound(&mut current)).id_salt("leading").unit_kind(UnitKind::Points).derived(true).show(ui);
        seen.set(out.response.id);
        shown.set(current);
    });
    fixture.settle();
    let text = fixture.tree().value(id.get()).unwrap_or_default().to_owned();
    assert!(!text.contains('('), "a blank value is not shown as a derived value: {text:?}");
    let field = fixture.by_label("Leading").expect("the Leading number field");
    fixture.click(&field).settle();
    fixture.type_text("12").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(12.0), "a number typed over the blank is the field's value");
}
