//! `NodeGraph` and `Node` as a user drives them: wire drags from outputs and inputs (reverse wires, detaching a fed
//! input, refused drops, loose ends in document units), wire selection beside the node selection, node moves and their
//! cancel, Dots that move from their body and wire from just outside it, and the port names, tooltip and footer
//! description a node's kind gives.

use craft_behavior::{CanvasTransform, EditEvent, Selection, keyed};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, CollectionEvent, Props as _, ShowCollection as _};
use craft_ui::graph::{Node, NodeEvent, NodeFooter, NodeGraph, NodeId, NodeKind, NodeShape, PortDirection, PortLabels, PortRef, Wire, WireEvent};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A card's size, in document units.
const CARD: Vec2 = Vec2::new(100.0, 60.0);

/// A kind with an input on the left and an output on the right.
#[derive(Clone)]
struct Kind {
    title: &'static str,
    ports: [&'static str; 2],
    takes_many: bool,
    /// Whether a drag from its fed input detaches the wire (`NodeKind::port_detaches`).
    detaches: bool,
    shape: NodeShape,
    tooltip: Option<&'static str>,
    footer: Option<&'static str>,
}

impl Kind {
    fn new(title: &'static str, ports: [&'static str; 2]) -> Self {
        Kind { title, ports, takes_many: false, detaches: true, shape: NodeShape::Card, tooltip: None, footer: None }
    }
}

impl NodeKind for Kind {
    fn title(&self) -> &str {
        self.title
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        self.ports[port].to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 0 { PortDirection::Input } else { PortDirection::Output }
    }

    fn port_takes_many(&self, port: usize) -> bool {
        port == 0 && self.takes_many
    }

    fn port_detaches(&self, _port: usize) -> bool {
        self.detaches
    }

    fn shape(&self) -> NodeShape {
        self.shape
    }

    fn tooltip(&self) -> Option<String> {
        self.tooltip.map(str::to_owned)
    }

    fn footer(&self) -> Option<NodeFooter<'_>> {
        self.footer.map(|text| NodeFooter::new(text, 14.0))
    }
}

type Events = Rc<RefCell<Vec<CollectionEvent<NodeId, NodeEvent>>>>;

/// A graph in a fixture, with what it reported and the selections the test owns.
struct Rig {
    fixture: Fixture,
    events: Events,
    wire_events: Rc<RefCell<Vec<WireEvent>>>,
    nodes: Rc<RefCell<Selection<NodeId>>>,
    wired: Rc<RefCell<Selection<u64>>>,
}

/// How a rig's graph is shown.
struct Setup {
    kinds: Vec<Kind>,
    places: Vec<Pos2>,
    sizes: Vec<Vec2>,
    wires: Vec<Wire>,
    zoom: f32,
    labels: PortLabels,
    /// A wire pulled in a pass inserts this node at the pull and selects it, in that pass.
    on_pull: Option<Kind>,
}

impl Setup {
    fn cards(kinds: Vec<Kind>, places: Vec<Pos2>) -> Self {
        let sizes = vec![CARD; places.len()];
        Setup { kinds, places, sizes, wires: Vec::new(), zoom: 1.0, labels: PortLabels::Hidden, on_pull: None }
    }
}

