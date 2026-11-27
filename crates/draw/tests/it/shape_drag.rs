//! The Shape tools and the Create Shape dialog. The view maps one document unit to one screen point.

use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::geometry::shape::ShapeGeometry;
use craft_draw::ui::shape::VectorMode;
use craft_draw::ui::shape_drag::{
    BoxConstraint, BoxConstraints, CreateShapeDialog, CreateShapeOptions, CreatedShape, ShapeDragEvent, ShapeDragOptions, ShapeDragTool, ShapeKind,
};
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::{Fixture, ToolFixture, trial};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// The view of a canvas filling `room`, with its middle at the document's middle.
fn room_view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

fn tool(kind: ShapeKind) -> ShapeDragTool {
    let mut tool = ShapeDragTool::new();
    tool.options(ShapeDragOptions::new(kind));
    tool
}

/// A Shape tool on a canvas, with the events it reported so far.
struct Shapes {
    fixture: ToolFixture<ShapeDragTool>,
    log: Vec<ShapeDragEvent>,
}

impl Shapes {
    fn new(options: ShapeDragOptions) -> Self {
        let mut tool = ShapeDragTool::new();
        tool.options(options);
        Shapes { fixture: ToolFixture::with_view("Shape", tool, room_view, |_, _| {}), log: Vec::new() }
    }

    fn sync(&mut self) {
        let events = self.fixture.tool_mut().take_events();
        self.log.extend(events);
    }

    /// The screen point of the document point `p`.
    fn at(&self, p: Point) -> Pos2 {
        room_view(self.fixture.canvas().rect).to_screen(p)
    }

    fn button(&mut self, p: Point, pressed: bool, modifiers: Modifiers) {
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }]);
        self.sync();
    }

    fn press(&mut self, p: Point, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerMoved(pos)]);
        self.button(p, true, modifiers);
    }

    fn move_to(&mut self, p: Point, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        let pos = self.at(p);
        self.fixture.fixture().frame(vec![Event::PointerMoved(pos)]);
        self.sync();
    }

    fn release(&mut self, p: Point, modifiers: Modifiers) {
        self.button(p, false, modifiers);
        self.fixture.fixture().hold(Modifiers::NONE);
        self.fixture.fixture().frame(Vec::new());
        self.sync();
    }

    /// Space goes down or up.
    fn space(&mut self, down: bool) {
        let key = Event::Key { key: egui::Key::Space, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE };
        self.fixture.fixture().frame(vec![key]);
        self.sync();
    }

    /// Press at `from`, move past the press slop, then to `to`, and release there, all with `modifiers`.
    fn drag(&mut self, from: Point, to: Point, modifiers: Modifiers) {
        self.press(from, modifiers);
        self.move_to(from + Vec2::new(40.0, 30.0), modifiers);
        self.move_to(to, modifiers);
        self.release(to, modifiers);
    }

    /// The shape the last drag committed.
    fn committed(&self) -> Option<ShapeGeometry> {
        self.log.iter().rev().find_map(|event| match event {
            ShapeDragEvent::Draw(EditEvent::Commit { after, .. }) => Some(after.clone()),
            _ => None,
        })
    }

    /// The shape of the last `Update`.
    fn proposed(&self) -> Option<ShapeGeometry> {
        self.log.iter().rev().find_map(|event| match event {
            ShapeDragEvent::Draw(EditEvent::Update { value, .. }) => Some(value.clone()),
            _ => None,
        })
    }

    /// The box the last drag committed.
    fn committed_box(&self) -> Option<Rect> {
        self.committed().map(|shape| shape.bounds())
    }

    /// Whether the last gesture ended as a click.
    fn clicked(&self) -> bool {
        matches!(self.log.last(), Some(ShapeDragEvent::Click { .. }))
    }

    fn forget(&mut self) {
        self.log.clear();
    }
}

const PRESS: Point = Point::new(100.0, 100.0);

