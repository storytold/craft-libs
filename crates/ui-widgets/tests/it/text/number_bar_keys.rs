//! photocraft#16808: an options bar's number field (`NumberKeys::Bar`). Escape puts back the value the field had when
//! it took the focus, past typed values that applied as they were typed and past steps, and leaves the field; Enter
//! commits and leaves; Tab and a click elsewhere commit. A plain field keeps its keys.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::with_env;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{NumberField, NumberKeys};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// A field over a host value that takes every value the field reports, as an app's tool option does
/// (`Bind::given` and `EditResponse::value`), in a scope of `keys`, beside a field that only takes focus; and the value.
fn bar(start: f64, keys: Option<NumberKeys>) -> (Fixture, Rc<RefCell<f64>>) {
    let value = Rc::new(RefCell::new(start));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let body = |ui: &mut egui::Ui| {
            let shown = *held.borrow();
            let out = NumberField::new("Feather", Bind::given(Some(shown))).id_salt("feather").range(0.0..=1000.0).show(ui);
            if let Some(Some(next)) = out.value().copied() {
                *held.borrow_mut() = next;
            }
            let _ = NumberField::new("Other", Bind::given(Some(1.0))).id_salt("other").show(ui);
        };
        match keys {
            Some(keys) => with_env(
                ui,
                |set| {
                    set.values.set(keys);
                },
                body,
            ),
            None => body(ui),
        }
    });
    fixture.settle();
    (fixture, value)
}

/// Clicks the field named `label`.
fn focus(fixture: &mut Fixture, label: &str) {
    let field = fixture.by_label(label).expect("the field");
    fixture.click(&field).settle();
}

fn typed(fixture: &mut Fixture, text: &str) {
    fixture.key(Key::A, Modifiers::COMMAND).type_text(text).settle();
}

fn key(fixture: &mut Fixture, key: Key) {
    fixture.key(key, Modifiers::NONE).settle();
}

#[trial]
fn escape_puts_back_the_value_the_field_had_when_it_took_the_focus() {
    let (mut fixture, value) = bar(10.0, Some(NumberKeys::Bar));
    focus(&mut fixture, "Feather");
    typed(&mut fixture, "60");
    assert_eq!(*value.borrow(), 60.0, "a typed number applies as it is typed");
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 10.0, "typed 60, Escape");

    focus(&mut fixture, "Feather");
    key(&mut fixture, Key::ArrowUp);
    key(&mut fixture, Key::ArrowUp);
    assert_eq!(*value.borrow(), 12.0, "two steps");
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 10.0, "Up, Up, Escape");

    // Steps and typing in one visit: Escape goes back past both.
    focus(&mut fixture, "Feather");
    key(&mut fixture, Key::ArrowUp);
    typed(&mut fixture, "25");
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 10.0, "Up, typed 25, Escape");
}

#[trial]
fn escape_leaves_the_field_so_the_next_keys_do_not_step_it() {
    let (mut fixture, value) = bar(10.0, Some(NumberKeys::Bar));
    focus(&mut fixture, "Feather");
    key(&mut fixture, Key::ArrowUp);
    key(&mut fixture, Key::Escape);
    assert!(fixture.focused().is_none(), "Escape leaves the field");
    key(&mut fixture, Key::ArrowUp);
    assert_eq!(*value.borrow(), 10.0, "an Up after the Escape does not step the field");
}

#[trial]
fn enter_commits_and_leaves_the_field() {
    let (mut fixture, value) = bar(10.0, Some(NumberKeys::Bar));
    focus(&mut fixture, "Feather");
    typed(&mut fixture, "40");
    key(&mut fixture, Key::Enter);
    assert_eq!(*value.borrow(), 40.0, "Enter commits the typed value");
    assert!(fixture.focused().is_none(), "Enter leaves the field");
    key(&mut fixture, Key::ArrowUp);
    assert_eq!(*value.borrow(), 40.0, "an Up after the Enter does not step the field");
    // Escape after the field was left puts nothing back.
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 40.0);
}

#[trial]
fn tab_and_a_click_elsewhere_commit_the_typed_value() {
    let (mut fixture, value) = bar(10.0, Some(NumberKeys::Bar));
    focus(&mut fixture, "Feather");
    typed(&mut fixture, "40");
    key(&mut fixture, Key::Tab);
    assert_eq!(*value.borrow(), 40.0, "Tab commits");

    focus(&mut fixture, "Feather");
    typed(&mut fixture, "35");
    focus(&mut fixture, "Other");
    assert_eq!(*value.borrow(), 35.0, "a click elsewhere commits");
    // The next visit starts from the committed value.
    focus(&mut fixture, "Feather");
    typed(&mut fixture, "70");
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 35.0);
}

#[trial]
fn a_field_sets_the_bar_keys_for_itself_and_a_plain_field_keeps_the_focus_after_enter() {
    // The field's own rule wins over the scope's.
    let value = Rc::new(RefCell::new(10.0));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let shown = *held.borrow();
        let out = NumberField::new("Feather", Bind::given(Some(shown))).id_salt("feather").keys(NumberKeys::Bar).show(ui);
        if let Some(Some(next)) = out.value().copied() {
            *held.borrow_mut() = next;
        }
    });
    fixture.settle();
    focus(&mut fixture, "Feather");
    key(&mut fixture, Key::ArrowUp);
    key(&mut fixture, Key::Escape);
    assert_eq!(*value.borrow(), 10.0, "the field's own Bar keys");

    let (mut fixture, value) = bar(10.0, None);
    focus(&mut fixture, "Feather");
    typed(&mut fixture, "40");
    key(&mut fixture, Key::Enter);
    assert_eq!(*value.borrow(), 40.0);
    assert!(fixture.focused().is_some(), "a plain field keeps the focus after Enter");
    key(&mut fixture, Key::ArrowUp);
    assert_eq!(*value.borrow(), 41.0, "and the next Up steps it");
}
