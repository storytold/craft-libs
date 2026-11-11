//! `Slider` and `RangeSlider`: the value maths that is the family's own: stepping in the reading direction, the scales
//! that spread a range over a track, and thumbs that never cross.

mod dials;

mod thumb_row;

use craft_behavior::{EditEvent, MotionMode};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::pointer;
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui::value_drag::Slider;
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const START: f64 = 50.0;

const FIELD: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(220.0, 50.0) };

/// A slider shown in `FIELD` that holds its value and the edit events of every pass.
struct Slide {
    value: RefCell<f64>,
    range: std::ops::RangeInclusive<f64>,
    step: Option<f64>,
    wheel_step: Option<f64>,
    events: RefCell<Vec<EditEvent<f64>>>,
}

impl Slide {
    fn new(value: f64) -> Self {
        Slide { value: RefCell::new(value), range: 0.0..=100.0, step: None, wheel_step: None, events: RefCell::default() }
    }

    fn show(&self, ui: &mut Ui) {
        let mut value = self.value.borrow_mut();
        let mut slider = Slider::new(Bind::bound(&mut *value), self.range.clone()).label("Opacity");
        if let Some(step) = self.step {
            slider = slider.step(step);
        }
        if let Some(step) = self.wheel_step {
            slider = slider.wheel_step(step);
        }
        self.events.borrow_mut().extend(slider.show_at(ui, FIELD).events);
    }

    fn value(&self) -> f64 {
        *self.value.borrow()
    }
}

/// Presses at `from`, moves `travel` along the track in four frames with `between` after the second, releases.
fn drag(frames: &mut Frames, from: Pos2, travel: f32, show: &mut impl FnMut(&mut Ui), mut between: impl FnMut(&mut Frames, &mut dyn FnMut(&mut Ui))) {
    frames.frame(vec![pointer::moved(from)], &mut *show);
    frames.frame(vec![pointer::press(from)], &mut *show);
    for step in 1..=4 {
        frames.frame(vec![pointer::moved(from + Vec2::new(travel * step as f32 / 4.0, 0.0))], &mut *show);
        if step == 2 {
            between(frames, &mut *show);
        }
    }
    frames.frame(vec![pointer::release(from + Vec2::new(travel, 0.0))], &mut *show);
    frames.idle(2, &mut *show);
}

/// The value after grabbing the slider's thumb `offset` from its centre and dragging `travel`.
fn dragged_from(offset: f32, travel: f32) -> f64 {
    let slide = Slide::new(START);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| slide.show(ui);
    frames.idle(3, &mut show);
    drag(&mut frames, FIELD.center() + Vec2::new(offset, 0.0), travel, &mut show, |_, _| {});
    slide.value()
}

#[trial]
fn a_grabbed_slider_thumb_moves_by_the_pointer_travel_and_a_click_on_it_changes_nothing() {
    let centred = dragged_from(0.0, 10.0);
    assert!(centred > START, "the drag moved the value: {centred}");
    let twice = dragged_from(0.0, 20.0);
    assert!(((twice - centred) - (centred - START)).abs() < 1e-3, "twice the travel is twice the change: {centred} then {twice}");
    let off_centre = dragged_from(3.0, 10.0);
    assert!((off_centre - centred).abs() < 1e-3, "a press 3 px off the centre does not add the offset: {off_centre}");

    let slide = Slide::new(START);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| slide.show(ui);
    frames.idle(3, &mut show);
    pointer::click(&mut frames, FIELD.center() + Vec2::new(3.0, 0.0), &mut show);
    assert_eq!(slide.value(), START, "a click on the thumb leaves the value");
}

/// A leaf with parts shown at a rect that is empty, not a number, infinite, over the extent cap, or one point wide
/// does not panic and answers presses and keys with nothing: its value stays as it was. It runs on a slider, because
/// only shared code can break it.
#[trial]
fn a_leaf_at_a_degenerate_rect_neither_panics_nor_changes_its_value() {
    let nan = f32::NAN;
    let rects = [
        Rect::ZERO,
        Rect::from_min_max(Pos2::new(nan, nan), Pos2::new(nan, nan)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(f32::INFINITY, f32::INFINITY)),
        Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0e9)),
        Rect::from_min_size(Pos2::new(5.0, 5.0), Vec2::splat(1.0)),
    ];
    for (index, rect) in rects.into_iter().enumerate() {
        let inert = index < 4;
        let value = Rc::new(RefCell::new(START));
        let shown = value.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut value = shown.borrow_mut();
            Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show_at(ui, rect);
        });
        fixture.motion(MotionMode::Off).settle();
        if !inert {
            continue;
        }
        for at in [Pos2::ZERO, Pos2::new(5.5, 5.5), Pos2::new(0.5, 0.5)] {
            fixture.frame(vec![pointer::moved(at)]);
            fixture.frame(vec![pointer::press(at)]);
            fixture.frame(vec![pointer::release(at)]);
        }
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        fixture.settle();
        assert_eq!(*value.borrow(), START, "a slider shown at {rect:?} changed its value");
    }
}
