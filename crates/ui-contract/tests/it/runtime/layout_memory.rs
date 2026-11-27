//! Layout memory's handoffs: what a run hands forward reaches code that reads it later in the pass.

use craft_ui::contract::MemoView;
use craft_ui::runtime::layout_memory;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, trial};
use egui::Id;
use std::cell::RefCell;
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