fn rig(setup: Setup) -> Rig {
    let events: Events = Rc::default();
    let wire_events = Rc::new(RefCell::new(Vec::new()));
    let nodes = Rc::new(RefCell::new(Selection::default()));
    let wired = Rc::new(RefCell::new(Selection::default()));
    let (events_in, wires_in, nodes_in, wired_in) = (events.clone(), wire_events.clone(), nodes.clone(), wired.clone());
    let Setup { mut kinds, places, sizes, wires, zoom, labels, on_pull } = setup;
    let mut rects: Vec<Rect> = places.iter().zip(&sizes).map(|(place, size)| Rect::from_min_size(*place, *size)).collect();
    let mut fixture = Fixture::new(move |ui| {
        let pulled = {
            let shown: Vec<&dyn NodeKind> = kinds.iter().map(|kind| kind as &dyn NodeKind).collect();
            let ids: Vec<NodeId> = (0..rects.len() as u32).map(NodeId::new).collect();
            let source = keyed(&ids, |id: &NodeId| *id);
            let (mut nodes, mut wired) = (nodes_in.borrow_mut(), wired_in.borrow_mut());
            let graph = NodeGraph::new(&rects, &shown, &wires)
                .label("Graph")
                .selection(Bind::bound(&mut *nodes))
                .wire_selection(Bind::bound(&mut *wired))
                .port_labels(labels)
                .zoom(CanvasTransform::new(zoom, Vec2::ZERO));
            let out = graph.show_items(ui, &source, &mut |id, _| Node::new(*id, shown[id.0 as usize]));
            events_in.borrow_mut().extend(out.collection.events);
            let pulled = out.wires.iter().find_map(|event| match event {
                WireEvent::Pulled { at, .. } => Some(*at),
                _ => None,
            });
            wires_in.borrow_mut().extend(out.wires);
            pulled
        };
        if let (Some(at), Some(kind)) = (pulled, &on_pull) {
            kinds.push(kind.clone());
            rects.push(Rect::from_center_size(at, CARD));
            *nodes_in.borrow_mut() = Selection::single(NodeId::new(rects.len() as u32 - 1));
        }
    });
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    Rig { fixture, events, wire_events, nodes, wired }
}

impl Rig {
    /// The centre of the node or port named `name` on screen.
    fn at(&self, name: &str) -> Pos2 {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("{name} shows")).rect.center()
    }

    /// A drag from `from` to `to` in a fresh gesture: no double click with the one before.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.events.borrow_mut().clear();
        self.fixture.advance(1.0).idle(1);
        self.fixture.drag_at(from, to - from).settle();
    }

    /// A primary click at `at` in a fresh gesture, with `mods` held.
    fn click(&mut self, at: Pos2, mods: Modifiers) {
        self.fixture.advance(1.0).idle(1);
        self.fixture.hold(mods).click_at(at).settle();
        self.fixture.hold(Modifiers::NONE);
    }

    /// A secondary click at `at` in a fresh gesture.
    fn secondary_click(&mut self, at: Pos2) {
        self.fixture.advance(1.0).idle(1);
        self.fixture.frame(vec![pointer::moved(at)]);
        self.fixture.frame(vec![pointer::secondary(at, true)]);
        self.fixture.frame(vec![pointer::secondary(at, false)]).settle();
    }

    fn wired(&self) -> Vec<(PortRef, PortRef, Option<u64>)> {
        let events = self.events.borrow();
        events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Item(_, NodeEvent::Wired { from, to, detached, .. }) => Some((*from, *to, *detached)),
                _ => None,
            })
            .collect()
    }

    fn ended(&self) -> Vec<(PortRef, Option<u64>, Option<Pos2>)> {
        let events = self.events.borrow();
        events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Item(_, NodeEvent::WireEnded { from, detached, loose, .. }) => Some((*from, *detached, *loose)),
                _ => None,
            })
            .collect()
    }

    /// The refusing port each `WireEnded` reported.
    fn refused(&self) -> Vec<Option<PortRef>> {
        let events = self.events.borrow();
        events
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Item(_, NodeEvent::WireEnded { refused, .. }) => Some(*refused),
                _ => None,
            })
            .collect()
    }

    /// How many `Move` events of each step the graph reported: begins, updates, commits and cancels.
    fn moves(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for event in self.events.borrow().iter() {
            if let CollectionEvent::Move(step) = event {
                let at = match step {
                    EditEvent::Begin { .. } => 0,
                    EditEvent::Update { .. } => 1,
                    EditEvent::Commit { .. } => 2,
                    EditEvent::Cancel { .. } => 3,
                    _ => continue,
                };
                counts[at] += 1;
            }
        }
        counts
    }

    fn wires_selected(&self) -> Vec<u64> {
        let mut items = self.wired.borrow().items.clone();
        items.sort_unstable();
        items
    }
}

fn port(node: u32, port: usize) -> PortRef {
    PortRef::new(NodeId::new(node), port)
}

