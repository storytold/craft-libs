//! `NumberField::explicit_plus`: a positive value shows its sign, zero shows none, and the shown text reads back as
//! itself.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::form::SliderField;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

fn angle(start: f64) -> (Fixture, Rc<Cell<Option<f64>>>) {
    let value = Rc::new(Cell::new(Some(start)));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        NumberField::new("Angle", Bind::bound(&mut now)).range(-45.0..=45.0).step(0.01).fixed_decimals(2).explicit_plus().show(ui);
        held.set(now);
    });
    fixture.settle();
    (fixture, value)
}

#[trial]
fn an_explicit_plus_field_writes_a_positive_angle_with_its_sign() {
    for (value, shown) in [(2.5, "+2.50"), (-3.25, "-3.25"), (0.0, "0.00")] {
        let (fixture, _) = angle(value);
        assert!(fixture.texts().iter().any(|painted| painted.text == shown), "{value} shows as {shown}: {:?}", fixture.texts());
    }
}

#[trial]
fn committing_an_explicit_plus_fields_text_unchanged_keeps_its_value() {
    let (mut fixture, value) = angle(2.5);
    let field = fixture.by_label("Angle").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(2.5), "\"+2.50\" reads back as 2.5, not 2.5 added to the start");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("+1.25");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(1.25), "a typed leading plus is the sign");
}

/// The field's own text, "+7", is a plain number: it applies as it is typed, not on commit, as "-7" does.
#[trial]
fn an_explicit_plus_fields_own_text_applies_as_it_is_typed() {
    let (mut fixture, value) = angle(2.5);
    let field = fixture.by_label("Angle").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("+7").settle();
    assert_eq!(value.get(), Some(7.0), "the typed value applies before Enter");
}

/// A slider field forwards the sign rule to its number field, so the two children show the value alike.
#[trial]
fn a_slider_field_writes_its_value_with_the_sign_it_is_given() {
    let value = Rc::new(Cell::new(2.5f64));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        SliderField::new("Angle", Bind::bound(&mut now), -45.0..=45.0).step(0.01).typed_decimals(2).explicit_plus().show(ui);
        held.set(now);
    });
    fixture.settle();
    assert!(fixture.texts().iter().any(|painted| painted.text == "+2.5"), "the number shows its sign: {:?}", fixture.texts());
}
