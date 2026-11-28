//! Custom track projections are generic: model values and all non-pointer edits keep their units.

use craft_behavior::MotionMode;
use craft_ui::contract::{Bind, Props as _, Role, Show as _};
use craft_ui::form::SliderField;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::value_drag::{Slider, SliderScale};
use egui::{Id, Key, Modifiers, Pos2, Rect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const MIN: f64 = -20.0;
const MAX: f64 = 180.0;
const FIELD: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(420.0, 50.0) };

// An arbitrary piecewise curve, not a product/unit-specific scale: the first fifth of the model range
// occupies half the track; the rest grows quadratically. Exercise both sides and their shared midpoint.
fn position(value: f64, min: f64, max: f64) -> f64 {
    assert!(min < max && (min..=max).contains(&value), "the widget bounds the callback input");
    let x = (value - min) / (max - min);
    if x <= 0.2 { x * 2.5 } else { 0.5 + 0.5 * ((x - 0.2) / 0.8).sqrt() }
}

fn value_at(t: f64, min: f64, max: f64) -> f64 {
    assert!(min < max && (0.0..=1.0).contains(&t), "the widget bounds the callback input");
    let x = if t <= 0.5 { t * 0.4 } else { 0.2 + 0.8 * (2.0 * t - 1.0).powi(2) };
    min + x * (max - min)
}

const CUSTOM: SliderScale = SliderScale::custom(position, value_at);

struct Rig {
    fixture: Fixture,
    value: Rc<RefCell<f64>>,
    id: Rc<Cell<Id>>,
}

impl Rig {
    fn new(scale: SliderScale, range: std::ops::RangeInclusive<f64>, step: Option<f64>) -> Self {
        let value = Rc::new(RefCell::new(*range.start()));
        let id = Rc::new(Cell::new(Id::NULL));
        let (held, seen) = (Rc::clone(&value), Rc::clone(&id));
        let mut fixture = Fixture::new(move |ui| {
            let mut value = held.borrow_mut();
            let slider = Slider::new(Bind::bound(&mut *value), range.clone()).scale(scale).label("Mapped");
            let slider = match step {
                Some(step) => slider.step(step),
                None => slider,
            };
            seen.set(slider.show_at(ui, FIELD).response.id);
        });
        fixture.motion(MotionMode::Off).settle();
        Self { fixture, value, id }
    }

    /// Sets the value, and runs a frame past the one that shows it: the thumb's bounds are read from last pass's
    /// shape record (`Fixture::part_rect`), and a settle may run only the one frame.
    fn set(&mut self, value: f64) {
        *self.value.borrow_mut() = value;
        self.fixture.settle().idle(1);
    }

    /// The thumb's sensed bounds, as last pass recorded them.
    fn thumb(&self) -> Rect {
        self.fixture.part_rect(self.id.get(), Slider::THUMB).expect("the thumb's sensed bounds")
    }

    fn at(&self, fraction: f64) -> Pos2 {
        let inset = self.thumb().width() / 2.0;
        Pos2::new(FIELD.left() + inset + fraction as f32 * (FIELD.width() - 2.0 * inset), FIELD.center().y)
    }

    fn click(&mut self, fraction: f64) {
        let at = self.at(fraction);
        self.fixture.advance(1.0).click_at(at).settle();
    }
}

#[trial]
fn custom_scale_positions_and_pointer_inverse_agree_across_the_midpoint() {
    let mut rig = Rig::new(CUSTOM, MIN..=MAX, None);
    for t in [0.0, 0.1, 0.25, 0.499, 0.5, 0.501, 0.65, 0.9, 1.0] {
        let value = value_at(t, MIN, MAX);
        rig.set(value);
        assert!((rig.thumb().center().x - rig.at(t).x).abs() < 0.001, "value {value} sits at {t}");
        rig.set(if t < 0.5 { MAX } else { MIN });
        rig.click(t);
        assert!((*rig.value.borrow() - value).abs() < 0.001, "position {t} gives {value}, got {}", rig.value.borrow());
    }
    rig.set(MIN - 100.0);
    assert!((rig.thumb().center().x - rig.at(0.0).x).abs() < 0.001);
    rig.set(MAX + 100.0);
    assert!((rig.thumb().center().x - rig.at(1.0).x).abs() < 0.001);
    let mut reversed = Rig::new(CUSTOM, MAX..=MIN, None);
    reversed.click(0.5);
    assert!((*reversed.value.borrow() - value_at(0.5, MIN, MAX)).abs() < 0.001, "callbacks receive ascending bounds");
    // A grabbed thumb uses the same inverse while crossing the join, not an absolute pointer jump.
    rig.set(value_at(0.4, MIN, MAX));
    let from = rig.thumb().center();
    let by = rig.at(0.7) - from;
    rig.fixture.advance(1.0).drag_at(from, by).settle();
    assert!((*rig.value.borrow() - value_at(0.7, MIN, MAX)).abs() < 0.001);
}