/// A ⇧ Line drag snaps onto the axis within 30° of it, keeping the offset along it and at least a pixel long, and
/// onto the diagonal at the shorter offset past that; the drag's last proposal is the line it commits.
#[trial]
fn a_shift_line_snaps_to_the_axis_or_the_diagonal_and_commits_what_it_proposed() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Line));
    let line_end = |shape: Option<ShapeGeometry>| match shape {
        Some(ShapeGeometry::Line { from, to, .. }) => Some(to - from),
        _ => None,
    };
    shapes.drag(PRESS, PRESS + Vec2::new(0.95, 0.4), Modifiers::SHIFT);
    assert_eq!(line_end(shapes.committed()), Some(Vec2::new(1.0, 0.0)), "a short drag still draws a 1 px horizontal line");
    assert_eq!(shapes.proposed(), shapes.committed(), "the last proposal is the committed line");

    shapes.forget();
    shapes.drag(PRESS, PRESS + Vec2::new(-60.0, 50.0), Modifiers::SHIFT);
    assert_eq!(line_end(shapes.committed()), Some(Vec2::new(-50.0, 50.0)), "past 30° the line takes the diagonal");
    assert_eq!(shapes.proposed(), shapes.committed());
}

/// ⇧ squares the box from the drag's shorter side, ⇧⌥ from the press as the centre; an Ellipse becomes a circle.
#[trial]
fn shift_squares_the_box_from_the_shorter_side() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle));
    shapes.drag(PRESS, PRESS + Vec2::new(60.0, 20.0), Modifiers::SHIFT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 120.0, 120.0)), "the square takes the shorter side at the press");

    shapes.forget();
    shapes.drag(PRESS, PRESS + Vec2::new(-60.0, 20.0), Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(80.0, 80.0, 120.0, 120.0)), "⇧⌥ centres the square on the press");

    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Ellipse));
    shapes.drag(PRESS, PRESS + Vec2::new(20.0, 60.0), Modifiers::SHIFT);
    assert_eq!(shapes.committed(), Some(ShapeGeometry::Ellipse { rect: Rect::new(100.0, 100.0, 120.0, 120.0) }), "a circle of the shorter side");
}

/// A Rectangle drag under the gear `constraint` with `change` applied to the gear's values.
fn gear(constraint: BoxConstraint, change: impl FnOnce(&mut BoxConstraints)) -> Shapes {
    let mut constraints = BoxConstraints { constraint, ..Default::default() };
    change(&mut constraints);
    Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle).constraints(constraints))
}

/// The gear's Square and From Center hold without ⇧ or ⌥, which turn them off; Fixed Size puts its size at the
/// release; Proportional draws the largest box of its ratio inside the drag; an empty Fixed Size field draws the
/// dragged box.
#[trial]
fn the_rectangle_gear_constrains_the_drag_and_shift_and_alt_turn_it_off() {
    let to = PRESS + Vec2::new(60.0, 20.0);
    let mut shapes = gear(BoxConstraint::Square, |_| {});
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 120.0, 120.0)), "Square holds without ⇧");
    shapes.drag(PRESS, to, Modifiers::SHIFT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "⇧ turns Square off");

    let mut shapes = gear(BoxConstraint::Unconstrained, |c| c.from_center = true);
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(40.0, 80.0, 160.0, 120.0)), "From Center draws about the press");
    shapes.drag(PRESS, to, Modifiers::ALT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "⌥ turns From Center off");

    let mut shapes = gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(30.0), Some(10.0)));
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(160.0, 120.0, 190.0, 130.0)), "Fixed Size puts its top left at the release");

    let mut shapes = gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(30.0), None));
    shapes.drag(PRESS, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 120.0)), "an empty field draws the dragged box");

    let mut shapes = gear(BoxConstraint::Proportional, |c| c.ratio = (Some(2.0), Some(1.0)));
    shapes.drag(PRESS, PRESS + Vec2::new(60.0, 50.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 100.0, 160.0, 130.0)), "the largest 2 : 1 box inside the drag");
}

