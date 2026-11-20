//! Numeric fields can keep fine discrete steps and a faster scrub.

use craft_ui::contract::{Bind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::testing::pointer::{moved, wheel_lines};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Key, Modifiers, vec2};
use std::cell::Cell;
use std::rc::Rc;

const START: f64 = 30.0;

fn angle_field(scrub_step: Option<f64>) -> (Fixture, Rc<Cell<Option<f64>>>) {
    let value = Rc::new(Cell::new(Some(START)));
    let shown = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        Field::new("Angle").side(SideLabel::new().scrub(true)).show(ui, |ui, _| {
            let mut current = shown.get();
            let field = NumberField::new("Angle", Bind::bound(&mut current)).step(0.01).fixed_decimals(2).range(-359.99..=359.99).scrub_by_label();
            let field = match scrub_step {
                Some(step) => field.scrub_step(step),
                None => field,
            };
            field.show(ui);
            shown.set(current);
        });
    });
    fixture.settle();
    (fixture, value)
}

fn scrub_caption(fixture: &mut Fixture) {
    let field = fixture.by_label("Angle").expect("the Angle number field");
    let caption = fixture.texts().iter().find(|text| text.text == "Angle").expect("the visible scrub caption").rect;
    assert!(!field.rect.intersects(caption), "the caption is a separate handle");
    fixture.drag_at(caption.center(), vec2(50.0, 0.0)).settle();
}

#[trial]
fn a_separate_scrub_rate_keeps_fine_key_and_wheel_steps() {
    let (mut fixture, value) = angle_field(Some(1.0));
    scrub_caption(&mut fixture);
    assert_eq!(value.get(), Some(80.0), "50 points of caption drag add 50 degrees");
    let field = fixture.by_label("Angle").expect("the Angle number field");
    fixture.click(&field).key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(value.get(), Some(80.01), "an arrow still adds one hundredth");
    fixture.frame(vec![moved(field.center())]);
    fixture.frame(vec![wheel_lines(1.0)]).settle();
    assert_eq!(value.get(), Some(80.02), "one wheel notch still adds one hundredth");
}

#[trial]
fn an_omitted_or_invalid_scrub_rate_uses_the_field_step() {
    for rate in [None, Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
        let (mut fixture, value) = angle_field(rate);
        scrub_caption(&mut fixture);
        assert_eq!(value.get(), Some(30.50), "the fallback rate for {rate:?} is 0.01 per point");
    }
}
