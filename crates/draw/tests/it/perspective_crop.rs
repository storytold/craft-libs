//! The Perspective Crop quad and tool through the public API: the pure quad rules, the quad's gestures and keys as a
//! canvas control, and the tool's box drag and four clicks on the background.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::{Point, Rect as DocRect, Size};
use craft_draw::ui::crop::{
    CropDocKey, PerspectiveCropEvent, PerspectiveCropQuad, PerspectiveCropTool, PerspectiveInput, grid_lines, grid_steps, is_valid,
};
use craft_ui::canvas::{Canvas, ToolHost, tool_pass};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};

const SIZE: Size = Size::new(400.0, 300.0);
const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
const ALT: Modifiers = Modifiers { alt: true, ..Modifiers::NONE };
const BOTH: Modifiers = Modifiers { shift: true, alt: true, ..Modifiers::NONE };
/// The bounds of the bounded quad, around the upright quad of the trials.
const BOUNDS: DocRect = DocRect::new(60.0, 50.0, 340.0, 240.0);

/// The screen point of document point `(x, y)` on the fixture's canvas: the view shows (200, 150) at its centre.
fn screen(fixture: &Fixture, x: f64, y: f64) -> Pos2 {
    let canvas = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas");
    canvas.center() + Vec2::new(x as f32 - 200.0, y as f32 - 150.0)
}

fn button(fixture: &mut Fixture, at: Pos2, pressed: bool, modifiers: Modifiers) {
    fixture.hold(modifiers);
    fixture.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }]);
}

/// A drag in document points over four moves; the last move and the release hold `last`, the others `first`.
fn drag_on(fixture: &mut Fixture, from: (f64, f64), to: (f64, f64), first: Modifiers, last: Modifiers) {
    let (from, to) = (screen(fixture, from.0, from.1), screen(fixture, to.0, to.1));
    fixture.hold(first);
    fixture.frame(vec![Event::PointerMoved(from)]);
    button(fixture, from, true, first);
    for step in 1..=4 {
        let mods = if step == 4 { last } else { first };
        fixture.hold(mods);
        fixture.frame(vec![Event::PointerMoved(from + (to - from) * (step as f32 / 4.0))]);
    }
    button(fixture, to, false, last);
    fixture.hold(Modifiers::NONE);
    fixture.frame(Vec::new());
}

/// The tool on a canvas showing its quad, driven by pointer events.
struct Rig {
    fixture: Fixture,
    tool: Rc<RefCell<PerspectiveCropTool>>,
    /// The active document the pass is given; `None` keys nothing.
    document: Rc<Cell<Option<CropDocKey>>>,
    /// Whether the quad reported `apply` in any frame since the last [`Rig::applied`].
    apply: Rc<Cell<bool>>,
}

impl Rig {
    fn new() -> Self {
        let tool = Rc::new(RefCell::new(PerspectiveCropTool::new()));
        let document = Rc::new(Cell::new(None));
        let apply = Rc::new(Cell::new(false));
        let (shown, active, applied) = (Rc::clone(&tool), Rc::clone(&document), Rc::clone(&apply));
        let mut fixture = Fixture::new(move |ui| {
            let mut tool = shown.borrow_mut();
            let room = ui.available_rect_before_wrap();
            let view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
            let input = PerspectiveInput { document: active.get(), ..Default::default() };
            let mut pass = tool.pass(input);
            let _ = Canvas::new("Document").document(SIZE).view(view).show(ui, |ui, body| {
                let output = pass.show_quad(ui, body.rect);
                applied.set(applied.get() || output.apply);
                if let Some(canvas) = tool_pass(&body.env) {
                    let _ = ToolHost::new().run(ui, &canvas, &mut pass);
                }
            });
        });
        fixture.settle();
        Rig { fixture, tool, document, apply }
    }

