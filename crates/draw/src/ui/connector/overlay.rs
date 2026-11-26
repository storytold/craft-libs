//! What the connector tool and the connector handles paint over the document: the route, the end and
//! bend handles and the attach points of a target. Painting functions over `Draw`, `pub(crate)`.

use craft_behavior::CanvasView;
use craft_ui::canvas_controls::path_runs;
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::{StyleColor, StyleCx};
use egui::Pos2;
use kurbo::{BezPath, Point};

use super::ConnectionTarget;
use super::place::visible_bends;
use crate::geometry::connector::Routing;

/// The colours and diameters the connector paints with, from the theme: the same at every size, as
/// the connector sits on the document. Look source: FigCraft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConnectorLook {
    /// The route and the rings.
    pub(crate) line: StyleColor,
    /// The handles' and attach points' fill.
    pub(crate) fill: StyleColor,
    /// An attach point's diameter.
    pub(crate) attach: f32,
    /// The diameter of the attach point an end would glue to.
    pub(crate) attach_active: f32,
    /// A handle's diameter.
    pub(crate) handle: f32,
}

impl ConnectorLook {
    /// The look in the theme of `cx`.
    pub(crate) fn of(cx: &StyleCx<'_>) -> Self {
        let tokens = cx.tokens;
        ConnectorLook {
            line: cx.color(tokens.color.accent.solid),
            fill: cx.color(tokens.color.overlay.handle_fill),
            attach: tokens.size.handle.sm,
            attach_active: tokens.size.handle.md,
            handle: tokens.size.handle.sm,
        }
    }
}

/// The screen runs of the route through `points` (document units): a curve as its cubic, flattened,
/// anything else as a polyline.
pub(crate) fn route_runs(points: &[Point], routing: Routing, view: &CanvasView) -> Vec<Vec<Pos2>> {
    if let (Routing::Curved, &[a, b, c, d]) = (routing, points) {
        let mut path = BezPath::new();
        path.move_to(a);
        path.curve_to(b, c, d);
        return path_runs(view, &path).into_iter().map(|(run, _)| run).collect();
    }
    let screen: Vec<Pos2> = points.iter().map(|&p| view.to_screen(p)).collect();
    if screen.len() < 2 { Vec::new() } else { vec![screen] }
}

/// The route through `points`.
pub(crate) fn paint_route(draw: &Draw<'_>, look: &ConnectorLook, points: &[Point], routing: Routing, view: &CanvasView) {
    for run in route_runs(points, routing, view) {
        draw.stroke(&PaintShape::Path(run), StrokeWidth::Medium, look.line);
    }
}

/// A disc of `diameter` at `at`, filled with `fill` and ringed by `ring`.
pub(crate) fn paint_disc(draw: &Draw<'_>, at: Pos2, diameter: f32, fill: StyleColor, ring: StyleColor, width: StrokeWidth) {
    let disc = PaintShape::Circle(at, diameter / 2.0);
    draw.fill(&disc, fill);
    draw.stroke(&disc, width, ring);
}

/// The handles of the route: a disc on each bend that can be grabbed, and on each end.
pub(crate) fn paint_handles(draw: &Draw<'_>, look: &ConnectorLook, points: &[Point], routing: Routing, view: &CanvasView) {
    for (_, at) in visible_bends(points, routing, view) {
        paint_disc(draw, view.to_screen(at), look.handle, look.line, look.fill, StrokeWidth::Thin);
    }
    for end in [points.first(), points.last()].into_iter().flatten() {
        paint_disc(draw, view.to_screen(*end), look.handle, look.fill, look.line, StrokeWidth::Medium);
    }
}

/// `target`'s outline and sites, `active` (a site index) as the larger see-through ring.
pub(crate) fn paint_sites(draw: &Draw<'_>, look: &ConnectorLook, view: &CanvasView, target: &ConnectionTarget, active: Option<u32>) {
    // inv-figcraft: crates/ui-egui/src/canvas/overlays.rs:407-418 `attach_points`, :73-79 (the drop
    // target outlined).
    draw.stroke(&PaintShape::Polygon(view.quad(target.bounds.abs()).to_vec()), StrokeWidth::Thin, look.line);
    for site in target.side_sites() {
        let at = view.to_screen(site.point);
        if active == Some(site.index) {
            draw.stroke(&PaintShape::Circle(at, look.attach_active / 2.0), StrokeWidth::Medium, look.line);
        } else {
            paint_disc(draw, at, look.attach, look.fill, look.line, StrokeWidth::Medium);
        }
    }
}
