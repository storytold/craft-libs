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