/// The corners of the triangle `shape` draws, from the apex clockwise.
fn corners(shape: Option<ShapeGeometry>) -> Vec<Point> {
    let Some(shape @ ShapeGeometry::Triangle { .. }) = shape else { return Vec::new() };
    shape.to_path(0.1).elements().iter().filter_map(|el| el.end_point()).collect()
}

/// A Triangle drag fills its box, apex on top; ⇧ draws the equilateral triangle in the circle of the shorter
/// side's square; ⇧⌥ centres it on the press.
#[trial]
fn a_triangle_fills_its_box_and_shift_makes_it_equilateral() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Triangle));
    let to = PRESS + Vec2::new(60.0, 40.0);
    shapes.drag(PRESS, to, Modifiers::NONE);
    let plain = corners(shapes.committed());
    assert_eq!(plain.first(), Some(&Point::new(130.0, 100.0)), "the apex is the box's top centre: {plain:?}");
    assert!(plain.len() >= 3 && plain[1..].iter().all(|p| p.y == 140.0), "the base lies on the bottom edge: {plain:?}");

    shapes.drag(PRESS, to, Modifiers::SHIFT);
    let equal = corners(shapes.committed());
    assert!(equal.len() >= 3, "a triangle: {equal:?}");
    let sides = [(equal[0] - equal[1]).hypot(), (equal[1] - equal[2]).hypot(), (equal[2] - equal[0]).hypot()];
    assert!(sides.iter().all(|side| (side - 20.0 * 3f64.sqrt()).abs() < 1e-9), "three equal sides in the 40 px square's circle: {sides:?}");
    assert!((equal[0] - Point::new(120.0, 100.0)).hypot() < 1e-9, "the apex is the top of the circle, got {:?}", equal[0]);

    shapes.drag(PRESS, to, Modifiers::SHIFT | Modifiers::ALT);
    let centred = corners(shapes.committed());
    assert!(centred.len() >= 3, "a triangle: {centred:?}");
    let centre = (centred[0].to_vec2() + centred[1].to_vec2() + centred[2].to_vec2()) / 3.0;
    assert!((centre.to_point() - PRESS).hypot() < 1e-9, "⇧⌥ puts the centre on the press, got {centre:?}");
}

/// A box drag lands on whole pixels by default; with `whole_pixels` off (Photoshop's Align Edges off) the box keeps
/// the drag's fractions, the preview the same as the commit, and a side under a pixel still draws nothing.
#[trial]
fn whole_pixels_off_keeps_the_drags_fractions() {
    let (from, to) = (PRESS + Vec2::new(0.25, 0.75), PRESS + Vec2::new(30.5, 20.25));
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle));
    shapes.drag(from, to, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 101.0, 130.0, 121.0)), "whole pixels by default");

    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Triangle, ShapeKind::Custom] {
        let mut shapes = Shapes::new(ShapeDragOptions::new(kind).whole_pixels(false));
        shapes.drag(from, to, Modifiers::NONE);
        assert_eq!(shapes.committed_box(), Some(Rect::new(100.25, 100.75, 130.5, 120.25)), "{kind:?} keeps the fractions");
        assert_eq!(shapes.proposed(), shapes.committed(), "{kind:?}: the preview is the commit");
        shapes.forget();
        shapes.drag(from, from + Vec2::new(0.75, 20.0), Modifiers::NONE);
        assert_eq!(shapes.committed(), None, "{kind:?}: a side under a pixel draws nothing");
    }

    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle).whole_pixels(false));
    shapes.drag(from, to, Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(80.75, 81.25, 119.75, 120.25)), "⇧⌥ squares and centres before keeping the fractions");
}

/// A Triangle drag under its gear's `constraint` with `change` applied to the gear's values.
fn triangle_gear(constraint: BoxConstraint, change: impl FnOnce(&mut BoxConstraints)) -> Shapes {
    let mut constraints = BoxConstraints { constraint, ..Default::default() };
    change(&mut constraints);
    Shapes::new(ShapeDragOptions::new(ShapeKind::Triangle).constraints(constraints))
}

