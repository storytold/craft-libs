//! The drawn look of the Ruler (photocraft#11988): the line and a protractor's arms are each ONE stroke one
//! physical pixel wide in the theme's overlay-stroke ink, with no halo under them; a plain ruler shows no marker at
//! its start or end; a protractor shows a small dot in the line's colour on its vertex, and its arm's free end keeps
//! today's cross.

use craft_behavior::kurbo::{Point, Vec2};
use craft_behavior::{CancelReason, CanvasView, Cursor, CursorIcon, MotionMode};
use craft_draw::ui::measure::{RulerLine, RulerOverlay, RulerTool, show_ruler};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Color32, Pos2, Shape};
use std::cell::RefCell;
use std::rc::Rc;

/// A tool with no pointer or key behaviour of its own: these trials paint the ruler in the canvas body; the tool's
/// press, drag and key rules are `measure.rs`' trials.
struct PaintOnly;

impl CanvasTool for PaintOnly {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        CursorIcon::Crosshair.into()
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Ruler Line... command")
    }
}

/// The view of a `room`-sized canvas at `zoom`: the document's centre point `(w/2, h/2)` at the room's centre, as
/// `measure.rs`' fixtures make it. Every trial that names exact screen points runs at `zoom` 1, where that view
/// maps a document point to the same screen point; the zoom trial measures the picture relationally instead,
/// because the fixture's painting pass can hand the view callback either a laid-out room or an empty one.
fn view(room: egui::Rect, zoom: f64) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), zoom)
}

/// The Ruler drawn read-only over `line` (what every other tool's body does, through `show_ruler`), at `zoom`.
fn painted(line: RulerLine, zoom: f64) -> ToolFixture<PaintOnly> {
    ToolFixture::with_view(
        "Ruler",
        PaintOnly,
        move |room| view(room, zoom),
        move |ui, body| {
            show_ruler(ui, body.rect, line);
        },
    )
}

/// The Ruler drawn under its own tool: the body runs `RulerPass::show_line`, as photocraft's canvas does.
fn painted_by_the_tool(line: RulerLine) -> ToolFixture<PaintOnly> {
    let tool = Rc::new(RefCell::new(RulerTool::new()));
    ToolFixture::with_view(
        "Ruler",
        PaintOnly,
        move |room| view(room, 1.0),
        move |ui, body| {
            tool.borrow_mut().pass(Some(line)).show_line(ui, body.rect);
        },
    )
}

/// A ruler through `at` along +x, 150 document units long.
fn plain(at: Point) -> RulerLine {
    RulerLine::new(at - Vec2::new(75.0, 0.0), at + Vec2::new(75.0, 0.0))
}

/// `plain` with a protractor arm 60 units down from its start (the vertex).
fn with_arm(at: Point) -> RulerLine {
    let line = plain(at);
    line.with_protractor(line.start + Vec2::new(0.0, 60.0))
}

/// The anchor these trials measure at: every end of the anchor ruler, at zoom 1, lies inside the fixture's
/// 400 x 300 canvas, and the whole figure is away from its edges.
fn anchor() -> Point {
    Point::new(100.0, 80.0)
}

/// One frame of `fixture` at `zoom` 1 (where the view maps document points to the same screen points), returning
/// its shapes and the screen point of a document point.
fn frame(fixture: &mut ToolFixture<PaintOnly>) -> (Vec<Shape>, impl Fn(Point) -> Pos2) {
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    (shapes, |p: Point| Pos2::new(p.x as f32, p.y as f32))
}

