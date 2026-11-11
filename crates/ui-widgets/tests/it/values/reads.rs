//! The read record: a value edit runs the pass again only when an earlier run read the edited value, or when the
//! editing component's size follows the value.

use super::kit::{AT, INITIAL, Scrub, Seen};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Pos2, Rect, Ui, Vec2};
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