/// The box `[x0, y0, x1, y1]` of the triangle the last drag committed, to hundredths.
fn triangle_box(shapes: &Shapes) -> Option<[f64; 4]> {
    match shapes.committed() {
        Some(ShapeGeometry::Triangle { rect }) => Some([rect.x0, rect.y0, rect.x1, rect.y1].map(|v| (v * 100.0).round() / 100.0)),
        _ => None,
    }
}

/// The press of storytold/photocraft#13361's runs, moved from 400,300 to fit the fixture's room: every drag and box
/// below is the issue's, 300 left and 250 up.
const TRIANGLE_PRESS: Point = Point::new(100.0, 50.0);

/// storytold/photocraft#13361: the Triangle gear's Equilateral draws ⇧'s equilateral triangle without ⇧ (D1, D1b), and
/// ⇧ then turns it off (D6); Fixed Size puts its box at the release (D2); Proportional draws in the largest box of its
/// ratio (D3, D3b); From Center draws about the press (D4) and ⌥ turns it off (D5).
#[trial]
fn the_triangle_gear_draws_the_measured_boxes() {
    let at = |x: f64, y: f64| Point::new(x - 300.0, y - 250.0);
    let mut shapes = triangle_gear(BoxConstraint::Square, |_| {});
    shapes.drag(TRIANGLE_PRESS, at(600.0, 400.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([106.7, 50.0, 193.3, 125.0]), "D1: the triangle in the circle of the shorter side's square");
    shapes.drag(TRIANGLE_PRESS, at(450.0, 500.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([103.35, 50.0, 146.65, 87.5]), "D1b: a square of 50, a circle of radius 25");
    shapes.drag(TRIANGLE_PRESS, at(600.0, 400.0), Modifiers::SHIFT);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 300.0, 150.0]), "D6: ⇧ turns Equilateral off");

    let mut shapes = triangle_gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(120.0), Some(60.0)));
    shapes.drag(TRIANGLE_PRESS, at(600.0, 400.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([300.0, 150.0, 420.0, 210.0]), "D2: the top left at the release");

    let mut shapes = triangle_gear(BoxConstraint::Proportional, |c| c.ratio = (Some(2.0), Some(1.0)));
    shapes.drag(TRIANGLE_PRESS, at(600.0, 500.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 300.0, 150.0]), "D3");
    shapes.drag(TRIANGLE_PRESS, at(450.0, 400.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 150.0, 75.0]), "D3b: the largest 2 : 1 box inside the drag");

    let mut shapes = triangle_gear(BoxConstraint::Unconstrained, |c| c.from_center = true);
    shapes.drag(TRIANGLE_PRESS, at(500.0, 350.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([0.0, 0.0, 200.0, 100.0]), "D4: about the press");
    shapes.drag(TRIANGLE_PRESS, at(500.0, 350.0), Modifiers::ALT);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 200.0, 100.0]), "D5: ⌥ turns From Center off");
}

/// A Triangle is equilateral only in a squared box: with a Fixed Size it keeps that size and with a ratio it fills
/// the box of that ratio, ⇧ or not; Equilateral with From Center draws the triangle in the circle about the press.
#[trial]
fn a_triangle_with_a_fixed_size_or_a_ratio_is_not_made_equilateral_by_shift() {
    let mut shapes = triangle_gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(120.0), Some(60.0)));
    shapes.drag(TRIANGLE_PRESS, Point::new(300.0, 150.0), Modifiers::SHIFT);
    assert_eq!(triangle_box(&shapes), Some([300.0, 150.0, 420.0, 210.0]), "⇧ leaves a Fixed Size as it is");

    let mut shapes = triangle_gear(BoxConstraint::Proportional, |c| c.ratio = (Some(2.0), Some(1.0)));
    shapes.drag(TRIANGLE_PRESS, Point::new(300.0, 200.0), Modifiers::SHIFT);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 250.0, 125.0]), "⇧ squares the drag, then the triangle fills its 2 : 1 box");

    // An empty Fixed Size field draws as Unconstrained, where ⇧ still makes the triangle equilateral.
    let mut shapes = triangle_gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(120.0), None));
    shapes.drag(TRIANGLE_PRESS, Point::new(300.0, 150.0), Modifiers::SHIFT);
    assert_eq!(triangle_box(&shapes), Some([106.7, 50.0, 193.3, 125.0]));

    let mut shapes = triangle_gear(BoxConstraint::Square, |c| c.from_center = true);
    shapes.drag(TRIANGLE_PRESS, Point::new(300.0, 150.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([13.4, -50.0, 186.6, 100.0]), "the triangle in the circle about the press");
}

