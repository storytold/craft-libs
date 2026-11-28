//! The Slice tools: the pixel rules of a new slice, the origin rules of the overlay's moves and resizes, and the keys
//! of the Select tool.

use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::ui::slice::{
    DivideBy, DivideSlices, DivideSlicesDialog, SliceCommand, SliceEvent, SliceItem, SliceMode, SliceOptions, SliceOptionsDialog, SliceOrigin,
    SliceOverlay, SliceRefusal, SliceTool, SliceToolSettings,
};
use craft_ui::contract::{CommandSet, Role, Show as _};
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, HoldsOk};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, ToolFixture, access, trial};
use egui::{Key, Modifiers, Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A canvas at 100 % whose document origin is the room's top left corner, so a document point is the room's corner
/// plus the point.
fn view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

/// The screen position of the document point `(x, y)` on the fixture's canvas.
fn at(fixture: &mut ToolFixture<SliceTool>, x: f64, y: f64) -> Pos2 {
    fixture.canvas().rect.min + Vec2::new(x as f32, y as f32)
}

/// The rectangle a slice event list ends a new slice on, if it commits one.
fn drawn(events: &[SliceEvent]) -> Option<Rect> {
    events.iter().find_map(|event| match event {
        SliceEvent::Draw(EditEvent::Commit { after, .. }) => Some(*after),
        _ => None,
    })
}

/// A new slice is rounded by its style: Fixed Ratio keeps the ratio on whole pixels and a click makes none; Fixed Size
/// is exactly its pixels on the press's pixel, drawn up and left too, and a click makes it; a Normal drag no wider
/// and no taller than two pixels is a click that selects.
/// A style case: its name, constraint, drag start, drag delta and the slice it draws.
type StyleCase = (&'static str, Constraint, (f64, f64), (f32, f32), Option<Rect>);

#[trial]
fn a_slice_is_drawn_by_its_style() {
    // Starts have a fractional part so that float noise in the pointer's position cannot move a pixel edge.
    let cases: [StyleCase; 6] = [
        ("Fixed Ratio 2:1 from a 20.8 px drag", Constraint::Ratio(2.0, 1.0), (50.5, 50.5), (20.8, 0.0), Some(Rect::new(50.0, 50.0, 71.0, 61.0))),
        ("Fixed Ratio click", Constraint::Ratio(2.0, 1.0), (50.5, 50.5), (0.0, 0.0), None),
        ("Fixed Size drawn up and left", Constraint::Size(20.0, 10.0), (60.4, 60.6), (-5.0, -5.0), Some(Rect::new(41.0, 51.0, 61.0, 61.0))),
        ("Fixed Size by a click", Constraint::Size(20.0, 10.0), (50.4, 50.6), (0.0, 0.0), Some(Rect::new(50.0, 50.0, 70.0, 60.0))),
        ("Normal drag of two pixels", Constraint::Free, (50.1, 50.1), (1.8, 1.8), None),
        ("Normal drag of three pixels", Constraint::Free, (50.1, 50.1), (3.0, 3.0), Some(Rect::new(50.0, 50.0, 54.0, 54.0))),
    ];
    for (name, style, (x, y), (dx, dy), expected) in cases {
        let mut tool = SliceTool::new();
        tool.mode(SliceMode::Slice).settings(SliceToolSettings::new(style));
        let mut fixture = ToolFixture::with_view("Slices", tool, view, |_, _| {});
        let from = at(&mut fixture, x, y);
        if (dx, dy) == (0.0, 0.0) {
            fixture.press_at(from).release_at(from);
        } else {
            fixture.drag_from(from, Vec2::new(dx, dy));
        }
        let events = fixture.tool_mut().take_events();
        assert_eq!(drawn(&events), expected, "{name}: {events:?}");
        if expected.is_none() {
            assert!(events.iter().all(|event| !matches!(event, SliceEvent::Draw(_))), "{name}: a click draws nothing: {events:?}");
            assert!(events.iter().any(|event| matches!(event, SliceEvent::Select { .. })), "{name}: a click selects: {events:?}");
        }
    }
}

/// The overlay in a canvas under a Select tool, with the events the overlay reported.
struct Board {
    fixture: ToolFixture<SliceTool>,
    events: Rc<RefCell<Vec<SliceEvent>>>,
}

/// One user slice at (40, 40)-(140, 120), an auto slice beside it and a layer slice below it.
fn items() -> Vec<SliceItem> {
    vec![
        SliceItem::new(1, Some(1), Rect::new(40.0, 40.0, 140.0, 120.0), SliceOrigin::User),
        SliceItem::new(2, None, Rect::new(140.0, 40.0, 240.0, 120.0), SliceOrigin::Auto),
        SliceItem::new(3, Some(3), Rect::new(40.0, 130.0, 140.0, 200.0), SliceOrigin::Layer),
    ]
}

fn board(selected: Option<usize>) -> Board {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (slices, seen) = (items(), Rc::clone(&events));
    let mut tool = SliceTool::new();
    tool.mode(SliceMode::Select).items(&slices).selected(selected);
    let fixture = ToolFixture::with_view("Slices", tool, view, move |ui, body| {
        let overlay = SliceOverlay::new("Slices", &slices).pick(true);
        let overlay = match selected {
            Some(number) => overlay.selected_slice(number),
            None => overlay,
        };
        seen.borrow_mut().extend(overlay.show_at(ui, body.rect).events);
    });
    Board { fixture, events }
}

/// What a drag on the overlay ends in.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// The slice moved to this rectangle.
    Moved(Rect),
    /// The slice was resized to this rectangle.
    Resized(Rect),
    /// Only this slice was selected.
    Selected(usize),
}

/// A drag of a user slice moves it by whole pixels (a `Move` from `Begin` to `Commit`); a corner and an edge handle
/// move only their edges and never empty the slice; a press on an auto or layer slice only selects.
/// A move case: its name, the user slice grabbed, drag start, drag delta and the outcome.
type MoveCase = (&'static str, Option<usize>, (f64, f64), (f32, f32), Outcome);

#[trial]
fn user_slices_move_and_resize_and_other_slices_only_select() {
    let cases: [MoveCase; 6] = [
        ("move by whole pixels", Some(1), (90.0, 80.0), (13.3, 7.6), Outcome::Moved(Rect::new(53.0, 48.0, 153.0, 128.0))),
        ("corner handle", Some(1), (140.0, 120.0), (10.0, 6.0), Outcome::Resized(Rect::new(40.0, 40.0, 150.0, 126.0))),
        ("edge handle", Some(1), (140.0, 80.0), (-30.0, 25.0), Outcome::Resized(Rect::new(40.0, 40.0, 110.0, 120.0))),
        ("corner dragged onto the opposite corner", Some(1), (140.0, 120.0), (-100.0, -80.0), Outcome::Resized(Rect::new(40.0, 40.0, 41.0, 41.0))),
        ("auto slice", None, (190.0, 80.0), (20.0, 10.0), Outcome::Selected(2)),
        ("layer slice", None, (90.0, 165.0), (20.0, 10.0), Outcome::Selected(3)),
    ];
    for (name, selected, (x, y), (dx, dy), expected) in cases {
        let mut board = board(selected);
        board.fixture.fixture().settle();
        let from = at(&mut board.fixture, x, y);
        board.fixture.drag_from(from, Vec2::new(dx, dy));
        let events = board.events.borrow().clone();
        let edits: Vec<(bool, Rect)> = events
            .iter()
            .filter_map(|event| match event {
                SliceEvent::Move { event: EditEvent::Commit { after, .. }, .. } => Some((false, *after)),
                SliceEvent::Resize { event: EditEvent::Commit { after, .. }, .. } => Some((true, *after)),
                _ => None,
            })
            .collect();
        let selects: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                SliceEvent::Select { number: Some(number), .. } => Some(*number),
                _ => None,
            })
            .collect();
        match expected {
            Outcome::Moved(rect) => {
                assert_eq!(edits, [(false, rect)], "{name}: {events:?}");
                let begun = events.iter().any(|event| matches!(event, SliceEvent::Move { event: EditEvent::Begin { .. }, .. }));
                assert!(begun, "{name}: a move begins before it commits: {events:?}");
            }
            Outcome::Resized(rect) => assert_eq!(edits, [(true, rect)], "{name}: {events:?}"),
            Outcome::Selected(number) => {
                assert!(edits.is_empty(), "{name}: a press on this slice moves nothing: {events:?}");
                assert!(selects.contains(&number), "{name}: it selects slice {number}: {events:?}");
            }
        }
    }
}

