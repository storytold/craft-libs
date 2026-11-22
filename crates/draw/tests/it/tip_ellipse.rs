//! The tip ellipse's least roundness: a dot drag and Alt+arrow steps stop at it.

use craft_draw::ui::brush_settings::{TipAngle, TipEllipse};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A fixture over a tip of roundness `start`, with the least roundness `least` or the widget's default.
fn shown(start: f64, least: Option<f64>) -> (Fixture, Rc<RefCell<TipAngle>>) {
    let angle = Rc::new(RefCell::new(TipAngle::new(0.0, start)));
    let value = Rc::clone(&angle);
    let mut fixture = Fixture::new(move |ui| {
        let mut angle = value.borrow_mut();
        let widget = TipEllipse::new("Tip angle and roundness", Bind::bound(&mut *angle));
        let _ = match least {
            Some(least) => widget.least_roundness(least),
            None => widget,
        }
        .show(ui);
    });
    fixture.room(Vec2::new(200.0, 200.0)).settle();
    (fixture, angle)
}

/// The roundness after a drag of the lower dot (on the short axis of a horizontal tip) to the widget's centre.
fn dragged_to_centre(least: Option<f64>) -> f64 {
    let (mut fixture, angle) = shown(0.5, least);
    let rect = fixture.by_role(Role::SLIDER).first().copied().expect("the tip widget shows").rect;
    let quarter = rect.height() / 4.0;
    fixture.drag_at(rect.center() + Vec2::new(0.0, quarter), Vec2::new(0.0, -quarter));
    angle.borrow().roundness
}

/// The roundness after Alt+Down pressed far more often than the start needs to reach any least.
fn stepped_down(least: Option<f64>) -> f64 {
    let (mut fixture, angle) = shown(0.04, least);
    fixture.key(Key::Tab, Modifiers::NONE);
    for _ in 0..12 {
        fixture.key(Key::ArrowDown, Modifiers::ALT);
    }
    angle.borrow().roundness
}

#[trial]
fn a_dot_drag_and_alt_keys_stop_at_the_least_roundness() {
    let (flat_drag, default_drag) = (dragged_to_centre(Some(0.0)), dragged_to_centre(None));
    assert!(flat_drag.abs() < 1e-3, "a flat tip's dot reaches the centre, got {flat_drag}");
    assert!(default_drag > 0.0, "the default least keeps the dot off the centre, got {default_drag}");
    assert!(flat_drag < default_drag, "the flat tip goes below the default least");

    let (flat_keys, default_keys) = (stepped_down(Some(0.0)), stepped_down(None));
    assert_eq!(flat_keys, 0.0, "Alt+Down reaches a roundness of 0 on a flat tip");
    assert!(default_keys > 0.0, "Alt+Down stops above 0 with the default least, got {default_keys}");
    assert!(flat_keys < default_keys, "the flat tip's keys go below the default least");
}
