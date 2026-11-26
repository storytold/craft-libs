//! `FocusRing`: rings and halos around a rect, circle, polygon or path.

use super::draw::PaintShape;
use crate::runtime::focus::FocusShape;
use crate::style::{Flags, FocusLook};
use egui::emath::GuiRounding as _;
use egui::{CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

/// How a focus indication is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FocusKind {
    /// A ring on the ring sublayer.
    #[default]
    Ring,
    /// A halo: a wide translucent stroke under the ring (canvas parts).
    Halo,
}

/// The focus ring painter.
pub struct FocusRing;

impl FocusRing {
    /// Whether a part with `flags` shows its ring: only when it is the focused component or its active part, with
    /// focus visible.
    pub fn visible(flags: Flags) -> bool {
        flags.contains(Flags::FOCUSED | Flags::FOCUS_VISIBLE)
    }

    /// The shape the ring is drawn around for a part declared with `shape` and laid out in `rect`: the rect, the circle
    /// inside it, or the polygon's points.
    pub(crate) fn outline(shape: &FocusShape, rect: Rect) -> PaintShape {
        match shape {
            FocusShape::Rect => PaintShape::Rect(rect, CornerRadius::ZERO),
            FocusShape::Circle => PaintShape::Circle(rect.center(), rect.width().min(rect.height()) / 2.0),
            FocusShape::Polygon(points) => PaintShape::Polygon(points.clone()),
        }
    }

    /// The shapes of the ring around `shape`, grown by `look.offset`. A halo adds a stroke twice as wide at a
    /// third of the opacity beneath the ring.
    pub fn shapes(look: &FocusLook, kind: FocusKind, shape: &PaintShape, pixels_per_point: f32) -> Vec<Shape> {
        let stroke = look.stroke;
        let mut out = Vec::new();
        if kind == FocusKind::Halo {
            out.push(Self::one(shape, look.offset, Stroke::new(stroke.width * 2.0, stroke.color.gamma_multiply(1.0 / 3.0)), pixels_per_point));
        }
        out.push(Self::one(shape, look.offset, stroke, pixels_per_point));
        out
    }

    fn one(shape: &PaintShape, offset: f32, stroke: Stroke, pixels_per_point: f32) -> Shape {
        match shape {
            PaintShape::Rect(rect, radius) => {
                let rect = rect.round_to_pixels(pixels_per_point);
                let grown = rect.expand(offset);
                let radius = CornerRadius {
                    nw: radius.nw.saturating_add(offset.max(0.0) as u8),
                    ne: radius.ne.saturating_add(offset.max(0.0) as u8),
                    sw: radius.sw.saturating_add(offset.max(0.0) as u8),
                    se: radius.se.saturating_add(offset.max(0.0) as u8),
                };
                Shape::rect_stroke(grown, radius, stroke, StrokeKind::Inside)
            }
            PaintShape::Circle(center, r) => Shape::circle_stroke(*center, r + offset, stroke),
            PaintShape::Polygon(points) => Shape::closed_line(grow_polygon(points, offset), stroke),
            PaintShape::Path(points) => Shape::line(points.clone(), stroke),
        }
    }
}

/// `points` moved outward by `offset`: each vertex slides along the bisector of its two edge normals by
/// `offset / cos(half angle)`, so every edge ends up `offset` from where it was (a mitred outline). Fewer than three
/// points, or an offset of zero, leave the polygon as it is.
fn grow_polygon(points: &[Pos2], offset: f32) -> Vec<Pos2> {
    let count = points.len();
    if count < 3 || offset == 0.0 {
        return points.to_vec();
    }
    // The sign of the area says which side of an edge is outside, whatever the winding.
    let area: f32 = (0..count).map(|i| points[i].x * points[(i + 1) % count].y - points[(i + 1) % count].x * points[i].y).sum();
    let side = if area < 0.0 { -1.0 } else { 1.0 };
    let normal = |from: Pos2, to: Pos2| -> Vec2 {
        let along = (to - from).normalized();
        Vec2::new(along.y, -along.x) * side
    };
    (0..count)
        .map(|i| {
            let (before, here, after) = (points[(i + count - 1) % count], points[i], points[(i + 1) % count]);
            let (n1, n2) = (normal(before, here), normal(here, after));
            // 1 + n1.n2 = 2 cos^2(half angle); it vanishes only where the edges fold back on themselves.
            let miter = 1.0 + n1.dot(n2);
            if miter < 1e-3 { here + n1 * offset } else { here + (n1 + n2) * (offset / miter) }
        })
        .collect()
}
