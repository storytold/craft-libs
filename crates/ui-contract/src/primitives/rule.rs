//! `Rule`: dividers, edges, grid lines and drop marks, with one pixel-snap rule.

use crate::style::RuleLook;
use egui::{Pos2, Shape, Stroke};

pub use crate::style::RuleKind;

/// The rule painter.
pub struct Rule;

impl Rule {
    /// The line from `from` to `to`, snapped across its direction: a line whose physical width is odd sits on a
    /// pixel centre, an even one on a pixel edge, and no line is thinner than one physical pixel.
    pub fn shapes(look: &RuleLook, from: Pos2, to: Pos2, pixels_per_point: f32) -> Vec<Shape> {
        let physical = (look.width * pixels_per_point).round().max(1.0);
        let snap = |v: f32| {
            let px = v * pixels_per_point;
            let snapped = if physical % 2.0 == 1.0 { px.floor() + 0.5 } else { px.round() };
            snapped / pixels_per_point
        };
        let (from, to) = if (from.y - to.y).abs() <= (from.x - to.x).abs() {
            (Pos2::new(from.x, snap(from.y)), Pos2::new(to.x, snap(to.y)))
        } else {
            (Pos2::new(snap(from.x), from.y), Pos2::new(snap(to.x), to.y))
        };
        vec![Shape::line_segment([from, to], Stroke::new(physical / pixels_per_point, look.color))]
    }
}
