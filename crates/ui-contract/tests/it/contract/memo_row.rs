//! A memoized view that is one unit of a shrinking row: the row's counting sweep calls it in another flow than its real
//! run (the content shows unbounded there), and that call leaves the replay decision of the pass to the real call and
//! reuses the measure of the last counting call.

use craft_ui::contract::{MemoProps, MemoView, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, last_pass_memo_runs, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

/// A view that shows one probe `width` wide.
#[derive(MemoProps)]
struct Widened {
    width: f32,
}

fn widened(props: &mut Widened, ui: &mut Ui) {
    let seen = RefCell::new(Seen::default());
    Probe::new(&seen).size(Vec2::new(props.width, 20.0)).id_salt("content").show(ui);
}

/// How the `Widened` views took part in `runs`.
fn of_view(runs: Vec<(&'static str, egui::Id, MemoRun)>) -> Vec<MemoRun> {
    runs.into_iter().filter(|(name, _, _)| name.contains("Widened")).map(|(_, _, run)| run).collect()
}

/// A memoized view in a row that is not short, whose new content is wider: it runs once in the real run that meets the
/// new props and once as a measuring run in the row's counting sweep, whose flow is not the real run's (the content
/// shows unbounded there). The real run of the discard pass that sweep belongs to replays, and so does
/// every call of the next frame, a counting sweep's included.
#[trial]
fn a_memoized_row_unit_replays_after_the_rows_counting_call() {
    let width = Cell::new(60.0);
    let mut frames = Frames::new();
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(400.0, 40.0)), |ui| {
                Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("row").show(ui, |ui, _| {
                    let _ = MemoView::new("view", Widened { width: width.get() }, widened).show(ui);
                    let seen = RefCell::new(Seen::default());
                    Probe::new(&seen).id_salt("after").show(ui);
                });
            });
        });
    };
    for _ in 0..30 {
        frame(&mut frames);
    }
    assert_eq!(of_view(last_frame_memo_runs(&frames.ctx)), [MemoRun::Replayed], "an idle frame replays the view");
    width.set(90.0);
    frame(&mut frames);
    let calls = of_view(last_frame_memo_runs(&frames.ctx));
    assert_eq!(calls.iter().filter(|run| **run == MemoRun::Ran).count(), 2, "one real run and one measuring run: {calls:?}");
    assert_eq!(of_view(last_pass_memo_runs(&frames.ctx)), [MemoRun::Replayed], "the discard pass's real call replays: {calls:?}");
    frame(&mut frames);
    let calls = of_view(last_frame_memo_runs(&frames.ctx));
    assert!(calls.iter().all(|run| *run == MemoRun::Replayed), "the next frame replays the view in every call: {calls:?}");
}

thread_local! {
    /// The rect the fill panel of a `Filled` view took in its last run.
    static FILLED: RefCell<Seen> = RefCell::default();
}

/// A view whose content is one panel that fills the main axis of the body around it (`axis`: 0 the width, 1 the height).
#[derive(MemoProps)]
struct Filled {
    axis: u8,
}

fn filled(props: &mut Filled, ui: &mut Ui) {
    fill_panel(ui, props.axis);
}

/// The panel that fills `axis`, holding one probe.
fn fill_panel(ui: &mut Ui, axis: u8) {
    let sizing = if axis == 0 { Sizing::FillWidth } else { Sizing::FillHeight };
    FILLED.with(|seen| {
        Panel::new().seen(seen).sizing(sizing).id_salt("fill").show(ui, |ui, _| {
            let inner = RefCell::new(Seen::default());
            Probe::new(&inner).id_salt("inner").show(ui);
        });
    });
}

/// The rects of the fill panel and the probes before and after it in a body along `axis` (0 a row, 1 a stack) given its
/// room, the panel shown through a memoized view when `memo`, after the frames settle; and how the view took part in
/// the calls of one more frame.
fn shared_body(axis: u8, memo: bool) -> ([Rect; 3], Vec<MemoRun>) {
    let (before, after) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut frames = Frames::new();
    let (layout, room) = if axis == 0 { (BodyLayout::row(), Vec2::new(400.0, 40.0)) } else { (BodyLayout::stack(), Vec2::new(200.0, 300.0)) };
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), room), |ui| {
                Panel::new().layout(layout.clone()).sizing(Sizing::Fill).id_salt("body").show(ui, |ui, _| {
                    Probe::new(&before).id_salt("before").show(ui);
                    if memo {
                        let _ = MemoView::new("view", Filled { axis }, filled).show(ui);
                    } else {
                        fill_panel(ui, axis);
                    }
                    Probe::new(&after).id_salt("after").show(ui);
                });
            });
        });
    };
    for _ in 0..30 {
        frame(&mut frames);
    }
    frame(&mut frames);
    let runs = last_frame_memo_runs(&frames.ctx).into_iter().filter(|(name, _, _)| name.contains("Filled")).map(|(_, _, run)| run).collect();
    ([before.borrow().rect, FILLED.with(|seen| seen.borrow().rect), after.borrow().rect], runs)
}

/// A memoized view whose content fills the width of the row around it is one of the children that fill the row, as the
/// content shown without the view is: it takes the row's free width and the probe after it keeps its place at the row's
/// end. With nothing changed every call of a frame replays the view.
#[trial]
fn a_memoized_view_whose_content_fills_the_row_takes_its_free_width() {
    let (direct, _) = shared_body(0, false);
    let (memo, runs) = shared_body(0, true);
    let [before, fill, after] = memo;
    assert!(fill.left() >= before.right() && after.left() >= fill.right(), "the view lies between the probes: {memo:?}");
    for (shown, alone) in memo.iter().zip(direct) {
        assert!(shown.min.distance(alone.min) < 0.5 && shown.max.distance(alone.max) < 0.5, "as without the view: {memo:?} and {direct:?}");
    }
    assert!(!runs.is_empty() && runs.iter().all(|run| *run == MemoRun::Replayed), "an idle frame replays the view in every call: {runs:?}");
}

/// A memoized view whose content fills the height of the stack around it takes its share of the stack's height, as the
/// content shown without the view does: the probe after it stays inside the stack. With nothing changed every call of a
/// frame replays the view.
#[trial]
fn a_memoized_view_whose_content_fills_the_stack_takes_its_share_of_the_height() {
    let (direct, _) = shared_body(1, false);
    let (memo, runs) = shared_body(1, true);
    let [before, fill, after] = memo;
    assert!(fill.top() >= before.bottom() && after.top() >= fill.bottom(), "the view lies between the probes: {memo:?}");
    for (shown, alone) in memo.iter().zip(direct) {
        assert!(shown.min.distance(alone.min) < 0.5 && shown.max.distance(alone.max) < 0.5, "as without the view: {memo:?} and {direct:?}");
    }
    assert!(!runs.is_empty() && runs.iter().all(|run| *run == MemoRun::Replayed), "an idle frame replays the view in every call: {runs:?}");
}
