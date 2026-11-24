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
    harness_with(mesh, rotation, flip, None)
}

/// A harness with the split tool `tool` armed.
fn harness_with(mesh: WarpMesh, rotation: f64, flip: bool, tool: Option<WarpSplit>) -> Harness {
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
            let grid = WarpGrid::new("Warp", Bind::bound(&mut *held), RECT);
            match tool {
                Some(tool) => grid.split(tool).show_at(ui, body.rect),
                None => grid.show_at(ui, body.rect),
            }
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
    let reach = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).size.hit.target * 1.46875;
    for (offset, taken) in [(reach - 0.5, true), (reach + 0.5, false)] {
        let mut h = harness_in(WarpMesh::identity(RECT, 1, 1), 30.0, true);
        // Out of the mesh along the bottom right anchor's diagonal, as the view draws it. The reach is measured from
        // where the pointer acts, half a document pixel up and left of it, so from half a pixel down and right of the
        // anchor.
        let anchor = h.screen(Point::new(RECT.x1 + 0.5, RECT.y1 + 0.5));
        let out = (anchor - h.screen(RECT.center())).normalized();
        let at = anchor + out * offset;
        let pull = 20.0_f32;
        h.drag(at, at + out * pull);
        match h.edit.get() {
            Some(WarpEdit::Point { index, .. }) => {
                assert!(taken, "{offset} pt");
                assert_eq!(index, 15);
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
fn a_dragged_point_jumps_to_half_a_pixel_up_and_left_of_the_pointer() {
    for (rotation, flip) in [(0.0, false), (30.0, true)] {
        let mut h = harness_in(WarpMesh::identity(RECT, 1, 1), rotation, flip);
        let view = h.view.get().expect("drawn");
        // 9 right of and 3 below the bottom right anchor, outside the mesh.
        let from = view.to_screen(Point::new(RECT.x1 + 9.0, RECT.y1 + 3.0));
        let to = view.to_screen(Point::new(RECT.x1 + 29.0, RECT.y1 + 13.0));
        h.drag(from, to);
        assert!(matches!(h.edit.get(), Some(WarpEdit::Point { index: 15, .. })), "{:?}", h.edit.get());
        let landed = h.mesh().points[15];
        let want = Point::new(RECT.x1 + 28.5, RECT.y1 + 12.5);
        assert!(landed.distance(want) < 1e-3, "{rotation} deg: {landed:?}, want {want:?}");
        assert_eq!(h.count(Seen::Commit), 1);
    }
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

#[trial]
fn a_split_cuts_half_a_pixel_up_and_left_of_the_pointer() {
    let mut h = harness(WarpMesh::identity(RECT, 2, 2));
    let at = h.screen(Point::new(RECT.x0 + 40.0, RECT.y0 + 30.0));
    h.click(at, Modifiers::ALT);
    assert!(matches!(h.edit.get(), Some(WarpEdit::Split { split: WarpSplit::Cross, .. })), "{:?}", h.edit.get());
    let mesh = h.mesh();
    // Within 0.02 of a pixel: the pointer is in f32 screen points.
    let (u, v) = (39.5 / RECT.width(), 29.5 / RECT.height());
    assert!(mesh.us.iter().any(|k| (k - u).abs() < 1e-4), "{:?}, want {u}", mesh.us);
    assert!(mesh.vs.iter().any(|k| (k - v).abs() < 1e-4), "{:?}, want {v}", mesh.vs);
}

#[trial]
fn a_split_less_than_a_pixel_beside_a_line_is_a_new_line_and_one_on_it_is_none() {
    // The 2 x 2 mesh's middle column is at x 190, its patches 90 wide: 0.6 is under 1 % of one.
    let line = RECT.x0 + 0.5 * RECT.width();
    for (beside, inserted) in [(0.6, true), (0.0, false)] {
        let mut h = harness_with(WarpMesh::identity(RECT, 2, 2), 0.0, false, Some(WarpSplit::Vertical));
        // The split cuts half a pixel left of the pointer.
        let at = h.screen(Point::new(line + beside + 0.5, RECT.y0 + 30.0));
        h.click(at, Modifiers::NONE);
        assert_eq!(h.size().0, if inserted { 3 } else { 2 }, "{beside} beside: {:?}", h.mesh().us);
    }
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

/// A canvas tool that takes every key the host offers it, so a test sees which reach the tool.
#[derive(Default)]
struct KeyTaker {
    seen: Vec<craft_ui::canvas::ToolKey>,
}

impl craft_ui::canvas::CanvasTool for KeyTaker {
    fn press(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) -> craft_ui::canvas::ToolAction {
        craft_ui::canvas::ToolAction::Ignored
    }

    fn drag(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn release(&mut self, _: &craft_ui::canvas::ToolPointer, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn cancel(&mut self, _: craft_behavior::CancelReason, _: &mut craft_ui::canvas::ToolContext<'_>) {}

    fn key(&mut self, key: &craft_ui::canvas::ToolKey, _: &mut craft_ui::canvas::ToolContext<'_>) -> bool {
        self.seen.push(key.clone());
        true
    }

    fn paint(&self, _: &craft_ui::primitives::Draw<'_>, _: &craft_ui::canvas::ToolContext<'_>) {}

    fn command_form(&self) -> craft_ui::canvas::ToolPath {
        craft_ui::canvas::ToolPath::Command("the tool's form")
    }
}

#[trial]
fn enter_after_a_point_drag_reaches_the_canvas_tool() {
    // A transform applies its warp on Enter from its canvas tool: the point a drag left focused takes no Enter of its
    // own (no split tool is armed), so the canvas's tool still gets it, as it gets the Escape.
    let mesh = Rc::new(RefCell::new(WarpMesh::identity(RECT, 1, 1)));
    let view = Rc::new(Cell::new(None));
    let (shown_mesh, shown_view) = (Rc::clone(&mesh), Rc::clone(&view));
    let mut canvas = craft_ui::testing::ToolFixture::with_view(
        "Document",
        KeyTaker::default(),
        |rect| CanvasView::new(rect, Point::new(190.0, 140.0), 1.0),
        move |ui, body| {
            if let Some(space) = craft_ui::canvas::canvas_space(&body.env) {
                shown_view.set(Some(space.view));
            }
            WarpGrid::new("Warp", Bind::bound(&mut *shown_mesh.borrow_mut()), RECT).show_at(ui, body.rect);
        },
    );
    canvas.fixture().settle();
    let corner = view.get().expect("drawn").to_screen(Point::new(RECT.x1, RECT.y1));
    drag_frames(canvas.fixture(), corner, corner + Vec2::new(-20.0, -15.0));
    assert_ne!(*mesh.borrow(), WarpMesh::identity(RECT, 1, 1), "the drag moved the corner point");
    assert!(canvas.fixture().focused_stop().is_some(), "the dragged point has focus");
    canvas.fixture().key(egui::Key::Enter, Modifiers::NONE).settle();
    let seen = &canvas.tool().seen;
    assert!(matches!(seen[..], [craft_ui::canvas::ToolKey::Submit]), "Enter reaches the tool: {seen:?}");
}

/// At rest, with the pointer away, the grid draws a dot on each handle of its border (where each edge's curve is pulled
/// from) and a smaller dot on each corner, as the reference application does, and nothing on the interior points. The
/// grid drew handles only near the pointer, so its edges' handles could not be seen until hovered.
#[trial]
fn the_borders_handles_are_drawn_at_rest() {
    let mut h = harness(WarpMesh::identity(RECT, 1, 1));
    let mut found = Vec::new();
    circles_with_radius(&h.fixture.frame_shapes(), &mut found);
    let mesh = h.mesh.borrow().clone();
    let at = |index: usize| h.screen(mesh.points[index]);
    let radius = |index: usize| found.iter().filter(|(c, _)| c.distance(at(index)) < 0.01).map(|(_, r)| *r).fold(0.0f32, f32::max);
    for index in [1usize, 2, 4, 7, 8, 11, 13, 14] {
        assert!(radius(index) > 0.0, "border handle {index} has a dot: {found:?}");
    }
    for corner in [0usize, 3, 12, 15] {
        assert!(radius(corner) > 0.0 && radius(corner) < radius(1), "corner {corner} has a smaller dot: {} vs {}", radius(corner), radius(1));
    }
    for index in [5usize, 6, 9, 10] {
        assert!(radius(index) == 0.0, "interior point {index} has no dot: {found:?}");
    }
}

fn circles_with_radius(shapes: &[egui::Shape], out: &mut Vec<(Pos2, f32)>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => circles_with_radius(inner, out),
            egui::Shape::Circle(circle) => out.push((circle.center, circle.radius)),
            _ => {}
        }
    }
}

/// The straight line from an anchor to each of its border handles is drawn at rest too: the handle ends' dots alone
/// left a pulled edge's curve unexplained, where the reference application shows the arm from the corner to the point
/// the curve is pulled from (along a straight edge it lies on the edge).
#[trial]
fn the_borders_arms_are_drawn_at_rest() {
    let mut mesh = WarpMesh::identity(RECT, 1, 1);
    mesh.points[1].y -= 30.0;
    let mut h = harness(mesh);
    let shapes = h.fixture.frame_shapes();
    let mesh = h.mesh.borrow().clone();
    let at = |index: usize| h.screen(mesh.points[index]);
    fn segments_of(shapes: &[egui::Shape], out: &mut Vec<(Pos2, Pos2)>) {
        for shape in shapes {
            match shape {
                egui::Shape::Vec(inner) => segments_of(inner, out),
                egui::Shape::LineSegment { points, .. } => out.push((points[0], points[1])),
                egui::Shape::Path(path) if path.points.len() == 2 => out.push((path.points[0], path.points[1])),
                _ => {}
            }
        }
    }
    let mut segments: Vec<(Pos2, Pos2)> = Vec::new();
    segments_of(&shapes, &mut segments);
    let arm = |a: usize, b: usize| segments.iter().any(|(p, q)| (p.distance(at(a)) < 0.01 && q.distance(at(b)) < 0.01) || (p.distance(at(b)) < 0.01 && q.distance(at(a)) < 0.01));
    for (anchor, handle) in [(0usize, 1usize), (0, 4), (3, 2), (3, 7), (12, 8), (12, 13), (15, 11), (15, 14)] {
        assert!(arm(anchor, handle), "arm from anchor {anchor} to its border handle {handle}: {segments:?}");
    }
}

/// A view the host scrolls under a surface pull (a drag past the canvas edge) moves the pointer on in the document, and
/// the pull follows the document point under it: the press's document point is kept from the press.
#[trial]
fn a_surface_pull_follows_the_document_point_under_the_pointer_when_the_view_scrolls() {
    let scroll = Rc::new(Cell::new(0.0f64));
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let edit = Rc::new(Cell::new(None::<WarpEdit>));
    let mesh = Rc::new(RefCell::new(WarpMesh::identity(RECT, 1, 1)));
    let shown = (Rc::clone(&scroll), Rc::clone(&view), Rc::clone(&edit), Rc::clone(&mesh));
    let mut fixture = Fixture::new(move |ui| {
        let (scroll, view, edit, mesh) = &shown;
        let room = ui.available_rect_before_wrap();
        let centre = Point::new(f64::from(room.width()) / 2.0 + scroll.get(), f64::from(room.height()) / 2.0);
        let shown_view = CanvasView::new(room, centre, 1.0);
        view.set(Some(shown_view));
        let canvas = Canvas::new("Document").view(shown_view).show(ui, |ui, body| {
            let mut held = mesh.borrow_mut();
            WarpGrid::new("Warp", Bind::bound(&mut *held), RECT).show_at(ui, body.rect)
        });
        if let Some(output) = canvas.inner
            && output.edit.is_some()
        {
            edit.set(output.edit);
        }
    });
    fixture.settle();
    let at = view.get().expect("drawn").to_screen(RECT.center());
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at)]);
    fixture.frame(vec![pointer::moved(at + egui::vec2(20.0, 0.0))]);
    // The view scrolls 30 document units to the right under the pointer, then the pointer moves on by one.
    scroll.set(30.0);
    fixture.frame(Vec::new());
    fixture.frame(vec![pointer::moved(at + egui::vec2(21.0, 0.0))]);
    match edit.get() {
        Some(WarpEdit::Surface { by, .. }) => assert!((by.x - 51.0).abs() < 1e-6 && by.y.abs() < 1e-6, "{by:?}"),
        other => panic!("a surface pull: {other:?}"),
    }
    fixture.frame(vec![pointer::release(at + egui::vec2(21.0, 0.0))]);
}
