//! `Slider` and `RangeSlider`: the value maths that is the family's own: stepping in the reading direction, the scales
//! that spread a range over a track, and thumbs that never cross.

pub(crate) mod primary_down;

pub(crate) mod mark_labels;

pub(crate) mod dials;

pub(crate) mod thumb_row;

use craft_behavior::{EditEvent, MotionMode};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::pointer;
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui::value_drag::{Slider, SliderScale};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const START: f64 = 50.0;

const FIELD: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(220.0, 50.0) };

/// A slider shown in `FIELD` that holds its value and the edit events of every pass.
struct Slide {
    value: RefCell<f64>,
    range: std::ops::RangeInclusive<f64>,
    scale: SliderScale,
    step: Option<f64>,
    wheel_step: Option<f64>,
    trim: Option<(f64, f64)>,
    events: RefCell<Vec<EditEvent<f64>>>,
}

impl Slide {
    fn new(value: f64) -> Self {
        Slide {
            value: RefCell::new(value),
            range: 0.0..=100.0,
            scale: SliderScale::Linear,
            step: None,
            wheel_step: None,
            trim: None,
            events: RefCell::default(),
        }
    }

    fn show(&self, ui: &mut Ui) {
        let mut value = self.value.borrow_mut();
        let mut slider = Slider::new(Bind::bound(&mut *value), self.range.clone()).scale(self.scale).label("Opacity");
        if let Some(step) = self.step {
            slider = slider.step(step);
        }
        if let Some(step) = self.wheel_step {
            slider = slider.wheel_step(step);
        }
        if let Some((start, end)) = self.trim {
            slider = slider.trim(start..=end);
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
    dragged(&Slide::new(START), offset, travel)
}

/// `slide`'s value after grabbing its thumb `offset` from its centre and dragging `travel`.
fn dragged(slide: &Slide, offset: f32, travel: f32) -> f64 {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| slide.show(ui);
    frames.idle(3, &mut show);
    drag(&mut frames, FIELD.center() + Vec2::new(offset, 0.0), travel, &mut show, |_, _| {});
    slide.value()
}

#[trial]
fn a_log_slider_steps_in_the_callers_values() {
    use craft_ui::testing::access::Declared;
    use craft_ui::testing::focus::AccessActionKind;

    let value = Rc::new(RefCell::new(0.01));
    let shown = value.clone();
    let events = Rc::new(RefCell::new(Vec::new()));
    let reported = events.clone();
    let mut fixture = Fixture::new(move |ui| {
        let slider = Slider::new(Bind::bound(&mut *shown.borrow_mut()), 0.01..=20.0)
            .label("Distance")
            .scale(SliderScale::Log)
            .stepper(|from, direction, steps| {
                let unit = if from < 0.1 || from == 0.1 && direction < 0 { 1.0 } else { 10.0 };
                let next = (from * 100.0 + f64::from(direction) * steps * unit).round() / 100.0;
                if direction > 0 { next.min(0.12).max(from) } else { next }
            })
            .show_at(ui, FIELD);
        reported.borrow_mut().extend(slider.events);
    });
    fixture.motion(MotionMode::Off).settle();
    let slider = fixture.by_label("Distance").expect("accessible slider");
    let original_text = fixture.tree().value_of(slider.node).unwrap().to_owned();
    assert_eq!(fixture.tree().numeric_of(slider.node).unwrap().step, None);
    assert!(fixture.tree().declares_key(slider.node, Declared::Increment));
    assert!(!fixture.tree().declares_key(slider.node, Declared::Decrement));
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(*value.borrow(), 0.02);
    assert_ne!(fixture.tree().value_of(slider.node), Some(original_text.as_str()), "a fine step changes the reported value");
    assert!(fixture.tree().declares_key(slider.node, Declared::Decrement));
    fixture.frame(vec![pointer::moved(FIELD.center())]);
    fixture.frame(vec![pointer::wheel_lines(-1.0)]);
    assert_eq!(*value.borrow(), 0.01);
    fixture.access(AccessActionKind::Increment, &slider).settle();
    assert_eq!(*value.borrow(), 0.02);
    fixture.access(AccessActionKind::Decrement, &slider).settle();
    assert_eq!(*value.borrow(), 0.01);
    fixture.key(Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(*value.borrow(), 0.11);
    fixture.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(*value.borrow(), 0.12);
    assert!(!fixture.tree().declares_key(slider.node, Declared::Increment), "the ladder ends before the slider's range");
    events.borrow_mut().clear();
    fixture.key(Key::ArrowUp, Modifiers::NONE);
    fixture.frame(vec![pointer::wheel_lines(1.0)]);
    fixture.settle();
    assert!(events.borrow().is_empty(), "a step at the end of the ladder makes no edit");
    fixture.key(Key::End, Modifiers::NONE);
    assert_eq!(*value.borrow(), 20.0);
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

/// A log slider changes its value by equal ratios for equal travels (a linear one by equal differences), and a travel
/// far past either end lands on the range's end.
#[trial]
fn a_log_slider_spreads_equal_ratios_evenly() {
    let (min, max) = (0.01, 20.0);
    let log = |value: f64| Slide { range: min..=max, scale: SliderScale::Log, ..Slide::new(value) };
    let travel = 20.0;
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-3 * a.abs().max(b.abs());

    let v0 = (min * max).sqrt();
    let v1 = dragged(&log(v0), 0.0, travel);
    let v2 = dragged(&log(v1), travel, travel);
    assert!(v1 > v0 && v2 > v1, "the drags moved the value up: {v0} {v1} {v2}");
    assert!(close(v1 / v0, v2 / v1), "equal travels give equal ratios: {v0} {v1} {v2}");

    let l1 = dragged_from(0.0, travel);
    let l2 = dragged(&Slide::new(l1), travel, travel);
    assert!(close(l1 - START, l2 - l1), "a linear slider gives equal differences: {START} {l1} {l2}");

    let far = 4.0 * FIELD.width();
    let top = dragged(&log(v0), 0.0, far);
    let bottom = dragged(&log(v0), 0.0, -far);
    assert!((top - max).abs() < 1e-2 * max, "a travel past the end lands on the maximum: {top}");
    assert!((bottom - min).abs() < 1e-2 * min, "a travel past the start lands on the minimum: {bottom}");
}

/// A trimmed span's ends are drawn across the track, not stops: a drag that ends beside one lands where the drag
/// puts it.
#[trial]
fn a_drag_that_ends_beside_a_trim_end_does_not_snap_to_it() {
    let free = dragged_from(0.0, 10.0);
    let trimmed = Slide { trim: Some((0.0, free + 0.5)), ..Slide::new(START) };
    let landed = dragged(&trimmed, 0.0, 10.0);
    assert!((landed - free).abs() < 1e-3, "the drag landed at {landed}, as without the trim ({free}), not on its end");
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

/// The wheel over a value control in a scrolling panel (a slider, a number field, a select) scrolls the panel while the
/// control has no focus; it steps the control once the control has focus, or without focus inside a `WheelScope::Free`
/// scope (an adjustment dialog, an options bar). An outer `WheelScope::Gated` scope (a card in the Overview) wins over a
/// free one inside it. A trackpad steps the control only while it has focus, in a free scope too.
#[trial]
fn the_wheel_steps_a_value_control_when_focused_or_freed_and_otherwise_scrolls_the_panel() {
    use craft_ui::contract::{ContainerProps, Entry, Scroll, ShowContainer as _};
    use craft_ui::pickers::Select;
    use craft_ui::runtime::env::{WheelScope, with_env};
    use craft_ui::runtime::focus::{FocusTarget, request};
    use craft_ui::testing::layout::{Block, in_region};
    use craft_ui::text::NumberField;
    use craft_ui_test_kit::body::Slab;
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug)]
    enum Control {
        Slider,
        Number,
        Select,
    }

    /// How far the value and the panel moved after one wheel turn over `control`, shown in the scopes `scopes`
    /// (outer first) inside the panel.
    fn turned(control: Control, scopes: &[WheelScope], focused: bool, turn: fn(f32) -> egui::Event) -> (f64, f32) {
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
                        let response = match control {
                            Control::Slider => Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show(ui).response,
                            Control::Number => {
                                let mut number = Some(*value);
                                let response = NumberField::new("Opacity", Bind::bound(&mut number)).show(ui).edit.response;
                                *value = number.unwrap_or(*value);
                                response
                            }
                            Control::Select => {
                                let entries: Vec<Entry<u8>> = (40..=60).map(|n| Entry::new(n, format!("{n} %"))).collect();
                                let mut pick = *value as u8;
                                let response = Select::new("Opacity", Bind::bound(&mut pick), &entries).show(ui).response;
                                *value = f64::from(pick);
                                response
                            }
                        };
                        slider.set(Some((response.id, response.rect)));
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
        frames.frame(vec![turn(-40.0)], &mut show);
        frames.idle(2, &mut show);
        (*value.borrow() - value_before, top_before - top.get())
    }

    for control in [Control::Slider, Control::Number, Control::Select] {
        let (value, panel) = turned(control, &[], false, pointer::wheel);
        assert!(value == 0.0 && panel > 0.0, "{control:?} unfocused: the panel scrolls and the value stays ({value}, {panel})");
        let (value, panel) = turned(control, &[], true, pointer::wheel);
        assert!(value != 0.0 && panel == 0.0, "{control:?} focused: it steps and the panel stays ({value}, {panel})");
        let (value, panel) = turned(control, &[WheelScope::Free], false, pointer::wheel);
        assert!(value != 0.0 && panel == 0.0, "{control:?} in a free scope: it steps unfocused ({value}, {panel})");
        let (value, panel) = turned(control, &[WheelScope::Gated, WheelScope::Free], false, pointer::wheel);
        assert!(value == 0.0 && panel > 0.0, "{control:?}: a gated scope wins over a free one inside it ({value}, {panel})");
        let (value, panel) = turned(control, &[WheelScope::Free], false, pointer::trackpad);
        assert!(value == 0.0 && panel > 0.0, "{control:?} in a free scope: a trackpad scrolls the panel unfocused ({value}, {panel})");
        let (value, panel) = turned(control, &[WheelScope::Free], true, pointer::trackpad);
        assert!(value != 0.0 && panel == 0.0, "{control:?} focused: a trackpad steps it ({value}, {panel})");
    }
}

/// A wheel gesture that starts over a free control (the font field in an options bar) steps it; once the pointer
/// leaves, the gesture is no longer the control's, and coming back over it in the same gesture does not step it again.
#[trial]
fn a_wheel_gesture_that_leaves_a_free_control_does_not_step_it_on_coming_back() {
    use craft_ui::contract::{ContainerProps, Entry, Scroll, ShowContainer as _};
    use craft_ui::pickers::Select;
    use craft_ui::runtime::env::{WheelScope, with_env};
    use craft_ui::testing::layout::{Block, in_region};
    use craft_ui_test_kit::body::Slab;
    use std::cell::Cell;

    let entries: Vec<Entry<u8>> = (40..=60).map(|n| Entry::new(n, format!("{n} pt"))).collect();
    let pick = RefCell::new(50_u8);
    let select = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 200.0)), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                with_env(
                    ui,
                    |set| set.wheel = Some(WheelScope::Free),
                    |ui| select.set(Select::new("Size", Bind::bound(&mut *pick.borrow_mut()), &entries).show(ui).response.rect),
                );
                Block::new(Vec2::new(100.0, 1000.0)).show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let on = select.get().center();
    let off = on + Vec2::new(0.0, 80.0);
    assert!(!select.get().contains(off), "the off point lies outside the select: {:?}", select.get());
    // One gesture, each event well inside the gap of the one before; the panel is at its top, so the upward scroll
    // moves nothing once the gesture is the panel's.
    frames.frame(vec![pointer::moved(on)], &mut show);
    frames.frame(vec![pointer::wheel(40.0)], &mut show);
    let stepped = *pick.borrow();
    assert_eq!(stepped.abs_diff(50), 1, "the gesture's start over the free select steps it once");
    frames.frame(vec![pointer::moved(off)], &mut show);
    frames.frame(vec![pointer::wheel(40.0)], &mut show);
    frames.frame(vec![pointer::moved(on)], &mut show);
    frames.frame(vec![pointer::wheel(40.0)], &mut show);
    frames.frame(vec![pointer::wheel(40.0)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(*pick.borrow(), stepped, "coming back over the select in the same gesture does not step it again");
}

/// A wheel notch down over a free select steps to the next entry, a notch up to the previous one, as over a combo box
/// and a tab strip.
#[trial]
fn a_wheel_notch_down_steps_a_free_select_to_the_next_entry() {
    use craft_ui::contract::Entry;
    use craft_ui::pickers::Select;
    use craft_ui::runtime::env::{WheelScope, with_env};
    use craft_ui::testing::layout::in_region;
    use std::cell::Cell;

    let entries: Vec<Entry<u8>> = (40..=60).map(|n| Entry::new(n, format!("{n} pt"))).collect();
    let pick = RefCell::new(50_u8);
    let select = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 200.0)), |ui| {
            with_env(
                ui,
                |set| set.wheel = Some(WheelScope::Free),
                |ui| select.set(Select::new("Size", Bind::bound(&mut *pick.borrow_mut()), &entries).show(ui).response.rect),
            );
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    frames.frame(vec![pointer::moved(select.get().center())], &mut show);
    frames.frame(vec![pointer::wheel_lines(-1.0)], &mut show);
    frames.advance(1.0);
    frames.idle(3, &mut show);
    assert_eq!(*pick.borrow(), 51, "a notch down steps to the next entry");
    frames.frame(vec![pointer::wheel_lines(1.0)], &mut show);
    frames.advance(1.0);
    frames.idle(3, &mut show);
    assert_eq!(*pick.borrow(), 50, "a notch up steps back");
}

#[trial]
fn a_wheel_gesture_keeps_scrolling_the_panel_over_a_free_slider() {
    use craft_behavior::GESTURE_GAP;
    use craft_ui::contract::{ContainerProps, Scroll, ShowContainer as _};
    use craft_ui::runtime::env::{WheelScope, with_env};
    use craft_ui::testing::layout::{Block, in_region};
    use craft_ui_test_kit::body::Slab;
    use std::cell::Cell;

    // The pointer stays over the panel above the slider; the panel's scroll carries the slider up under it. The panel
    // is as wide as its widest member, so the point lies within the blocks' and the slider's span.
    let pointer_at = Pos2::new(60.0, 150.0);
    let value = RefCell::new(START);
    let (top, slider, passed_under) = (Cell::new(0.0_f32), Cell::new(Rect::NOTHING), Cell::new(false));
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 300.0)), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                top.set(ui.next_widget_position().y);
                Block::new(Vec2::new(100.0, 200.0)).show(ui);
                with_env(
                    ui,
                    |set| set.wheel = Some(WheelScope::Free),
                    |ui| {
                        let mut value = value.borrow_mut();
                        let rect = Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show(ui).response.rect;
                        slider.set(rect);
                        passed_under.set(passed_under.get() || rect.contains(pointer_at));
                    },
                );
                Block::new(Vec2::new(100.0, 1000.0)).show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    assert!(slider.get().min.y > pointer_at.y, "the slider starts below the pointer: {:?}", slider.get());
    let top_before = top.get();
    frames.frame(vec![pointer::moved(pointer_at)], &mut show);
    // One gesture: a wheel event every frame, each well inside the gap of the one before.
    for _ in 0..15 {
        frames.frame(vec![pointer::wheel(-8.0)], &mut show);
    }
    frames.idle(2, &mut show);
    assert!(passed_under.get(), "the slider passed under the pointer: {:?}", slider.get());
    assert!(top_before - top.get() > 60.0, "the panel kept scrolling: {} -> {}", top_before, top.get());
    assert_eq!(*value.borrow(), START, "the free slider the gesture passed over did not step");

