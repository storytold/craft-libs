//! The warp grid, its options-bar group and the mesh model.

use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{CanvasView, EditEvent};
use craft_imaging::ui::warp::{CylinderValues, CylinderWarp, WarpEdit, WarpGrid, WarpMesh, WarpSplit};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Id, Modifiers, Pos2, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A move of the pointer, in points.
fn shift(dx: f32, dy: f32) -> Vec2 {
    Vec2::new(dx, dy)
}

/// A move in document units.
fn doc_shift(dx: f64, dy: f64) -> DocVec {
    DocVec::new(dx, dy)
}

const RECT: DocRect = DocRect::new(100.0, 80.0, 280.0, 200.0);

/// What one pass reported about the edit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Begin,
    Update,
    Commit,
    Cancel,
    Set,
}

struct Harness {
    fixture: Fixture,
    mesh: Rc<RefCell<WarpMesh>>,
    view: Rc<Cell<Option<CanvasView>>>,
    seen: Rc<RefCell<Vec<Seen>>>,
    edit: Rc<Cell<Option<WarpEdit>>>,
    picked: Rc<Cell<Option<usize>>>,
}

fn harness(mesh: WarpMesh) -> Harness {
    harness_in(mesh, 0.0, false)
}

fn harness_in(mesh: WarpMesh, rotation: f64, flip: bool) -> Harness {
    let id = Rc::new(Cell::new(Id::NULL));
    let mesh = Rc::new(RefCell::new(mesh));
    let view = Rc::new(Cell::new(None));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let edit = Rc::new(Cell::new(None));
    let picked = Rc::new(Cell::new(None));
    let shown = (Rc::clone(&id), Rc::clone(&mesh), Rc::clone(&view), Rc::clone(&seen), Rc::clone(&edit), Rc::clone(&picked));
    let mut fixture = Fixture::new(move |ui| {
        let (id, mesh, view, seen, edit, picked) = &shown;
        let room = ui.available_rect_before_wrap();
        let centre = Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0);
        let shown_view = CanvasView::new(room, centre, 1.0).rotation(rotation).flip_x(flip);
        view.set(Some(shown_view));
        let canvas = Canvas::new("Document").view(shown_view).show(ui, |ui, body| {
            let mut held = mesh.borrow_mut();
            WarpGrid::new("Warp", Bind::bound(&mut *held), RECT).show_at(ui, body.rect)
        });
        if let Some(output) = canvas.inner {
            id.set(output.mesh.response.id);
            if output.edit.is_some() {
                edit.set(output.edit);
            }
            picked.set(output.picked);
            let phases = output.mesh.events.iter().map(|event| match event {
                EditEvent::Begin { .. } => Seen::Begin,
                EditEvent::Update { .. } => Seen::Update,
                EditEvent::Commit { .. } => Seen::Commit,
                EditEvent::Cancel { .. } => Seen::Cancel,
                _ => Seen::Set,
            });
            seen.borrow_mut().extend(phases);
        }
    });
    fixture.settle();
    Harness { fixture, mesh, view, seen, edit, picked }
}

/// Presses at `from`, moves to `to` in four steps and releases.
fn drag_frames(fixture: &mut Fixture, from: Pos2, to: Pos2) {
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 4.0))]);
    }
    fixture.frame(vec![pointer::release(to)]);
    fixture.frame(Vec::new());
}

impl Harness {
    fn screen(&self, p: Point) -> Pos2 {
        self.view.get().expect("drawn").to_screen(p)
    }

    /// The pointer at fractions `(u, v)` across `RECT`.
    fn at_fraction(&self, u: f64, v: f64) -> Pos2 {
        self.screen(Point::new(RECT.x0 + u * RECT.width(), RECT.y0 + v * RECT.height()))
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        drag_frames(&mut self.fixture, from, to);
    }

    fn click(&mut self, at: Pos2, modifiers: Modifiers) {
        self.fixture.hold(modifiers);
        self.fixture.click_at(at);
        self.fixture.hold(Modifiers::NONE);
    }

