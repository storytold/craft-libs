//! The rules of the panels family: an inspector keys its controls by the target and shows the empty notice for no
//! target.

pub(crate) mod history;
pub(crate) mod palette;
pub(crate) mod tour;

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::patterns::panels::Inspector;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The values the targets hold and the edits their number fields reported.
struct Values {
    a: f64,
    b: f64,
    landed: Vec<(&'static str, f64)>,
}

/// What the inspector's body showed in the last pass.
struct Rig {
    fixture: Fixture,
    target: Rc<Cell<Option<&'static str>>>,
    field: Rc<Cell<Option<Id>>>,
    values: Rc<RefCell<Values>>,
}

fn rig(first: Option<&'static str>) -> Rig {
    let target = Rc::new(Cell::new(first));
    let field = Rc::new(Cell::new(None));
    let values = Rc::new(RefCell::new(Values { a: 10.0, b: 77.0, landed: Vec::new() }));
    let (shown_target, shown_field, shown_values) = (target.clone(), field.clone(), values.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut inspector = Inspector::new("Properties").header("Layer");
        if let Some(target) = shown_target.get() {
            inspector = inspector.target(target);
        }
        inspector.show(ui, |ui, _| {
            shown_field.set(Some(Checkbox::new(Bind::given(false)).label("Visible").show(ui).response.id));
            if let Some(target) = shown_target.get() {
                let mut values = shown_values.borrow_mut();
                let held = if target == "a" { values.a } else { values.b };
                let shown = NumberField::new("Value", Bind::given(Some(held))).show(ui);
                if let Some(landed) = shown.edit.events.landed().copied().flatten() {
                    values.landed.push((target, landed));
                    if target == "a" {
                        values.a = landed;
                    } else {
                        values.b = landed;
                    }
                }
            }
        });
    });
    fixture.settle();
    Rig { fixture, target, field, values }
}

impl Rig {
    fn show(&mut self, target: Option<&'static str>) {
        self.target.set(target);
        self.field.set(None);
        self.fixture.settle();
    }
}

/// A control of the body keeps its id for a target, so its typing and scrub state stay with that target.
#[trial]
fn an_inspector_keys_its_controls_by_the_target() {
    let mut rig = rig(Some("a"));
    let first = rig.field.get().expect("the body showed for the target");
    rig.show(Some("b"));
    let second = rig.field.get().expect("the body showed for the other target");
    assert_ne!(first, second, "another target gets other control ids");
    rig.show(Some("a"));
    assert_eq!(rig.field.get(), Some(first), "the first target gets its ids back");
}

/// What a number field typed into under one target does when another target is shown: nothing typed under A reaches B,
/// and A's field never shows B's value.
#[trial]
fn an_inspector_keeps_a_draft_with_its_target() {
    let mut rig = rig(Some("a"));
    let field = rig.fixture.by_label("Value").expect("the body shows a number field for target a");
    rig.fixture.click(&field);
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.type_text("+25");
    rig.fixture.idle(2);
    rig.show(Some("b"));
    {
        let values = rig.values.borrow();
        assert_eq!(values.b, 77.0, "the draft of target a does not land on b");
        assert!(values.landed.iter().all(|(target, _)| *target != "b"), "no edit is reported for b: {:?}", values.landed);
    }
    assert!(rig.fixture.texts().iter().any(|run| run.text.contains("77")), "b's field shows b's value");
    rig.show(Some("a"));
    assert!(rig.fixture.texts().iter().all(|run| !run.text.contains("77")), "a's field never shows b's value");
    assert_eq!(rig.values.borrow().b, 77.0);
}
