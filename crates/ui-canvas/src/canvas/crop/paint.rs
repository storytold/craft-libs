//! Painting the crop overlay: shield, frame line, guides, corner brackets, edge bars, focus and the Straighten line.

use super::gesture::{Drag, Kind};
use super::{BAR_SHARE, CropOverlay, CropScope, GRID_MAX, screen_corners};
use crate::canvas::CanvasSpace;
use crate::contract::{Cx, PaintUi};
use crate::primitives::{Draw, PaintShape, StrokeWidth};
use crate::style::DataColor;
use craft_behavior::geometry::{BoxPart, CropBox, turn_grid};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::{CornerRadius, Pos2, Rect, Vec2};

/// How many Grid guide steps apart the turn grid's lines are: production's 48 points.
const TURN_GRID_STEPS: f64 = 3.0;

pub(super) struct Scene<'a, 'b> {
    pub space: &'a CanvasSpace,
    pub frame: CropBox,
    pub drag: Option<&'a Drag>,
    pub picked: Option<BoxPart>,
    pub refused: bool,
    /// The crop has started (`CropOverlay::started`): the shield and the guide show and the outline is solid.
    pub started: bool,
    pub overlay: &'a CropOverlay<'b>,
}

pub(super) fn paint(ui: &mut PaintUi<'_>, cx: &Cx<'_, CropOverlay<'_>>, scene: &Scene<'_, '_>) {
    if scene.overlay.hidden {
        return;
    }
    let style = cx.style;
    let draw = ui.draw();
    paint_frame(&draw, cx, scene);
    if let Some(Drag { kind: Kind::Line { from, to }, .. }) = scene.drag {
        let line = PaintShape::Path(vec![*from, *to]);
        draw.stroke(&line, StrokeWidth::Thick, style.halo);
        draw.stroke(&line, StrokeWidth::Medium, style.straighten);
    }
}

