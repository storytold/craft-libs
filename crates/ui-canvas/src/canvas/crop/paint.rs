//! Painting the crop overlay: shield, frame line, guides, corner brackets, edge bars, focus and the Straighten line.

use super::gesture::{Drag, Kind};
use super::{BAR_SHARE, CropOverlay, GRID_MAX, STRAIGHTEN_MIN, screen_corners};
use crate::canvas::CanvasSpace;
use crate::contract::{Cx, PaintUi};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::DataColor;
use craft_behavior::geometry::{BoxPart, CropBox};
use egui::{CornerRadius, Pos2, Rect, Vec2};

pub(super) struct Scene<'a, 'b> {
    pub space: &'a CanvasSpace,
    pub frame: CropBox,
    pub drag: Option<&'a Drag>,
    pub picked: Option<BoxPart>,
    pub refused: bool,
    pub overlay: &'a CropOverlay<'b>,
}

/// Whether the screen quad `q` has horizontal and vertical sides.
fn axis_aligned(q: &[Pos2; 4]) -> bool {
    (0..4).all(|i| {
        let d = q[(i + 1) % 4] - q[i];
        d.x.abs() < 0.01 || d.y.abs() < 0.01
    })
}

pub(super) fn paint(ui: &mut PaintUi<'_>, cx: &Cx<'_, CropOverlay<'_>>, scene: &Scene<'_, '_>) {
    let style = cx.style;
    let draw = ui.draw();
    let overlay = scene.overlay;
    let view = &scene.space.view;
    let area = cx.rect;
    if let Some(Drag { kind: Kind::Line { from, to }, .. }) = scene.drag {
        let line = PaintShape::Path(vec![*from, *to]);
        draw.stroke(&line, StrokeWidth::Thick, style.halo);
        draw.stroke(&line, StrokeWidth::Medium, style.line);
        let _ = STRAIGHTEN_MIN;
        return;
    }
    let q = screen_corners(view, &scene.frame);
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    let (lu, lv) = (u.length(), v.length());
    if !scene.frame.is_finite() || !(lu.is_finite() && lv.is_finite()) || lu < 1e-3 || lv < 1e-3 {
        return;
    }
    let editing = scene.drag.is_some();
    let canvas = style.canvas.egui();
    if let Some(fill) = overlay.shield.fill(canvas, editing) {
        let fill = DataColor::new(fill);
        if axis_aligned(&q) {
            let r = Rect::from_points(&q);
            for band in [
                Rect::from_min_max(area.min, Pos2::new(area.max.x, r.min.y)),
                Rect::from_min_max(Pos2::new(area.min.x, r.max.y), area.max),
                Rect::from_min_max(Pos2::new(area.min.x, r.min.y), Pos2::new(r.min.x, r.max.y)),
                Rect::from_min_max(Pos2::new(r.max.x, r.min.y), Pos2::new(area.max.x, r.max.y)),
            ] {
                if band.is_positive() {
                    draw.data_fill(&PaintShape::Rect(band, CornerRadius::ZERO), fill);
                }
            }
        } else {
            // Four trapezoids from the frame out to the frame grown past the whole area.
            let at = |a: f32, b: f32| q[0] + u * a + v * b;
            let reach = area.size().length() + (area.center() - at(0.5, 0.5)).length();
            let (eu, ev) = (reach / lu, reach / lv);
            let outer = [at(-eu, -ev), at(1.0 + eu, -ev), at(1.0 + eu, 1.0 + ev), at(-eu, 1.0 + ev)];
            for i in 0..4 {
                let j = (i + 1) % 4;
                draw.data_fill(&PaintShape::Polygon(vec![q[i], q[j], outer[j], outer[i]]), fill);
            }
        }
    }
    let line = if scene.refused { style.invalid } else { style.line };
    let outline = PaintShape::Polygon(q.to_vec());
    draw.stroke(&outline, StrokeWidth::Thin, line);
    if let Some((guide, orientation)) = overlay.guide
        && overlay.guide_show.shows(editing)
    {
        let at = |p: craft_behavior::kurbo::Point| q[0] + u * p.x as f32 + v * p.y as f32;
        for polyline in guide.guides(orientation, f64::from(lu), f64::from(lv), f64::from(style.grid_step), GRID_MAX) {
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
