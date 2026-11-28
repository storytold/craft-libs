//! The history panel applies the host's delete rule on both paths a row can be deleted by, and a press on a row's
//! source cell sets the source without moving the document's history.

use craft_ui::patterns::panels::{HistoryEvent, HistoryFooter, HistoryKey, HistoryPanel, HistoryRow};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<HistoryEvent>>>;

/// A panel over four rows with the oldest state undeletable, the second state current and Delete in the footer;
/// `source` shows the source column with that row marked.
fn rig(source: Option<HistoryKey>, with_source: bool) -> (Fixture, Seen) {
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let rows = vec![HistoryRow::state(1, "Open"), HistoryRow::state(2, "Brush"), HistoryRow::state(3, "Fill"), HistoryRow::snapshot(7, "Snapshot 1")];
    let fixture = Fixture::new(move |ui| {
        let mut panel = HistoryPanel::new(&rows, Some(HistoryKey::State(2)))
            .can_delete(|key| *key != HistoryKey::State(1))
            .footer(HistoryFooter::default().delete(true));
        if with_source {
            panel = panel.source(source);
        }
        sink.borrow_mut().extend(panel.show(ui).events);
    });
    (fixture, seen)
}

fn deletes(seen: &Seen) -> Vec<HistoryEvent> {
    seen.borrow().iter().filter(|event| matches!(event, HistoryEvent::Delete(_))).cloned().collect()
}

#[trial]
fn history_rows_drop_on_delete_by_the_hosts_rule() {
    let (mut fixture, seen) = rig(None, false);
    fixture.settle();
    let button = fixture.by_label("Delete Current State").expect("the Delete button");

    let fill = fixture.by_label("Fill").expect("the Fill row");
    fixture.drag(&fill, button.center() - fill.center()).settle();
    assert_eq!(deletes(&seen), vec![HistoryEvent::Delete(Some(HistoryKey::State(3)))], "a deletable row dropped on Delete is reported");

    seen.borrow_mut().clear();
    let open = fixture.by_label("Open").expect("the Open row");
    fixture.drag(&open, button.center() - open.center()).settle();
    assert_eq!(deletes(&seen), Vec::<HistoryEvent>::new(), "the oldest state starts no deleting drag and Delete takes no drop of it");

    seen.borrow_mut().clear();
    fixture.click(&button).settle();
    assert_eq!(deletes(&seen), vec![HistoryEvent::Delete(None)], "the button press deletes the picked row");
}

#[trial]
fn history_source_cell_sets_the_source_without_selecting() {
    let (mut fixture, seen) = rig(Some(HistoryKey::State(2)), true);
    fixture.settle();
    let cell = fixture.by_label("Set Fill as History Brush Source").expect("Fill's source cell");
    fixture.click(&cell).settle();
    {
        let events = seen.borrow();
        assert!(events.contains(&HistoryEvent::SetSource(HistoryKey::State(3))), "{events:?}");
        assert!(!events.iter().any(|event| matches!(event, HistoryEvent::Select(_))), "the source press must not select: {events:?}");
    }

    seen.borrow_mut().clear();
    let label = fixture.by_label("Fill").expect("the Fill row");
    fixture.click(&label).settle();
    assert!(seen.borrow().contains(&HistoryEvent::Select(HistoryKey::State(3))), "{:?}", seen.borrow());
}
