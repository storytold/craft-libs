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
