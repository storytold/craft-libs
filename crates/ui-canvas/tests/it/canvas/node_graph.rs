//! A canvas as a node graph's host: the graph follows the view through its layer transform and builds the nodes the
//! view shows, and a document with no edges fits what it is asked to within its zoom scheme.

use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CanvasView, Selection, ZoomFloor, ZoomScheme, keyed};
use craft_ui::canvas::{Canvas, ViewCommands, ViewRequest, ViewTarget, canvas_space};
use craft_ui::contract::{Bind, CollectionEvent, Props as _, ShowCollection as _, ShowContainer as _};
use craft_ui::graph::{Node, NodeEvent, NodeGraph, NodeId, NodeKind, PortDirection, PortRef, Wire};
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, FireSource};
use craft_ui::runtime::env::with_env;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Modifiers, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A node with an input on the left and an output on the right, whose ports are named after the node, so a trial
/// finds each node's ports by name.
struct Named(&'static str);

impl NodeKind for Named {
    fn title(&self) -> &str {
        self.0
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        format!("{} {}", self.0, if port == 0 { "in" } else { "out" })
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 0 { PortDirection::Input } else { PortDirection::Output }
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }
}

static KINDS: [Named; 4] = [Named("A"), Named("B"), Named("C"), Named("D")];

/// The nodes in document units: two near the origin, one far off at negative points and one far off at positive ones.
const NODES: [Rect; 4] = [
    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(140.0, 72.0)),
    Rect::from_min_max(Pos2::new(200.0, 60.0), Pos2::new(340.0, 132.0)),
    Rect::from_min_max(Pos2::new(-2000.0, -1500.0), Pos2::new(-1860.0, -1428.0)),
    Rect::from_min_max(Pos2::new(5000.0, 4000.0), Pos2::new(5140.0, 4072.0)),
];

/// Where the canvas lies in the window: away from its origin, so a transform measured from the window's origin shows.
const CANVAS: Rect = Rect::from_min_max(Pos2::new(80.0, 60.0), Pos2::new(560.0, 380.0));

/// The document point at the middle of a node.
fn middle_of(node: Rect) -> Point {
    let centre = node.center();
    Point::new(f64::from(centre.x), f64::from(centre.y))
}

/// The document rect of a node.
fn doc_rect(node: Rect) -> DocRect {
    DocRect::new(f64::from(node.min.x), f64::from(node.min.y), f64::from(node.max.x), f64::from(node.max.y))
}

/// One wheel line of `dy` with `modifiers` at `at`.
fn line(fixture: &mut Fixture, at: Pos2, dy: f32, modifiers: Modifiers) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers,
        source: egui::MouseWheelSource::Wheel,
    }]);
}