/// A drag from the fed input of a kind that keeps its wires starts no wire: nothing detaches, connects or ends. Beside
/// it, a fed input of a kind that detaches still gives its wire up.
#[trial]
fn a_kind_that_keeps_its_wires_leaves_a_press_on_its_fed_input_alone() {
    let keeps = Kind { detaches: false, ..Kind::new("B", ["B in", "B out"]) };
    let kinds = vec![Kind::new("A", ["A in", "A out"]), keeps, Kind::new("C", ["C in", "C out"]), Kind::new("D", ["D in", "D out"])];
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0), Pos2::new(40.0, 180.0), Pos2::new(240.0, 180.0)];
    let wires = vec![Wire::new(7, port(0, 1), port(1, 0)), Wire::new(8, port(0, 1), port(3, 0))];
    let mut graph = rig(Setup { wires, ..Setup::cards(kinds, places) });

    graph.drag(graph.at("B in"), graph.at("C in"));
    assert!(graph.wired().is_empty() && graph.ended().is_empty(), "the wire stays: {:?} {:?}", graph.wired(), graph.ended());

    graph.drag(graph.at("D in"), graph.at("C in"));
    assert_eq!(graph.wired(), [(port(0, 1), port(2, 0), Some(8))], "a kind that detaches gives its wire up");
}

/// A drag from an unfed input is a reverse wire reported output end first; one from a fed input carries the wire it
/// detaches from its source; one from an input that takes many wires leaves them and starts a reverse wire. A drop on
/// the same node or on a port of the drag's own direction is refused, and a release that no port takes reports where,
/// in document units, at any zoom.
#[trial]
fn a_wire_from_an_input_connects_to_an_output_and_detaches_when_connected() {
    let many = Kind { takes_many: true, ..Kind::new("C", ["C in", "C out"]) };
    let kinds = vec![Kind::new("A", ["A in", "A out"]), Kind::new("B", ["B in", "B out"]), many];
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0), Pos2::new(40.0, 180.0)];
    let wires = vec![Wire::new(7, port(0, 1), port(1, 0)), Wire::new(8, port(0, 1), port(2, 0))];
    let mut graph = rig(Setup { wires: wires.clone(), ..Setup::cards(kinds.clone(), places.clone()) });

    graph.drag(graph.at("A in"), graph.at("B out"));
    assert_eq!(graph.wired(), [(port(1, 1), port(0, 0), None)], "a reverse wire connects output end first");

    graph.drag(graph.at("B in"), graph.at("C in"));
    assert_eq!(graph.wired(), [(port(0, 1), port(2, 0), Some(7))], "a fed input gives its wire up to the drag, from its source");
    assert!(graph.ended().iter().all(|(from, detached, _)| *from == port(0, 1) && *detached == Some(7)), "{:?}", graph.ended());

    graph.drag(graph.at("C in"), graph.at("B out"));
    assert_eq!(graph.wired(), [(port(1, 1), port(2, 0), None)], "an input that takes many keeps its wires and starts a reverse wire");

    for (to, refuser, why) in [("A out", port(0, 1), "a port of the same node"), ("B in", port(1, 0), "a port of the drag's own direction")] {
        graph.drag(graph.at("A in"), graph.at(to));
        assert!(graph.wired().is_empty(), "{why} refuses the wire: {:?}", graph.wired());
        assert!(matches!(graph.ended().as_slice(), [(_, None, Some(_))]), "{why}: the release is loose: {:?}", graph.ended());
        assert_eq!(graph.refused(), [Some(refuser)], "{why}: the end names the port that refused");
    }

    // The output of A sits at (140, 70) in the document; the release lands (60, 60) document units past it.
    let past = Vec2::new(60.0, 60.0);
    for zoom in [1.0, 0.5] {
        let mut zoomed = rig(Setup { wires: wires.clone(), zoom, ..Setup::cards(kinds.clone(), places.clone()) });
        let from = zoomed.at("A out");
        zoomed.drag(from, from + past * zoom);
        let ended = zoomed.ended();
        let [(start, None, Some(loose))] = ended.as_slice() else { panic!("at {zoom}: one loose end: {ended:?}") };
        assert_eq!(*start, port(0, 1), "at {zoom}");
        assert!(loose.distance(Pos2::new(140.0, 70.0) + past) < 1.0, "at {zoom}: the loose end is in document units: {loose:?}");
        assert!(zoomed.wired().is_empty(), "at {zoom}: nothing connected");
        assert_eq!(zoomed.refused(), [None], "at {zoom}: no port refused a release over empty space");
    }
}

