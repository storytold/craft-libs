//! The reset dot of a text control: a number field with a reset value shows the dot in its field's label while its
//! value differs, and the dot's tooltip names the default as the field words it.

use craft_behavior::PartKey;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::runtime::memory::sub;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Id, Pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The label leaf's reset dot.
const DOT: PartKey = PartKey::new("reset");

fn texts(fixture: &Fixture) -> Vec<String> {
    fixture.texts().iter().map(|run| run.text.clone()).collect()
}

#[trial]
fn a_text_field_with_a_reset_value_shows_the_dot_and_its_tooltip_names_the_default() {
    let value = Rc::new(RefCell::new(Some(40.0_f64)));
    let field = Rc::new(Cell::new(Id::NULL));
    let (bound, seen) = (Rc::clone(&value), Rc::clone(&field));
    let mut fixture = Fixture::new(move |ui| {
        let mut value = bound.borrow_mut();
        let shown = Field::new("Opacity").side(SideLabel::new().reset_dot(true)).id_salt("reset").show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::bound(&mut *value).reset(Some(25.0))).show(ui);
        });
        seen.set(shown.id);
    });
    fixture.settle();
    let label = sub(field.get(), "label");
    let dot = fixture.part_rect(label, DOT).unwrap_or_else(|| panic!("a value away from its reset value shows the dot"));
    let before = texts(&fixture);
    fixture.frame(vec![moved(dot.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    let tips: Vec<String> = texts(&fixture).into_iter().filter(|text| !before.contains(text)).collect();
    fixture.frame(vec![moved(Pos2::ZERO)]);
    fixture.settle();
    fixture.click_at(dot.center());
    fixture.settle();
    assert_eq!(*value.borrow(), Some(25.0), "a click on the dot resets the value");
    // The field's own text for the reset value: what it shows now that it did not show at 40.
    let shown: Vec<String> = texts(&fixture).into_iter().filter(|text| !before.contains(text) && !tips.contains(text)).collect();
    let default = shown.first().unwrap_or_else(|| panic!("the field shows its reset value"));
    assert!(
        tips.iter().any(|tip| tip != default && tip.contains(default.as_str())),
        "the dot's tooltip names the default as the field shows it ({default:?}): {tips:?}"
    );
}