    /// The screen point of document point `(x, y)`: the view shows (200, 150) at the canvas's centre.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        screen(&self.fixture, x, y)
    }

    fn button(&mut self, at: Pos2, pressed: bool, modifiers: Modifiers) {
        button(&mut self.fixture, at, pressed, modifiers);
    }

    /// A drag over four moves; the last move and the release hold `last`, the others `first`.
    fn drag_with(&mut self, from: (f64, f64), to: (f64, f64), first: Modifiers, last: Modifiers) {
        drag_on(&mut self.fixture, from, to, first, last);
    }

    fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        self.drag_with(from, to, mods, mods);
    }

    fn click(&mut self, at: (f64, f64)) {
        let at = self.at(at.0, at.1);
        self.fixture.hold(Modifiers::NONE);
        self.fixture.frame(vec![Event::PointerMoved(at)]);
        self.button(at, true, Modifiers::NONE);
        self.button(at, false, Modifiers::NONE);
        self.fixture.frame(Vec::new());
    }

    /// Moves the pointer to document point `(x, y)` and whether a painted path has a point at it: the preview
    /// outline's last corner sits at the pointer.
    fn preview_reaches(&mut self, (x, y): (f64, f64)) -> bool {
        let pointer = self.at(x, y);
        self.fixture.frame(vec![Event::PointerMoved(pointer)]);
        let shapes = self.fixture.frame_shapes();
        shapes.iter().any(|shape| match shape {
            egui::Shape::Path(path) => path.points.iter().any(|p| (*p - pointer).length() < 0.5),
            _ => false,
        })
    }

    fn set(&self, quad: [(f64, f64); 4]) {
        self.tool.borrow_mut().set(Some(quad.map(|(x, y)| Point::new(x, y))));
    }

    fn quad(&self) -> Option<[Point; 4]> {
        *self.tool.borrow().quad()
    }

    fn placing(&self) -> usize {
        self.tool.borrow().placing()
    }

    /// Presses Enter and whether the quad applied.
    fn enter_applies(&mut self) -> bool {
        self.fixture.frame(Vec::new());
        self.apply.set(false);
        self.fixture.key(Key::Enter, Modifiers::NONE);
        self.fixture.frame(Vec::new());
        self.apply.take()
    }
}

fn near(got: Option<[Point; 4]>, want: [(f64, f64); 4]) -> bool {
    got.is_some_and(|q| q.iter().zip(want).all(|(p, (x, y))| (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3))
}

fn rectangle((a, b): (Point, Point)) -> [(f64, f64); 4] {
    let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// The quad alone on a canvas, kept inside [`BOUNDS`], starting at `start`; and its value.
fn bounded(start: [Point; 4]) -> (Fixture, Rc<RefCell<Option<[Point; 4]>>>) {
    let quad = Rc::new(RefCell::new(Some(start)));
    let shown = Rc::clone(&quad);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
        let _ = Canvas::new("Document").document(SIZE).view(view).show(ui, |ui, body| {
            let mut quad = shown.borrow_mut();
            let _ = PerspectiveCropQuad::new("Perspective crop", Bind::bound(&mut *quad)).bounds(BOUNDS).show_at(ui, body.rect);
        });
    });
    fixture.settle();
    (fixture, quad)
}

fn inside(q: &[Point; 4]) -> bool {
    q.iter().all(|p| p.x >= BOUNDS.x0 - 1e-6 && p.x <= BOUNDS.x1 + 1e-6 && p.y >= BOUNDS.y0 - 1e-6 && p.y <= BOUNDS.y1 + 1e-6)
}

/// Whether `b` is `a` moved without a change of shape.
fn same_shape(a: &[Point; 4], b: &[Point; 4]) -> bool {
    let by = b[0] - a[0];
    a.iter().zip(b).all(|(p, q)| ((*q - *p) - by).hypot() < 1e-6)
}

fn on_right_edge(x: f64) -> bool {
    (x - BOUNDS.x1).abs() < 1e-6
}