#[trial]
fn custom_scale_keys_wheel_and_accessibility_stay_in_model_units() {
    for step in [None, Some(5.0)] {
        let mut rig = Rig::new(CUSTOM, MIN..=MAX, step);
        rig.set(20.0);
        let target = rig.fixture.by_role(Role::SLIDER).first().copied().expect("slider");
        let numeric = rig.fixture.tree().numeric_of(target.node).expect("numeric slider");
        assert_eq!(numeric.value, Some(20.0));
        assert_eq!(numeric.range, MIN..=MAX);
        let small = step.unwrap_or((MAX - MIN) * 0.01);
        assert_eq!(numeric.step, Some(small));
        rig.fixture.access(AccessActionKind::Focus, &target).settle();
        rig.fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
        assert_eq!(*rig.value.borrow(), 20.0 + small);
        rig.fixture.access(AccessActionKind::Increment, &target).settle();
        assert_eq!(*rig.value.borrow(), 20.0 + 2.0 * small);
        let before = *rig.value.borrow();
        let at = rig.thumb().center();
        rig.fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::wheel_lines(1.0)]).advance(1.0).settle();
        assert_eq!((*rig.value.borrow() - before).abs(), small, "one wheel notch is one model step");
    }
}

#[trial]
fn custom_scale_slider_field_types_and_reads_model_values() {
    let value = Rc::new(RefCell::new(20.0));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        SliderField::new("Mapped", Bind::bound(&mut *held.borrow_mut()), MIN..=MAX).scale(CUSTOM).step(5.0).show(ui);
    });
    fixture.motion(MotionMode::Off).settle();
    let field = fixture.by_role(Role::SPIN_BUTTON).first().copied().expect("number field");
    fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("135").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), 135.0);
    let slider = fixture.by_role(Role::SLIDER).first().copied().expect("slider");
    assert_eq!(fixture.tree().numeric_of(slider.node).expect("numeric slider").value, Some(135.0));
    fixture.access(AccessActionKind::Focus, &slider).key(Key::ArrowLeft, Modifiers::NONE).settle();
    assert_eq!(*value.borrow(), 130.0, "typing did not change the model-unit nudge");
}

#[trial]
fn custom_scale_bounds_and_nonfinite_results_are_contained() {
    fn linear(value: f64, min: f64, max: f64) -> f64 {
        (value - min) / (max - min)
    }
    fn nan(_: f64, _: f64, _: f64) -> f64 {
        f64::NAN
    }
    fn infinity(_: f64, _: f64, _: f64) -> f64 {
        f64::INFINITY
    }
    fn below(_: f64, _: f64, _: f64) -> f64 {
        -1000.0
    }
    fn above(_: f64, _: f64, _: f64) -> f64 {
        1000.0
    }
    for (callback, expected) in [(nan as fn(f64, f64, f64) -> f64, 0.75), (infinity, 0.75), (below, 0.0), (above, 1.0)] {
        let mut rig = Rig::new(SliderScale::custom(callback, value_at), MIN..=MAX, None);
        rig.set(MIN + 0.75 * (MAX - MIN));
        assert!((rig.thumb().center().x - rig.at(expected).x).abs() < 0.001);
        let mut rig = Rig::new(SliderScale::custom(linear, callback), MIN..=MAX, None);
        rig.click(0.75);
        let expected_value = MIN + expected * (MAX - MIN);
        assert!((*rig.value.borrow() - expected_value).abs() < 0.001);
    }
    let mut singleton = Rig::new(CUSTOM, 42.0..=42.0, None);
    singleton.click(0.75);
    assert_eq!(*singleton.value.borrow(), 42.0);
}

#[trial]
fn custom_scale_does_not_change_builtin_linear_power_or_log_tracks() {
    for (scale, range, value) in
        [(SliderScale::Linear, 0.0..=100.0, 50.0), (SliderScale::Power(2.0), 0.0..=100.0, 25.0), (SliderScale::Log, 1.0..=10000.0, 100.0)]
    {
        let mut rig = Rig::new(scale, range, None);
        rig.click(0.5);
        assert!((*rig.value.borrow() - value).abs() < 0.001, "{scale:?} midpoint");
        assert!((rig.thumb().center().x - rig.at(0.5).x).abs() < 0.001);
    }
}
