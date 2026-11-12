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

/// The wheel over a slider in a scrolling panel scrolls the panel while the slider has no focus; it steps the slider
/// once the slider has focus, or without focus inside a `WheelScope::Free` scope (an adjustment dialog). An outer
/// `WheelScope::Gated` scope (a card in the Overview) wins over a free one inside it.
#[trial]
fn the_wheel_steps_a_slider_when_focused_or_freed_and_otherwise_scrolls_the_panel() {
    use craft_ui::contract::{ContainerProps, Scroll, ShowContainer as _};
    use craft_ui::runtime::env::{WheelScope, with_env};
    use craft_ui::runtime::focus::{FocusTarget, request};
    use craft_ui::testing::layout::{Block, in_region};
    use craft_ui_test_kit::body::Slab;
    use std::cell::Cell;

    /// How far the value and the panel moved after one wheel turn over the slider, shown in the scopes `scopes`
    /// (outer first) inside the panel.
    fn turned(scopes: &[WheelScope], focused: bool) -> (f64, f32) {
        fn nested(ui: &mut Ui, scopes: &[WheelScope], body: &mut dyn FnMut(&mut Ui)) {
            match scopes.split_first() {
                Some((scope, inner)) => with_env(ui, |set| set.wheel = Some(*scope), |ui| nested(ui, inner, body)),
                None => body(ui),
            }
        }
        let value = RefCell::new(START);
        let (top, slider) = (Cell::new(0.0_f32), Cell::new(None));
        let mut show = |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 200.0)), |ui| {
                Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                    top.set(ui.next_widget_position().y);
                    nested(ui, scopes, &mut |ui| {
                        let mut value = value.borrow_mut();
                        let shown = Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show(ui);
                        slider.set(Some((shown.response.id, shown.response.rect)));
                    });
                    Block::new(Vec2::new(100.0, 1000.0)).show(ui);
                });
            });
        };
        let mut frames = Frames::new();
        frames.idle(3, &mut show);
        let (id, rect) = slider.get().expect("the slider showed");
        if focused {
            request(&frames.ctx, FocusTarget::Id(id));
            frames.idle(2, &mut show);
        }
        let (value_before, top_before) = (*value.borrow(), top.get());
        frames.frame(vec![pointer::moved(rect.center())], &mut show);
        frames.frame(vec![pointer::wheel(-40.0)], &mut show);
        frames.idle(2, &mut show);
        (*value.borrow() - value_before, top_before - top.get())
    }

    let (value, panel) = turned(&[], false);
    assert!(value == 0.0 && panel > 0.0, "unfocused: the panel scrolls and the value stays ({value}, {panel})");
    let (value, panel) = turned(&[], true);
    assert!(value != 0.0 && panel == 0.0, "focused: the slider steps and the panel stays ({value}, {panel})");
    let (value, panel) = turned(&[WheelScope::Free], false);
    assert!(value != 0.0 && panel == 0.0, "free scope: the unfocused slider steps ({value}, {panel})");
    let (value, panel) = turned(&[WheelScope::Gated, WheelScope::Free], false);
    assert!(value == 0.0 && panel > 0.0, "a gated scope wins over a free one inside it ({value}, {panel})");
}

/// The rings a frame paints inside `within`: outlines with no fill.
fn rings(shapes: &[egui::Shape], within: Rect) -> usize {
    shapes
        .iter()
        .filter(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill.a() == 0 && rect.stroke.color.a() > 0 && within.expand(8.0).contains_rect(rect.rect)))
        .count()
}

/// The thumbs a press at the pointer would move wear the ring: over the span between a range slider's thumbs both do,
/// over one thumb that one alone, and over a slider's bare track its thumb. A click on the span focuses the nearer
/// thumb and leaves the range as it was.
#[trial]
fn the_thumbs_a_press_would_move_wear_the_ring_and_a_click_on_the_span_focuses_the_nearer() {
    use craft_behavior::Mixed;
    use craft_ui::value_drag::RangeSlider;
    let range = Rc::new(RefCell::new((Mixed::Same(20.0), Mixed::Same(80.0))));
    let shown = range.clone();
    let mut fixture = Fixture::new(move |ui| {
        RangeSlider::new(Bind::bound(&mut *shown.borrow_mut()), 0.0..=100.0).label("Hue").show_at(ui, FIELD);
    });
    fixture.motion(MotionMode::Off).settle();
    let rest = rings(&fixture.frame_shapes(), FIELD);
    // The thumbs' centres run between half a thumb in from each end: 20 and 80 lie a fifth and four fifths along.
    let x_at = |share: f32| FIELD.left() + FIELD.width() * share;
    let on = |fixture: &mut Fixture, x: f32| {
        fixture.frame(vec![pointer::moved(Pos2::new(x, FIELD.center().y))]);
        rings(&fixture.frame_shapes(), FIELD) - rest
    };
    assert_eq!(on(&mut fixture, x_at(0.45)), 2, "over the span both thumbs wear the ring");
    let upper = FIELD.left() + 4.0 + (FIELD.width() - 8.0) * 0.8;
    assert_eq!(on(&mut fixture, upper), 1, "over a thumb that thumb alone wears it");
    fixture.click_at(Pos2::new(x_at(0.6), FIELD.center().y)).settle();
    assert!(fixture.focused_stop().is_some_and(|(_, part)| part.is_some()), "a click on the span focuses a thumb");
    assert_eq!(*range.borrow(), (Mixed::Same(20.0), Mixed::Same(80.0)), "the click moves neither thumb");

    let slide = Rc::new(Slide::new(START));
    let showing = slide.clone();
    let mut fixture = Fixture::new(move |ui| showing.show(ui));
    fixture.motion(MotionMode::Off).settle();
    let rest = rings(&fixture.frame_shapes(), FIELD);
    fixture.frame(vec![pointer::moved(Pos2::new(FIELD.left() + 12.0, FIELD.center().y))]);
    assert_eq!(rings(&fixture.frame_shapes(), FIELD) - rest, 1, "over the bare track the slider's thumb wears the ring");
}
