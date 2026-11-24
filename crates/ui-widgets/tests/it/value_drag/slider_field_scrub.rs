//! A `SliderField`'s scrub rate reaches its number field: a sideways drag on the value moves it by the rate per
//! point, as the same drag on the thumb would with a rate of the range over the track's width.

use craft_behavior::{EditEvent, MotionMode};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::form::SliderField;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, vec2};
use std::cell::RefCell;
use std::rc::Rc;

const ROW: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(420.0, 80.0) };

/// The fixture of an exposure field, its value and the edit events it reported.
type Exposure = (Fixture, Rc<RefCell<f64>>, Rc<RefCell<Vec<EditEvent<f64>>>>);

/// A field over -5..=5 on hundredths, scrubbing by `rate` when one is given; the value and the edit events it saw.
fn exposure(rate: Option<f64>) -> Exposure {
    let (value, events) = (Rc::new(RefCell::new(0.5)), Rc::new(RefCell::new(Vec::new())));
    let (held, seen) = (Rc::clone(&value), Rc::clone(&events));
    let mut fixture = Fixture::new(move |ui| {
        let mut current = held.borrow_mut();
        let field = SliderField::new("Exposure", Bind::bound(&mut *current), -5.0..=5.0).step(0.01);
        let field = match rate {
            Some(rate) => field.scrub_step(rate),
            None => field,
        };
        seen.borrow_mut().extend(field.show_at(ui, ROW).edit.events.iter().cloned());
    });
    fixture.motion(MotionMode::Off).settle();
    (fixture, value, events)
}

fn scrub_value(fixture: &mut Fixture, by: f32) {
    let field = fixture.by_role(Role::SPIN_BUTTON).first().copied().expect("the number field");
    fixture.drag_at(field.center(), vec2(by, 0.0)).settle();
}

#[trial]
fn a_slider_field_scrubs_its_value_at_the_rate_it_is_given() {
    let (mut fixture, value, events) = exposure(Some(0.05));
    scrub_value(&mut fixture, 40.0);
    assert!((*value.borrow() - 2.5).abs() < 1e-9, "40 points at 0.05 a point add 2: {}", value.borrow());
    let seen = events.take();
    let count = |begin: bool| {
        seen.iter().filter(|event| if begin { matches!(event, EditEvent::Begin { .. }) } else { matches!(event, EditEvent::Commit { .. }) }).count()
    };
    assert_eq!((count(true), count(false)), (1, 1), "the scrub is one gesture: {seen:?}");
    scrub_value(&mut fixture, -400.0);
    assert_eq!(*value.borrow(), -5.0, "a scrub stops at the range");
}

#[trial]
fn a_slider_field_without_a_scrub_rate_scrubs_by_its_step() {
    for rate in [None, Some(0.0), Some(-1.0), Some(f64::NAN)] {
        let (mut fixture, value, _) = exposure(rate);
        scrub_value(&mut fixture, 40.0);
        assert!((*value.borrow() - 0.9).abs() < 1e-9, "40 points at the 0.01 step add 0.4 for {rate:?}: {}", value.borrow());
    }
}
