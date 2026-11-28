//! `PageList`: with a document, a rename left open and the scroll position do not carry over to another document.

use craft_behavior::Selection;
use craft_pages::ui::{PageList, PageListEvent, PageListRow};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The room of the list: wide enough for the names, tall enough for a few rows and the footer.
const PANEL: Vec2 = Vec2::new(320.0, 300.0);

type Seen = Rc<RefCell<Vec<PageListEvent>>>;

/// The list over the rows of the document the trial sets, with the events it reports.
struct Rig {
    fixture: Fixture,
    seen: Seen,
    document: Rc<Cell<u32>>,
}

/// The list over `rows(document)`, starting in document 0.
fn rig(rows: fn(u32) -> Vec<PageListRow>) -> Rig {
    let seen: Seen = Rc::default();
    let document = Rc::new(Cell::new(0_u32));
    let picked = Rc::new(RefCell::new(Selection::single(0_usize)));
    let (sink, shown, choice) = (Rc::clone(&seen), Rc::clone(&document), Rc::clone(&picked));
    let mut fixture = Fixture::new(move |ui| {
        let rows = rows(shown.get());
        let mut picked = choice.borrow_mut();
        let out = PageList::new("Artboards", &rows, &mut picked, 0).document(shown.get()).show(ui);
        sink.borrow_mut().extend(out.events);
    });
    fixture.room(PANEL);
    fixture.settle();
    Rig { fixture, seen, document }
}

/// Two rows, named apart in each document.
fn two_rows(document: u32) -> Vec<PageListRow> {
    let names = if document == 0 { ["Cover", "Back"] } else { ["Front", "Inside"] };
    names.into_iter().map(PageListRow::new).collect()
}

/// Forty rows, more than the panel shows, named by document and number.
fn many_rows(document: u32) -> Vec<PageListRow> {
    let prefix = if document == 0 { "Sheet" } else { "Page" };
    (1..=40).map(|n| PageListRow::new(format!("{prefix} {n}"))).collect()
}

/// The renames reported since the last call, as row and name.
fn renames(seen: &Seen) -> Vec<(usize, String)> {
    seen.borrow_mut()
        .drain(..)
        .filter_map(|event| if let PageListEvent::Rename { row, text, .. } = event { Some((row, text)) } else { None })
        .collect()
}

/// Row numbers restart in every document, so the rename field of row 2 in one document is not the field of row 2 in
/// another: a switch with the field open renames nothing in the document switched to.
#[trial]
fn a_rename_left_open_does_not_carry_over_to_another_document() {
    let mut rig = rig(two_rows);
    let back = rig.fixture.by_label("Back").expect("the second row shows");
    rig.fixture.click(&back);
    rig.fixture.idle(2);
    rig.fixture.key(Key::F2, Modifiers::NONE);
    rig.fixture.idle(2);
    rig.fixture.type_text("X");
    rig.fixture.idle(2);
    assert!(renames(&rig.seen).is_empty(), "typing in the field renames nothing yet");

    rig.document.set(1);
    rig.fixture.idle(3);
    assert!(rig.fixture.by_label("Inside").is_some(), "the other document's rows show");
    rig.fixture.type_text("Y");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.idle(2);
    let other = renames(&rig.seen);
    assert!(other.is_empty(), "the field of the first document does not open on the second's row 2: {other:?}");

    rig.document.set(0);
    rig.fixture.idle(3);
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.idle(2);
    let back = renames(&rig.seen);
    assert!(back.iter().all(|(_, name)| !name.contains('Y')), "a rename back in the first document has nothing typed in the second: {back:?}");
}

/// A list scrolled down in one document shows another document's list from its own position, the top.
#[trial]
fn the_scroll_position_does_not_carry_over_to_another_document() {
    let mut rig = rig(many_rows);
    let first = rig.fixture.by_label("Sheet 1").expect("the first row shows");
    rig.fixture.click(&first);
    rig.fixture.idle(2);
    rig.fixture.key(Key::End, Modifiers::NONE);
    rig.fixture.idle(3);
    // A row is in view when its rect is inside the room the list is shown in.
    let shown = |rig: &Rig, name: &str| rig.fixture.by_label(name).is_some_and(|row| rig.fixture.shown().contains_rect(row.rect));
    assert!(shown(&rig, "Sheet 40") && !shown(&rig, "Sheet 1"), "End scrolls the first document's list to its last row");

    rig.document.set(1);
    rig.fixture.idle(3);
    assert!(shown(&rig, "Page 1"), "the second document's list shows its first row");
}