    // A new gesture over the slider steps it and leaves the panel where it is.
    frames.advance(GESTURE_GAP);
    frames.frame(vec![pointer::moved(slider.get().center())], &mut show);
    let top_before = top.get();
    frames.frame(vec![pointer::wheel(-40.0)], &mut show);
    frames.idle(2, &mut show);
    assert_ne!(*value.borrow(), START, "a new gesture over the free slider steps it");
    assert_eq!(top.get(), top_before, "the panel stays");
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

/// The filled rects a frame paints inside `within`.
fn fills(shapes: &[egui::Shape], within: Rect) -> usize {
    shapes.iter().filter(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill.a() > 0 && within.expand(8.0).contains_rect(rect.rect))).count()
}

/// A slider draws the ticks it is asked for: `ticks(10)` paints eleven marks under the track more than no ticks.
#[trial]
fn a_slider_draws_the_ticks_it_is_asked_for() {
    let painted = |ticks: usize| {
        let mut fixture = Fixture::new(move |ui| {
            let mut value = START;
            Slider::new(Bind::bound(&mut value), 0.0..=100.0).ticks(ticks).label("Opacity").show_at(ui, FIELD);
        });
        fixture.motion(MotionMode::Off).settle();
        fills(&fixture.frame_shapes(), FIELD)
    };
    assert_eq!(painted(10) - painted(0), 11, "ten intervals draw eleven ticks");
}

/// A slider drag on a value the app applies each frame runs each drag frame once.
#[trial]
fn a_slider_drag_on_a_given_value_runs_one_pass_a_frame() {
    use craft_ui::contract::Role;
    let model = Rc::new(std::cell::Cell::new(50.0));
    let host = model.clone();
    let mut fixture = Fixture::new(move |ui| {
        if let Some(value) = Slider::new(Bind::given(host.get()), 0.0..=100.0).label("Opacity").show(ui).value() {
            host.set(*value);
        }
    });
    fixture.settle();
    let thumb = fixture.by_role(Role::SLIDER).first().expect("the slider").center();
    fixture.frame(vec![pointer::moved(thumb)]);
    fixture.frame(vec![pointer::press(thumb)]);
    let mut passes = Vec::new();
    for step in 1..=6 {
        fixture.frame(vec![pointer::moved(thumb + Vec2::new(5.0 * step as f32, 0.0))]);
        passes.push(fixture.passes());
    }
    assert!(model.get() > 50.0, "the drag moved the app's value");
    assert!(passes.iter().all(|&n| n == 1), "slider drag frames ran {passes:?} passes");
}