    fn mesh(&self) -> WarpMesh {
        self.mesh.borrow().clone()
    }

    fn size(&self) -> (usize, usize) {
        let mesh = self.mesh.borrow();
        (mesh.columns(), mesh.rows())
    }

    fn count(&self, phase: Seen) -> usize {
        self.seen.borrow().iter().filter(|seen| **seen == phase).count()
    }
}

#[trial]
fn the_nearest_point_within_reach_takes_the_press_in_a_turned_flipped_view() {
    let reach = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.hit.target;
    for (offset, taken) in [(reach - 0.5, true), (reach + 0.5, false)] {
        let mut h = harness_in(WarpMesh::identity(RECT, 1, 1), 30.0, true);
        // Out of the mesh along the bottom right anchor's diagonal, as the view draws it.
        let anchor = h.screen(Point::new(RECT.x1, RECT.y1));
        let out = (anchor - h.screen(RECT.center())).normalized();
        let at = anchor + out * offset;
        let pull = 20.0_f32;
        h.drag(at, at + out * pull);
        match h.edit.get() {
            Some(WarpEdit::Point { index, by, .. }) => {
                assert!(taken, "{offset} pt");
                assert_eq!(index, 15);
                assert!((by.hypot() - f64::from(pull)).abs() < 1e-3, "{by:?}");
            }
            other => assert!(!taken && other.is_none(), "{offset} pt: {other:?}"),
        }
    }

    // On a small mesh a handle 3 pt away beats its anchor 5 pt away.
    let mut h = harness(WarpMesh::identity(DocRect::new(190.0, 140.0, 214.0, 158.0), 1, 1));
    let at = h.screen(Point::new(195.0, 140.0));
    h.drag(at, at + shift(0.0, -10.0));
    assert!(matches!(h.edit.get(), Some(WarpEdit::Point { index: 1, .. })), "{:?}", h.edit.get());
    assert_eq!(h.picked.get(), Some(1));
}

#[trial]
fn a_press_on_the_surface_pulls_it_at_its_parameters() {
    let mut h = harness(WarpMesh::identity(RECT, 1, 1));
    let from = h.screen(RECT.center());
    h.drag(from, from + shift(20.0, 10.0));
    assert_eq!(h.seen.borrow().first(), Some(&Seen::Begin));
    assert_eq!(h.count(Seen::Commit), 1, "one drag commits once");
    let Some(WarpEdit::Surface { s, t, by, .. }) = h.edit.get() else { panic!("{:?}", h.edit.get()) };
    assert!((s - 0.5).abs() < 1e-3 && (t - 0.5).abs() < 1e-3, "({s}, {t})");
    assert!((by - doc_shift(20.0, 10.0)).hypot() < 1e-3, "{by:?}");
    assert!(h.mesh().eval(0.5, 0.5).distance(RECT.center() + by) < 1e-6, "the pull follows the pointer");

    // Off the mesh and off the points, with no modifier and no armed split, the press does nothing.
    h.edit.set(None);
    let off = h.screen(Point::new(40.0, 40.0));
    h.drag(off, off + shift(10.0, 0.0));
    assert_eq!(h.edit.get(), None);
}

