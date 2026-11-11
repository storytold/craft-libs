//! `Handle`: canvas handle shapes with state.

use crate::style::{HandleLook, StateSet};
use egui::{Pos2, Rect, Shape, Stroke, vec2};

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