#[trial]
fn grid_lines_meet_at_the_quads_vanishing_points() {
    let view = CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0)), Point::new(200.0, 150.0), 1.0);
    let upright = [Point::new(100.0, 50.0), Point::new(300.0, 50.0), Point::new(300.0, 190.0), Point::new(100.0, 190.0)];
    let lines = grid_lines(&view, &upright, 48.0);
    assert_eq!(lines.len(), 2 * (grid_steps(200.0, 48.0) - 1), "one line per step boundary in each direction");
    assert!(lines.iter().all(|[a, b]| (a.x - b.x).abs() < 1e-3 || (a.y - b.y).abs() < 1e-3), "an upright box has an axis-aligned grid");

    // A trapezoid narrower at the top: its rows crowd towards the far edge, as the object's own lines do.
    let trapezoid = [Point::new(150.0, 50.0), Point::new(250.0, 50.0), Point::new(300.0, 250.0), Point::new(100.0, 250.0)];
    let mut rows: Vec<f32> = grid_lines(&view, &trapezoid, 48.0).iter().filter(|[a, b]| (a.y - b.y).abs() < 1e-3).map(|[a, _]| a.y).collect();
    rows.sort_by(f32::total_cmp);
    let gaps: Vec<f32> = rows.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(gaps.len() >= 2 && gaps.windows(2).any(|pair| (pair[0] - pair[1]).abs() > 0.5), "perspective rows are not evenly spaced: {gaps:?}");
}

