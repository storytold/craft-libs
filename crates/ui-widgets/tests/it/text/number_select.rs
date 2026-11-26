//! A number field's `select_on_focus`: a click on the value selects its whole text, so typing replaces it.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::form::SliderField;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// A field that writes every decimal ("0.00"): a click on it and "0.5" typed give 0.5, not "0.000.5".
#[trial]
fn a_click_on_a_select_on_focus_number_field_selects_its_value() {
    let value = Rc::new(Cell::new(Some(0.0)));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        NumberField::new("Amount", Bind::bound(&mut now)).range(-1.0..=1.0).step(0.01).fixed_decimals(2).select_on_focus().show(ui);
        held.set(now);
    });
    fixture.settle();
    let field = fixture.by_label("Amount").expect("the field");
    fixture.click(&field).settle();
    fixture.type_text("0.5");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(0.5), "the typed value replaced the shown one");
}

/// A slider field passes the option to its number field.
#[trial]
fn a_click_on_a_select_on_focus_slider_fields_value_selects_it() {
    let value = Rc::new(Cell::new(20.0));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        SliderField::new("Size", Bind::bound(&mut now), 0.0..=100.0).step(1.0).select_on_focus().show(ui);
        held.set(now);
    });
    fixture.settle();
    let field = fixture.by_label("Size").expect("the field");
    fixture.click(&field).settle();
    fixture.type_text("36");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), 36.0, "the typed value replaced the shown one");
}
