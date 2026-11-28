//! `Rule`: dividers, edges, grid lines and drop marks, with one pixel-snap rule.

use crate::style::RuleLook;
use egui::{Mesh, Pos2, Rect, Shape, Stroke, Vec2};

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

    /// The side of a pane at `edge` that faces the window's centre, mirrored for a start or end pane under
    /// right-to-left; none for the centre.
    pub const fn toward_centre(edge: crate::contract::PaneEdge, rtl: bool) -> Option<RectSide> {
        use crate::contract::PaneEdge;
        match edge {
            PaneEdge::Start => Some(RectSide::end(rtl)),
            PaneEdge::End => Some(RectSide::start(rtl)),
            PaneEdge::Top => Some(RectSide::Bottom),
            PaneEdge::Bottom => Some(RectSide::Top),
            PaneEdge::Center => None,
        }
    }
}

/// `v` (points) snapped for a line `physical` pixels wide: an odd width on a physical pixel centre, an even one on a
/// pixel edge.
fn snap(v: f32, physical: f32, pixels_per_point: f32) -> f32 {
    let px = v * pixels_per_point;
    let snapped = if physical % 2.0 == 1.0 { px.floor() + 0.5 } else { px.round() };
    snapped / pixels_per_point
}

impl Rule {
    /// The line from `from` to `to`, snapped across its direction: a line whose physical width is odd sits on a
    /// pixel centre, an even one on a pixel edge, and no line is thinner than one physical pixel.
    pub fn shapes(look: &RuleLook, from: Pos2, to: Pos2, pixels_per_point: f32) -> Vec<Shape> {
        let physical = look.width.pixels(pixels_per_point);
        let snap = |v: f32| snap(v, physical, pixels_per_point);
        let (from, to) = if (from.y - to.y).abs() <= (from.x - to.x).abs() {
            (Pos2::new(from.x, snap(from.y)), Pos2::new(to.x, snap(to.y)))
        } else {
            (Pos2::new(snap(from.x), from.y), Pos2::new(snap(to.x), to.y))
        };
        vec![Shape::line_segment([from, to], Stroke::new(physical / pixels_per_point, look.color))]
    }

    /// The dotted form of the line from `from` to `to`: square dots as wide as the line, `gap` points apart, the run
    /// of whole dots centred between the ends (a dotted guide or grid line). The gap is whole physical pixels, at least
    /// one, and each dot's centre is snapped on both axes by the rule `shapes` snaps across with, so every dot covers
    /// whole physical pixels and dots on a level or upright line are evenly spaced. The dots are one mesh.
    pub fn dotted(look: &RuleLook, from: Pos2, to: Pos2, gap: f32, pixels_per_point: f32) -> Vec<Shape> {
        let side = look.width.points(pixels_per_point);
        Self::ticks(look, from, to, gap, side, pixels_per_point)
    }

    /// The line from `from` to `to` as a run of ticks: marks as wide as the line along it and `across` points long
    /// across it, `gap` points apart, the run of whole marks centred between the ends (a grip's row of upright ticks).
    /// With `across` the line's width the marks are the square dots of [`dotted`](Self::dotted). The gap and both
    /// sides of a mark are whole physical pixels, at least one, and each mark's centre is snapped on both axes, so every
    /// mark covers whole physical pixels. A mark stands across a level line and lies across an upright one; a slanted
    /// line takes the level form. The marks are one mesh.
    pub fn ticks(look: &RuleLook, from: Pos2, to: Pos2, gap: f32, across: f32, pixels_per_point: f32) -> Vec<Shape> {
        let ppp = pixels_per_point.max(f32::EPSILON);
        let side = look.width.pixels(ppp);
        let long = (across * ppp).round().max(1.0);
        let gap = (gap * ppp).round().max(1.0);
        let length = (to - from).length() * ppp;
        // The run leaves at least a physical pixel of the line free, so snapping, which moves a mark at most half a
        // pixel, keeps every mark between the ends.
        let count = ((length - 1.0 + gap) / (side + gap)).floor();
        if !count.is_finite() || count < 1.0 {
            return Vec::new();
        }
        let direction = (to - from).normalized();
        let level = direction.x.abs() >= direction.y.abs();
        // The mark's physical size on the x and y axes.
        let (wide, tall) = if level { (side, long) } else { (long, side) };
        let first = (length - (count * side + (count - 1.0) * gap)) / 2.0 + side / 2.0;
        let mark = Vec2::new(wide / ppp, tall / ppp);
        // One mesh of plain quads: a filled rect two pixels wide or less is tessellated as a line segment, whose ends
        // the tessellator rounds inward, and a one-pixel mark drawn that way vanishes.
        let mut mesh = Mesh::default();
        for i in 0..count as usize {
            let centre = from + direction * ((first + i as f32 * (side + gap)) / ppp);
            let centre = Pos2::new(snap(centre.x, wide, ppp), snap(centre.y, tall, ppp));
            mesh.add_colored_rect(Rect::from_center_size(centre, mark), look.color);
        }
        vec![Shape::mesh(mesh)]
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
