//! The graph's annotation layers: where they stack among the nodes, the one selection they share with the nodes, and
//! the gestures a backdrop reports.

use craft_behavior::{CancelReason, CanvasTransform, EditEvent, GestureId, Mixed, Selection, keyed};
use craft_ui::contract::{Bind, Props as _, Role, ShowCollection as _};
use craft_ui::graph::{AnnotationEvent, AnnotationKey, AnnotationRect, Backdrop, Marker, Node, NodeGraph, NodeId, NodeKind, Note};
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
const NODES: [Rect; 2] =
    [Rect { min: Pos2::new(40.0, 40.0), max: Pos2::new(180.0, 110.0) }, Rect { min: Pos2::new(240.0, 40.0), max: Pos2::new(380.0, 110.0) }];
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
    /// The backdrop's rect when the host previews each resize proposal on it, as an app does; `None` keeps `FRAME`.
    frame: Option<Rect>,
    /// The annotation part rects of the last pass.
    rects: Vec<AnnotationRect>,
}

/// A graph at `zoom` with the backdrop, the note and the marker, both selections bound to `seen`. Every frame runs
/// its pass twice, so a rule that reports again in a rerun pass shows in the counts.
fn graph(seen: &Rc<RefCell<Seen>>, zoom: f32) -> Fixture {
    let held = Rc::clone(seen);
    let mut fixture = Fixture::new(move |ui| {
        let kinds: [&dyn NodeKind<()>; 2] = [&PLAIN, &PLAIN];
        let ids = [NodeId::new(0), NodeId::new(1)];
        let source = keyed(&ids, |id: &NodeId| *id);
        let mut seen = held.borrow_mut();
        let Seen { annotation, nodes, events, frame, rects } = &mut *seen;
        let fill = DataColor::new(Color32::from_rgba_premultiplied(40, 80, 48, 96));
        let backdrops = [Backdrop::new(0, frame.unwrap_or(FRAME), fill).title("Group")];
        let response = NodeGraph::new(&NODES, &kinds, &[])
            .label("Graph")
            .zoom(CanvasTransform::new(zoom, Vec2::ZERO))
            .backdrops(&backdrops)
            .notes(&NOTES)
            .markers(&MARKERS)
            .selection(Bind::bound(nodes))
            .annotation_selection(Bind::bound(annotation))
            .show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
        if let Some(frame) = frame {
            for event in &response.annotations {
                if let AnnotationEvent::Resize { edit: EditEvent::Update { value, .. } | EditEvent::Commit { after: value, .. }, .. } = event {
                    *frame = *value;
                }
            }
        }
        events.extend(response.annotations);
        *rects = response.annotation_rects;
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
    let seen = Rc::new(RefCell::new(Seen { nodes: Selection::single(NodeId::new(1)), ..Seen::default() }));
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
    assert_eq!(seen.nodes.items, [NodeId::new(0)], "the node inside the backdrop takes the press");
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

#[trial]
fn the_response_reports_where_each_annotation_part_shows_at_a_zoom() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let fixture = graph(&seen, 2.0);
    let seen = seen.borrow();
    let shown = [(AnnotationKey::Backdrop(0), "Group"), (AnnotationKey::Note(1), "Check the blend"), (AnnotationKey::Marker(2), "Review")];
    assert_eq!(seen.rects.len(), shown.len(), "the response reports one rect per annotation shown");
    for (key, label) in shown {
        let rect = seen.rects.iter().find(|rect| rect.key == key).unwrap_or_else(|| panic!("the response reports {key:?}"));
        let node = rect_of(&fixture, label);
        assert!(
            (rect.screen.min - node.min).length() < 0.5 && (rect.screen.max - node.max).length() < 0.5,
            "{label}: {rect:?} is where it shows, {node:?}"
        );
        let half = rect.screen.size() / 2.0;
        assert!((rect.graph.size() - half).length() < 0.5, "{label}: at zoom 2 the graph rect is half the shown one: {rect:?}");
    }
    let note = seen.rects.iter().find(|rect| rect.key == AnnotationKey::Note(1)).map(|rect| rect.graph);
    assert_eq!(note.map(|rect| rect.min), Some(NOTES[0].rect.min), "a note's graph rect starts where the app placed it");
}

#[trial]
fn a_previewed_resize_reports_the_press_rect_and_the_press_rect_plus_the_drag() {
    let seen = Rc::new(RefCell::new(Seen { frame: Some(FRAME), ..Seen::default() }));
    let mut fixture = graph(&seen, 1.0);
    let by = Vec2::new(60.0, 40.0);

    fixture.drag_at(rect_of(&fixture, "Group").max - Vec2::splat(2.0), by);
    let seen = seen.borrow();
    let commits: Vec<(&Mixed<Rect>, Rect)> = seen
        .events
        .iter()
        .filter_map(|event| match event {
            AnnotationEvent::Resize { edit: EditEvent::Commit { before, after, .. }, .. } => Some((before, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(commits.len(), 1, "a grip drag commits one resize");
    let (before, after) = commits[0];
    assert_eq!(*before, Mixed::Same(FRAME), "the commit's before is the rect at the press, not the host's preview");
    assert_eq!(after.min, FRAME.min, "a resize keeps the backdrop's top left");
    let grown = after.size() - FRAME.size();
    assert!((grown - by).length() < 0.5, "the backdrop grew by the drag once, though the host previewed each step: {grown:?}");
}

/// A node with an input on its left edge and an output on its right, at mid height.
struct Ported(&'static str, [&'static str; 2]);

impl NodeKind<()> for Ported {
    fn title(&self) -> &str {
        self.0
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        self.1[port].to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }

    fn port_direction(&self, port: usize) -> craft_ui::graph::PortDirection {
        if port == 0 { craft_ui::graph::PortDirection::Input } else { craft_ui::graph::PortDirection::Output }
    }
}

/// q133: the wires lie in the body over the backdrops, through the zoom with the nodes: a press on a wire where it
/// crosses a backdrop takes the wire, not the backdrop.
#[trial]
fn a_press_on_a_wire_inside_a_backdrop_takes_the_wire_at_a_zoom() {
    use craft_ui::graph::{PortRef, Wire};
    let (wired, annotation) = (Rc::new(RefCell::new(Selection::<u64>::default())), Rc::new(RefCell::new(None::<AnnotationKey>)));
    let (wired_in, annotation_in) = (Rc::clone(&wired), Rc::clone(&annotation));
    let mut fixture = Fixture::new(move |ui| {
        let (a, b) = (Ported("A", ["A in", "A out"]), Ported("B", ["B in", "B out"]));
        let kinds: [&dyn NodeKind<()>; 2] = [&a, &b];
        let ids = [NodeId::new(0), NodeId::new(1)];
        let source = keyed(&ids, |id: &NodeId| *id);
        let wires = [Wire::new(9, PortRef::new(NodeId::new(0), 1), PortRef::new(NodeId::new(1), 0))];
        // Around both nodes, so the wire between them crosses it.
        let around = Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(400.0, 160.0));
        let backdrops = [Backdrop::new(0, around, DataColor::new(Color32::from_rgba_premultiplied(40, 80, 48, 96))).title("Group")];
        let (mut wired, mut annotation) = (wired_in.borrow_mut(), annotation_in.borrow_mut());
        let _ = NodeGraph::new(&NODES, &kinds, &wires)
            .label("Graph")
            .zoom(CanvasTransform::new(2.0, Vec2::ZERO))
            .backdrops(&backdrops)
            .wire_selection(Bind::bound(&mut *wired))
            .annotation_selection(Bind::bound(&mut *annotation))
            .show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
    });
    fixture.room(Vec2::new(900.0, 400.0)).settle();
    // The wire runs level between ports at one height, so its middle lies halfway between them.
    let middle = rect_of(&fixture, "A out").center().lerp(rect_of(&fixture, "B in").center(), 0.5);
    assert!(rect_of(&fixture, "Group").contains(middle), "the trial presses the wire inside the backdrop");
    fixture.click_at(middle);
    assert_eq!(wired.borrow().items, [9], "the press takes the wire over the backdrop");
    assert_eq!(*annotation.borrow(), None, "and not the backdrop under it");
}
