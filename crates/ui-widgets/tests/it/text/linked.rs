//! `LinkedFields`: the ratio and equal rules, and a typed edit keeping the gesture's starting ratio.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::LinkedFields;
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

type Parts = [Mixed<Option<f64>>; 2];
type Values = Rc<Cell<Parts>>;

/// Both fields shared.
fn same(width: f64, height: f64) -> Parts {
    Mixed::each([Some(width), Some(height)])
}

/// Linked width and height with the link on, the values kept in `values` between frames.
fn rig(values: &Values) -> Fixture {
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut current = shown.get();
        LinkedFields::new("Size", Bind::bound(&mut current), Bind::given(true)).captions(["W", "H"]).show(ui);
        shown.set(current);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture
}

/// Digits typed one by one into the width scale the height from the values the typing began with, not from the
/// previous digit.
#[trial]
fn typing_into_a_linked_field_keeps_the_starting_ratio() {
    let values: Values = Rc::new(Cell::new(same(500.0, 400.0)));
    let mut fixture = rig(&values);
    fixture.type_text("100");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(values.get(), same(100.0, 80.0));
}
