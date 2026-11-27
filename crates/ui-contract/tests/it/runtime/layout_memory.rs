//! Layout memory's handoffs: what a run hands forward reaches code that reads it later in the pass.

use craft_ui::contract::{MemoView, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::runtime::layout_memory;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::Panel;
use egui::{Id, Rect, UiBuilder, Vec2};
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

/// A grid sweeps a content column at a trial width and then at the width it settles on, so a memoized view in the
/// column is measured in two rooms in one pass, and code after each sizing call reads what the view handed forward
/// there. In the passes where both calls take the view's kept measures, each still hands forward what the run in its
/// room handed forward.
#[trial]
fn a_view_measured_in_two_rooms_hands_forward_each_rooms_width_when_it_takes_its_kept_measures() {
    const ROOMS: [f32; 2] = [120.0, 180.0];
    let read: Rc<RefCell<Vec<Option<f32>>>> = Rc::default();
    let seen = Rc::clone(&read);
    let mut fixture = Fixture::new(move |ui| {
        let start = ui.available_rect_before_wrap().min;
        let view = |ui: &mut egui::Ui| {
            let _ = MemoView::new("measured", 1_u32, |_: &mut u32, ui: &mut egui::Ui| {
                layout_memory::emit_width(ui.ctx(), handed(), ui.available_width());
                ui.label("Handed forward");
            })
            .show(ui);
        };
        let cell_id = ui.scope_id().with("cell");
        let cell = |room: Vec2| UiBuilder::new().max_rect(Rect::from_min_size(start, room)).scope_id(cell_id);
        for width in ROOMS {
            view(&mut ui.new_child(cell(Vec2::new(width, f32::INFINITY)).sizing_pass().invisible()));
            seen.borrow_mut().push(layout_memory::width(ui.ctx(), handed()));
        }
        view(&mut ui.new_child(cell(Vec2::new(ROOMS[1], 100.0))));
    });
    fixture.room(Vec2::new(300.0, 200.0));
    fixture.settle();
    let ran: Vec<Option<f32>> = read.borrow()[read.borrow().len() - ROOMS.len()..].to_vec();
    assert!(ran[0].is_some() && ran[0] != ran[1], "each room's run hands forward its own width: {ran:?}");
    for at in 0..3 {
        read.borrow_mut().clear();
        fixture.frame(Vec::new());
        let runs = last_frame_memo_runs(fixture.ctx());
        assert!(!runs.is_empty() && runs.iter().all(|(_, _, run)| *run == MemoRun::Replayed), "frame {at}: every call replays: {runs:?}");
        assert_eq!(*read.borrow(), ran, "frame {at}: each sizing call hands forward its room's width");
    }
}
