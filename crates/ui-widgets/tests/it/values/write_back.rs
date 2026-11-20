//! A host that stores a proposal at the field's precision has not changed the document under the gesture.

use crate::label::kit::Field;
use craft_behavior::{CancelReason, EditEvent, NumberCodec, Reader};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui::text::NumberField;
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

const ROW: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 40.0) };

const START: f64 = 10.0;

/// The decimals of the field's codec.
const DECIMALS: usize = 2;

/// How far the host moves a value it overwrites.
const OVERWRITE: f64 = 5.0;

/// What the host stores for a proposal: its value as the float32 the document keeps, at the precision of the field.
fn stored(value: f64) -> f64 {
    f64::from(value as f32)
}

/// Scrubs a given-bound field by its label; the host applies each event after `host` changed what it stores.
/// Returns the cancel reasons and whether a commit arrived.
fn scrub(host: impl Fn(f64) -> f64) -> (Vec<CancelReason>, bool) {
    let mut frames = Frames::new();
    let mut doc = Some(START);
    let cancels = RefCell::new(Vec::new());
    let committed = RefCell::new(false);
    let mut show = |ui: &mut Ui| {
        Field::new("Size", SideLabel::new()).id_salt("row").show_at(ui, ROW, |ui, _| {
            let codec = NumberCodec::new(Reader::new(), DECIMALS);
            let out = NumberField::new("Size", Bind::given(doc)).codec(codec).scrub_by_label().id_salt("scrubbed").show(ui);
            for event in &out.events {
                match event {
                    EditEvent::Update { value, .. } => doc = value.map(&host),
                    EditEvent::Commit { after, .. } => {
                        doc = after.map(&host);
                        *committed.borrow_mut() = true;
                    }
                    EditEvent::Cancel { reason, .. } => cancels.borrow_mut().push(*reason),
                    _ => {}
                }
            }
        });
    };
    frames.idle(4, &mut show);
    let from = ROW.min + Vec2::new(4.0, 12.0);
    frames.frame(vec![moved(from)], &mut show);
    frames.frame(vec![press(from)], &mut show);
    for travel in [20.0_f32, 23.37, 29.173, 36.357] {
        frames.frame(vec![moved(from + Vec2::new(travel, 0.0))], &mut show);
    }
    frames.frame(vec![release(from + Vec2::new(36.357, 0.0))], &mut show);
    frames.idle(2, &mut show);
    (cancels.take(), committed.take())
}

/// A drag whose proposals the host stores at a float32's precision ends with a commit; a host that writes a different
/// value cancels with `HostChanged`.
#[trial]
fn a_write_back_at_the_codecs_precision_is_the_gestures_own() {
    let (cancels, committed) = scrub(stored);
    assert!(cancels.is_empty() && committed, "rounded write-back: cancels {cancels:?}, committed {committed}");
    let (cancels, _) = scrub(|value| value + OVERWRITE);
    assert_eq!(cancels, vec![CancelReason::HostChanged], "a different value is a host change");
}

/// A slider field whose host keeps the value in float32 (a percentage stored as a fraction) gets each proposal back
/// rounded to it: the drag runs on and ends with one commit, with no host-change cancel.
#[trial]
fn a_slider_fields_drag_survives_a_float32_write_back() {
    use craft_ui::form::SliderField;
    use craft_ui::runtime::access::Role;
    use craft_ui::testing::Fixture;
    use std::rc::Rc;
    let doc = Rc::new(RefCell::new(0.5_f32));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (held, seen) = (Rc::clone(&doc), Rc::clone(&events));
    let mut fixture = Fixture::new(move |ui| {
        let given = f64::from(*held.borrow()) * 100.0;
        let out = SliderField::new("Density", Bind::given(given), 0.0..=100.0).show(ui);
        for event in &out.events {
            if let Some(value) = event.landed() {
                *held.borrow_mut() = (value / 100.0) as f32;
            }
            seen.borrow_mut().push(event.clone());
        }
    });
    fixture.settle();
    let track = fixture.by_role(Role::SLIDER).first().expect("the slider").rect;
    fixture.drag_at(track.left_center() + Vec2::new(track.width() * 0.3, 0.0), Vec2::new(track.width() * 0.237, 0.0));
    fixture.settle();
    let events = events.borrow();
    assert!(!events.iter().any(|event| matches!(event, EditEvent::Cancel { .. })), "no cancel: {events:?}");
    assert_eq!(events.iter().filter(|event| matches!(event, EditEvent::Commit { .. })).count(), 1, "one commit: {events:?}");
    assert!((f64::from(*doc.borrow()) - 0.5).abs() > 0.01, "the drag moved the value");
}

#[trial]
fn typed_slider_value_keeps_precision_independent_of_step() {
    use craft_ui::form::SliderField;
    use craft_ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use std::cell::Cell;
    use std::rc::Rc;
    let value = Rc::new(Cell::new(20.0));
    let held = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        SliderField::new("Size", Bind::bound(&mut now), 0.0..=100.0).step(1.0).typed_decimals(4).show(ui);
        held.set(now);
    });
    fixture.settle();
    let field = fixture.by_label("Size").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("36.1231");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!((value.get() - 36.1231_f64).abs() < 0.00001, "typed value: {}", value.get());
}
