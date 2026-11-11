//! Text AccessKit and the field's visible states: value, `SetValue`, selection, INVALID, selectable text.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use std::cell::Cell;
use std::rc::Rc;

/// A masked field's node carries no value, where the same field unmasked carries its text.
#[trial]
fn a_masked_field_exposes_no_value() {
    let (open, masked) = (Rc::new(Cell::new(egui::Id::NULL)), Rc::new(Cell::new(egui::Id::NULL)));
    let (plain, hidden) = (open.clone(), masked.clone());
    let mut fixture = Fixture::new(move |ui| {
        plain.set(TextField::new("Name", Bind::given("secret".to_owned())).id_salt("name").show(ui).response.id);
        hidden.set(TextField::new("Password", Bind::given("secret".to_owned())).id_salt("password").masked().show(ui).response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(open.get()), Some("secret"));
    assert_eq!(fixture.tree().value(masked.get()), None, "a masked node has no value to read out");
}