#[trial]
fn alt_or_ctrl_with_no_tool_is_a_quick_split_chosen_by_the_pointer() {
    // In the open a cross.
    let mut h = harness(WarpMesh::identity(RECT, 2, 2));
    let open = h.at_fraction(0.25, 0.25);
    h.click(open, Modifiers::ALT);
    assert!(matches!(h.edit.get(), Some(WarpEdit::Split { split: WarpSplit::Cross, .. })), "{:?}", h.edit.get());
    assert_eq!(h.size(), (3, 3));

    // Close to an existing line of a direction, the perpendicular line.
    for (modifiers, u, v, want) in [(Modifiers::COMMAND, 0.51, 0.25, WarpSplit::Horizontal), (Modifiers::ALT, 0.25, 0.49, WarpSplit::Vertical)] {
        let mut h = harness(WarpMesh::identity(RECT, 2, 2));
        let at = h.at_fraction(u, v);
        h.click(at, modifiers);
        assert!(matches!(h.edit.get(), Some(WarpEdit::Split { split, .. }) if split == want), "{:?}", h.edit.get());
    }

    // Off the mesh, Alt or Cmd splits at the nearest point of the mesh.
    for modifiers in [Modifiers::ALT, Modifiers::COMMAND] {
        let mut h = harness(WarpMesh::identity(RECT, 2, 2));
        let above = h.screen(Point::new(RECT.x0 + 0.25 * RECT.width(), RECT.y0 - 30.0));
        h.click(above, modifiers);
        assert!(matches!(h.edit.get(), Some(WarpEdit::Split { .. })), "{modifiers:?}: {:?}", h.edit.get());
        assert_eq!(h.size().0, 3, "{modifiers:?}: the line is cut at the nearest point's column");
    }

    // A one-patch mesh's thirds lines count as existing lines.
    let mut h = harness(WarpMesh::identity(RECT, 1, 1));
    let at = h.at_fraction(1.0 / 3.0 + 0.01, 0.5);
    h.click(at, Modifiers::ALT);
    assert!(matches!(h.edit.get(), Some(WarpEdit::Split { split: WarpSplit::Horizontal, .. })), "{:?}", h.edit.get());

    // Without the key the press pulls the surface as ever.
    let mut h = harness(WarpMesh::identity(RECT, 1, 1));
    let from = h.at_fraction(0.5, 0.5);
    h.drag(from, from + shift(10.0, 0.0));
    assert!(matches!(h.edit.get(), Some(WarpEdit::Surface { .. })), "{:?}", h.edit.get());
}

fn warped(mesh: &mut WarpMesh) {
    for (k, p) in mesh.points.iter_mut().enumerate() {
        *p += doc_shift((k % 5) as f64 * 3.0, (k % 7) as f64 * 2.0);
    }
}

fn uneven() -> WarpMesh {
    WarpMesh::identity_with_knots(RECT, vec![0.0, 0.1, 0.2, 1.0], vec![0.0, 0.7, 1.0]).unwrap()
}

#[trial]
fn an_uneven_split_keeps_the_surface_where_it_was() {
    let mut mesh = uneven();
    warped(&mut mesh);
    let cut = mesh.split(WarpSplit::Cross, 2, 1, 0.3, 0.6);
    for (s, t) in [(0.05, 0.2), (0.15, 0.8), (0.4, 0.75), (0.9, 0.95), (0.5, 0.9)] {
        assert!(mesh.eval(s, t).distance(cut.eval(s, t)) < 1e-9, "({s}, {t})");
    }
}

#[trial]
fn a_mesh_replaced_during_a_grid_press_cancels_the_press() {
    let mut h = harness(WarpMesh::identity(RECT, 1, 1));
    let from = h.screen(Point::new(RECT.x1, RECT.y1));
    h.fixture.frame(vec![pointer::moved(from)]);
    h.fixture.frame(vec![pointer::press(from)]);
    h.fixture.frame(vec![pointer::moved(from + shift(10.0, 6.0))]);
    h.fixture.frame(vec![pointer::moved(from + shift(14.0, 8.0))]);
    assert_eq!(h.seen.borrow().first(), Some(&Seen::Begin), "the anchor's drag began");
    assert_eq!(h.size(), (1, 1));

    // The host splits the mesh while the press runs; the press holds the old mesh's knots.
    let split = h.mesh().split(WarpSplit::Cross, 0, 0, 0.5, 0.5);
    *h.mesh.borrow_mut() = split.clone();
    for step in [shift(30.0, 20.0), shift(40.0, 30.0)] {
        h.fixture.frame(vec![pointer::moved(from + step)]);
    }
    h.fixture.frame(vec![pointer::release(from + shift(40.0, 30.0))]);
    h.fixture.frame(Vec::new());
    assert_eq!(h.count(Seen::Cancel), 1, "{:?}", h.seen.borrow());
    assert_eq!(h.count(Seen::Commit), 0, "the press does not commit over the split");
    assert_eq!(h.size(), (2, 2), "the moves after the split do not bring the old mesh back");
    assert_eq!(h.mesh().points, split.points, "and do not move its points");
}