/// A Polygon drag (5 sides) under its gear's `constraint` with `change` applied to the gear's values.
fn polygon_gear(constraint: BoxConstraint, change: impl FnOnce(&mut BoxConstraints)) -> Shapes {
    let mut constraints = BoxConstraints { constraint, ..Default::default() };
    change(&mut constraints);
    Shapes::new(ShapeDragOptions::new(ShapeKind::Polygon).sides(5).constraints(constraints))
}

/// The corners of the polygon the last drag committed, in path order.
fn polygon_corners(shapes: &Shapes) -> Vec<Point> {
    let Some(shape) = shapes.committed() else { return Vec::new() };
    shape.to_path(0.1).elements().iter().filter_map(|el| el.end_point()).collect()
}

/// Asserts the corners the last drag committed are `want` (storytold/photocraft#13306's points moved 300 left and 250
/// up, as [`TRIANGLE_PRESS`]), in order, to 0.01.
fn assert_corners(shapes: &Shapes, want: &[(f64, f64)], what: &str) {
    let got = polygon_corners(shapes);
    let near =
        got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g.x - (w.0 - 300.0)).abs() <= 0.01 && (g.y - (w.1 - 250.0)).abs() <= 0.01);
    assert!(near, "{what}: {got:?} vs {want:?} (moved by -300, -250)");
}

/// The issue's drag end `(x, y)`, moved as [`TRIANGLE_PRESS`].
fn issue(x: f64, y: f64) -> Point {
    Point::new(x - 300.0, y - 250.0)
}

/// H2: Freeform 400,300 → 600,380, centred on 500,340 with its first corner at the release.
const H2: [(f64, f64); 5] = [(600.0, 380.0), (492.86, 447.47), (395.59, 366.42), (442.61, 248.86), (568.94, 257.26)];
/// H3: the same drag centred on the press (circumradius 215.41).
const H3: [(f64, f64); 5] = [(600.0, 380.0), (385.72, 514.93), (191.17, 352.84), (285.22, 117.72), (537.89, 134.51)];

/// storytold/photocraft#13306: the Polygon gear's Symmetric draws the regular polygon in the circle of the shorter
/// side's square (G7) and ⇧ turns it off (H6); Freeform centres the polygon on the middle of the drag with its first
/// corner at the release (G8, H2) and ⌥ centres it on the press (H3); From Center, Proportional and Fixed Size place
/// its box (G9, G10, H5, G11).
#[trial]
fn the_polygon_gear_draws_the_measured_polygons() {
    let mut shapes = polygon_gear(BoxConstraint::Square, |_| {});
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    let g7 = [(450.0, 300.0), (497.55, 334.55), (479.39, 390.45), (420.61, 390.45), (402.45, 334.55)];
    assert_corners(&shapes, &g7, "G7");
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::SHIFT);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 50.0, 300.0, 150.0)), "H6: ⇧ turns Symmetric off");

    let mut shapes = polygon_gear(BoxConstraint::Freeform, |_| {});
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    let g8 = [(600.0, 400.0), (483.35, 460.56), (389.71, 368.33), (448.49, 250.77), (578.45, 270.35)];
    assert_corners(&shapes, &g8, "G8");
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 380.0), Modifiers::NONE);
    assert_corners(&shapes, &H2, "H2");
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 380.0), Modifiers::ALT);
    assert_corners(&shapes, &H3, "H3: ⌥ centres it on the press");

    let mut shapes = polygon_gear(BoxConstraint::Unconstrained, |c| c.from_center = true);
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(-100.0, -50.0, 300.0, 150.0)), "G9: about the press");
    let mut shapes = polygon_gear(BoxConstraint::Proportional, |c| c.ratio = (Some(2.0), Some(1.0)));
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 50.0, 300.0, 150.0)), "G10");
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 500.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(100.0, 50.0, 300.0, 150.0)), "H5");
    let mut shapes = polygon_gear(BoxConstraint::FixedSize, |c| c.fixed = (Some(120.0), Some(60.0)));
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(300.0, 150.0, 420.0, 210.0)), "G11: the top left at the release");
}

