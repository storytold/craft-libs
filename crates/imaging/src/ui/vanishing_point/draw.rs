//! Vanishing Point's drawing: planes, grids, nodes, the pending corners and the stamp.

use egui::{CornerRadius, Pos2, Rect, Vec2};

use craft_behavior::CanvasView;
use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::StyleColor;

use super::VanishingTool;
use super::editor::{Work, is_unset};
use super::planes::{MAX_GRID_LINES, PlaneCheck, TornPlane, VanishingPlane};
use super::style::VanishingPointStyle;

/// What one pass draws and declares parts for, read once from the editor: the planes with the scene's verdict on each,
/// the selection, Create Plane's pending corners, the pointer and the stamp.
pub(super) struct Snapshot {
    pub planes: Vec<VanishingPlane>,
    pub torn: Vec<Option<TornPlane>>,
    pub checks: Vec<PlaneCheck>,
    pub frames: Vec<Option<(Homography, f64, f64)>>,
    pub selected: Option<usize>,
    pub tool: VanishingTool,
    pub pending: Vec<[f64; 2]>,
    pub pointer: Option<[f64; 2]>,
    pub grid_size: f64,
    pub source: Option<[f64; 2]>,
    pub ring: Option<Vec<Point>>,
}

impl Snapshot {
    pub(super) fn of(work: &Work<'_>) -> Snapshot {
        let (editor, session, scene, settings) = (&*work.editor, &*work.session, work.scene, work.settings);
        let planes = session.planes.clone();
        let checks = (0..planes.len()).map(|i| scene.check(&planes, i)).collect();
        let frames = (0..planes.len())
            .map(|i| scene.frame(&planes, i).filter(|(_, lu, lv)| lu.is_finite() && lv.is_finite() && *lu > 1e-12 && *lv > 1e-12))
            .collect();
        let ring = (editor.tool == VanishingTool::Stamp)
            .then(|| editor.pointer.and_then(|at| scene.stamp_ring(&planes, at, settings.stamp.diameter)))
            .flatten();
        Snapshot {
            torn: session.torn.clone(),
            planes,
            checks,
            frames,
            selected: editor.selected,
            tool: editor.tool,
            pending: editor.pending.clone(),
            pointer: editor.pointer,
            grid_size: settings.grid_size,
            source: session.source,
            ring,
        }
    }

    pub(super) fn selected_plane(&self) -> Option<usize> {
        self.selected.filter(|i| *i < self.planes.len())
    }
}

