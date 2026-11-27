//! Layout memory's handoffs: what a run hands forward reaches code that reads it later in the pass.

use craft_ui::contract::{MemoView, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::runtime::layout_memory;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::Panel;
use egui::{Id, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Where the view in these trials hands forward the width it measured.
fn handed() -> Id {
    Id::NULL.with("handed width")
}

/// A memoized view hands forward the width of what it shows. Code after the view reads that width in every pass,
/// the passes where the view replays included.
#[trial]
fn a_replayed_view_hands_forward_the_width_its_run_measured() {
    let read: Rc<RefCell<Vec<Option<f32>>>> = Rc::default();
    let seen = Rc::clone(&read);
    let mut fixture = Fixture::new(move |ui| {
        let _ = MemoView::new("measuring", 1u32, |_: &mut u32, ui: &mut egui::Ui| {
            let took = ui.label("Handed forward").rect.width();
            layout_memory::emit_width(ui.ctx(), handed(), took);
        })
        .show(ui);
        seen.borrow_mut().push(layout_memory::width(ui.ctx(), handed()));
    });
    fixture.room(egui::Vec2::new(200.0, 100.0));
    fixture.settle();
    let ran = read.borrow().last().copied().flatten();
    assert!(ran.is_some_and(|width| width > 0.0), "the view's run hands forward its width: {ran:?}");
    for _ in 0..3 {
        read.borrow_mut().clear();
        fixture.frame(Vec::new());
        let runs = last_frame_memo_runs(fixture.ctx());
        assert!(!runs.is_empty() && runs.iter().all(|(_, _, run)| *run == MemoRun::Replayed), "an idle frame replays the view: {runs:?}");
        assert!(read.borrow().iter().all(|width| *width == ran), "a replay hands forward the width the run measured: {:?}", read.borrow());
    }
}

/// A capped body drafts its content at its content height and, taller than its room, takes the draft back and lays the
/// content out at its room, so a memoized view in it is measured in two contexts in one pass. The view around the body
/// takes a new key each frame, so the body measures afresh under its new content version; the inner view's own inputs
/// stay the same, so it replays in the draft, in the measure at the room and in the real run. Views are counted by their
/// props type: `u32` around the body, `u64` inside it.
#[trial]
fn a_view_in_a_drafted_body_replays_in_both_measures_while_the_view_around_it_changes() {
    let step = Rc::new(Cell::new(0_u32));
    let shown = Rc::clone(&step);
    let mut fixture = Fixture::new(move |ui| {
        let _ = MemoView::new("around", shown.get(), |_: &mut u32, ui: &mut egui::Ui| {
            Panel::new().id_salt("layer").show(ui, |ui, _| {
                Panel::new().sizing(Sizing::Fill).id_salt("capped").show(ui, |ui, _| {
                    Block::new(Vec2::new(100.0, 20.0)).show(ui);
                    let _ = MemoView::new("list", 1_u64, |_: &mut u64, ui: &mut egui::Ui| {
                        Block::new(Vec2::new(100.0, 600.0)).show(ui);
                    })
                    .show(ui);
                });
            });
        })
        .show(ui);
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    for at in 1..5 {
        step.set(at);
        fixture.frame(Vec::new());
        let runs = last_frame_memo_runs(fixture.ctx());
        let of = |name: &str| runs.iter().filter(|(view, _, _)| *view == name).map(|(_, _, run)| *run).collect::<Vec<_>>();
        let list = of("u64");
        assert!(of("u32").contains(&MemoRun::Ran), "step {at}: the new key runs the view around the body: {runs:?}");
        assert!(list.len() >= 3, "step {at}: the body measures the inner view as a draft and at its room, then shows it: {runs:?}");
        assert!(list.iter().all(|run| *run == MemoRun::Replayed), "step {at}: the inner view's inputs are unchanged, so no call runs it: {runs:?}");
    }
}