/// Freeform with From Center draws about the press and ⌥ then turns it off; ⇧ turns the first corner to a multiple of
/// 45° from the centre at the same distance (not measured but its 21.8° run: straight right); a Radius sets the
/// circumradius; a kind other than the Polygon draws Freeform as Unconstrained.
#[trial]
fn freeform_follows_from_center_alt_shift_and_its_radius() {
    let mut shapes = polygon_gear(BoxConstraint::Freeform, |c| c.from_center = true);
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 380.0), Modifiers::NONE);
    assert_corners(&shapes, &H3, "From Center");
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 380.0), Modifiers::ALT);
    assert_corners(&shapes, &H2, "⌥ turns From Center off");

    let mut shapes = polygon_gear(BoxConstraint::Freeform, |_| {});
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 380.0), Modifiers::SHIFT);
    let flat = [(607.70, 340.0), (533.28, 442.43), (412.87, 403.31), (412.87, 276.69), (533.28, 237.57)];
    assert_corners(&shapes, &flat, "⇧ at 21.8°: straight right, radius 107.70");
    shapes.drag(TRIANGLE_PRESS, issue(500.0, 500.0), Modifiers::SHIFT);
    let (r, s) = (50f64.hypot(100.0), std::f64::consts::FRAC_1_SQRT_2);
    let first = polygon_corners(&shapes).first().copied();
    let want = Point::new(450.0 + r * s - 300.0, 400.0 + r * s - 250.0);
    assert!(first.is_some_and(|p| (p - want).hypot() <= 0.01), "⇧ at 63.4°: the first corner at 45°, got {first:?}");

    let mut shapes = polygon_gear(BoxConstraint::Freeform, |c| c.radius = Some(50.0));
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    let sized = [(544.72, 372.36), (492.55, 399.44), (450.68, 358.20), (476.96, 305.62), (535.09, 314.38)];
    assert_corners(&shapes, &sized, "a Radius of 50 sizes it, the drag turns it");

    let mut constraints = BoxConstraints { constraint: BoxConstraint::Freeform, ..Default::default() };
    constraints.radius = Some(50.0);
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Triangle).constraints(constraints));
    shapes.drag(TRIANGLE_PRESS, issue(600.0, 400.0), Modifiers::NONE);
    assert_eq!(triangle_box(&shapes), Some([100.0, 50.0, 300.0, 150.0]), "a Triangle draws Freeform as Unconstrained");
}

