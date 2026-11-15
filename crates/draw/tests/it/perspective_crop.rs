//! The Perspective Crop quad and tool through the public API: the pure quad rules, the quad's gestures and keys as a
//! canvas control, and the tool's box drag and four clicks on the background.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::{Point, Size};
use craft_draw::ui::crop::{CropDocKey, PerspectiveCropEvent, PerspectiveCropTool, PerspectiveInput, grid_lines, grid_steps, is_valid};
use craft_ui::canvas::{Canvas, ToolHost};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};

const SIZE: Size = Size::new(400.0, 300.0);
const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
const ALT: Modifiers = Modifiers { alt: true, ..Modifiers::NONE };
const BOTH: Modifiers = Modifiers { shift: true, alt: true, ..Modifiers::NONE };

/// The tool on a canvas showing its quad, driven by pointer events.
struct Rig {
    fixture: Fixture,
    tool: Rc<RefCell<PerspectiveCropTool>>,
    /// The active document the pass is given; `None` keys nothing.
    document: Rc<Cell<Option<CropDocKey>>>,
}

impl Rig {
    fn new() -> Self {
        let tool = Rc::new(RefCell::new(PerspectiveCropTool::new()));
        let document = Rc::new(Cell::new(None));
        let (shown, active) = (Rc::clone(&tool), Rc::clone(&document));
        let mut fixture = Fixture::new(move |ui| {
            let mut tool = shown.borrow_mut();
            let room = ui.available_rect_before_wrap();
            let view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
            let mut input = PerspectiveInput::default();
            input.document = active.get();
            let mut pass = tool.pass(input);
            let canvas = Canvas::new("Document").document(SIZE).view(view).show(ui, |ui, body| {
                let _ = pass.show_quad(ui, body.rect);
            });
            let _ = ToolHost::new().run(ui, &canvas.output, &mut pass);
        });
        fixture.settle();
        Rig { fixture, tool, document }
    }

    /// The screen point of document point `(x, y)`: the view shows (200, 150) at the canvas's centre.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        let canvas = self.fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas");
        canvas.center() + Vec2::new(x as f32 - 200.0, y as f32 - 150.0)
    }

    fn button(&mut self, at: Pos2, pressed: bool, modifiers: Modifiers) {
        self.fixture.hold(modifiers);
        self.fixture.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }]);
    }

    /// A drag over four moves; the last move and the release hold `last`, the others `first`.
    fn drag_with(&mut self, from: (f64, f64), to: (f64, f64), first: Modifiers, last: Modifiers) {
        let (from, to) = (self.at(from.0, from.1), self.at(to.0, to.1));
        self.fixture.hold(first);
        self.fixture.frame(vec![Event::PointerMoved(from)]);
        self.button(from, true, first);
        for step in 1..=4 {
            let mods = if step == 4 { last } else { first };
            self.fixture.hold(mods);
            self.fixture.frame(vec![Event::PointerMoved(from + (to - from) * (step as f32 / 4.0))]);
        }
        self.button(to, false, last);
        self.fixture.hold(Modifiers::NONE);
        self.fixture.frame(Vec::new());
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
}

fn near(got: Option<[Point; 4]>, want: [(f64, f64); 4]) -> bool {
    got.is_some_and(|q| q.iter().zip(want).all(|(p, (x, y))| (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3))
}

fn rectangle((a, b): (Point, Point)) -> [(f64, f64); 4] {
    let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
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
    rig.drag((300.0, 220.0), (50.0, 40.0), ALT);
    assert!(rig.quad().is_some_and(|q| is_valid(&q)), "Alt past the opposite corner stays a valid quad: {:?}", rig.quad());

    // A click on a corner with no drag changes nothing.
    rig.set(upright);
    rig.click((300.0, 220.0));
    assert!(near(rig.quad(), upright), "a click moves nothing: {:?}", rig.quad());
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