/// The shield, the frame line, the guide, the brackets, the bars and the focus line.
fn paint_frame(draw: &Draw<'_>, cx: &Cx<'_, CropOverlay<'_>>, scene: &Scene<'_, '_>) {
    let style = cx.style;
    let overlay = scene.overlay;
    let view = &scene.space.view;
    let area = cx.rect;
    let q = screen_corners(view, &scene.frame);
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    let (lu, lv) = (u.length(), v.length());
    if !scene.frame.is_finite() || !(lu.is_finite() && lv.is_finite()) || lu < 1e-3 || lv < 1e-3 {
        return;
    }
    let editing = scene.drag.is_some();
    if scene.started
        && let Some(fill) = overlay.shield.fill_on(scene.space, style.canvas.egui(), editing)
    {
        // A picture's crop dims only its own source, so the shield's outer quad is the source on screen, not the canvas.
        let corners = match overlay.scope {
            CropScope::Object { source } => {
                let at = |x: f64, y: f64| view.to_screen(Point::new(x, y));
                [at(source.x0, source.y0), at(source.x1, source.y0), at(source.x1, source.y1), at(source.x0, source.y1)]
            }
            _ => [area.left_top(), area.right_top(), area.right_bottom(), area.left_bottom()],
        };
        super::paint_outside(draw, corners, &q, DataColor::new(fill));
    }
    if let Some(cut) = overlay.cut_area(view) {
        draw.stroke(&PaintShape::Polygon(cut.to_vec()), StrokeWidth::Thin, style.guide);
    }
    let line = if scene.refused { style.invalid } else { style.line };
    if scene.started {
        draw.stroke(&PaintShape::Polygon(q.to_vec()), StrokeWidth::Thin, line);
    } else {
        let mut ring = q.to_vec();
        ring.push(q[0]);
        draw.dashed(&ring, draw.dashes(), StrokeWidth::Thin, line);
    }
    if scene.started && overlay.guide_show.shows(editing) {
        let turning = overlay.image_mode && matches!(scene.drag, Some(Drag { kind: Kind::Turn { .. }, .. }));
        let lines = if turning {
            // An image-mode turn shows its own grid in place of the chosen guide (#10931).
            match visible_frame(&q, area, lu, lv) {
                Some(visible) => turn_grid(f64::from(lu), f64::from(lv), TURN_GRID_STEPS * f64::from(style.grid_step), visible),
                None => Vec::new(),
            }
        } else {
            overlay.guide.map_or_else(Vec::new, |(guide, orientation)| {
                guide.guides(orientation, f64::from(lu), f64::from(lv), f64::from(style.grid_step), GRID_MAX)
            })
        };
        let at = |p: Point| q[0] + u * p.x as f32 + v * p.y as f32;
        for polyline in lines {
            draw.stroke(&PaintShape::Path(polyline.into_iter().map(at).collect()), StrokeWidth::Thin, style.guide);
        }
    }
    let l = style.arm.min(lu / 3.0).min(lv / 3.0);
    let (du, dv) = (u / lu, v / lv);
    let at = |a: f32, b: f32| q[0] + u * a + v * b;
    let mut bars: Vec<[Pos2; 2]> = Vec::with_capacity(16);
    for (c, su, sv) in [(q[0], 1.0, 1.0), (q[1], -1.0, 1.0), (q[3], 1.0, -1.0), (q[2], -1.0, -1.0)] {
        bars.push([c, c + du * (l * su)]);
        bars.push([c, c + dv * (l * sv)]);
    }
    let (lx, ly) = (l.min(lu / BAR_SHARE) / 2.0, l.min(lv / BAR_SHARE) / 2.0);
    for (c, e) in [(at(0.5, 0.0), du * lx), (at(0.5, 1.0), du * lx), (at(0.0, 0.5), dv * ly), (at(1.0, 0.5), dv * ly)] {
        bars.push([c - e, c + e]);
    }
    for (width, color) in [(StrokeWidth::Thick, style.halo), (StrokeWidth::Medium, line)] {
        for bar in &bars {
            draw.stroke(&PaintShape::Path(bar.to_vec()), width, color);
        }
    }
    let Some(picked) = scene.picked else { return };
    let gap = style.focus_gap;
    match picked {
        BoxPart::Corner(i) => {
            let (c, su, sv) = [(q[0], 1.0, 1.0), (q[1], -1.0, 1.0), (q[2], -1.0, -1.0), (q[3], 1.0, -1.0)][usize::from(i % 4)];
            let centre = c + (du * su + dv * sv) * (l / 2.0);
            draw.stroke(
                &PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(l + gap)), CornerRadius::ZERO),
                StrokeWidth::Medium,
                style.focus,
            );
        }
        BoxPart::Edge(i) => {
            let (centre, half) = [(at(0.5, 0.0), lx), (at(1.0, 0.5), ly), (at(0.5, 1.0), lx), (at(0.0, 0.5), ly)][usize::from(i % 4)];
            draw.stroke(
                &PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(2.0 * half + gap)), CornerRadius::ZERO),
                StrokeWidth::Medium,
                style.focus,
            );
        }
        _ => {
            let out = |c: Pos2, su: f32, sv: f32| c + (du * su + dv * sv) * gap;
            let ring = vec![out(q[0], -1.0, -1.0), out(q[1], 1.0, -1.0), out(q[2], 1.0, 1.0), out(q[3], -1.0, 1.0)];
            draw.stroke(&PaintShape::Polygon(ring), StrokeWidth::Medium, style.focus);
        }
    }
}

/// The part of the control's rect `clip` inside the frame `q` (screen corners clockwise from the top left of the
/// upright frame, sides `w` and `h` long), in the frame's own screen axes: 0..`w` along its top edge, 0..`h` down
/// its left. A turned, flipped or stretched view maps through the inverse of the frame's axes. `None` when the
/// frame has no area on screen or anything is not finite.
fn visible_frame(q: &[Pos2; 4], clip: Rect, w: f32, h: f32) -> Option<DocRect> {
    if !(clip.is_finite() && clip.is_positive()) {
        return None;
    }
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    let det = f64::from(u.x) * f64::from(v.y) - f64::from(u.y) * f64::from(v.x);
    if !det.is_finite() || det.abs() < f64::EPSILON {
        return None;
    }
    let (w, h) = (f64::from(w), f64::from(h));
    let mut corners = [clip.left_top(), clip.right_top(), clip.right_bottom(), clip.left_bottom()].into_iter().map(|p| {
        let d = p - q[0];
        let x = (f64::from(d.x) * f64::from(v.y) - f64::from(d.y) * f64::from(v.x)) / det * w;
        let y = (f64::from(u.x) * f64::from(d.y) - f64::from(u.y) * f64::from(d.x)) / det * h;
        Point::new(x, y)
    });
    let first = corners.next()?;
    let visible = corners.fold(DocRect::from_points(first, first), |acc, p| acc.union_pt(p));
    let bounded = DocRect::new(visible.x0.max(0.0), visible.y0.max(0.0), visible.x1.min(w), visible.y1.min(h));
    [bounded.x0, bounded.y0, bounded.x1, bounded.y1].iter().all(|c| c.is_finite()).then_some(bounded)
}