/// A refusing port names itself only when its node is the topmost target under the release: a node shown over it
/// takes the release, and with no port of its own there, the end names none.
#[trial]
fn a_refusing_port_under_another_node_is_not_reported() {
    let kinds = vec![Kind::new("A", ["A in", "A out"]), Kind::new("B", ["B in", "B out"]), Kind::new("C", ["C in", "C out"])];
    // C, shown last, covers B's input at (240, 70), 40 units from C's own input and 60 from its output.
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0), Pos2::new(200.0, 40.0)];
    let mut graph = rig(Setup::cards(kinds, places));
    graph.drag(graph.at("A in"), graph.at("B in"));
    assert!(graph.wired().is_empty(), "nothing connected: {:?}", graph.wired());
    assert_eq!(graph.refused(), [None], "the node over the refusing port takes the release");
}

/// A Dot's ports reach around its circle and into it only as far as their own handle: a drag from inside the circle
/// moves the Dot, one from just outside it or from the port's centre, on the circle's edge, wires from the port, at a
/// small and a large zoom alike.
#[trial]
fn a_dot_moves_from_its_body_and_wires_from_just_outside_at_any_zoom() {
    let dot = Kind { shape: NodeShape::Dot, ..Kind::new("Dot", ["Dot in", "Dot out"]) };
    for zoom in [0.5, 2.0] {
        let mut setup = Setup { zoom, ..Setup::cards(vec![dot.clone()], vec![Pos2::new(60.0, 60.0)]) };
        setup.sizes = vec![Vec2::splat(14.0)];
        let mut rig = rig(setup);
        let centre = rig.at("Dot");
        // The output port sits on the circle's right edge, 7 document units from the centre.
        let inside = centre + Vec2::new(3.0, 0.0) * zoom;
        rig.drag(inside, inside + Vec2::new(40.0, 0.0));
        assert_eq!(rig.moves()[2], 1, "at {zoom}: a drag from inside the circle moves the Dot: {:?}", rig.events.borrow());
        assert!(rig.ended().is_empty(), "at {zoom}: and starts no wire");

        let outside = centre + Vec2::new(9.0, 0.0) * zoom;
        rig.drag(outside, outside + Vec2::new(0.0, 60.0));
        assert!(matches!(rig.ended().as_slice(), [(from, None, Some(_))] if *from == port(0, 1)), "at {zoom}: {:?}", rig.ended());
        assert_eq!(rig.moves(), [0; 4], "at {zoom}: a drag from just outside the circle moves nothing");

        let on_port = rig.at("Dot out");
        rig.drag(on_port, on_port + Vec2::new(0.0, 60.0));
        assert!(matches!(rig.ended().as_slice(), [(from, None, Some(_))] if *from == port(0, 1)), "at {zoom}: {:?}", rig.ended());
        assert_eq!(rig.moves(), [0; 4], "at {zoom}: a drag from the port's centre moves nothing");
    }
}