/// Draws the editor over the image, in this order (the paste, an overlay of the canvas, lies under all of it, so a
/// plane's lines stay visible across it):
///
/// 1. Each plane (not one the scene could not rebuild): an invalid plane only its outline; the others their grid (cells
///    of `grid_size` at the nearest edge, at most `MAX_GRID_LINES` a direction) and outline, in the check's colour; the
///    outline over the halo.
/// 2. The selected plane's nodes: corners and edge middles as handle squares with the plane's colour outline.
/// 3. Create Plane's pending corners as squares with the rubber band to the pointer.
/// 4. Stamp: the source cross and the foreshortened ring of the diameter, each a line over the halo.
pub(super) fn paint(
    draw: &Draw<'_>,
    view: &CanvasView,
    snap: &Snapshot,
    style: &VanishingPointStyle,
    node: &dyn Fn(usize, usize, bool) -> StyleColor,
) {
    // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1078 `draw_content`, :1091 `draw` (colors, widths, nodes :1091-1135),
    // :1148 `draw_stamp`, :1189 `grid_steps`.
    let screen = |p: [f64; 2]| view.to_screen(Point::new(p[0], p[1]));
    let selected = snap.selected_plane();
    let color_of = |check: PlaneCheck| match check {
        PlaneCheck::Valid => (style.valid, style.valid_grid),
        PlaneCheck::Questionable => (style.questionable, style.questionable_grid),
        _ => (style.invalid, style.invalid),
    };
    for (i, plane) in snap.planes.iter().enumerate() {
        if is_unset(plane) {
            continue;
        }
        let check = snap.checks[i];
        let (color, grid) = color_of(check);
        if check != PlaneCheck::Invalid
            && let Some((h, lu, lv)) = snap.frames[i]
        {
            let at = |u: f64, v: f64| {
                let p = h.apply(Point::new(u, v));
                screen([p.x, p.y])
            };
            for u in grid_steps(plane, lu, lv, snap.grid_size, true) {
                draw.stroke(&PaintShape::Path(vec![at(u, 0.0), at(u, 1.0)]), StrokeWidth::Hairline, grid);
            }
            for v in grid_steps(plane, lu, lv, snap.grid_size, false) {
                draw.stroke(&PaintShape::Path(vec![at(0.0, v), at(1.0, v)]), StrokeWidth::Hairline, grid);
            }
        }
        let mut ring: Vec<Pos2> = plane.corners.iter().map(|c| screen(*c)).collect();
        ring.push(ring[0]);
        let outline = PaintShape::Path(ring);
        if selected == Some(i) {
            draw.stroke(&outline, StrokeWidth::Thick, color);
        } else {
            draw.stroke(&outline, StrokeWidth::Medium, style.halo);
            draw.stroke(&outline, StrokeWidth::Thin, color);
        }
    }

    if let Some(i) = selected.filter(|i| !is_unset(&snap.planes[*i])) {
        let plane = &snap.planes[i];
        let (color, _) = color_of(snap.checks[i]);
        let nodes = plane.corners.iter().copied().map(|c| (c, true)).chain(plane.edge_nodes().into_iter().map(|c| (c, false)));
        for (at, (centre, corner)) in nodes.enumerate() {
            let square = PaintShape::Rect(Rect::from_center_size(screen(centre), Vec2::splat(style.handle.side)), CornerRadius::ZERO);
            let index = if corner { at } else { at - 4 };
            draw.fill(&square, node(i, index, corner));
            draw.stroke(&square, StrokeWidth::Thin, color);
        }
    }

    if !snap.pending.is_empty() {
        let mut points: Vec<Pos2> = snap.pending.iter().map(|c| screen(*c)).collect();
        let corners = points.len();
        if let Some(pointer) = snap.pointer {
            points.push(screen(pointer));
        }
        draw.stroke(&PaintShape::Path(points.clone()), StrokeWidth::Thin, style.valid);
        for c in points.iter().take(corners) {
            let square = PaintShape::Rect(Rect::from_center_size(*c, Vec2::splat(style.handle.side)), CornerRadius::ZERO);
            draw.fill(&square, style.valid);
        }
    }

    if snap.tool == VanishingTool::Stamp {
        paint_stamp(draw, &screen, snap, style);
    }
}

/// Stamp: the source as a cross and the brush under the pointer as the circle it paints on the plane, foreshortened.
fn paint_stamp(draw: &Draw<'_>, screen: &dyn Fn([f64; 2]) -> Pos2, snap: &Snapshot, style: &VanishingPointStyle) {
    // PhotoCraft's ring and cross: a halo under a light line.
    let lines = |shape: PaintShape| {
        draw.stroke(&shape, StrokeWidth::Medium, style.halo);
        draw.stroke(&shape, StrokeWidth::Thin, style.ink);
    };
    if let Some(source) = snap.source {
        let c = screen(source);
        for arm in [Vec2::new(style.arm, 0.0), Vec2::new(0.0, style.arm)] {
            lines(PaintShape::Path(vec![c - arm, c + arm]));
        }
    }
    let Some(ring) = &snap.ring else { return };
    let mut ring: Vec<Pos2> = ring.iter().map(|p| screen([p.x, p.y])).collect();
    if ring.len() > 2 && ring.iter().all(|p| p.is_finite()) {
        ring.push(ring[0]);
        lines(PaintShape::Path(ring));
    }
}

/// Plane coordinates (0..1, exclusive) of the grid lines across `u` (or `v`) for cells of `size` pixels at the plane's
/// nearest edge, square in the plane's metric.
fn grid_steps(plane: &VanishingPlane, lu: f64, lv: f64, size: f64, along_u: bool) -> Vec<f64> {
    let q = &plane.corners;
    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    // Image pixels per metric unit at the nearest (longest-looking) edge.
    let k = [d(q[0], q[1]) / lu, d(q[3], q[2]) / lu, d(q[0], q[3]) / lv, d(q[1], q[2]) / lv].into_iter().fold(0.0, f64::max);
    let cell = size.max(1.0) / k;
    let len = if along_u { lu } else { lv };
    if !cell.is_finite() || cell <= 0.0 || !len.is_finite() {
        return Vec::new();
    }
    let step = cell / len;
    (1..MAX_GRID_LINES).map(|j| j as f64 * step).take_while(|t| *t < 1.0 - 1e-9).collect()
}