/// The cylinder editor over `RECT`'s cylinder, with the edit phases it reported.
struct CylinderHarness {
    fixture: Fixture,
    values: Rc<RefCell<CylinderValues>>,
    view: Rc<Cell<Option<CanvasView>>>,
    seen: Rc<RefCell<Vec<Seen>>>,
}

fn cylinder_harness() -> CylinderHarness {
    let values = Rc::new(RefCell::new(CylinderValues::for_box([RECT.x0, RECT.y0, RECT.x1, RECT.y1])));
    let view = Rc::new(Cell::new(None));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let shown = (Rc::clone(&values), Rc::clone(&view), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let (values, view, seen) = &shown;
        let room = ui.available_rect_before_wrap();
        let centre = Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0);
        let shown_view = CanvasView::new(room, centre, 1.0);
        view.set(Some(shown_view));
        let mesh = WarpMesh::identity(RECT, 1, 1);
        let canvas = Canvas::new("Document").view(shown_view).show(ui, |ui, body| {
            let mut held = values.borrow_mut();
            CylinderWarp::new("Cylinder", Bind::bound(&mut *held), &mesh).show_at(ui, body.rect)
        });
        if let Some(output) = canvas.inner {
            let phases = output.values.events.iter().map(|event| match event {
                EditEvent::Begin { .. } => Seen::Begin,
                EditEvent::Update { .. } => Seen::Update,
                EditEvent::Commit { .. } => Seen::Commit,
                EditEvent::Cancel { .. } => Seen::Cancel,
                _ => Seen::Set,
            });
            seen.borrow_mut().extend(phases);
        }
    });
    fixture.settle();
    CylinderHarness { fixture, values, view, seen }
}

impl CylinderHarness {
    fn screen(&self, x: f64, y: f64) -> Pos2 {
        self.view.get().expect("drawn").to_screen(Point::new(x, y))
    }

    fn values(&self) -> [f64; 7] {
        self.values.borrow().to_array()
    }

    fn count(&self, phase: Seen) -> usize {
        self.seen.borrow().iter().filter(|seen| **seen == phase).count()
    }
}

/// `[left, bottom, right, top, top depth, bottom depth, wrap]` near each other.
fn near(got: [f64; 7], want: [f64; 7], what: &str) {
    assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-3), "{what}: {got:?} against {want:?}");
}