/// A press moved 2 × 1 points and released: a Rectangle in Shape mode draws it, in Path mode it is a click, and an
/// Ellipse in Shape mode keeps the press slop.
#[trial]
fn a_shape_mode_rectangle_ends_a_click_within_one_point() {
    let nudge = |kind: ShapeKind, mode: VectorMode| {
        let mut shapes = Shapes::new(ShapeDragOptions::new(kind).mode(mode));
        let to = PRESS + Vec2::new(2.0, 1.0);
        shapes.press(PRESS, Modifiers::NONE);
        shapes.move_to(to, Modifiers::NONE);
        shapes.release(to, Modifiers::NONE);
        shapes
    };
    let drawn = nudge(ShapeKind::Rectangle, VectorMode::Shape);
    assert!(!drawn.clicked(), "the Rectangle in Shape mode drew: {:?}", drawn.log);
    assert_eq!(drawn.committed_box(), Some(Rect::new(100.0, 100.0, 102.0, 101.0)));
    assert!(nudge(ShapeKind::Rectangle, VectorMode::Path).clicked(), "in Path mode the press slop holds");
    assert!(nudge(ShapeKind::Ellipse, VectorMode::Shape).clicked(), "other kinds keep the press slop");
}

/// Space held moves the box by the pointer's whole motion, the first move after Space went down included; released,
/// the drag resizes again.
#[trial]
fn space_moves_the_box_by_the_full_motion() {
    let mut shapes = Shapes::new(ShapeDragOptions::new(ShapeKind::Rectangle));
    let corner = PRESS + Vec2::new(40.0, 30.0);
    shapes.press(PRESS, Modifiers::NONE);
    shapes.move_to(corner, Modifiers::NONE);
    assert_eq!(shapes.proposed().map(|shape| shape.bounds()), Some(Rect::new(100.0, 100.0, 140.0, 130.0)));

    shapes.space(true);
    shapes.move_to(corner + Vec2::new(10.0, 5.0), Modifiers::NONE);
    assert_eq!(shapes.proposed().map(|shape| shape.bounds()), Some(Rect::new(110.0, 105.0, 150.0, 135.0)), "the first move moves the box");

    shapes.space(false);
    let end = corner + Vec2::new(30.0, 25.0);
    shapes.move_to(end, Modifiers::NONE);
    shapes.release(end, Modifiers::NONE);
    assert_eq!(shapes.committed_box(), Some(Rect::new(110.0, 105.0, 170.0, 155.0)), "without Space the drag resizes from the moved press");
}

#[trial]
fn the_shape_tool_owes_the_canvas_what_every_tool_owes() {
    ToolFixture::with_view("Shape", tool(ShapeKind::Rectangle), room_view, |_, _| {}).check();
}

/// Opens Create Shape for a click of `kind` at (10, 20) and returns its id.
fn open_create(host: &mut DialogHost, kind: ShapeKind) -> DialogId {
    host.open(CreateShapeDialog::new(CreateShapeOptions::initial(kind, Point::new(10.0, 20.0))))
}

/// The fields automation lists for the open dialog `id`.
fn fields(host: &DialogHost, id: DialogId) -> Value {
    host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null)
}

/// OK through the host; the shape it answered.
fn confirm(host: &mut DialogHost, ctx: &egui::Context) -> Option<CreatedShape> {
    assert!(host.confirm::<CreateShapeDialog>(ctx), "OK is enabled");
    host.take::<CreateShapeDialog>().and_then(DialogResult::ok)
}

/// The bounds and star ratio of a polygon or star.
fn star_of(shape: &CreatedShape) -> Option<(Rect, f64)> {
    match shape.geometry {
        ShapeGeometry::Polygon { rect, star_ratio, .. } => Some((rect, star_ratio)),
        _ => None,
    }
}

