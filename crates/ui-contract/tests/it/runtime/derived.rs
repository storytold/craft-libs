//! Derived values: built once per key, kept while the key is unchanged, and dropped by the memo rule once absent.

use craft_ui::runtime::derived::derived;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, last_pass_derived_builds, trial};
use egui::Id;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// A fixture whose pass derives a value under each id `ids` holds, keyed by `key`; `seen` collects the values.
fn deriving(ids: Rc<RefCell<Vec<u32>>>, key: Rc<Cell<u32>>, seen: Rc<RefCell<Vec<Arc<String>>>>) -> Fixture {
    Fixture::new(move |ui| {
        seen.borrow_mut().clear();
        for id in ids.borrow().iter() {
            let value = derived(ui.ctx(), Id::NULL.with(("derived", *id)), &key.get(), || format!("{id} at {}", key.get()));
            seen.borrow_mut().push(value);
        }
    })
}

/// The number of builds in the last pass.
fn builds(fixture: &Fixture) -> u32 {
    last_pass_derived_builds(fixture.ctx()).iter().map(|(_, builds)| builds).sum()
}

/// An unchanged key returns the kept value without a build; a changed key builds once, then keeps that.
#[trial]
fn a_derived_value_builds_once_per_key() {
    let (ids, key, seen) = (Rc::new(RefCell::new(vec![1])), Rc::new(Cell::new(7)), Rc::default());
    let mut fixture = deriving(Rc::clone(&ids), Rc::clone(&key), Rc::clone(&seen));
    fixture.frame(Vec::new());
    let first = Arc::clone(&seen.borrow()[0]);
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert_eq!(builds(&fixture), 0, "an unchanged key builds nothing");
        assert!(Arc::ptr_eq(&seen.borrow()[0], &first), "and returns the kept value");
    }
    key.set(8);
    fixture.frame(Vec::new());
    assert_eq!(builds(&fixture), 1, "a changed key builds once");
    assert_eq!(*seen.borrow()[0], "1 at 8");
    fixture.frame(Vec::new());
    assert_eq!(builds(&fixture), 0, "then keeps the new value");
}

/// A value no pass asks for is kept while the absent ones are at most twice the shown ones, and dropped past that.
#[trial]
fn an_absent_derived_value_is_dropped_by_the_memo_rule() {
    let (ids, key, seen) = (Rc::new(RefCell::new(vec![1, 2, 3])), Rc::new(Cell::new(0)), Rc::default());
    let mut fixture = deriving(Rc::clone(&ids), Rc::clone(&key), Rc::clone(&seen));
    fixture.frame(Vec::new()).frame(Vec::new());
    *ids.borrow_mut() = vec![1];
    fixture.frame(Vec::new()).frame(Vec::new()).frame(Vec::new());
    *ids.borrow_mut() = vec![1, 2, 3];
    fixture.frame(Vec::new());
    assert_eq!(builds(&fixture), 0, "two absent values beside one shown are kept");
    *ids.borrow_mut() = Vec::new();
    fixture.frame(Vec::new()).frame(Vec::new()).frame(Vec::new());
    *ids.borrow_mut() = vec![1];
    fixture.frame(Vec::new());
    assert_eq!(builds(&fixture), 1, "with none shown, every absent value was dropped");
}

/// A memoized view that replays asks for no derived value, and its values stay kept: when the pointer then runs the
/// view, it finds them.
#[trial]
fn a_replayed_view_keeps_the_values_it_derived() {
    use craft_ui::contract::MemoView;
    use craft_ui::testing::pointer;
    let at: Rc<Cell<egui::Pos2>> = Rc::default();
    let shown = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| {
        let out = MemoView::new("deriving", 1u32, |key: &mut u32, ui: &mut egui::Ui| {
            let value = derived(ui.ctx(), Id::NULL.with("derived"), key, || key.to_string());
            ui.label(value.as_str()).rect.center()
        })
        .show(ui);
        if let Some(out) = out {
            shown.set(out);
        }
    });
    fixture.room(egui::Vec2::new(200.0, 100.0));
    fixture.settle().idle(3);
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert_eq!(builds(&fixture), 0, "an idle frame builds nothing");
    }
    fixture.frame(vec![pointer::moved(at.get())]);
    assert_eq!(last_frame_memo_runs(fixture.ctx()).iter().filter(|(_, _, run)| *run == MemoRun::Ran).count(), 1, "the pointer runs the view");
    assert_eq!(builds(&fixture), 0, "the view runs for the pointer and finds its value kept");
}