/// The app's own commands on the deleting keys: Delete Layer on Delete, Clear on Backspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "sliceTrial")]
#[non_exhaustive]
enum AppDelete {
    #[command(label = "Delete Layer", default = "Delete")]
    DeleteLayer,
    #[command(label = "Clear", default = "Backspace")]
    Clear,
}

/// In Select mode with the canvas focused, Delete and Backspace on a stored slice report `Command(Delete)`, on an auto
/// slice nothing, and with Lock Slices `Refused(Locked)`, and in each case the app's Delete Layer and Clear never run;
/// in Slice mode the tool claims no delete, so the key is the app's.
#[trial]
fn delete_takes_the_key_for_the_selected_slice() {
    let cases: [(&str, SliceMode, usize, bool, Vec<SliceEvent>); 5] = [
        ("a user slice", SliceMode::Select, 1, false, vec![SliceEvent::Command(SliceCommand::Delete)]),
        ("a layer slice", SliceMode::Select, 3, false, vec![SliceEvent::Command(SliceCommand::Delete)]),
        ("an auto slice", SliceMode::Select, 2, false, vec![]),
        ("a locked slice", SliceMode::Select, 1, true, vec![SliceEvent::Refused(SliceRefusal::Locked)]),
        ("Slice mode leaves the key to the app", SliceMode::Slice, 1, false, vec![]),
    ];
    for (name, mode, selected, locked, expected) in cases {
        for (key, app) in [(Key::Delete, AppDelete::DeleteLayer), (Key::Backspace, AppDelete::Clear)] {
            let mut tool = SliceTool::new();
            tool.mode(mode).items(&items()).selected(Some(selected)).locked(locked);
            let mut fixture = ToolFixture::new("Slices", tool);
            fixture.fixture().settle();
            // The canvas is the first Tab stop; the tool's claim is made from its state at the end of the last pass.
            fixture.fixture().key(Key::Tab, Modifiers::NONE).settle();
            assert_eq!(fixture.fixture().focused(), Some(fixture.canvas_id()), "{name}: Tab focuses the canvas");
            fixture.tool_mut().take_events();
            take_output(fixture.fixture().ctx());
            fixture.fixture().key(key, Modifiers::NONE).settle();
            assert_eq!(fixture.tool_mut().take_events(), expected, "{name}, {key:?}");
            let ran = take_output(fixture.fixture().ctx()).fired.iter().filter(|fired| fired.command == CommandRef::from(app)).count();
            let reaches_the_app = usize::from(mode == SliceMode::Slice);
            assert_eq!(ran, reaches_the_app, "{name}, {key:?}: the app's {app:?} runs only when the tool leaves the key");
        }
    }
}