#[trial]
fn perspective_box_and_corner_modifiers_and_clicks() {
    let mut rig = Rig::new();

    // A box drag follows the shared marquee rule: Shift squares it, Alt draws it from the press, both do both.
    let (press, to) = ((100.0, 80.0), (200.0, 150.0));
    for (mods, square, centre) in [(Modifiers::NONE, false, false), (SHIFT, true, false), (ALT, false, true), (BOTH, true, true)] {
        rig.tool.borrow_mut().clear();
        rig.drag(press, to, mods);
        let want = rectangle(Constraint::Free.corners(Point::new(press.0, press.1), Point::new(to.0, to.1), square, centre));
        assert!(near(rig.quad(), want), "box with {mods:?}: {:?} vs {want:?}", rig.quad());
    }

    // A drag that makes no box keeps the clicks; one that makes a box clears them.
    rig.tool.borrow_mut().clear();
    rig.click((100.0, 50.0));
    rig.click((300.0, 200.0));
    assert_eq!(rig.placing(), 2);
    assert!(rig.preview_reaches((250.0, 120.0)), "after two clicks the preview's last corner sits at the pointer");
    rig.drag((50.0, 250.0), (200.0, 250.0), Modifiers::NONE);
    assert!(rig.quad().is_none() && rig.placing() == 2, "a flat drag leaves no quad and keeps the clicks");
    rig.click((300.0, 50.0));
    assert_eq!(rig.placing(), 3);
    assert!(rig.preview_reaches((120.0, 180.0)), "after three clicks the preview's last corner sits at the pointer");
    rig.click((100.0, 200.0));
    assert!(
        near(rig.quad(), [(100.0, 50.0), (300.0, 50.0), (300.0, 200.0), (100.0, 200.0)]),
        "the fourth click puts the corners in order: {:?}",
        rig.quad()
    );
    assert_eq!(rig.placing(), 0);
    rig.tool.borrow_mut().clear();
    rig.click((100.0, 50.0));
    rig.drag((20.0, 20.0), (120.0, 120.0), Modifiers::NONE);
    assert!(rig.quad().is_some() && rig.placing() == 0, "a box drag clears the clicks");

    // Corner drags on an upright quad.
    let upright = [(100.0, 80.0), (300.0, 80.0), (300.0, 220.0), (100.0, 220.0)];
    let by = (340.0, 230.0);
    rig.set(upright);
    rig.drag((300.0, 220.0), by, SHIFT);
    assert!(
        near(rig.quad(), [(100.0, 80.0), (300.0, 80.0), (340.0, 220.0), (100.0, 220.0)]),
        "Shift locks the axis moved less along: {:?}",
        rig.quad()
    );
    rig.set(upright);
    rig.drag((300.0, 220.0), by, ALT);
    assert!(near(rig.quad(), [(100.0, 80.0), (340.0, 80.0), (340.0, 230.0), (100.0, 230.0)]), "Alt keeps the rectangle: {:?}", rig.quad());
    rig.set(upright);
    rig.drag_with((300.0, 220.0), by, ALT, Modifiers::NONE);
    assert!(
        near(rig.quad(), [(100.0, 80.0), (300.0, 80.0), (340.0, 230.0), (100.0, 220.0)]),
        "releasing Alt mid-drag puts the neighbours back: {:?}",
        rig.quad()
    );
    rig.set([(120.0, 80.0), (300.0, 80.0), (300.0, 220.0), (100.0, 220.0)]);
    rig.drag((300.0, 220.0), (320.0, 230.0), ALT);
    assert!(near(rig.quad(), [(120.0, 80.0), (300.0, 80.0), (320.0, 230.0), (100.0, 220.0)]), "a skewed quad's corner moves alone: {:?}", rig.quad());
    rig.set(upright);
    rig.drag((300.0, 220.0), (50.0, 220.0), ALT);
    assert!(
        near(rig.quad(), [(100.0, 80.0), (50.0, 80.0), (50.0, 220.0), (100.0, 220.0)]),
        "Alt past the opposite corner along x keeps the corners as they lie, a mirrored box: {:?}",
        rig.quad()
    );
    assert!(rig.quad().is_some_and(|q| is_valid(&q)), "a mirrored box is valid");
    assert!(rig.enter_applies(), "Enter applies a mirrored box");
    rig.set(upright);
    rig.drag((300.0, 220.0), (50.0, 40.0), ALT);
    assert!(
        near(rig.quad(), [(100.0, 80.0), (50.0, 80.0), (50.0, 40.0), (100.0, 40.0)]),
        "Alt past the opposite corner on both axes keeps the corners as they lie, a box turned upside down: {:?}",
        rig.quad()
    );
    assert!(rig.enter_applies(), "Enter applies an upside-down box");
    // A bow-tie and a quad with three corners on a line are invalid in either winding and do not apply.
    for quad in [[(100.0, 80.0), (300.0, 220.0), (300.0, 80.0), (100.0, 220.0)], [(100.0, 80.0), (200.0, 80.0), (300.0, 80.0), (200.0, 220.0)]] {
        let points = quad.map(|(x, y)| Point::new(x, y));
        assert!(!is_valid(&points) && !is_valid(&[points[0], points[3], points[2], points[1]]), "{quad:?} is invalid");
        rig.set(quad);
        assert!(!rig.enter_applies(), "Enter does not apply {quad:?}");
    }

    // A click on a corner with no drag changes nothing.
    rig.set(upright);
    rig.click((300.0, 220.0));
    assert!(near(rig.quad(), upright), "a click moves nothing: {:?}", rig.quad());
}

