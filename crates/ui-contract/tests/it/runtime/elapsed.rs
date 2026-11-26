//! The pass's elapsed time: a frame's later passes share its pass clock, so time-driven work steps once a frame
//! however many passes the frame runs.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::runtime::frame::{discard_pass, running};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const ROWS: usize = 40;
const ROW_H: f32 = 20.0;

/// A frame forced to run twice reports the time since the previous frame in its first pass and none in its second.
#[trial]
fn a_frames_second_pass_reports_no_elapsed_time() {
    let mut frames = Frames::new();
    frames.idle(2, |_| {});
    let seen = RefCell::new(Vec::new());
    let passes = frames.frame(Vec::new(), |ui| {
        let facts = running(ui.ctx()).expect("the frame's pass is running");
        if seen.borrow().is_empty() {
            discard_pass(ui.ctx(), "trial");
        }
        seen.borrow_mut().push(facts.elapsed);
    });
    let seen = seen.into_inner();
    assert_eq!(passes, 2, "the discard runs the frame twice");
    assert!(seen[0] > 0.0, "the first pass steps by the time since the previous frame: {seen:?}");
    assert!(seen[1].abs() < f64::EPSILON, "the second pass shares the frame's clock: {seen:?}");
}

/// A scroll strip under the resting pointer scrolls as far over a frame forced to run twice as over the frames
/// around it: the step follows the clock, not the passes. A row's rect is its part in view, so the step is read from
/// the rows wholly in view before and after a frame.
#[trial]
fn a_scroll_strip_steps_once_a_frame_however_many_passes_it_runs() {
    let rows = RefCell::new(vec![Rect::NOTHING; ROWS]);
    let discard = Cell::new(false);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        if discard.take() {
            discard_pass(ui.ctx(), "trial");
        }
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical().strips())).id_salt("strips").show(ui, |ui, _| {
                for row in 0..ROWS {
                    rows.borrow_mut()[row] = Block::new(Vec2::new(VIEW.x, ROW_H)).show(ui).rect;
                }
            });
        });
    };
    frames.idle(3, &mut show);
    // At the start the first row sits right under the back strip.
    let top0 = rows.borrow()[0].min.y;
    frames.frame(vec![pointer::moved(Pos2::new(ORIGIN.x + VIEW.x / 2.0, top0 + 40.0))], &mut show);
    frames.frame(vec![pointer::wheel(-100_000.0)], &mut show);
    let whole = |rect: &Rect| (rect.height() - ROW_H).abs() < 0.01;
    let mut step = |events: Vec<egui::Event>, forced: bool| {
        let before = rows.borrow().clone();
        discard.set(forced);
        frames.frame(events, &mut show);
        let after = rows.borrow();
        let row = (0..ROWS).find(|&row| whole(&before[row]) && whole(&after[row])).expect("a row stays wholly in view");
        after[row].min.y - before[row].min.y
    };
    // The first frame on the strip is the reference: it runs both passes of a scrolling frame and has no step left
    // over from an earlier frame.
    let plain = step(vec![pointer::moved(Pos2::new(ORIGIN.x + VIEW.x / 2.0, top0 - 1.0))], false);
    let (forced, after) = (step(Vec::new(), true), step(Vec::new(), false));
    assert!(plain > 0.0, "resting on the back strip scrolls back: {plain}");
    for (name, moved) in [("forced to run twice", forced), ("after it", after)] {
        assert!(moved > plain * 0.5 && moved < plain * 1.5, "the frame {name} scrolls as far as the one before it: {moved} vs {plain}");
    }
}