#[trial]
fn a_node_graph_follows_the_canvas_view() {
    let extent = NODES.iter().fold(doc_rect(NODES[0]), |all, node| all.union(doc_rect(*node)));
    let view = Rc::new(Cell::new(CanvasView::new(Rect::ZERO, Point::new(200.0, 100.0), 1.0)));
    let selection = Rc::new(RefCell::new(Selection::<NodeId>::default()));
    let (shown, selected) = (Rc::clone(&view), Rc::clone(&selection));
    let mut fixture = Fixture::new(move |ui| {
        in_region(ui, CANVAS, |ui| {
            // A view with a rect is shown as given: no first fit.
            let mut given = shown.get();
            given.rect = ui.available_rect_before_wrap();
            let kinds: Vec<&dyn NodeKind> = KINDS.iter().map(|kind| kind as &dyn NodeKind).collect();
            let ids: Vec<NodeId> = (0..NODES.len() as u32).map(NodeId::new).collect();
            let source = keyed(&ids, |id| *id);
            let mut selection = selected.borrow_mut();
            let canvas = Canvas::new("Graph").document(ViewTarget::unbounded(extent)).view(given).show(ui, |ui, body| {
                let Some(space) = canvas_space(&body.env) else { return };
                let mut graph = NodeGraph::new(&NODES, &kinds, &[]).label("Nodes").selection(Bind::bound(&mut *selection));
                if let Some(transform) = space.view.layer_transform() {
                    graph = graph.zoom(transform);
                }
                let _ = graph.show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
            });
            shown.set(canvas.output.view.unwrap_or(given));
        });
    });
    fixture.room(Vec2::new(640.0, 440.0));
    fixture.settle();
    let built = |fixture: &Fixture, name: &str| fixture.by_label(&format!("{name} out")).is_some();
    assert!(built(&fixture, "A") && built(&fixture, "B"), "the nodes the view shows are built");
    assert!(!built(&fixture, "C") && !built(&fixture, "D"), "the nodes far off it are not");

    // A pan and a zoom through the wheel over a node: the ports stay on the document points they belong to.
    let over = view.get().to_screen(middle_of(NODES[0]));
    let before = view.get();
    line(&mut fixture, over, 1.0, Modifiers::ALT);
    assert!(view.get().zoom > before.zoom, "the wheel over a node zooms the view: {before:?} -> {:?}", view.get());
    let zoomed = view.get();
    line(&mut fixture, over, -1.0, Modifiers::NONE);
    assert!(view.get().center.y > zoomed.center.y, "a plain line pans the view");
    fixture.frame(Vec::new());
    let moved = view.get();
    // A's output on its right edge and B's input on its left, both inside the canvas at this view.
    for (name, x, node) in [("A out", NODES[0].max.x, NODES[0]), ("B in", NODES[1].min.x, NODES[1])] {
        let port = fixture.by_label(name).expect("the node is still on screen");
        let at = moved.to_screen(Point::new(f64::from(x), f64::from(node.center().y)));
        assert!(port.rect.center().distance(at) < 1.0, "{name} lies at its document point: {:?} vs {at:?}", port.rect.center());
    }

    // A primary press on a node selects it.
    fixture.click_at(moved.to_screen(middle_of(NODES[0])));
    assert!(selection.borrow().contains(&NodeId::new(0)), "a click on a node selects it");

    // The view at a node far off at negative points builds it and not the ones near the origin; the same far off at
    // positive points. A, the lead the click made, stays built out of view, as a collection keeps its lead.
    for (name, node) in [("C", NODES[2]), ("D", NODES[3])] {
        let mut far = view.get();
        far.center = middle_of(node);
        view.set(far);
        fixture.frame(Vec::new()).frame(Vec::new());
        assert!(built(&fixture, name), "{name} is built while the view shows it");
        assert!(!built(&fixture, "B"), "B is not built while the view is at {name}");
        let port = fixture.by_label(&format!("{name} in")).expect("the node is built");
        assert!(CANVAS.contains(port.rect.center()), "{name}'s input port is on the canvas: {:?}", port.rect);
    }
}

/// What a graph in a canvas reported: the node clicks and the wires selected.
struct Hosted {
    fixture: Fixture,
    clicked: Rc<Cell<usize>>,
    wired: Rc<RefCell<Selection<u64>>>,
}