/// A primary drag of a node moves it; Escape cancels the move once and the release then commits nothing; the release
/// of another button during the move changes nothing, and the primary release commits it.
#[trial]
fn a_node_move_ignores_other_buttons_and_escape_cancels_it() {
    let mut rig = rig(Setup::cards(vec![Kind::new("A", ["A in", "A out"])], vec![Pos2::new(40.0, 40.0)]));
    let body = rig.at("A");
    let by = Vec2::new(40.0, 0.0);

    rig.drag(body, body + by);
    let [begins, updates, commits, cancels] = rig.moves();
    assert!(begins == 1 && updates > 0 && commits == 1 && cancels == 0, "a drag moves the node: {:?}", rig.moves());

    rig.events.borrow_mut().clear();
    rig.fixture.advance(1.0).idle(1);
    rig.fixture.drag_at_pausing(body, by, |fixture| {
        fixture.key(Key::Escape, Modifiers::NONE);
    });
    rig.fixture.settle();
    assert_eq!(rig.moves()[2..], [0, 1], "Escape cancels the move once and the release commits nothing");

    rig.events.borrow_mut().clear();
    rig.fixture.advance(1.0).idle(1);
    rig.fixture.drag_hold_at(body, by);
    rig.fixture.frame(vec![pointer::secondary(body + by, true)]);
    rig.fixture.frame(vec![pointer::secondary(body + by, false)]).idle(1);
    assert_eq!(rig.moves()[2..], [0, 0], "another button's release leaves the move running");
    rig.fixture.release_at(body + by).settle();
    assert_eq!(rig.moves()[2..], [1, 0], "the primary release commits it");
}

/// With names on hover, the ports' names show while the pointer is over their node and not otherwise; the ports
/// show either way.
#[trial]
fn hovered_node_shows_its_port_labels() {
    let kinds = vec![Kind::new("A", ["A in", "A out"]), Kind::new("B", ["B in", "B out"])];
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0)];
    let mut rig = rig(Setup { labels: PortLabels::OnHover, ..Setup::cards(kinds, places) });
    let (body, input) = (rig.at("A"), rig.at("A in"));
    let painted = |rig: &Rig, text: &str| rig.fixture.texts().iter().any(|run| run.text == text);
    let port_drawn = |shapes: &[egui::Shape]| {
        let square = Rect::from_center_size(input, Vec2::splat(24.0));
        shapes.iter().any(|shape| square.contains_rect(shape.visual_bounding_rect()))
    };

    rig.fixture.frame(vec![pointer::moved(body)]).settle();
    assert!(painted(&rig, "A in") && painted(&rig, "A out"), "the hovered node names its ports");
    assert!(!painted(&rig, "B in"), "a node the pointer is not over does not");
    assert!(port_drawn(rig.fixture.frame_shapes().as_slice()), "the port shows under its name");

    let away = rig.fixture.by_label("Graph").expect("the graph shows").rect.left_bottom() + Vec2::new(20.0, -20.0);
    rig.fixture.frame(vec![pointer::moved(away)]).settle();
    assert!(!painted(&rig, "A in") && !painted(&rig, "A out"), "with the pointer away the names hide");
    assert!(port_drawn(rig.fixture.frame_shapes().as_slice()), "and the port still shows");
}

/// A plain click on a wire selects it alone and clears the nodes; Shift adds a wire and Cmd toggles one, both keeping
/// the nodes; a secondary click selects an unselected wire alone and keeps a selected one; a plain press on a node or
/// on empty space clears the wires.
#[trial]
fn wire_clicks_select_wires_beside_the_node_selection() {
    let kinds = vec![
        Kind::new("A", ["A in", "A out"]),
        Kind::new("B", ["B in", "B out"]),
        Kind::new("C", ["C in", "C out"]),
        Kind::new("D", ["D in", "D out"]),
    ];
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0), Pos2::new(40.0, 180.0), Pos2::new(240.0, 180.0)];
    let wires = vec![Wire::new(1, port(0, 1), port(1, 0)), Wire::new(2, port(2, 1), port(3, 0))];
    let mut rig = rig(Setup { wires, ..Setup::cards(kinds, places) });
    // Each wire runs level between ports at one height, so its middle lies halfway between them.
    let first = rig.at("A out").lerp(rig.at("B in"), 0.5);
    let second = rig.at("C out").lerp(rig.at("D in"), 0.5);
    let nodes = |rig: &Rig| rig.nodes.borrow().items.clone();

    rig.click(rig.at("A"), Modifiers::NONE);
    assert_eq!(nodes(&rig), [NodeId::new(0)], "the node is picked");
    rig.click(first, Modifiers::NONE);
    assert_eq!(rig.wires_selected(), [1], "a plain click selects the wire");
    assert!(nodes(&rig).is_empty(), "and clears the nodes");

    rig.click(rig.at("B"), Modifiers::NONE);
    assert!(rig.wires_selected().is_empty(), "a plain press on a node clears the wires");
    rig.click(first, Modifiers::SHIFT);
    rig.click(second, Modifiers::SHIFT);
    assert_eq!(rig.wires_selected(), [1, 2], "Shift adds wires");
    rig.click(first, Modifiers::COMMAND);
    assert_eq!(rig.wires_selected(), [2], "Cmd toggles a wire");
    assert_eq!(nodes(&rig), [NodeId::new(1)], "Shift and Cmd keep the nodes");

    rig.secondary_click(first);
    assert_eq!(rig.wires_selected(), [1], "a secondary click selects an unselected wire alone");
    rig.secondary_click(first);
    assert_eq!(rig.wires_selected(), [1], "and keeps a selected one");

    let empty = rig.fixture.by_label("Graph").expect("the graph shows").rect.left_bottom() + Vec2::new(20.0, -20.0);
    rig.click(empty, Modifiers::NONE);
    assert!(rig.wires_selected().is_empty(), "a plain press on empty space clears the wires");
}

