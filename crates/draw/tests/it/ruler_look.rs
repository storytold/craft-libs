//! The drawn look of the Ruler (photocraft#11988): the line and a protractor's arms are each ONE stroke one
//! physical pixel wide in the theme's overlay-stroke ink, with no halo under them; a plain ruler shows no marker at
//! its start or end; a protractor shows a small dot in the line's colour on its vertex, and its arm's free end keeps
//! today's cross.

use craft_behavior::kurbo::{Point, Vec2};
use craft_behavior::{CancelReason, CanvasView, Cursor, CursorIcon};
use craft_draw::ui::measure::{RulerLine, RulerTool, show_ruler};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Color32, Pos2, Rect, Shape};
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

/// The view of a `room`-sized canvas: the document's centre point `(w/2, h/2)` at the room's centre, at `zoom`.
fn view(room: Rect, zoom: f64) -> CanvasView {
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
        |room| view(room, 1.0),
        move |ui, body| {
            tool.borrow_mut().pass(Some(line)).show_line(ui, body.rect);
        },
    )
}

/// The document point at the centre of a `fixture`'s canvas.
fn middle<T: CanvasTool + 'static>(fixture: &ToolFixture<T>) -> Point {
    let rect = fixture.canvas().rect;
    Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0)
}

/// A fixture showing the line `line_of` puts at the canvas's centre, and that line: one throwaway fixture gives
/// the centre, the real one shows the line.
fn painted_centred(line_of: impl Fn(Point) -> RulerLine, zoom: f64) -> (ToolFixture<PaintOnly>, RulerLine) {
    let probe = painted(RulerLine::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)), zoom);
    let line = line_of(middle(&probe));
    (painted(line, zoom), line)
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

/// One frame of `fixture` (whose view runs at `zoom`), with its shapes and the screen point of a document point:
/// the view puts the document's centre at the canvas's centre, so a zoom `z` maps `at` to the centre plus
/// `(at - centre) * z`.
fn frame(fixture: &mut ToolFixture<PaintOnly>, zoom: f64) -> (Vec<Shape>, impl Fn(Point) -> Pos2) {
    fixture.fixture().frame(Vec::new());
    let shapes = fixture.fixture().frame_shapes();
    let rect = fixture.canvas().rect;
    let (w, h) = (f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0);
    let z = zoom as f32;
    let screen = move |p: Point| rect.center() + egui::vec2(((p.x - w) as f32) * z, ((p.y - h) as f32) * z);
    (shapes, screen)
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
    let (mut fixture, line) = painted_centred(plain, 1.0);
    fixture.fixture().pixels_per_point(2.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    let strokes = along(&shapes, screen(line.start), screen(line.end));
    assert_eq!(strokes.len(), 1, "the line is one stroke, with no halo under it: {strokes:?}");
    let (width, color) = strokes[0];
    assert_eq!(width, 0.5, "one physical pixel at 2 device pixels per point is a 0.5 point stroke: {width}");
    assert_eq!(color, ink(), "the line's ink is the theme's overlay stroke");
}

/// The line the Ruler tool's own pass shows (photocraft draws it through `RulerPass::show_line`) follows the same
/// one-stroke rule.
#[trial]
fn the_active_tool_paints_the_line_the_same_way() {
    let probe = painted_by_the_tool(RulerLine::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)));
    let line = plain(middle(&probe));
    let mut fixture = painted_by_the_tool(line);
    fixture.fixture().pixels_per_point(2.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    let strokes = along(&shapes, screen(line.start), screen(line.end));
    assert_eq!(strokes.len(), 1, "one stroke, no halo: {strokes:?}");
    assert_eq!(strokes[0].0, 0.5, "one physical pixel at 2 device pixels per point: {:?}", strokes[0]);
    assert_eq!(strokes[0].1, ink(), "the line's ink is the theme's overlay stroke");
}