/// Every two-point stroked open path in a solid colour: its endpoints, stroke width and colour.
fn paths(shapes: &[Shape]) -> Vec<(Vec<Pos2>, f32, Color32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Path(path) if path.stroke.width > 0.0 && path.points.len() == 2 => match path.stroke.color {
                egui::epaint::ColorMode::Solid(color) => Some((path.points.clone(), path.stroke.width, color)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The paths stroked exactly along `a`-`b` (today's line and its halo share the endpoints): width and colour.
fn along(shapes: &[Shape], a: Pos2, b: Pos2) -> Vec<(f32, Color32)> {
    let near = |p: Pos2, at: Pos2| (p - at).length() < 0.6;
    paths(shapes)
        .into_iter()
        .filter(|(points, _, _)| (near(points[0], a) && near(points[1], b)) || (near(points[0], b) && near(points[1], a)))
        .map(|(_, width, color)| (width, color))
        .collect()
}

/// The cross arms centred on `at`: two-point paths whose middle lies on it. The ruler line is centred on the
/// segment's middle, away from every end of a measured line, and a cross arm runs straight through its centre.
fn crosses_at(shapes: &[Shape], at: Pos2) -> usize {
    paths(shapes).into_iter().filter(|(points, _, _)| ((points[0].to_vec2() + points[1].to_vec2()) * 0.5 - at.to_vec2()).length() < 0.6).count()
}

/// Filled circles centred within 0.5 of `at`: radius and fill.
fn dots_at(shapes: &[Shape], at: Pos2) -> Vec<(f32, Color32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Circle(circle) if (circle.center - at).length() < 0.5 && circle.stroke.width == 0.0 => Some((circle.radius, circle.fill)),
            _ => None,
        })
        .collect()
}

/// The theme's `overlay.stroke` as a `Color32`: the ink the ruler's one-pixel line and its marks paint in.
fn ink() -> Color32 {
    craft_ui::style::rgba_color(craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard).color.overlay.stroke)
}

/// The measured line: painted once, one physical pixel wide — at 2 device pixels per point a stroke of 0.5 points,
/// where the theme's `Thin` line would already be 2 physical pixels — in `overlay.stroke`, with no wider halo
/// stroke under it.
#[trial]
fn the_ruler_line_is_one_physical_pixel_painted_once_with_no_halo() {
    let line = plain(anchor());
    let mut fixture = painted(line, 1.0);
    fixture.fixture().pixels_per_point(2.0);
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    let strokes = along(&shapes, Pos2::new(25.0, 80.0), Pos2::new(175.0, 80.0));
    assert_eq!(strokes.len(), 1, "the line is one stroke, with no halo under it: {strokes:?}");
    let (width, color) = strokes[0];
    assert_eq!(width, 0.5, "one physical pixel at 2 device pixels per point is a 0.5 point stroke: {width}");
    assert_eq!(color, ink(), "the line's ink is the theme's overlay stroke");
}

/// The line the Ruler tool's own pass shows (photocraft draws it through `RulerPass::show_line`) follows the same
/// one-stroke rule.
#[trial]
fn the_active_tool_paints_the_line_the_same_way() {
    let line = plain(anchor());
    let mut fixture = painted_by_the_tool(line);
    fixture.fixture().pixels_per_point(2.0);
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    let strokes = along(&shapes, Pos2::new(25.0, 80.0), Pos2::new(175.0, 80.0));
    assert_eq!(strokes.len(), 1, "one stroke, no halo: {strokes:?}");
    assert_eq!(strokes[0].0, 0.5, "one physical pixel at 2 device pixels per point: {:?}", strokes[0]);
    assert_eq!(strokes[0].1, ink(), "the line's ink is the theme's overlay stroke");
}

/// A plain ruler's ends: nothing painted there — no cross, no dot (the issue: "a plain ruler shows no markers at
/// its start or end").
#[trial]
fn a_plain_ruler_shows_no_marker_at_its_start_or_end() {
    let line = plain(anchor());
    let mut fixture = painted(line, 1.0);
    let (shapes, screen) = frame(&mut fixture);
    for (name, at) in [("start", screen(line.start)), ("end", screen(line.end))] {
        assert_eq!(crosses_at(&shapes, at), 0, "no cross at the ruler's {name}");
        assert!(dots_at(&shapes, at).is_empty(), "no dot at the ruler's {name}");
    }
}

/// A protractor's vertex gets a small dot about 7 screen points across, centred on the vertex, in the line's
/// colour; its other ends show no dot.
#[trial]
fn a_protractor_marks_its_vertex_with_a_small_dot_in_the_line_colour() {
    let line = with_arm(anchor());
    let mut fixture = painted(line, 1.0);
    let (shapes, screen) = frame(&mut fixture);
    let dots = dots_at(&shapes, screen(line.start));
    assert_eq!(dots.len(), 1, "one dot on the vertex");
    let (radius, fill) = dots[0];
    assert!((2.5..=4.5).contains(&radius), "the measured circle is about 7 screen px across: radius {radius}");
    assert_eq!(fill, ink(), "the dot is in the same colour as the line");
    assert!(dots_at(&shapes, screen(line.end)).is_empty(), "the dot is on the vertex, not on the line's end");
    assert!(dots_at(&shapes, screen(line.protractor.unwrap())).is_empty(), "and not on the protractor arm's end");
}

/// A protractor's two arms are each one physical pixel with no halo (measured at 2 device pixels per point).
#[trial]
fn a_protractor_draws_its_arms_as_one_physical_pixel_each() {
    let line = with_arm(anchor());
    let mut fixture = painted(line, 1.0);
    fixture.fixture().pixels_per_point(2.0);
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    let (start, end, arm) = (Pos2::new(25.0, 80.0), Pos2::new(175.0, 80.0), Pos2::new(25.0, 140.0));
    for (name, a, b) in [("the line", start, end), ("the protractor arm", start, arm)] {
        let strokes = along(&shapes, a, b);
        assert_eq!(strokes.len(), 1, "{name} is one stroke with no halo: {strokes:?}");
        assert_eq!(strokes[0].0, 0.5, "{name} is one physical pixel at 2 device pixels per point: {:?}", strokes[0]);
        assert_eq!(strokes[0].1, ink(), "{name}'s ink is the theme's overlay stroke");
    }
}

/// The protractor arm's free end keeps today's cross (the issue leaves that marker unmeasured, so its look is
/// unchanged): the two arms of the cross, each the halo stroke under the line stroke.
#[trial]
fn the_arm_end_keeps_its_cross() {
    let line = with_arm(anchor());
    let mut fixture = painted(line, 1.0);
    let (shapes, screen) = frame(&mut fixture);
    assert_eq!(crosses_at(&shapes, screen(line.protractor.unwrap())), 4, "the arm end keeps its cross (two arms, halo and line)");
}

/// The line and the vertex dot are screen marks at every zoom: the line is one physical pixel wide and `150 * zoom`
/// screen points long, the dot is the scene's only circle, sitting on the end the two arms share, and the same at
/// 50 %, 100 % and 200 % (the issue: keep the circle a constant screen size). The picture is read by its relations
/// (lengths, shared ends), not absolute points, because the fixture's view room differs between passes.
#[trial]
fn the_line_and_the_dot_keep_their_screen_size_at_other_zooms() {
    let line = with_arm(anchor());
    let mut seen: Vec<(f32, f32)> = Vec::new();
    for zoom in [0.5f64, 1.0, 2.0] {
        let long = 150.0 * zoom as f32;
        let arm = 60.0 * zoom as f32;
        let mut fixture = painted(line, zoom);
        fixture.fixture().frame(Vec::new());
        let shapes = fixture.fixture().frame_shapes();
        let main: Vec<_> = paths(&shapes)
            .into_iter()
            .filter(|(points, width, color)| ((points[0] - points[1]).length() - long).abs() < 0.6 && *width == 1.0 && *color == ink())
            .collect();
        assert_eq!(main.len(), 1, "zoom {zoom}: one stroke, one physical pixel wide, {long} points long: {main:?}");
        let dots: Vec<_> = shapes.iter().filter(|shape| matches!(shape, Shape::Circle(c) if c.stroke.width == 0.0)).collect();
        assert_eq!(dots.len(), 1, "zoom {zoom}: the vertex dot is the scene's only circle");
        let Shape::Circle(dot) = dots[0] else { unreachable!() };
        let vertex = main[0]
            .0
            .iter()
            .find(|end| (dot.center - **end).length() < 0.6)
            .unwrap_or_else(|| panic!("zoom {zoom}: the dot sits on an end of the line: {:?} / {:?}", dot.center, main[0].0));
        assert!(
            paths(&shapes)
                .iter()
                .any(|(points, _, _)| (dot.center - points[0]).length() < 0.6 && ((points[1] - points[0]).length() - arm).abs() < 0.6),
            "zoom {zoom}: the arm starts at the dotted vertex and is {arm} points long: {vertex:?}"
        );
        seen.push((main[0].1, dot.radius));
    }
    assert!(seen.iter().all(|(width, _)| *width == seen[0].0), "the line keeps its one-pixel width at every zoom: {seen:?}");
    assert!(seen.windows(2).all(|pair| pair[0].1 == pair[1].1), "the dot keeps its screen size at every zoom: {seen:?}");
}

/// The Ruler overlay shown interactive (its own tool's control), over the fixed `line`: the body binds the
/// overlay to the line and drops its edit, so the marks under the pointer stay put while they are dragged.
fn interactive(line: RulerLine) -> ToolFixture<PaintOnly> {
    let held = Rc::new(RefCell::new(Some(line)));
    ToolFixture::with_view(
        "Ruler",
        PaintOnly,
        move |room| view(room, 1.0),
        move |ui, body| {
            let mut value = *held.borrow();
            RulerOverlay::new("Ruler", Bind::bound(&mut value)).show_at(ui, body.rect);
        },
    )
}

/// The theme's accent hover and pressed: the colours a hovered or dragged point paints its mark in.
fn accent() -> (Color32, Color32) {
    let tokens = craft_ui::testing::style::tokens(craft_theme::Mode::Light, craft_theme::Contrast::Standard);
    (craft_ui::style::rgba_color(tokens.color.accent.hover), craft_ui::style::rgba_color(tokens.color.accent.pressed))
}

/// The vertex dot reads the Start point's own state, like the arm end's cross: it keeps the line's colour while
/// the pointer is away from the vertex, turns the accent while the vertex is pressed and dragged, and takes the
/// line's colour back once the drag ends and the point rests.
#[trial]
fn the_vertex_dot_turns_accent_while_the_vertex_is_dragged() {
    let line = with_arm(anchor());
    let mut fixture = interactive(line);
    fixture.fixture().motion(MotionMode::Off);
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    let vertex = Pos2::new(line.start.x as f32, line.start.y as f32); // zoom 1: the screen point of the document point
    let rest = dots_at(&shapes, vertex);
    assert_eq!(rest.len(), 1, "one dot on the vertex");
    assert_eq!(rest[0].1, ink(), "at rest the dot keeps the line's colour");

    let (hover, pressed) = accent();
    fixture.press_at(vertex);
    let dragged = vertex + egui::vec2(12.0, 8.0);
    fixture.move_to(dragged);
    let dragging = dots_at(&fixture.fixture().frame_shapes(), vertex);
    assert_eq!(dragging.len(), 1, "the dot stays on the vertex through the drag");
    assert!(dragging[0].1 == hover || dragging[0].1 == pressed, "while the vertex is dragged the dot is the accent: {:?}", dragging[0].1);

    fixture.release_at(dragged);
    fixture.move_to(vertex + egui::vec2(150.0, 120.0));
    fixture.fixture().frame(Vec::new());
    let after = dots_at(&fixture.fixture().frame_shapes(), vertex);
    assert_eq!(after.len(), 1, "the dot is on the vertex again after the release");
    assert_eq!(after[0].1, ink(), "the dot takes the line's colour back once the point rests");
}

/// A drag that has barely begun — ends under 2 screen points apart — draws no line, as the line preview lags
/// today; past the lag the single hairline paints. (The short-line half is a guard: it passes before and after.)
#[trial]
fn a_line_shorter_than_two_screen_points_draws_nothing() {
    let at = anchor();
    let short = RulerLine::new(at - Vec2::new(0.75, 0.0), at + Vec2::new(0.75, 0.0));
    let mut fixture = painted(short, 1.0);
    let (shapes, screen) = frame(&mut fixture);
    assert!(along(&shapes, screen(short.start), screen(short.end)).is_empty(), "under the lag: no line");

    let over = RulerLine::new(at - Vec2::new(1.25, 0.0), at + Vec2::new(1.25, 0.0));
    let mut fixture = painted(over, 1.0);
    let (shapes, screen) = frame(&mut fixture);
    let strokes = along(&shapes, screen(over.start), screen(over.end));
    assert_eq!(strokes.len(), 1, "past the lag: one stroke: {strokes:?}");
}
