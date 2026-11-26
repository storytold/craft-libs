//! The graph's annotation layers: where they stack among the nodes, the one selection they share with the nodes, and
//! the gestures a backdrop reports.

use craft_behavior::{CancelReason, CanvasTransform, EditEvent, GestureId, Selection, keyed};
use craft_ui::contract::{Bind, Props as _, Role, ShowCollection as _};
use craft_ui::graph::{AnnotationEvent, AnnotationKey, Backdrop, Marker, Node, NodeGraph, NodeId, NodeKind, Note};
use craft_ui::style::DataColor;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A node with no ports.
struct Plain;

impl NodeKind<()> for Plain {
    fn title(&self) -> &str {
        "Plain"
    }

    fn port_count(&self) -> usize {
        0
    }

    fn port_name(&self, _: usize) -> String {
        String::new()
    }

    fn port_offset(&self, _: usize, size: Vec2) -> Vec2 {
        size / 2.0
    }
}

const PLAIN: Plain = Plain;

/// Node 0 sits inside the backdrop; node 1 lies under the note and the marker.
const NODES: [Rect; 2] = [
    Rect { min: Pos2::new(40.0, 40.0), max: Pos2::new(180.0, 110.0) },
    Rect { min: Pos2::new(240.0, 40.0), max: Pos2::new(380.0, 110.0) },
];
/// Around node 0, with room below it that no node covers.
const FRAME: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(200.0, 160.0) };
const NOTES: [Note<'static>; 1] = [Note::new(1, Rect { min: Pos2::new(240.0, 50.0), max: Pos2::new(370.0, 100.0) }, "Check the blend")];
/// Its pin points at the note's bottom-right, so the two overlap there and the note's middle stays the note's.
const MARKERS: [Marker<'static>; 1] = [Marker::new(2, Pos2::new(360.0, 100.0), "Review")];

/// What the graph's binds hold and what it reported, across frames.
#[derive(Default)]
struct Seen {
    annotation: Option<AnnotationKey>,
    nodes: Selection<NodeId>,
    events: Vec<AnnotationEvent>,
}

/// A graph at `zoom` with the backdrop, the note and the marker, both selections bound to `seen`. Every frame runs
/// its pass twice, so a rule that reports again in a rerun pass shows in the counts.
fn graph(seen: &Rc<RefCell<Seen>>, zoom: f32) -> Fixture {
    let held = Rc::clone(seen);
    let mut fixture = Fixture::new(move |ui| {
        let kinds: [&dyn NodeKind<()>; 2] = [&PLAIN, &PLAIN];
        let ids = [NodeId(0), NodeId(1)];
        let source = keyed(&ids, |id: &NodeId| *id);
        let backdrops = [Backdrop::new(0, FRAME, DataColor::new(Color32::from_rgba_premultiplied(40, 80, 48, 96))).title("Group")];
        let mut seen = held.borrow_mut();
        let Seen { annotation, nodes, events } = &mut *seen;
        let response = NodeGraph::new(&NODES, &kinds, &[])
            .label("Graph")
            .zoom(CanvasTransform::new(zoom, Vec2::ZERO))
            .backdrops(&backdrops)
            .notes(&NOTES)
            .markers(&MARKERS)
            .selection(Bind::bound(nodes))
            .annotation_selection(Bind::bound(annotation))
            .show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
        events.extend(response.annotations);
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial reruns every pass");
        }
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    fixture
}

/// The rect of the access node named `label`.
fn rect_of(fixture: &Fixture, label: &str) -> Rect {
    fixture.by_label(label).unwrap_or_else(|| panic!("the graph shows {label}")).rect
}

/// A point inside the backdrop that no node covers: near its bottom-left corner, away from the grip.
fn backdrop_only(fixture: &Fixture) -> Pos2 {
    let frame = rect_of(fixture, "Group");
    Pos2::new(frame.left() + 8.0, frame.bottom() - 8.0)
}

#[trial]
fn annotations_stack_under_and_over_nodes_and_share_one_selection() {
    let seen = Rc::new(RefCell::new(Seen { nodes: Selection::single(NodeId(1)), ..Seen::default() }));
    let mut fixture = graph(&seen, 1.0);

    fixture.click_at(rect_of(&fixture, "Check the blend").center());
    assert_eq!(seen.borrow().annotation, Some(AnnotationKey::Note(1)), "a press on a note over a node selects the note");
    assert!(seen.borrow().nodes.items.is_empty(), "selecting an annotation empties the node selection");

    let marker = rect_of(&fixture, "Review");
    let overlap = Pos2::new(marker.left() + 3.0, marker.bottom() - 3.0);
    assert!(rect_of(&fixture, "Check the blend").contains(overlap), "the trial presses where the marker covers the note");
    fixture.click_at(overlap);
    assert_eq!(seen.borrow().annotation, Some(AnnotationKey::Marker(2)), "a marker takes the press where it covers a note");

    fixture.click_at(backdrop_only(&fixture));
    assert_eq!(seen.borrow().annotation, Some(AnnotationKey::Backdrop(0)), "a press on the backdrop off the nodes selects it");

    seen.borrow_mut().events.clear();
    let node = fixture.by_role(Role::GRAPHICS_OBJECT).first().copied().expect("the graph shows its nodes");
    assert!(rect_of(&fixture, "Group").contains(node.center()), "the trial presses the node inside the backdrop");
    fixture.click(&node);
    let seen = seen.borrow();
    assert_eq!(seen.nodes.items, [NodeId(0)], "the node inside the backdrop takes the press");
    assert_eq!(seen.annotation, None, "pressing a node clears the annotation selection");
    let cleared = seen.events.iter().filter(|event| **event == AnnotationEvent::Selected(None)).count();
    assert_eq!(cleared, 1, "the cleared selection is reported once, though every pass ran twice");
}

/// The gesture ids of the `Move` begins of backdrop 0 in `events`.
fn move_begins(events: &[AnnotationEvent]) -> Vec<GestureId> {
    events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::Move { key: AnnotationKey::Backdrop(0), edit: EditEvent::Begin { id, .. }, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

/// The offsets the `Move` commits of backdrop 0 in `events` carry.
fn move_commits(events: &[AnnotationEvent]) -> Vec<Vec2> {
    events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::Move { key: AnnotationKey::Backdrop(0), edit: EditEvent::Commit { after, .. }, .. } => Some(*after),
            _ => None,
        })
        .collect()
}

#[trial]
fn a_backdrop_drag_and_grip_drag_report_one_gesture_each() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut fixture = graph(&seen, 2.0);
    let from = backdrop_only(&fixture);
    let by = Vec2::new(40.0, 0.0);

    fixture.drag_at(from, by);
    let events = std::mem::take(&mut seen.borrow_mut().events);
    assert_eq!(move_begins(&events).len(), 1, "a body drag begins one move");
    let commits = move_commits(&events);
    assert_eq!(commits.len(), 1, "a body drag commits once, though every pass ran twice");
    assert!((commits[0] - by / 2.0).length() < 0.01, "at zoom 2 the offset is half the drag, in document units: {:?}", commits[0]);

    fixture.drag_hold_at(from, by);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release_at(from + by);
    let events = std::mem::take(&mut seen.borrow_mut().events);
    let cancelled: Vec<GestureId> = events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::Move { edit: EditEvent::Cancel { id, reason: CancelReason::Escape, .. }, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled.len(), 1, "Escape mid-drag cancels the move");
    assert!(move_commits(&events).is_empty(), "a cancelled move commits nothing at the release");

    fixture.drag_at(from, by);
    let events = std::mem::take(&mut seen.borrow_mut().events);
    let again = move_begins(&events);
    assert_eq!(again.len(), 1, "the same drag after a cancel begins a move");
    assert_ne!(again[0], cancelled[0], "nothing is carried between passes: the new press is a new gesture");

    let grip = rect_of(&fixture, "Group").max - Vec2::splat(2.0);
    fixture.drag_at(grip, Vec2::new(-360.0, -280.0));
    let events = std::mem::take(&mut seen.borrow_mut().events);
    let resized: Vec<Rect> = events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::Resize { key: AnnotationKey::Backdrop(0), edit: EditEvent::Commit { after, .. }, .. } => Some(*after),
            _ => None,
        })
        .collect();
    assert_eq!(resized.len(), 1, "a grip drag commits one resize");
    assert!(move_commits(&events).is_empty(), "the grip takes the press from the body under it");
    let rect = resized[0];
    assert_eq!(rect.min, FRAME.min, "a resize keeps the backdrop's top left");
    assert!(rect.width() > 0.0 && rect.height() > 0.0, "a drag past the top left stops at the least size: {rect:?}");
    assert!(rect.width() < FRAME.width() && rect.height() < FRAME.height(), "the drag shrank the backdrop: {rect:?}");

    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::secondary(from, true)]);
    fixture.frame(vec![pointer::secondary(from, false)]);
    fixture.idle(1);
    let events = std::mem::take(&mut seen.borrow_mut().events);
    let menus: Vec<Pos2> = events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::ContextMenu { key: AnnotationKey::Backdrop(0), at, .. } => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(menus.len(), 1, "a secondary click asks for the backdrop's menu once");
    assert!(menus[0].distance(from) < 0.5, "the menu opens at the click in the parent's layer space, not through the zoom");
}