/// Two nodes joined by a wire in a graph in a canvas, as an app hosts one, settled with nothing pressed yet. The
/// nodes lie far up and left of the document's origin, as a fitted graph of an app often does, so the graph's body
/// layer shows far from its own origin.
fn graph_in_canvas() -> Hosted {
    let nodes = NODES.map(|node| node.translate(Vec2::new(-900.0, -700.0)));
    let view = CanvasView::new(Rect::ZERO, Point::new(-700.0, -600.0), 1.0);
    let clicked = Rc::new(Cell::new(0_usize));
    let wired = Rc::new(RefCell::new(Selection::<u64>::default()));
    let (seen, wire_selection) = (Rc::clone(&clicked), Rc::clone(&wired));
    let mut fixture = Fixture::new(move |ui| {
        in_region(ui, CANVAS, |ui| {
            let mut given = view;
            given.rect = ui.available_rect_before_wrap();
            let kinds: Vec<&dyn NodeKind> = KINDS[..2].iter().map(|kind| kind as &dyn NodeKind).collect();
            let ids: Vec<NodeId> = (0..2).map(NodeId::new).collect();
            let source = keyed(&ids, |id| *id);
            let wires = [Wire::new(1, PortRef::new(NodeId::new(0), 1), PortRef::new(NodeId::new(1), 0))];
            let mut wire_selection = wire_selection.borrow_mut();
            let extent = doc_rect(nodes[0]).union(doc_rect(nodes[1]));
            Canvas::new("Graph").document(ViewTarget::unbounded(extent)).view(given).show(ui, |ui, body| {
                let Some(space) = canvas_space(&body.env) else { return };
                let mut graph = NodeGraph::new(&nodes[..2], &kinds, &wires).label("Nodes").wire_selection(Bind::bound(&mut *wire_selection));
                if let Some(transform) = space.view.layer_transform() {
                    graph = graph.zoom(transform);
                }
                let out = graph.show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
                let clicks = out.collection.events.iter().filter(|event| matches!(event, CollectionEvent::Item(_, NodeEvent::Clicked { .. })));
                seen.set(seen.get() + clicks.count());
            });
        });
    });
    fixture.room(Vec2::new(640.0, 440.0));
    fixture.settle();
    Hosted { fixture, clicked, wired }
}

/// The very first press over a graph in a canvas reaches what lies under it: a wire is selected, and a node's port
/// reports its click.
#[trial]
fn the_first_press_over_a_graph_in_a_canvas_reaches_it() {
    let port = |fixture: &Fixture, name: &str| fixture.by_label(name).expect("the node is built").rect.center();
    let mut hosted = graph_in_canvas();
    // The wire runs from A's output to B's input.
    let wire = port(&hosted.fixture, "A out").lerp(port(&hosted.fixture, "B in"), 0.5);
    hosted.fixture.click_at(wire);
    assert!(hosted.wired.borrow().contains(&1), "the first press over a wire selects it: {:?}", hosted.wired.borrow());

    let mut hosted = graph_in_canvas();
    let out = port(&hosted.fixture, "A out");
    hosted.fixture.click_at(out);
    assert!(hosted.clicked.get() > 0, "the first press over a port reaches its node");
}

/// The scheme of the unbounded canvas's env: 50 % to 200 %.
const GRAPH_SCHEME: ZoomScheme = ZoomScheme::new(&[0.5, 1.0, 2.0], ZoomFloor::Fixed(0.5), 2.0);

/// Whether `view` shows all of `rect` with its middle at the view's middle.
fn frames(view: &CanvasView, rect: DocRect) -> bool {
    let extent = view.extent(rect);
    let centre = view.to_screen(rect.center());
    extent.x <= view.rect.width() && extent.y <= view.rect.height() && centre.distance(view.rect.center()) < 1.0
}

