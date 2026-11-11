//! The rules of the panels family: an inspector keys its controls by the target and shows the empty notice for no
//! target.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::patterns::panels::Inspector;
use craft_ui::testing::{Fixture, trial};
use egui::Id;
use std::cell::Cell;
use std::rc::Rc;

/// What the inspector's body showed in the last pass.
struct Rig {
    fixture: Fixture,
    target: Rc<Cell<Option<&'static str>>>,
    field: Rc<Cell<Option<Id>>>,
}

fn rig(first: Option<&'static str>) -> Rig {
    let target = Rc::new(Cell::new(first));
    let field = Rc::new(Cell::new(None));
    let (shown_target, shown_field) = (target.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut inspector = Inspector::new("Properties").header("Layer");
        if let Some(target) = shown_target.get() {
            inspector = inspector.target(target);
        }
        inspector.show(ui, |ui, _| {
            shown_field.set(Some(Checkbox::new(Bind::given(false)).label("Visible").show(ui).response.id));
        });
    });
    fixture.settle();
    Rig { fixture, target, field }
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
