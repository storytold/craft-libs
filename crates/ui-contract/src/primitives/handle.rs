//! `Handle`: canvas handle shapes with state.

use super::{DataWidth, Draw, PaintShape};
use crate::style::{HandleLook, StateSet};
use egui::{Pos2, Rect, Shape, Stroke, vec2};

/// The half-length of each stroke of the overflow plus, as a share of the handle side (3 of 11 points in the type
/// tool's frame).
const PLUS_HALF_IN_SIDES: f32 = 0.27;

/// The outline of a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HandleShape {
    /// A square.
    #[default]
    Square,
    /// A circle.
    Circle,
    /// A square turned 45 degrees.
    Diamond,
}

/// The handle painter.
pub struct Handle;

impl Handle {
    /// The overflow mark: two strokes through `center`, each half `0.27 * side` long, `look.stroke_width` wide in
    /// `look.stroke`.
    pub fn plus(draw: &Draw<'_>, look: &HandleLook, center: Pos2, side: f32) {
        let half = PLUS_HALF_IN_SIDES * side;
        let width = DataWidth::points(look.stroke_width);
        for arm in [vec2(half, 0.0), vec2(0.0, half)] {
            draw.stroke(&PaintShape::Path(vec![center - arm, center + arm]), width, look.stroke);
        }
    }

    /// The shapes of a handle of `shape` centred on `center`, filled for `state`.
    pub fn shapes(look: &HandleLook, shape: HandleShape, center: Pos2, state: &StateSet) -> Vec<Shape> {
        let fill = state.resolve(&look.fill).egui();
        let stroke = Stroke::new(look.stroke_width, look.stroke.egui());
        let half = look.side / 2.0;
        match shape {
            HandleShape::Square => vec![
                Shape::rect_filled(Rect::from_center_size(center, vec2(look.side, look.side)), 0.0, fill),
                Shape::rect_stroke(Rect::from_center_size(center, vec2(look.side, look.side)), 0.0, stroke, egui::StrokeKind::Inside),
            ],
            HandleShape::Circle => vec![Shape::circle_filled(center, half, fill), Shape::circle_stroke(center, half, stroke)],
            HandleShape::Diamond => {
                let points = vec![center + vec2(0.0, -half), center + vec2(half, 0.0), center + vec2(0.0, half), center + vec2(-half, 0.0)];
                vec![Shape::convex_polygon(points, fill, stroke)]
            }
        }
    }
}