/// A press on a wire that moves past the slop reports `Pulled` once; a node the app inserts and selects in that pass
/// follows the pointer, as the graph's one free move, which commits once.
#[trial]
fn a_press_on_a_wire_past_the_slop_reports_pulled_and_the_selection_follows_the_pointer() {
    let kinds = vec![Kind::new("A", ["A in", "A out"]), Kind::new("B", ["B in", "B out"])];
    let places = vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0)];
    let wires = vec![Wire::new(1, port(0, 1), port(1, 0))];
    let mut rig = rig(Setup { wires, on_pull: Some(Kind::new("Reroute", ["R in", "R out"])), ..Setup::cards(kinds, places) });
    let middle = rig.at("A out").lerp(rig.at("B in"), 0.5);

    rig.drag(middle, middle + Vec2::new(0.0, 80.0));
    let pulls = rig.wire_events.borrow().iter().filter(|event| matches!(event, WireEvent::Pulled { wire: 1, .. })).count();
    assert_eq!(pulls, 1, "the pull is reported once: {:?}", rig.wire_events.borrow());
    assert_eq!(rig.nodes.borrow().items, [NodeId::new(2)], "the inserted node is the selection");
    let [_, updates, commits, cancels] = rig.moves();
    assert!(updates > 0 && commits == 1 && cancels == 0, "the inserted node follows the pointer in one move: {:?}", rig.moves());
}

/// A kind's tooltip shows on a rest over its node and a kind without one shows none; a kind's footer is its node's
/// accessible description and a node without a footer has none.
#[trial]
fn a_node_shows_its_kinds_tooltip_and_publishes_its_footer_as_its_description() {
    const TIP: &str = "Previews are limited in size while editing";
    const FOOTER: &str = "512x512 Preview";
    let tipped = Kind { tooltip: Some(TIP), footer: Some(FOOTER), ..Kind::new("A", ["A in", "A out"]) };
    let kinds = vec![tipped, Kind::new("B", ["B in", "B out"])];
    let mut rig = rig(Setup::cards(kinds, vec![Pos2::new(40.0, 40.0), Pos2::new(240.0, 40.0)]));
    let shows_tip = |rig: &mut Rig, at: Pos2| {
        rig.fixture.frame(vec![pointer::moved(at)]);
        rig.fixture.advance(DELAY);
        rig.fixture.settle();
        rig.fixture.texts().iter().any(|run| run.text == TIP)
    };
    let (a, b) = (rig.at("A"), rig.at("B"));
    assert!(shows_tip(&mut rig, a), "the kind's tooltip shows on a rest over its node");
    assert!(!shows_tip(&mut rig, b), "a kind without a tooltip shows none");

    let description = |rig: &Rig, name: &str| {
        let node = rig.fixture.by_label(name).unwrap_or_else(|| panic!("{name} shows")).node;
        rig.fixture.tree().description_of(node).map(str::to_owned)
    };
    assert_eq!(description(&rig, "A").as_deref(), Some(FOOTER), "the footer is the node's description");
    assert_eq!(description(&rig, "B"), None, "a node without a footer has no description");
}