/// A handle case: the handle's place, the drag in document units, the values after and the name.
type HandleCase = ((f64, f64), (f32, f32), [f64; 7], &'static str);

#[trial]
fn cylinder_handles_drag_their_values_and_keep_them_sound() {
    // RECT is 180 wide: the half axis is 90, the centre 190. Depths 0.5, wrap 0.85 (its handle 0.15 of the half axis out).
    let start = [100.0, 200.0, 280.0, 80.0, 0.5, 0.5, 0.85];
    let (top_right, bottom_left, bend, bottom, wrap, surface) =
        ((280.0, 80.0), (100.0, 200.0), (190.0, 125.0), (190.0, 245.0), (203.5, 185.0), (130.0, 160.0));
    // (handle's place, the drag in document units, the values after)
    let cases: [HandleCase; 10] = [
        (top_right, (-30.0, 15.0), [100.0, 200.0, 250.0, 95.0, 0.5, 0.5, 0.85], "the top right corner takes its silhouette and its edge"),
        (bottom_left, (10.0, -20.0), [110.0, 180.0, 280.0, 80.0, 0.5, 0.5, 0.85], "the bottom left corner takes its silhouette and its edge"),
        (bend, (0.0, 9.0), [100.0, 200.0, 280.0, 80.0, 0.6, 0.6, 0.85], "Bend moves both depths by the drag over the half axis"),
        (bend, (0.0, 150.0), [100.0, 200.0, 280.0, 80.0, 1.0, 1.0, 0.85], "a depth stays within the half axis"),
        (bottom, (0.0, 30.0), [100.0, 230.0, 280.0, 80.0, 0.5, 0.5, 0.85], "Bottom moves the lower edge"),
        (bottom, (0.0, -150.0), [100.0, 81.0, 280.0, 80.0, 0.5, 0.5, 0.85], "the lower edge stops 1 short of the top"),
        (wrap, (9.0, 0.0), [100.0, 200.0, 280.0, 80.0, 0.5, 0.5, 0.75], "Wrap follows the pointer along the axis"),
        (surface, (15.0, 5.0), [115.0, 205.0, 295.0, 85.0, 0.5, 0.5, 0.85], "the surface moves all four edges"),
        (top_right, (-190.0, 0.0), [100.0, 200.0, 101.0, 80.0, 0.5, 0.5, 0.85], "the right silhouette stops 1 short of the left"),
        (bottom_left, (190.0, 0.0), [279.0, 200.0, 280.0, 80.0, 0.5, 0.5, 0.85], "the left silhouette stops 1 short of the right"),
    ];
    for (at, (dx, dy), want, what) in cases {
        let mut h = cylinder_harness();
        near(h.values(), start, "the start");
        let from = h.screen(at.0, at.1);
        drag_frames(&mut h.fixture, from, from + shift(dx, dy));
        near(h.values(), want, what);
        assert_eq!(h.seen.borrow().first(), Some(&Seen::Begin), "{what}");
        assert_eq!(h.count(Seen::Commit), 1, "{what}: one drag commits once");
    }

    // A press that does not move changes nothing and begins no edit; one off the cylinder grabs nothing.
    let mut h = cylinder_harness();
    let at = h.screen(top_right.0, top_right.1);
    h.fixture.click_at(at);
    near(h.values(), start, "a click");
    assert_eq!(h.count(Seen::Begin), 0);
    let off = h.screen(40.0, 40.0);
    drag_frames(&mut h.fixture, off, off + shift(10.0, 10.0));
    near(h.values(), start, "a drag off the cylinder");

    // Escape puts the values back to what they were at the press.
    let from = h.screen(bend.0, bend.1);
    h.fixture.frame(vec![pointer::moved(from)]);
    h.fixture.frame(vec![pointer::press(from)]);
    h.fixture.frame(vec![pointer::moved(from + shift(0.0, 9.0))]);
    h.fixture.frame(vec![pointer::moved(from + shift(0.0, 18.0))]);
    assert!(h.values()[4] > 0.5, "the drag moved the depth: {:?}", h.values());
    h.fixture.key(egui::Key::Escape, Modifiers::NONE);
    h.fixture.frame(vec![pointer::release(from + shift(0.0, 18.0))]);
    h.fixture.frame(Vec::new());
    near(h.values(), start, "Escape");
    assert_eq!(h.count(Seen::Cancel), 1);
    assert_eq!(h.count(Seen::Commit), 0);

    // An arrow moves the picked handle one unit, Shift ten, each as one Set; a pass applies the nudge once.
    let mut h = cylinder_harness();
    let at = h.screen(top_right.0, top_right.1);
    h.fixture.click_at(at);
    h.fixture.key(egui::Key::ArrowRight, Modifiers::NONE);
    near(h.values(), [100.0, 200.0, 281.0, 80.0, 0.5, 0.5, 0.85], "an arrow");
    h.fixture.key(egui::Key::ArrowRight, Modifiers::SHIFT);
    near(h.values(), [100.0, 200.0, 291.0, 80.0, 0.5, 0.5, 0.85], "Shift and an arrow");
    assert_eq!(h.count(Seen::Set), 2);
}
