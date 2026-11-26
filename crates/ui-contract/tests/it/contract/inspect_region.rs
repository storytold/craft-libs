//! An inspected region and a memoized view: whether a region is open is part of the view's key, since a run records
//! the region's steps for its components and a replay adds only the steps its run took.

use craft_ui::contract::{MemoProps, MemoView, Props as _, Show as _};
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Id, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The view's props: one probe `size` wide.
#[derive(MemoProps)]
struct Content {
    size: f32,
}

fn content(props: &mut Content, ui: &mut Ui) {
    let seen = RefCell::new(Seen::default());
    Probe::new(&seen).size(Vec2::splat(props.size)).id_salt("content").show(ui);
}

/// The view's calls in a frame that shows it `inside` the region or not; the region's record goes to `recorded`.
fn frame(frames: &mut Frames, inside: bool, recorded: &Rc<RefCell<Option<Arc<Recorded>>>>) -> Vec<MemoRun> {
    frames.frame(Vec::new(), |ui| {
        let view = |ui: &mut Ui| {
            let _ = MemoView::new("view", Content { size: 40.0 }, content).show(ui);
        };
        if inside {
            let (_, record) = inspect::record(ui, Id::NULL.with("trial.inspect.memo_region"), view);
            *recorded.borrow_mut() = record;
        } else {
            view(ui);
            *recorded.borrow_mut() = None;
        }
    });
    last_frame_memo_runs(&frames.ctx).into_iter().filter(|(name, _, _)| name.contains("Content")).map(|(_, _, run)| run).collect()
}

/// Whether every call of the frame replayed the view.
fn settled(runs: &[MemoRun]) -> bool {
    !runs.is_empty() && runs.iter().all(|run| *run == MemoRun::Replayed)
}

/// The components of the region's record, by type name.
fn components(recorded: &Arc<Recorded>) -> Vec<&'static str> {
    recorded.elements.iter().filter(|element| element.part.is_none()).map(|element| element.component).collect()
}

/// A view settled outside a region is not replayed inside one: it runs, and its run records the region's components.
/// A run recorded inside a region is replayed there, and is not replayed outside it either.
#[trial]
fn whether_a_region_is_open_is_part_of_a_memoized_views_key() {
    let recorded: Rc<RefCell<Option<Arc<Recorded>>>> = Rc::default();
    let mut frames = Frames::new();
    for _ in 0..3 {
        frame(&mut frames, false, &recorded);
    }
    let runs = frame(&mut frames, false, &recorded);
    assert!(settled(&runs), "an idle frame outside a region replays the view: {runs:?}");

    // The region opens around the unchanged view, which runs: its run records the region's components.
    let runs = frame(&mut frames, true, &recorded);
    assert!(runs.contains(&MemoRun::Ran), "a view settled outside a region runs when one opens around it: {runs:?}");
    let record = recorded.borrow().clone().expect("the region ran");
    assert_eq!(components(&record), ["Probe"], "the run records the region's components");

    // Inside the region, unchanged, the view replays: its replay adds those steps to the region again.
    let runs = frame(&mut frames, true, &recorded);
    assert!(settled(&runs), "inside the region the settled view replays: {runs:?}");
    let record = recorded.borrow().clone().expect("the region ran");
    assert_eq!(components(&record), ["Probe"], "the replay adds the region's steps");

    // The region closes around the view that ran inside it: it runs again, outside any region.
    let runs = frame(&mut frames, false, &recorded);
    assert!(runs.contains(&MemoRun::Ran), "a run recorded inside a region is not replayed outside one: {runs:?}");
    let runs = frame(&mut frames, false, &recorded);
    assert!(settled(&runs), "and settles outside the region as before: {runs:?}");
}
