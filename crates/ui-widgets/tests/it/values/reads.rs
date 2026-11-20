//! The read record: a value edit runs the pass again only when an earlier run read the edited value, or when the
//! editing component's size follows the value.

use super::kit::{AT, INITIAL, Scrub, Seen};
use craft_behavior::Mixed;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Frames, StoryData, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::any::TypeId;
use std::cell::Cell;

/// Where a second scrub bound to the same value is shown: above `AT` in the pass when `before`, else below it.
const READER: Rect = Rect { min: Pos2::new(10.0, 60.0), max: Pos2::new(110.0, 80.0) };

/// Presses at `from` and drags right in three steps; returns the passes of each drag frame, with what `after` reads at
/// the end of that frame.
fn drag<T>(frames: &mut Frames, from: Pos2, show: &mut impl FnMut(&mut Ui), after: impl Fn() -> T) -> Vec<(usize, T)> {
    frames.idle(2, &mut *show);
    frames.frame(vec![moved(from)], &mut *show);
    frames.frame(vec![press(from)], &mut *show);
    let mut seen = Vec::new();
    for step in 1..=3 {
        let passes = frames.frame(vec![moved(from + Vec2::new(10.0 * step as f32 + 10.0, 0.0))], &mut *show);
        seen.push((passes, after()));
    }
    frames.frame(vec![release(from + Vec2::new(40.0, 0.0))], &mut *show);
    seen
}

/// Two scrubs bound to one value; the first one in the pass is the reader when `reader_first`. Returns, per drag frame
/// of the other scrub, the passes, the value and what the reader showed.
fn dragged_with_reader(reader_first: bool) -> Vec<(usize, (f32, Option<f32>))> {
    let mut frames = Frames::new();
    let value = Cell::new(INITIAL);
    let seen = Seen::default();
    let mut show = |ui: &mut Ui| {
        let mut held = value.get();
        if reader_first {
            Scrub::new(Bind::bound(&mut held)).id_salt("reader").seen(&seen).show_at(ui, READER);
        }
        Scrub::new(Bind::bound(&mut held)).id_salt("scrub").show_at(ui, AT);
        if !reader_first {
            Scrub::new(Bind::bound(&mut held)).id_salt("reader").seen(&seen).show_at(ui, READER);
        }
        value.set(held);
    };
    drag(&mut frames, Pos2::new(AT.left() + 10.0, AT.center().y), &mut show, || (value.get(), seen.shown.get()))
}

/// A drag whose value no earlier run read runs one pass a frame; a reader after the control shows the new value.
#[trial]
fn a_value_drag_with_no_earlier_reader_runs_one_pass_per_frame() {
    let frames = dragged_with_reader(false);
    assert!(frames.iter().all(|(passes, _)| *passes == 1), "one pass per drag frame: {frames:?}");
    assert!(frames.iter().all(|(_, (value, shown))| *shown == Some(*value)), "the reader below shows the dragged value: {frames:?}");
    assert!(frames.windows(2).all(|pair| matches!(pair, [(_, (a, _)), (_, (b, _))] if b > a)), "the drag moves the value: {frames:?}");
}

/// A reader drawn before the control read the old value, so the frame runs again and the reader shows the new value in
/// it.
#[trial]
fn a_reader_above_the_control_shows_the_dragged_value_in_the_same_frame() {
    let frames = dragged_with_reader(true);
    assert!(frames.iter().all(|(_, (value, shown))| *shown == Some(*value)), "the reader above shows the dragged value: {frames:?}");
    assert!(frames.iter().all(|(passes, _)| *passes == 2), "the frame runs again once: {frames:?}");
}