#[trial]
fn a_bounded_quad_stays_inside_its_bounds_whatever_moves_it() {
    let upright = [(100.0, 80.0), (300.0, 80.0), (300.0, 220.0), (100.0, 220.0)].map(|(x, y)| Point::new(x, y));
    let (mut fixture, quad) = bounded(upright);
    let value = |quad: &Rc<RefCell<Option<[Point; 4]>>>| quad.borrow().expect("a quad");

    // An Alt corner drag past the bounds' corner: the corner and its moved neighbours stop at the edges.
    drag_on(&mut fixture, (300.0, 220.0), (370.0, 270.0), ALT, ALT);
    let q = value(&quad);
    assert!(inside(&q), "an Alt corner drag stays inside: {q:?}");
    assert!(on_right_edge(q[2].x) && (q[2].y - BOUNDS.y1).abs() < 1e-6, "the dragged corner stops at the edges: {q:?}");
    assert!((q[1].x - q[2].x).abs() < 1e-6 && (q[3].y - q[2].y).abs() < 1e-6 && q[0] == upright[0], "Alt still keeps the rectangle: {q:?}");

    // A whole-quad drag past the bounds keeps its shape: the farthest corner stops at the edge, the rest follow.
    *quad.borrow_mut() = Some(upright);
    fixture.frame(Vec::new());
    drag_on(&mut fixture, (200.0, 150.0), (290.0, 180.0), Modifiers::NONE, Modifiers::NONE);
    let moved = value(&quad);
    assert!(inside(&moved) && same_shape(&upright, &moved), "a moved quad stays inside with its shape: {moved:?}");
    assert!(on_right_edge(moved[1].x) && moved != upright, "the move stops at the edge: {moved:?}");

    // The arrows on the whole quad: one step in, then a far step out stops at the edge.
    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    fixture.frame(Vec::new());
    let left = value(&quad);
    assert!(left[1].x < moved[1].x && same_shape(&moved, &left), "an arrow moves the whole quad: {left:?}");
    fixture.key(Key::ArrowRight, SHIFT);
    fixture.frame(Vec::new());
    let back = value(&quad);
    assert!(inside(&back) && same_shape(&moved, &back) && on_right_edge(back[1].x), "a nudge out stops at the edge: {back:?}");

    // The arrows on a picked corner: a press on it picks it, and a far nudge out stops it at the edge.
    let corner = screen(&fixture, back[2].x, back[2].y);
    fixture.frame(vec![Event::PointerMoved(corner)]);
    button(&mut fixture, corner, true, Modifiers::NONE);
    button(&mut fixture, corner, false, Modifiers::NONE);
    fixture.frame(Vec::new());
    assert_eq!(value(&quad), back, "a click on a corner moves nothing");
    fixture.key(Key::ArrowLeft, SHIFT);
    fixture.frame(Vec::new());
    let pulled = value(&quad);
    assert!(pulled[2].x < back[2].x && pulled[1] == back[1], "an arrow moves the picked corner alone: {pulled:?}");
    for _ in 0..2 {
        fixture.key(Key::ArrowRight, SHIFT);
        fixture.frame(Vec::new());
    }
    let pushed = value(&quad);
    assert!(inside(&pushed) && on_right_edge(pushed[2].x), "a corner nudged out stops at the edge: {pushed:?}");
}

#[trial]
fn a_session_starts_once_and_a_document_switch_discards_it() {
    let mut rig = Rig::new();
    let (first, second) = (CropDocKey::new(1), CropDocKey::new(2));
    rig.document.set(Some(first));
    rig.fixture.frame(Vec::new());
    let started = |events: &[PerspectiveCropEvent]| events.iter().filter(|event| matches!(event, PerspectiveCropEvent::Started)).count();

    rig.click((100.0, 50.0));
    rig.click((300.0, 200.0));
    let events = rig.tool.borrow_mut().take_events();
    assert_eq!(started(&events), 1, "the first click starts the session, the second does not: {events:?}");
    assert!(rig.tool.borrow().pending(), "clicked corners are pending");

    rig.document.set(Some(second));
    rig.fixture.frame(Vec::new());
    let events = rig.tool.borrow_mut().take_events();
    assert!(events.contains(&PerspectiveCropEvent::Discard(first)), "switching documents drops the clicks of the first: {events:?}");
    assert!(!rig.tool.borrow().pending() && rig.placing() == 0, "nothing is left pending");

    rig.drag((20.0, 20.0), (120.0, 120.0), Modifiers::NONE);
    let events = rig.tool.borrow_mut().take_events();
    assert_eq!(started(&events), 1, "a box drag on the new document starts a new session: {events:?}");
    rig.fixture.frame(Vec::new());
    assert!(rig.tool.borrow_mut().take_events().is_empty(), "the same document keeps its quad");
}