/// The names a screen reader reads for the nodes of `role` in the shown Divide Slice dialog, in tree order.
fn divide_names(divide: DivideSlices, role: Role) -> Vec<String> {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 700.0));
    host.borrow_mut().open(DivideSlicesDialog::new(divide));
    fixture.idle(10);
    let tree = fixture.tree();
    fixture.by_role(role).iter().map(|target| access::name(tree, target.node).unwrap_or_default().to_owned()).collect()
}

/// Divide Slice's two axes show the same Divide By choice and value rows; each axis's radio group and value field
/// name their axis by its caption after their visible label, so a screen reader tells the rows from the columns.
#[trial]
fn divide_slice_names_each_axis_control_by_its_axis() {
    let by_count = DivideSlices::new(Some(DivideBy::Count(2)), Some(DivideBy::Count(3)));
    let by_size = DivideSlices::new(Some(DivideBy::Size(72.0)), Some(DivideBy::Size(36.0)));
    let cases = [
        ("the Divide By groups", by_count, Role::RADIO_GROUP, "Divide By"),
        ("the count fields", by_count, Role::SPIN_BUTTON, "slices, evenly spaced"),
        ("the size fields", by_size, Role::SPIN_BUTTON, "per slice"),
    ];
    for (what, divide, role, label) in cases {
        let names = divide_names(divide, role);
        let [rows, columns] = names.as_slice() else { panic!("{what}: one for each axis, got {names:?}") };
        assert_ne!(rows, columns, "{what} have different names");
        for (name, axis) in [(rows, "Divide Horizontally Into"), (columns, "Divide Vertically Into")] {
            assert!(name.starts_with(label), "{what}: {name:?} starts with the visible label {label:?}");
            assert!(name.contains(axis), "{what}: {name:?} names its axis {axis:?}");
        }
    }
}

