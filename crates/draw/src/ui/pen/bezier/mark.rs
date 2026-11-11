//! What the path overlay draws besides outlines: the selected segment's look ([`SegmentMark`]), whether anchors show
//! ([`AnchorsShown`]), and the hover look of the part a press would take ([`HitMark`]).

use craft_behavior::CanvasView;
use craft_theme::Srgba;
use craft_ui::canvas_controls::path_runs;
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::{DataColor, HandleLook, StyleColor};
use egui::{CornerRadius, Pos2, Rect, Vec2};
use kurbo::{BezPath, ParamCurve as _};

use super::{BezierHit, BezierStyle, EditPath};

/// How a clicked segment ([`PathSelection::segment`](super::PathSelection::segment)) shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SegmentMark {
    /// Its two handles only, no thicker line (PhotoCraft, Photoshop).
    #[default]
    HandlesOnly,
    /// Its two handles and the segment in the selected segment.
    Line,
}

/// Whether the overlay draws anchors and handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnchorsShown {
    /// Anchors, and the handles [`HandlesShown`](super::HandlesShown) names.
    #[default]
    All,
    /// Outlines only: no anchor, no handle, no part, no AccessKit object (a path under a transform box, PhotoCraft's
    /// `draw_outline`). Use it with [`BezierTool::Anchors`](super::BezierTool::Anchors).
    Hidden,
}

/// The hover look of the part of a path a press would take, drawn by the overlay for the Add, Delete, Auto
/// Add/Delete and Curvature tools and by the pen draft for Auto Add/Delete on the draft:
///
/// - a segment: the segment in the segment width over a halo, then a hollow anchor at the segment's point at `t`,
///   where a click adds one;
/// - an anchor: the anchor enlarged to the close anchor;
/// - a handle's end: the end enlarged to the close anchor.
///
/// Colours are the path's own or the style's, as the overlay draws that path.
#[derive(Clone, Copy, Debug)]
pub struct HitMark<'a> {
    path: &'a EditPath,
    hit: BezierHit,
}

impl<'a> HitMark<'a> {
    /// The mark of `hit` on `path` (a hit on another path draws nothing).
    pub fn new(path: &'a EditPath, hit: BezierHit) -> Self {
        Self { path, hit }
    }
}

/// Strokes `shape` in the path's colour, else `fallback`.
pub(super) fn stroke(draw: &Draw<'_>, shape: &PaintShape, width: StrokeWidth, color: Option<Srgba>, fallback: StyleColor) {
    match color {
        Some(color) => draw.stroke(shape, width, DataColor::new(color.into())),
        None => draw.stroke(shape, width, fallback),
    }
}

/// The square of an anchor of `side` centred on `at`.
pub(super) fn square(at: Pos2, side: f32) -> PaintShape {
    PaintShape::Rect(Rect::from_center_size(at, Vec2::splat(side)), CornerRadius::ZERO)
}

/// A hollow anchor (or a handle end, `circle`) centred on `at`, filled with `fill`, outlined in the path's colour or the
/// look's stroke.
pub(super) fn paint_handle(draw: &Draw<'_>, look: &HandleLook, circle: bool, at: Pos2, fill: Fill, color: Option<Srgba>) {
    let shape = if circle { PaintShape::Circle(at, look.side / 2.0) } else { square(at, look.side) };
    match fill {
        Fill::Style(fill) => draw.fill(&shape, fill),
        Fill::Path(fallback) => match color {
            Some(color) => draw.data_fill(&shape, DataColor::new(color.into())),
            None => draw.fill(&shape, fallback),
        },
    }
    match color {
        Some(color) => draw.stroke(&shape, StrokeWidth::Thin, DataColor::new(color.into())),
        None => draw.stroke(&shape, StrokeWidth::Thin, look.stroke),
    }
}

/// How a handle is filled.
#[derive(Clone, Copy)]
pub(super) enum Fill {
    /// The state's fill from the look.
    Style(StyleColor),
    /// The path's colour (the style's line without one): a selected anchor.
    Path(StyleColor),
}

/// The segment leaving a knot as a path of its own.
pub(super) fn segment_path(curve: kurbo::CubicBez) -> BezPath {
    let mut segment = BezPath::new();
    segment.move_to(curve.p0);
    segment.curve_to(curve.p1, curve.p2, curve.p3);
    segment
}

/// The screen points of `path` under `view` as the open or closed runs `path_runs` flattens it to, each as a
/// `PaintShape::Path` (a closed run repeats its first point).
pub(super) fn run_shapes(view: &CanvasView, path: &BezPath) -> Vec<PaintShape> {
    path_runs(view, path)
        .into_iter()
        .map(|(mut points, closed)| {
            if closed {
                points.extend(points.first().copied());
            }
            PaintShape::Path(points)
        })
        .collect()
}

/// Draws the hover mark of `mark` over `view` with `style`.
pub(crate) fn paint_mark(draw: &Draw<'_>, view: &CanvasView, mark: HitMark<'_>, style: &BezierStyle) {
    // pc: crates/ui-egui/src/tools/anchor_tools.rs:171-188 `draw_hit` (segment, then `draw_anchor` at `t`).
    let HitMark { path, hit } = mark;
    if hit.path() != path.key {
        return;
    }
    let color = path.color;
    match hit {
        BezierHit::Segment { segment, t, .. } => {
            let Some(curve) = path.subpaths.get(segment.subpath).and_then(|sub| sub.segment(segment.knot)) else { return };
            for shape in run_shapes(view, &segment_path(curve)) {
                draw.stroke(&shape, StrokeWidth::Thick, style.halo);
                stroke(draw, &shape, StrokeWidth::Medium, color, style.line);
            }
            let at = view.to_screen(curve.eval(t.clamp(0.0, 1.0)));
            paint_handle(draw, &style.anchor, false, at, Fill::Style(style.anchor.fill.off.rest), color);
        }
        BezierHit::Anchor { knot, .. } => {
            if let Some(knot) = path.knot(knot) {
                paint_handle(draw, &style.close, false, view.to_screen(knot.anchor), Fill::Style(style.close.fill.off.hover), color);
            }
        }
        BezierHit::Handle { knot, side, .. } => {
            if let Some(knot) = path.knot(knot) {
                paint_handle(draw, &style.close, true, view.to_screen(knot.handle(side)), Fill::Style(style.close.fill.off.hover), color);
            }
        }
    }
}
