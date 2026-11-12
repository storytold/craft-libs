//! `Rule`: dividers, edges, grid lines and drop marks, with one pixel-snap rule.

use crate::style::RuleLook;
use egui::{Pos2, Rect, Shape, Stroke};

pub use crate::style::RuleKind;

/// The rule painter.
pub struct Rule;

/// A side of a rect, on screen: left and right stay put under right-to-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RectSide {
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
}

impl RectSide {
    /// The start side in reading order: left, or right under right-to-left.
    pub const fn start(rtl: bool) -> RectSide {
        if rtl { RectSide::Right } else { RectSide::Left }
    }

    /// The end side in reading order: right, or left under right-to-left.
    pub const fn end(rtl: bool) -> RectSide {
        if rtl { RectSide::Left } else { RectSide::Right }
    }
}

impl Rule {
    /// The line from `from` to `to`, snapped across its direction: a line whose physical width is odd sits on a
    /// pixel centre, an even one on a pixel edge, and no line is thinner than one physical pixel.
    pub fn shapes(look: &RuleLook, from: Pos2, to: Pos2, pixels_per_point: f32) -> Vec<Shape> {
        let physical = look.width.pixels(pixels_per_point);
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

    /// The line along the `side` edge of `rect`, on the physical pixels just inside it: a row's bottom rule or a
    /// cell's gridline, which a line centred on the edge would put on the first pixel outside, under the next row's
    /// clip or the next cell's fill.
    pub fn edge(look: &RuleLook, rect: Rect, side: RectSide, pixels_per_point: f32) -> Vec<Shape> {
        let half = look.width.points(pixels_per_point) / 2.0;
        let (from, to) = match side {
            RectSide::Top => (Pos2::new(rect.left(), rect.top() + half), Pos2::new(rect.right(), rect.top() + half)),
            RectSide::Bottom => (Pos2::new(rect.left(), rect.bottom() - half), Pos2::new(rect.right(), rect.bottom() - half)),
            RectSide::Left => (Pos2::new(rect.left() + half, rect.top()), Pos2::new(rect.left() + half, rect.bottom())),
            RectSide::Right => (Pos2::new(rect.right() - half, rect.top()), Pos2::new(rect.right() - half, rect.bottom())),
        };
        Self::shapes(look, from, to, pixels_per_point)
    }
}
