//! `AutoScrollMark`: the mark a latched middle-button auto-scroll shows at its anchor: a ring on the panel fill, a dot
//! and two arrowheads in text ink.

use crate::style::AutoScrollMarkLook;
use egui::{Pos2, Shape, Stroke, vec2};

/// The dot's radius, as a share of the mark's radius.
const DOT: f32 = 2.0 / 13.0;
/// Half an arrowhead's base, as a share of the mark's radius.
const ARROW_HALF_WIDTH: f32 = 4.0 / 13.0;
/// How far an arrowhead's base sits from the centre, as a share of the mark's radius.
const ARROW_BASE: f32 = 6.0 / 13.0;
/// How far an arrowhead's tip sits from the centre, as a share of the mark's radius.
const ARROW_TIP: f32 = 10.0 / 13.0;

/// The auto-scroll mark painter.
pub struct AutoScrollMark;

impl AutoScrollMark {
    /// The mark centred on `anchor`: the ring, the dot, and an arrowhead above and below, all sized from the look's
    /// diameter.
    pub fn shapes(look: &AutoScrollMarkLook, anchor: Pos2) -> Vec<Shape> {
        let radius = look.diameter / 2.0;
        let ink = look.ink.egui();
        let mut shapes = vec![
            Shape::circle_filled(anchor, radius, look.fill.egui()),
            Shape::circle_stroke(anchor, radius - look.ring_width, Stroke::new(look.ring_width, look.ring.egui())),
            Shape::circle_filled(anchor, radius * DOT, ink),
        ];
        let (half, base, tip) = (radius * ARROW_HALF_WIDTH, radius * ARROW_BASE, radius * ARROW_TIP);
        for direction in [-1.0, 1.0] {
            shapes.push(Shape::convex_polygon(
                vec![anchor + vec2(-half, direction * base), anchor + vec2(half, direction * base), anchor + vec2(0.0, direction * tip)],
                ink,
                Stroke::NONE,
            ));
        }
        shapes
    }
}
