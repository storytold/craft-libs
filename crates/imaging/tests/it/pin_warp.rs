//! Puppet Warp: the session on a recording solver, the pins on a canvas and the options bar.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, EditEvent};
use craft_imaging::ui::warp::pin::{Pin, PinEdit, PinMeshView, PinSolver, PinWarp, PinWarpOptions, PinWarpSession, PinWarpSettings};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Modifiers, Pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Debug, PartialEq)]
enum Call {
    Rebind(Vec<Pin>),
    Solve(Vec<Pin>),
    Remesh(PinWarpOptions),
}

/// A solver that records its calls over a mesh of two triangles.
#[derive(Default)]
struct Fake {
    calls: Vec<Call>,
    rest: Vec<Point>,
    deformed: Vec<Point>,
    triangles: Vec<[u32; 3]>,
    order: Vec<u32>,
}

impl PinSolver for Fake {
    fn rebind(&mut self, pins: &[Pin], _: &PinWarpOptions) {
        self.calls.push(Call::Rebind(pins.to_vec()));
    }
    fn solve(&mut self, pins: &[Pin], _: &PinWarpOptions) {
        self.calls.push(Call::Solve(pins.to_vec()));
    }
    fn remesh(&mut self, options: &PinWarpOptions) -> Result<(), String> {
        self.calls.push(Call::Remesh(*options));
        Ok(())
    }
    fn mesh(&self) -> PinMeshView<'_> {
        PinMeshView::new(&self.rest, &self.deformed, &self.triangles, &self.order)
    }
}

impl Fake {
    /// A rectangle mesh of two triangles from (40, 40) to (360, 260), undeformed.
    fn flat() -> Self {
        let rest = vec![Point::new(40.0, 40.0), Point::new(360.0, 40.0), Point::new(360.0, 260.0), Point::new(40.0, 260.0)];
        Fake { deformed: rest.clone(), rest, triangles: vec![[0, 1, 2], [0, 2, 3]], order: vec![0, 1], ..Fake::default() }
    }
}

fn pin(x: f64, y: f64) -> Pin {
    Pin::new(Point::new(x, y), Point::new(x, y))
}

fn session() -> PinWarpSession {
    PinWarpSession::new(&PinWarpSettings::default())
}

fn set(s: &mut PinWarpSession, f: &mut Fake, pins: Vec<Pin>) {
    let edit = PinEdit::replace(s.pins(), pins, "Pins");
    s.apply(edit, f).unwrap();
}

#[trial]
fn a_click_on_the_mesh_places_a_pin_at_the_rest_position_under_it() {
    let rest = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)];
    // The deformed triangle is the rest one shifted by (100, 0) and drawn twice: the later entry of the order is on
    // top and wins.
    let deformed = [Point::new(100.0, 0.0), Point::new(110.0, 0.0), Point::new(100.0, 10.0)];
    let rest2 = [Point::new(50.0, 50.0), Point::new(60.0, 50.0), Point::new(50.0, 60.0)];
    let rest_all = [rest, rest2].concat();
    let deformed_all = [deformed, deformed].concat();
    let triangles = [[0, 1, 2], [3, 4, 5]];
    let view = |order: &'static [u32]| PinMeshView::new(&rest_all, &deformed_all, &triangles, order);
    let under = Point::new(102.0, 3.0);
    let top_last = view(&[0, 1]).rest_of(under).unwrap();
    assert!((top_last.x - 52.0).abs() < 1e-9 && (top_last.y - 53.0).abs() < 1e-9);
    let top_first = view(&[1, 0]).rest_of(under).unwrap();
    assert!((top_first.x - 2.0).abs() < 1e-9 && (top_first.y - 3.0).abs() < 1e-9);
    assert_eq!(view(&[0, 1]).rest_of(Point::new(0.0, 0.0)), None, "off the mesh");
}

#[trial]
fn alt_click_deletes_a_pin_as_one_step() {
    let (mut s, mut f) = (session(), Fake::default());
    let pins = vec![pin(1.0, 1.0), pin(2.0, 2.0), pin(3.0, 3.0)];
    set(&mut s, &mut f, pins.clone());
    s.apply(PinEdit::Select(vec![0, 2, 9]), &mut f).unwrap();
    assert_eq!(s.selected(), &[0, 2], "out-of-range indices are dropped");
    f.calls.clear();
    set(&mut s, &mut f, vec![pins[0], pins[2]]);
    assert_eq!(f.calls.len(), 1);
    assert!(matches!(f.calls[0], Call::Rebind(_)));
    assert_eq!(s.selected(), &[0, 1]);
    set(&mut s, &mut f, vec![pins[2]]);
    assert_eq!(s.selected(), &[0], "pin 0 was deleted and pin 2 slid down");
    assert!(s.undo(&mut f));
    assert_eq!(s.pins().len(), 2);
}

