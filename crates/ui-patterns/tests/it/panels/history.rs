//! The history panel applies the host's delete rule on both paths a row can be deleted by, a press on a row's source
//! cell sets the source without moving the document's history, and a row marked italic draws its label that way.

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

/// The format the panel painted the label `label` in: the style of the one run its galley holds. A label that came
/// back as more than one run is not the label this panel draws, so the trial stops rather than read the first style.
fn painted_label(shapes: &[egui::Shape], label: &str) -> egui::TextFormat {
    let galley = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.galley.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the row {label:?} paints its label as text"));
    let [section] = galley.job.sections.as_slice() else {
        panic!("the label {label:?} paints as one run of one style, got {:?}", galley.job.sections);
    };
    section.format.clone()
}

/// A row the host marks italic draws its label in italics, and italics change no ink: a redone row stays dimmed
/// whether or not it is italic, an italic row that is not redone reads in the ink of the plain row next to it, and a
/// row without the flag stands upright. A snapshot row takes the two flags the same way a state row does.
#[trial]
fn an_italic_label_slants_itself_and_leaves_the_ink_to_redo() {
    let rows = vec![
        HistoryRow::state(1, "Open"),
        HistoryRow::state(2, "Brush").redo(true),
        HistoryRow::state(3, "Fill").redo(true).italic(true),
        HistoryRow::state(4, "Text").italic(true),
        HistoryRow::state(5, "Crop"),
        HistoryRow::snapshot(9, "Before Crop").redo(true).italic(true),
    ];
    let mut fixture = Fixture::new(move |ui| {
        let _ = HistoryPanel::new(&rows, Some(HistoryKey::State(1))).show(ui);
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let current = painted_label(&shapes, "Open");
    let redo = painted_label(&shapes, "Brush");
    let both = painted_label(&shapes, "Fill");
    let italic_only = painted_label(&shapes, "Text");
    let plain = painted_label(&shapes, "Crop");
    let snapshot = painted_label(&shapes, "Before Crop");
    assert!(!current.italics && !redo.italics && !plain.italics, "a row without the flag draws its label upright: {current:?} {redo:?} {plain:?}");
    assert!(both.italics && italic_only.italics, "both flagged rows draw their labels in italics: {both:?} {italic_only:?}");
    assert_ne!(redo.color, plain.color, "a redone row reads dimmer than a plain one: {redo:?} {plain:?}");
    assert_eq!(both.color, redo.color, "the italic redone row keeps the dimmed ink: {both:?} {redo:?}");
    assert_eq!(italic_only.color, plain.color, "italics alone change no ink: {italic_only:?} {plain:?}");
    assert!(snapshot.italics, "a flagged snapshot row slants too: {snapshot:?}");
    assert_eq!(snapshot.color, redo.color, "and takes the same dimmed ink as a flagged state row: {snapshot:?} {redo:?}");
}

/// A panel the host disables still draws a flagged row's label in italics: the flag is not an enable state, and the
/// stories show the look under a disabled panel.
#[trial]
fn a_disabled_panel_still_draws_the_flagged_label_italic() {
    let rows = vec![HistoryRow::state(1, "Open"), HistoryRow::state(2, "Brush").redo(true).italic(true)];
    let mut fixture = Fixture::new(move |ui| {
        let _ = HistoryPanel::new(&rows, Some(HistoryKey::State(1))).enabled(false).show(ui);
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let plain = painted_label(&shapes, "Open");
    let kept = painted_label(&shapes, "Brush");
    assert!(!plain.italics, "the unflagged row stands upright while the panel is disabled: {plain:?}");
    assert!(kept.italics, "the flagged row keeps its slant while the panel is disabled: {kept:?}");
}

/// The panel's source row, marked redo and italic at once, draws its label slanted and dimmed and keeps its source
/// cell and the accessible name the label gives the row.
#[trial]
fn a_source_row_marked_italic_keeps_its_cell_and_its_dimmed_ink() {
    let rows = vec![HistoryRow::state(1, "Open"), HistoryRow::state(2, "Brush").redo(true), HistoryRow::state(3, "Fill").redo(true).italic(true)];
    let mut fixture = Fixture::new(move |ui| {
        let _ = HistoryPanel::new(&rows, Some(HistoryKey::State(1))).source(Some(HistoryKey::State(3))).show(ui);
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let both = painted_label(&shapes, "Fill");
    let redo = painted_label(&shapes, "Brush");
    assert!(both.italics, "the source row draws its label in italics: {both:?}");
    assert_eq!(both.color, redo.color, "and stays dimmed as a redone row is: {both:?} {redo:?}");
    assert!(fixture.by_label("Set Fill as History Brush Source").is_some(), "its source cell is still there");
    assert!(fixture.by_label("Fill").is_some(), "and the row is still named by its label");
}