/// A plain ruler's ends: nothing painted there — no cross, no dot (the issue: "a plain ruler shows no markers at
/// its start or end").
#[trial]
fn a_plain_ruler_shows_no_marker_at_its_start_or_end() {
    let (mut fixture, line) = painted_centred(plain, 1.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    for (name, at) in [("start", screen(line.start)), ("end", screen(line.end))] {
        assert_eq!(crosses_at(&shapes, at), 0, "no cross at the ruler's {name}");
        assert!(dots_at(&shapes, at).is_empty(), "no dot at the ruler's {name}");
    }
}

/// A protractor's vertex gets a small dot about 7 screen points across, centred on the vertex, in the line's
/// colour; its other ends show no dot.
#[trial]
fn a_protractor_marks_its_vertex_with_a_small_dot_in_the_line_colour() {
    let (mut fixture, line) = painted_centred(with_arm, 1.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
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
    let (mut fixture, line) = painted_centred(with_arm, 1.0);
    fixture.fixture().pixels_per_point(2.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    let arm = line.protractor.expect("the arm");
    for (name, a, b) in [("the line", line.start, line.end), ("the protractor arm", line.start, arm)] {
        let strokes = along(&shapes, screen(a), screen(b));
        assert_eq!(strokes.len(), 1, "{name} is one stroke with no halo: {strokes:?}");
        assert_eq!(strokes[0].0, 0.5, "{name} is one physical pixel at 2 device pixels per point: {:?}", strokes[0]);
        assert_eq!(strokes[0].1, ink(), "{name}'s ink is the theme's overlay stroke");
    }
}

/// The protractor arm's free end keeps today's cross (the issue leaves that marker unmeasured, so its look is
/// unchanged): the two arms of the cross, each the halo stroke under the line stroke.
#[trial]
fn the_arm_end_keeps_its_cross() {
    let (mut fixture, line) = painted_centred(with_arm, 1.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    assert_eq!(crosses_at(&shapes, screen(line.protractor.unwrap())), 4, "the arm end keeps its cross (two arms, halo and line)");
}

/// The line and the vertex dot are screen marks: at other zooms they keep the same physical width and the same
/// screen size (the issue: keep the circle a constant screen size).
#[trial]
fn the_line_and_the_dot_keep_their_screen_size_at_other_zooms() {
    let (probe, line) = painted_centred(with_arm, 1.0);
    drop(probe);
    let dots: Vec<f32> = [0.5f64, 1.0, 2.0]
        .into_iter()
        .map(|zoom| {
            let mut fixture = painted(line, zoom);
            let (shapes, screen) = frame(&mut fixture, zoom);
            let strokes = along(&shapes, screen(line.start), screen(line.end));
            assert_eq!(strokes.len(), 1, "zoom {zoom}: one stroke: {strokes:?}");
            assert_eq!(strokes[0].0, 1.0, "zoom {zoom}: one physical pixel at 1 device pixel per point: {strokes:?}");
            let dots = dots_at(&shapes, screen(line.start));
            assert_eq!(dots.len(), 1, "zoom {zoom}: the vertex dot paints");
            dots[0].0
        })
        .collect();
    assert!(dots.windows(2).all(|pair| pair[0] == pair[1]), "the dot keeps its screen size at every zoom: {dots:?}");
}

/// A drag that has barely begun — ends under 2 screen points apart — draws no line, as the line preview lags
/// today; past the lag the single hairline paints. (The short-line half is a guard: it passes before and after.)
#[trial]
fn a_line_shorter_than_two_screen_points_draws_nothing() {
    let (probe, at_line) = painted_centred(plain, 1.0);
    let at = at_line.start.lerp(at_line.end, 0.5);
    drop(probe);

    let short = RulerLine::new(at - Vec2::new(0.75, 0.0), at + Vec2::new(0.75, 0.0));
    let mut fixture = painted(short, 1.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    assert!(along(&shapes, screen(short.start), screen(short.end)).is_empty(), "under the lag: no line");

    let over = RulerLine::new(at - Vec2::new(1.25, 0.0), at + Vec2::new(1.25, 0.0));
    let mut fixture = painted(over, 1.0);
    let (shapes, screen) = frame(&mut fixture, 1.0);
    let strokes = along(&shapes, screen(over.start), screen(over.end));
    assert_eq!(strokes.len(), 1, "past the lag: one stroke: {strokes:?}");
}