/// OK remembers width, height, From Center and the star ratio for the kind, and the next open of that kind starts
/// from them (before any pass, as automation meets it); Cancel remembers nothing; the unit picked for a field is not
/// remembered; another kind keeps its own memory.
#[trial]
fn create_shape_ok_remembers_the_size_and_cancel_and_a_picked_unit_do_not() {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();

    let id = open_create(&mut host, ShapeKind::Star);
    assert_eq!(fields(&host, id)["width"], json!(100.0), "a first open starts at 100 × 100 px");
    for (field, value) in [("width", json!(250)), ("height", json!(80)), ("starRatio", json!(40)), ("fromCenter", json!(true))] {
        assert_eq!(host.set_json_field(&ctx, id, field, value), Ok(()), "{field} takes its value");
    }
    assert!(host.set_json_field(&ctx, id, "depth", json!(3)).is_err(), "an unknown field is refused");
    assert_eq!(host.set_json_field(&ctx, id, "widthUnit", json!("inches")), Ok(()));
    let listed = fields(&host, id);
    assert_eq!((listed["widthUnit"].clone(), listed["height"].clone()), (json!("inches"), json!(80.0)), "the set values are listed");
    let shape = confirm(&mut host, &ctx).expect("OK answers the shape");
    let centred = Rect::new(10.0 - 125.0, 20.0 - 40.0, 10.0 + 125.0, 20.0 + 40.0);
    assert_eq!(star_of(&shape), Some((centred, 0.4)), "the answer carries the set values, centred on the click");
    assert!(shape.from_center);

    let id = open_create(&mut host, ShapeKind::Star);
    assert_eq!(host.set_json_field(&ctx, id, "width", json!(300)), Ok(()));
    let listed = fields(&host, id);
    assert_eq!(listed["widthUnit"], json!("pixels"), "the picked unit is not remembered: the width is in pixels again");
    assert_eq!((listed["height"].clone(), listed["starRatio"].clone(), listed["fromCenter"].clone()), (json!(80.0), json!(40.0), json!(true)));
    assert_eq!(host.set_json_field(&ctx, id, "fromCenter", json!(false)), Ok(()));
    assert!(host.cancel::<CreateShapeDialog>(&ctx));
    assert_eq!(host.take::<CreateShapeDialog>(), Some(DialogResult::Cancel));

    open_create(&mut host, ShapeKind::Star);
    let shape = confirm(&mut host, &ctx).expect("OK before any pass answers the remembered values");
    assert_eq!(star_of(&shape), Some((centred, 0.4)), "Cancel remembered nothing: the OK'd size and ratio come back");
    assert!(shape.from_center);

    open_create(&mut host, ShapeKind::Ellipse);
    let ellipse = confirm(&mut host, &ctx).expect("an ellipse answers");
    assert_eq!(ellipse.geometry.bounds(), Rect::new(10.0, 20.0, 110.0, 120.0), "an ellipse keeps its own memory: the defaults");
}

/// A width typed past 1–300,000 px keeps 300,000 and raises the range alert once, for every kind; a width in range
/// raises none.
#[trial]
fn create_shape_alerts_once_for_a_size_typed_past_its_range() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Triangle] {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(egui::Vec2::new(1200.0, 900.0));
        let options = CreateShapeOptions::initial(kind, Point::new(10.0, 20.0)).recalled();
        host.borrow_mut().open(CreateShapeDialog::new(options).alert_title("PhotoCraft"));
        fixture.settle();
        let width = fixture
            .by_role(Role::SPIN_BUTTON)
            .into_iter()
            .find(|field| fixture.tree().name_of(field.node) == Some("Width"))
            .expect("the Width field");
        let type_width = |fixture: &mut Fixture, text: &str| {
            fixture.click(&width).settle();
            fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
            fixture.key(Key::Tab, Modifiers::NONE).settle();
        };
        let width_now = |host: &Rc<RefCell<DialogHost>>| host.borrow().get::<CreateShapeDialog>().map(|dialog| dialog.options().size.0);
        let take = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().get_mut::<CreateShapeDialog>().and_then(CreateShapeDialog::take_range_alert);

        type_width(&mut fixture, "400000");
        assert_eq!(width_now(&host), Some(300_000.0), "{kind:?}: the field keeps the end of the range");
        assert!(take(&host).is_some(), "{kind:?}: the typed width raises the range alert");
        assert!(take(&host).is_none(), "{kind:?}: the alert is taken once");

        type_width(&mut fixture, "500");
        assert_eq!(width_now(&host), Some(500.0));
        assert!(take(&host).is_none(), "{kind:?}: a width in range raises no alert");
    }
}