/// A control whose size follows its value, with no other reader, read the old value in its own real run: its edit
/// runs the frame again, so the frame shows it at the new value's size.
#[trial]
fn a_drag_that_resizes_the_control_runs_the_frame_again() {
    let mut frames = Frames::new();
    let value = Cell::new(INITIAL);
    let mut show = |ui: &mut Ui| {
        let mut held = value.get();
        Scrub::new(Bind::bound(&mut held)).id_salt("scrub").grows().show_at(ui, AT);
        value.set(held);
    };
    let frames = drag(&mut frames, Pos2::new(AT.left() + 10.0, AT.center().y), &mut show, || value.get());
    assert!(frames.iter().all(|(passes, _)| *passes == 2), "the frame runs again once: {frames:?}");
    assert!(frames.windows(2).all(|pair| matches!(pair, [(_, a), (_, b)] if b > a)), "the drag moves the value: {frames:?}");
}

/// The level the Properties editor writes into the story value.
const EDITED: f32 = 42.0;

/// A story value with two fields.
#[derive(Clone, PartialEq)]
struct Pair {
    level: f32,
    other: f32,
}

/// The story binds one field of its value (`value_at`) before the Properties editor, which writes the whole value: the
/// frame runs again, and the field shows the new value in it.
#[trial]
fn a_whole_story_value_edit_shows_in_a_field_bound_earlier_in_the_same_frame() {
    let mut frames = Frames::new();
    let data = std::cell::RefCell::new(StoryData::default());
    let seen = Seen::default();
    let edit = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        let mut data = data.borrow_mut();
        let (value, at) = data.value_at("pair", Pair { level: INITIAL, other: 0.0 });
        Scrub::new(at.bind(&mut value.level)).id_salt("reader").seen(&seen).show_at(ui, READER);
        if edit.take() {
            let key = ("pair", TypeId::of::<Pair>());
            if let Some(pair) = data.__slot(key).and_then(|slot| slot.value.any_mut().downcast_mut::<Pair>()) {
                *pair = Pair { level: EDITED, other: 1.0 };
            }
            data.__wrote(ui.ctx(), key);
        }
    };
    frames.idle(2, &mut show);
    edit.set(true);
    let passes = frames.frame(Vec::new(), &mut show);
    assert_eq!(seen.shown.get(), Some(EDITED), "the field shows the whole value's edit in the frame of the edit");
    assert_eq!(passes, 2, "the frame runs again once");
}

/// A reader of the whole story value (the Properties editor's read) runs before the story's field bind, which a click
/// edits: the frame runs again, and the whole value's reader shows the new field in it.
#[trial]
fn a_story_field_edit_shows_in_a_whole_value_read_earlier_in_the_same_frame() {
    let data = std::rc::Rc::new(std::cell::RefCell::new(StoryData::default()));
    let held = std::rc::Rc::clone(&data);
    let mut fixture = Fixture::new(move |ui| {
        let mut data = held.borrow_mut();
        let start = Pair { level: 1.0, other: 0.0 };
        RadioGroup::new("Whole", data.bind("pair", start.clone())).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (level, label) in [(1.0, "Whole 1"), (2.0, "Whole 2")] {
                    Radio::new(Pair { level, other: 0.0 }).label(label).show(ui);
                }
            });
        });
        let (value, at) = data.value_at("pair", start);
        RadioGroup::new("Field", at.bind(&mut value.level)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (level, label) in [(1.0_f32, "Field 1"), (2.0, "Field 2")] {
                    Radio::new(level).label(label).show(ui);
                }
            });
        });
    });
    fixture.settle();
    let field = fixture.by_label("Field 2").expect("the field's radio").center();
    let level = || data.borrow_mut().value_at("pair", Pair { level: 1.0, other: 0.0 }).0.level;
    fixture.frame(vec![moved(field)]);
    // The frame in which the click edits the field: the press or the release.
    let edited = [vec![press(field)], vec![release(field)]].into_iter().find_map(|events| {
        fixture.frame(events);
        (level() == 2.0).then(|| {
            let whole = fixture.by_label("Whole 2").expect("the whole value's radio");
            (fixture.tree().toggled_of(whole.node), fixture.passes())
        })
    });
    let (toggled, passes) = edited.expect("the click edits the field");
    assert_eq!(toggled, Some(Mixed::Same(true)), "the whole value shows the field's edit in that frame");
    assert_eq!(passes, 2, "the frame runs again once");
}