#[trial]
fn turn_at_reads_the_rotation_of_the_triangle_under_a_rest_point() {
    let rest = vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 100.0)];
    let (sin, cos) = 25f64.to_radians().sin_cos();
    // Stretched 2 x along x, then rotated 25 degrees.
    let deformed: Vec<Point> = rest.iter().map(|p| Point::new(2.0 * p.x * cos - p.y * sin, 2.0 * p.x * sin + p.y * cos)).collect();
    let fake = Fake { rest, deformed, triangles: vec![[0, 1, 2]], order: vec![0], ..Fake::default() };
    let turn = fake.mesh().turn_at(Point::new(20.0, 20.0)).expect("on the mesh");
    assert!((turn - 25.0).abs() < 1e-9, "{turn}");
    assert!(fake.mesh().turn_at(Point::new(90.0, 90.0)).is_none());
}

/// What one pass reported about the pin edit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Begin,
    Commit,
    Cancel,
    Set,
}

struct State {
    session: PinWarpSession,
    solver: Fake,
    seen: Vec<Seen>,
}

struct Harness {
    fixture: Fixture,
    state: Rc<RefCell<State>>,
    view: Rc<Cell<Option<CanvasView>>>,
}

fn harness(pins: Vec<Pin>, selected: Vec<usize>) -> Harness {
    let mut solver = Fake::flat();
    let mut session = PinWarpSession::from_pins(pins, &PinWarpSettings::default());
    session.apply(PinEdit::Select(selected), &mut solver).expect("selects");
    let state = Rc::new(RefCell::new(State { session, solver, seen: Vec::new() }));
    let view = Rc::new(Cell::new(None));
    let (shown_state, shown_view) = (Rc::clone(&state), Rc::clone(&view));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let canvas_view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        shown_view.set(Some(canvas_view));
        let output = {
            let held = shown_state.borrow();
            Canvas::new("Document")
                .view(canvas_view)
                .show(ui, |ui, body| {
                    PinWarp::new("Pins", Bind::given(held.session.pins().to_vec()), held.solver.mesh())
                        .selected_pins(held.session.selected())
                        .show_mesh(true)
                        .show_at(ui, body.rect)
                })
                .inner
        };
        let Some(output) = output else { return };
        let mut held = shown_state.borrow_mut();
        let State { session, solver, seen } = &mut *held;
        for edit in output.edits() {
            if let PinEdit::Pins(event) = &edit {
                match event {
                    EditEvent::Begin { .. } => seen.push(Seen::Begin),
                    EditEvent::Commit { .. } => seen.push(Seen::Commit),
                    EditEvent::Cancel { .. } => seen.push(Seen::Cancel),
                    EditEvent::Set { .. } => seen.push(Seen::Set),
                    _ => {}
                }
            }
            session.apply(edit, solver).expect("applies");
        }
    });
    fixture.settle();
    Harness { fixture, state, view }
}

impl Harness {
    fn at(&self, x: f64, y: f64) -> Pos2 {
        self.view.get().expect("shown").to_screen(Point::new(x, y))
    }

    /// Presses at `from`, moves to `to` in two steps and releases, with `mods` held.
    fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
        let (a, b) = (self.at(from.0, from.1), self.at(to.0, to.1));
        self.fixture.hold(mods);
        self.fixture.frame(vec![pointer::moved(a)]);
        self.fixture.frame(vec![pointer::press(a)]);
        self.fixture.frame(vec![pointer::moved(a.lerp(b, 0.5))]);
        self.fixture.frame(vec![pointer::moved(b)]);
        self.fixture.frame(vec![pointer::release(b)]);
        self.fixture.frame(Vec::new());
        self.fixture.hold(Modifiers::NONE);
    }

    fn seen(&self) -> Vec<Seen> {
        self.state.borrow().seen.clone()
    }
}

/// The radius of the ring a pin's fixed rotation is set on, from the tokens.
fn turn_ring() -> f64 {
    f64::from(craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.handle.ring)
}

/// How far inside or outside the turn ring a press is placed.
const RING_MARGIN: f64 = 1.0;

#[trial]
fn alt_drag_near_a_selected_pin_turns_it_and_fixes_its_rotation() {
    let ring = turn_ring();
    let (centre, release) = (Point::new(200.0, 150.0), Point::new(200.0, 190.0));
    let press = Point::new(centre.x + ring - RING_MARGIN, centre.y);
    // The turn is the angle the drag sweeps around the pin, from the press to the release.
    let swept = ((release.y - centre.y).atan2(release.x - centre.x) - (press.y - centre.y).atan2(press.x - centre.x)).to_degrees();
    let mut h = harness(vec![pin(centre.x, centre.y)], vec![0]);
    h.drag((press.x, press.y), (release.x, release.y), Modifiers::ALT);
    let turned = h.state.borrow().session.pins()[0];
    assert!((turned.angle.expect("fixed") - swept).abs() < 1e-9, "{:?} vs {swept}", turned.angle);
    assert_eq!(turned.at, centre, "the pin stays where it is");
    assert_eq!(h.seen(), [Seen::Begin, Seen::Commit]);

    h.state.borrow_mut().seen.clear();
    h.drag((centre.x + ring + RING_MARGIN, centre.y), (release.x, release.y), Modifiers::ALT);
    assert!(h.seen().is_empty(), "just outside the turn ring the press turns nothing: {:?}", h.seen());

    h.drag((300.0, 150.0), (300.0, 190.0), Modifiers::ALT);
    assert!(h.seen().is_empty(), "far from the pin the press turns nothing: {:?}", h.seen());
}