#[trial]
fn an_unbounded_canvas_fits_its_layers_within_its_scheme() {
    // Larger than the canvas: a fit has to zoom out, and stays above the scheme's least zoom.
    let wide = DocRect::new(-100.0, -50.0, 500.0, 350.0);
    // Small: a fit uncapped would magnify it many times.
    let small = DocRect::new(1000.0, 1000.0, 1040.0, 1020.0);
    let layers = DocRect::new(-3000.0, -3000.0, -2990.0, -2995.0);
    let extent = Rc::new(Cell::new(wide));
    let view = Rc::new(Cell::new(CanvasView::new(Rect::ZERO, Point::ZERO, 1.0)));
    let queue = Rc::new(RefCell::new(Vec::<ViewRequest>::new()));
    let (shown, asked, target) = (Rc::clone(&view), Rc::clone(&queue), Rc::clone(&extent));
    let mut fixture = Fixture::new(move |ui| {
        with_env(
            ui,
            |scope| {
                scope.values.set(GRAPH_SCHEME);
            },
            |ui| {
                // Never given a rect, the view opens fitted in the first pass that has room.
                let given = shown.get();
                let commands = asked.borrow().clone();
                let bounds = || Some(layers);
                let canvas = Canvas::new("Graph")
                    .document(ViewTarget::unbounded(target.get()))
                    .layer_bounds(&bounds)
                    .queued(&commands)
                    .view(given)
                    .show(ui, |_, _| {});
                shown.set(canvas.output.view.unwrap_or(given));
                asked.borrow_mut().clear();
            },
        );
    });
    fixture.room(Vec2::new(480.0, 320.0));
    fixture.settle();
    let ppp = fixture.ctx().pixels_per_point();
    let device = |view: &Cell<CanvasView>| zoom::device(&view.get(), ppp);
    // The fit ceiling of a target that sets none: 100 %.
    let ceiling = ViewTarget::unbounded(wide).fit_ceiling;

    let opened = view.get();
    assert!(frames(&opened, wide), "a never-shown view opens fitted to the extent: {opened:?}");
    assert!(device(&view) < ceiling && device(&view) >= GRAPH_SCHEME.min(craft_behavior::kurbo::Size::ZERO) - 1e-9, "zoomed out within the scheme");

    // Fit on Screen frames the extent, up to the fit ceiling.
    extent.set(small);
    queue.borrow_mut().push(ViewRequest::Place(ViewCommands::FitOnScreen));
    fixture.frame(Vec::new());
    assert!(frames(&view.get(), small), "Fit on Screen frames the extent: {:?}", view.get());
    assert!((device(&view) - ceiling).abs() < 1e-6, "and stops at the fit ceiling: {}", device(&view));

    // Fit Layers frames the layers, up to the same ceiling.
    let canvas = fixture.by_role(Role::CANVAS)[0];
    fixture.click(&canvas);
    assert!(commands::fire(fixture.ctx(), ViewCommands::FitLayers, FireSource::Menu), "a menu row fires Fit Layers");
    fixture.frame(Vec::new());
    let fitted = view.get();
    assert!(frames(&fitted, layers), "Fit Layers frames the layers: {fitted:?}");
    assert!(device(&view) <= ceiling + 1e-9, "and stops at the fit ceiling: {}", device(&view));

    // The wheel stays inside the env's scheme.
    let at = fitted.rect.center();
    for _ in 0..40 {
        line(&mut fixture, at, 1.0, Modifiers::ALT);
    }
    let most = device(&view);
    for _ in 0..40 {
        line(&mut fixture, at, -1.0, Modifiers::ALT);
    }
    let least = device(&view);
    assert!(most > ceiling && most <= GRAPH_SCHEME.max_device() + 1e-9, "the wheel zooms in up to the scheme's greatest zoom: {most}");
    assert!(least < ceiling && least >= GRAPH_SCHEME.min(craft_behavior::kurbo::Size::ZERO) - 1e-9, "and out to its least: {least}");

    // Panning far past the extent never clamps: every line moves the view as far, and the view stays where it is left.
    let mut centres = vec![view.get().center.y];
    for _ in 0..30 {
        line(&mut fixture, at, -1.0, Modifiers::NONE);
        centres.push(view.get().center.y);
    }
    let steps: Vec<f64> = centres.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(steps.iter().all(|step| *step > 0.0 && (step - steps[0]).abs() < 1e-6), "each line pans as far: {steps:?}");
    let left = view.get();
    assert!(left.to_screen(Point::new(layers.x1, layers.y1)).y < left.rect.min.y, "the view is past the layers it framed: {left:?}");
    fixture.idle(3);
    assert_eq!(view.get().center, left.center, "the view stays where the user left it");
}
