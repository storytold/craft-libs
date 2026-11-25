//! The focus ring's thickness is a theme role (`stroke.focus`), thicker than the standard line, as production's.

use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Shape};

/// The widest rectangle stroke in the latest frame.
fn widest_stroke(fixture: &mut Fixture) -> f32 {
    fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if !rect.stroke.is_empty() => Some(rect.stroke.width),
            _ => None,
        })
        .fold(0.0, f32::max)
}

/// A button with keyboard focus is ringed with a line of at least two points; unfocused, nothing is that thick.
#[trial]
fn the_focus_ring_is_two_points_thick() {
    let mut fixture = Fixture::new(|ui| {
        let _ = Button::new("Go").show(ui);
    });
    fixture.settle();
    assert!(widest_stroke(&mut fixture) < 2.0, "an unfocused button has no thick line");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.focused().is_some(), "Tab focused the button");
    let width = widest_stroke(&mut fixture);
    assert!((width - 2.0).abs() < 0.01, "the ring is the theme's focus stroke of two points: {width}");
}
