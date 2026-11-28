//! The Paths panel's two rules a click or a key reaches: a ⌘-click on the thumbnail only loads the path, and a work
//! path's rename saves it under the proposed name even when the text is unchanged.

use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_draw::ui::paths::{PathKind, PathRow, PathsEvent, PathsPanel};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn outline() -> kurbo::BezPath {
    let mut path = kurbo::BezPath::new();
    path.move_to((10.0, 10.0));
    path.line_to((90.0, 20.0));
    path.line_to((50.0, 80.0));
    path.close_path();
    path
}

/// A panel over a saved row (1, "Outline") and a work row (2, "Work Path"); a `Select` moves the selected row, as the
/// app's does. Returns the fixture and the events reported since the last take.
fn panel() -> (Fixture, Rc<RefCell<Vec<PathsEvent<u32>>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let selected = Rc::new(Cell::new(None::<u32>));
    let (sink, picked) = (Rc::clone(&events), Rc::clone(&selected));
    let mut fixture = Fixture::new(move |ui| {
        let rows = vec![
            PathRow { selected: picked.get() == Some(1), ..PathRow::new(1, PathKind::Saved, "Outline", outline()) },
            PathRow { selected: picked.get() == Some(2), ..PathRow::new(2, PathKind::Work, "Work Path", outline()) },
        ];
        let out = PathsPanel::new(&rows, kurbo::Size::new(100.0, 100.0)).work_name("Path 1").show(ui);
        for event in &out.events {
            if let PathsEvent::Select(key) = event {
                picked.set(*key);
            }
        }
        sink.borrow_mut().extend(out.events);
    });
    fixture.settle();
    (fixture, events)
}

#[trial]
fn a_thumbnail_command_click_loads_and_a_work_rename_saves_under_the_proposed_name() {
    let (mut fixture, events) = panel();
    let row = fixture.by_label("Outline").expect("the saved row shows").rect;

    // The thumbnail leads the row: a ⌘-click there loads and does not select.
    events.borrow_mut().clear();
    fixture.hold(Modifiers::COMMAND);
    fixture.click_at(egui::pos2(row.left() + 20.0, row.center().y));
    fixture.hold(Modifiers::NONE);
    assert_eq!(*events.borrow(), vec![PathsEvent::load_selection(1, SelectionOperation::New)], "a thumbnail ⌘-click only loads");

    // On the name the same click is a plain select.
    events.borrow_mut().clear();
    fixture.hold(Modifiers::COMMAND);
    fixture.click(&fixture.by_label("Outline").expect("the saved row shows"));
    fixture.hold(Modifiers::NONE);
    assert_eq!(*events.borrow(), vec![PathsEvent::Select(Some(1))], "a name ⌘-click only selects");

    // A click on the empty area below the rows clears the selection.
    events.borrow_mut().clear();
    let work = fixture.by_label("Work Path").expect("the work row shows").rect;
    fixture.click_at(egui::pos2(work.center().x, work.bottom() + 30.0));
    assert_eq!(*events.borrow(), vec![PathsEvent::Select(None)], "a click below the rows selects nothing");

    // A saved row renamed to its own name reports nothing.
    events.borrow_mut().clear();
    fixture.key(Key::F2, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    assert!(!events.borrow().iter().any(|event| matches!(event, PathsEvent::Rename { .. })), "an unchanged saved name reports nothing");

    // The work row's field starts with the proposed name, and committing it unchanged saves under that name.
    fixture.click(&fixture.by_label("Work Path").expect("the work row shows"));
    events.borrow_mut().clear();
    fixture.key(Key::F2, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    assert!(events.borrow().contains(&PathsEvent::rename(2, "Path 1")), "the work path is saved under the proposed name: {:?}", events.borrow());
}