const STALE_SELECTION: &str = "Select the opening slices and try again.";

#[trial]
fn slice_options_keeps_its_draft_after_the_app_refuses_ok() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 700.0));
    let id = host.borrow_mut().open(SliceOptionsDialog::new(SliceOptions::default()).held_ok());
    fixture.idle(10);
    host.borrow_mut().set_json_field(fixture.ctx(), id, "name", serde_json::json!("Draft name")).unwrap();
    fixture.idle(4);

    fixture.click(&fixture.by_label("OK").unwrap()).idle(4);
    assert_eq!(host.borrow().held_state(id), Some(HeldOk::Waiting));
    let draft = host.borrow_mut().get_mut::<SliceOptionsDialog>().unwrap().take_ok().unwrap();
    assert_eq!(draft.name, "Draft name");
    assert!(host.borrow_mut().get_mut::<SliceOptionsDialog>().unwrap().take_ok().is_none());
    assert!(host.borrow_mut().take::<SliceOptionsDialog>().is_none());
    host.borrow_mut().get_mut::<SliceOptionsDialog>().unwrap().answer_ok(Err(STALE_SELECTION.into()));
    fixture.idle(4);
    assert_eq!(host.borrow().get::<SliceOptionsDialog>().unwrap().options(), &draft);
    assert!(fixture.by_label(STALE_SELECTION).is_some(), "the refusal shows in the dialog");

    fixture.click(&fixture.by_label("OK").unwrap()).idle(4);
    let mut open = host.borrow_mut();
    let dialog = open.get_mut::<SliceOptionsDialog>().unwrap();
    assert_eq!(dialog.take_ok(), Some(draft.clone()));
    dialog.answer_ok(Ok(()));
    drop(open);
    fixture.idle(4);
    assert_eq!(host.borrow_mut().take::<SliceOptionsDialog>().and_then(DialogResult::ok), Some(draft));
}

#[trial]
fn divide_slices_can_retry_a_held_answer_and_cancel_it_while_waiting() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 700.0));
    let id = host.borrow_mut().open(DivideSlicesDialog::new(DivideSlices::new(None, None)).held_ok());
    fixture.idle(10);
    assert!(!host.borrow_mut().confirm::<DivideSlicesDialog>(fixture.ctx()));
    assert!(host.borrow().held_state(id).is_none(), "two disabled axes produce no answer");
    host.borrow_mut().set_json_field(fixture.ctx(), id, "rows", serde_json::json!(3)).unwrap();
    fixture.idle(4);

    let ok = fixture.by_label("OK").unwrap();
    assert!(!fixture.tree().is_disabled_key(ok.node), "setting one axis enables OK");
    fixture.click(&ok).idle(4);
    let draft = DivideSlices::new(Some(DivideBy::Count(3)), None);
    let mut open = host.borrow_mut();
    let dialog = open.get_mut::<DivideSlicesDialog>().unwrap();
    assert_eq!(dialog.take_ok(), Some(draft));
    dialog.answer_ok(Err(STALE_SELECTION.into()));
    drop(open);
    fixture.idle(4);
    assert!(fixture.by_label(STALE_SELECTION).is_some());
    assert!(!host.borrow_mut().confirm::<DivideSlicesDialog>(fixture.ctx()), "programmatic OK files before closing");
    let mut open = host.borrow_mut();
    let dialog = open.get_mut::<DivideSlicesDialog>().unwrap();
    assert_eq!(dialog.take_ok(), Some(draft));
    dialog.answer_ok(Ok(()));
    assert!(open.confirm::<DivideSlicesDialog>(fixture.ctx()));
    assert_eq!(open.take::<DivideSlicesDialog>().and_then(DialogResult::ok), Some(draft));
    open.open(DivideSlicesDialog::new(draft).held_ok());
    assert!(!open.confirm::<DivideSlicesDialog>(fixture.ctx()));
    drop(open);

    fixture.idle(4);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(4);
    assert!(matches!(host.borrow_mut().take::<DivideSlicesDialog>(), Some(DialogResult::Cancel)));
    host.borrow_mut().open(DivideSlicesDialog::new(draft));
    assert!(host.borrow_mut().confirm::<DivideSlicesDialog>(fixture.ctx()), "ordinary callers close immediately");
    assert_eq!(host.borrow_mut().take::<DivideSlicesDialog>().and_then(DialogResult::ok), Some(draft));
}
