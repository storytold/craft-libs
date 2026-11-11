//! Hit shapes, their priority and the arena's resolve rule (8.3).

use crate::part::PartRef;
use crate::sense::Sense;
use emath::{Pos2, Rect, Vec2};
use std::cmp::Ordering;

/// How far from a line a point still hits it, in points.
pub const LINE_REACH: f32 = 4.0;

/// The geometry a part registers: the shape painted is the shape hit.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HitShape {
    /// An axis-aligned rectangle.
    Rect(Rect),
    /// A disc.
    #[non_exhaustive]
    Circle {
        /// The centre.
        center: Pos2,
        /// The radius.
        radius: f32,
    },
    /// A line segment, hit within [`LINE_REACH`].
    #[non_exhaustive]
    Segment {
        /// One end.
        a: Pos2,
        /// The other end.
        b: Pos2,
    },
    /// A chain of segments, hit within [`LINE_REACH`].
    #[non_exhaustive]
    Polyline {
        /// The corners in order.
        points: Vec<Pos2>,
        /// Whether the last corner joins the first.
        closed: bool,
    },
    /// A filled polygon.
    Polygon(Vec<Pos2>),
    /// The part of a rectangle outside a polygon (even-odd): what a box's surroundings hit, so a press inside a turned
    /// box is not theirs while one just outside a corner still is.
    #[non_exhaustive]
    OutsidePolygon {
        /// The area the shape lies in.
        within: Rect,
        /// The corners of the polygon cut out of `within`.
        polygon: Vec<Pos2>,
    },
}

impl HitShape {
    /// A disc.
    pub const fn circle(center: Pos2, radius: f32) -> Self {
        HitShape::Circle { center, radius }
    }

    /// A line segment.
    pub const fn segment(a: Pos2, b: Pos2) -> Self {
        HitShape::Segment { a, b }
    }

    /// A chain of segments, closed when `closed`.
    pub fn polyline(points: Vec<Pos2>, closed: bool) -> Self {
        HitShape::Polyline { points, closed }
    }

    /// The part of `within` outside `polygon`.
    pub fn outside_polygon(within: Rect, polygon: Vec<Pos2>) -> Self {
        HitShape::OutsidePolygon { within, polygon }
    }

    /// The smallest rectangle around the shape.
    pub fn bounds(&self) -> Rect {
        match self {
            HitShape::Rect(rect) | HitShape::OutsidePolygon { within: rect, .. } => *rect,
            HitShape::Circle { center, radius } => Rect::from_center_size(*center, Vec2::splat(radius * 2.0)),
            HitShape::Segment { a, b } => Rect::from_two_pos(*a, *b),
            HitShape::Polyline { points, .. } | HitShape::Polygon(points) => {
                points.iter().fold(Rect::NOTHING, |bounds, point| bounds.union(Rect::from_min_max(*point, *point)))
            }
        }
    }

    /// Whether `point` hits the shape grown to be at least `across` wide and high: the hit area of a part is never
    /// smaller than the pointer's target size.
    fn hit_across(&self, point: Pos2, across: f32) -> bool {
        let reach = LINE_REACH.max(across / 2.0);
        match self {
            HitShape::Rect(rect) => {
                let grow = Vec2::new((across - rect.width()).max(0.0), (across - rect.height()).max(0.0)) / 2.0;
                rect.expand2(grow).contains(point)
            }
            HitShape::Circle { center, radius } => center.distance(point) <= radius.max(across / 2.0),
            HitShape::Segment { a, b } => segment_distance(point, *a, *b) <= reach,
            HitShape::Polyline { points, closed } => {
                let closing = closed.then(|| points.first().zip(points.last())).flatten();
                points.windows(2).any(|pair| segment_distance(point, pair[0], pair[1]) <= reach)
                    || closing.is_some_and(|(first, last)| segment_distance(point, *last, *first) <= reach)
            }
            HitShape::OutsidePolygon { within, polygon } => within.contains(point) && !inside_polygon(point, polygon),
            HitShape::Polygon(points) => {
                let bounds = self.bounds();
                let small = bounds.width() < across || bounds.height() < across;
                inside_polygon(point, points)
                    || (small && points.iter().zip(points.iter().cycle().skip(1)).any(|(a, b)| segment_distance(point, *a, *b) <= across / 2.0))
            }
        }
    }

    /// Whether `point` hits the shape.
    pub fn hit(&self, point: Pos2) -> bool {
        match self {
            HitShape::Rect(rect) => rect.contains(point),
            HitShape::Circle { center, radius } => center.distance(point) <= *radius,
            HitShape::Segment { a, b } => segment_distance(point, *a, *b) <= LINE_REACH,
            HitShape::Polyline { points, closed } => {
                let closing = closed.then(|| points.first().zip(points.last())).flatten();
                points.windows(2).any(|pair| segment_distance(point, pair[0], pair[1]) <= LINE_REACH)
                    || closing.is_some_and(|(first, last)| segment_distance(point, *last, *first) <= LINE_REACH)
            }
            HitShape::Polygon(points) => inside_polygon(point, points),
            HitShape::OutsidePolygon { within, polygon } => within.contains(point) && !inside_polygon(point, polygon),
        }
    }
}

impl From<Rect> for HitShape {
    fn from(rect: Rect) -> Self {
        HitShape::Rect(rect)
    }
}

fn segment_distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let line = b - a;
    let length_sq = line.length_sq();
    let t = if length_sq == 0.0 { 0.0 } else { ((point - a).dot(line) / length_sq).clamp(0.0, 1.0) };
    (a + line * t).distance(point)
}

/// Even-odd containment.
fn inside_polygon(point: Pos2, points: &[Pos2]) -> bool {
    let mut inside = false;
    let mut previous = match points.last() {
        Some(last) => *last,
        None => return false,
    };
    for &current in points {
        if (current.y > point.y) != (previous.y > point.y) {
            let x = (previous.x - current.x) * (point.y - current.y) / (previous.y - current.y) + current.x;
            if point.x < x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// The tier of a shape inside one stack position; a higher level wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum HitLevel {
    /// A tool's background.
    Background,
    /// A component body.
    Body,
    /// An edge grip.
    Edge,
    /// A handle.
    Handle,
}

/// A part's rank among the shapes of its stack position: a node editor ranks port above wire above node body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HitPriority {
    /// The tier.
    pub level: HitLevel,
    /// The rank inside the tier.
    pub sub: u8,
}

impl HitPriority {
    /// A priority at `level` and `sub`.
    pub const fn new(level: HitLevel, sub: u8) -> Self {
        HitPriority { level, sub }
    }

    /// The priority of a component body.
    pub const BODY: HitPriority = HitPriority::new(HitLevel::Body, 0);
}

impl Default for HitPriority {
    fn default() -> Self {
        HitPriority::BODY
    }
}

/// One registered shape of the shape record: a root's, a member's or a part's.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Shape<O> {
    /// The geometry.
    pub shape: HitShape,
    /// What it senses.
    pub sense: Sense,
    /// The component that owns it: the arena root or a member.
    pub owner: O,
    /// The part, `None` for the owner's own root shape.
    pub part: Option<PartRef>,
    /// The rank inside its stack position.
    pub priority: HitPriority,
    /// The stack position, bottom to top: the component's own parts, each member in show order, the
    /// component's `over` parts.
    pub position: u32,
    /// The shape is in the body layer's space (a member of a transformed body), not the root's.
    pub local: bool,
    /// The least size across the hit area has, in points.
    pub min_across: f32,
}

impl<O> Shape<O> {
    /// A shape of `owner` at stack `position`.
    pub const fn new(shape: HitShape, sense: Sense, owner: O, part: Option<PartRef>, priority: HitPriority, position: u32) -> Self {
        Shape { shape, sense, owner, part, priority, position, local: false, min_across: 0.0 }
    }

    /// The hit area is at least `across` points wide and high.
    pub const fn min_across(mut self, across: f32) -> Self {
        self.min_across = across;
        self
    }

    /// The shape lies in the body layer's space.
    pub const fn local(mut self, local: bool) -> Self {
        self.local = local;
        self
    }
}

/// What a resolution is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolveFor {
    /// A press: only shapes that sense a click or a drag take part, as in egui.
    Press,
    /// Hover: every registered shape takes part.
    Hover,
}

/// The shape that won a resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Winner<O> {
    /// Its place in the list that was resolved.
    pub index: usize,
    /// Its owner.
    pub owner: O,
    /// Its part.
    pub part: Option<PartRef>,
    /// What it senses.
    pub sense: Sense,
    /// Its shape lies in the body layer's space.
    pub local: bool,
}

/// The winner among `shapes` at `point`: stack position first, then level, sub-level, the nearest centre, the last
/// registered. A press considers only shapes that sense a click or a drag.
pub fn resolve<O: Copy>(shapes: &[Shape<O>], point: Pos2, what: ResolveFor) -> Option<Winner<O>> {
    resolve_mapped(shapes, point, what, |_| point)
}

/// [`resolve`] where shapes with `local` set are tested at `to_local(point)`.
pub fn resolve_mapped<O: Copy>(shapes: &[Shape<O>], point: Pos2, what: ResolveFor, to_local: impl Fn(Pos2) -> Pos2) -> Option<Winner<O>> {
    let at = |shape: &Shape<O>| if shape.local { to_local(point) } else { point };
    let distance = |shape: &Shape<O>| shape.shape.bounds().center().distance(at(shape));
    shapes
        .iter()
        .enumerate()
        .filter(|(_, shape)| what == ResolveFor::Hover || shape.sense.takes_press())
        .filter(|(_, shape)| shape.shape.hit_across(at(shape), shape.min_across))
        .max_by(|(ia, a), (ib, b)| {
            (a.position, a.priority.level, a.priority.sub)
                .cmp(&(b.position, b.priority.level, b.priority.sub))
                .then_with(|| distance(b).partial_cmp(&distance(a)).unwrap_or(Ordering::Equal))
                .then(ia.cmp(ib))
        })
        .map(|(index, shape)| Winner { index, owner: shape.owner, part: shape.part, sense: shape.sense, local: shape.local })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part::PartKey;
    use emath::pos2;

    const PART: PartKey = PartKey::new("p");

    fn rect(x: f32, y: f32, w: f32, h: f32) -> HitShape {
        Rect::from_min_size(pos2(x, y), Vec2::new(w, h)).into()
    }

    fn shape(shape: HitShape, sense: Sense, owner: u8, position: u32, level: HitLevel, sub: u8) -> Shape<u8> {
        Shape::new(shape, sense, owner, Some(PART.into()), HitPriority::new(level, sub), position)
    }

    #[test]
    fn inside_one_position_level_then_sub_then_nearest_then_last_decide() {
        let at = pos2(50.0, 50.0);
        let owner = |shapes: &[Shape<u8>]| resolve(shapes, at, ResolveFor::Press).map(|w| w.owner);
        let big = || rect(0.0, 0.0, 100.0, 100.0);
        let level = [shape(big(), Sense::CLICK, 1, 0, HitLevel::Edge, 0), shape(big(), Sense::CLICK, 2, 0, HitLevel::Body, 5)];
        assert_eq!(owner(&level), Some(1));
        let sub = [shape(big(), Sense::CLICK, 1, 0, HitLevel::Body, 3), shape(big(), Sense::CLICK, 2, 0, HitLevel::Body, 1)];
        assert_eq!(owner(&sub), Some(1));
        let near = [
            shape(rect(0.0, 0.0, 100.0, 100.0), Sense::CLICK, 1, 0, HitLevel::Body, 0),
            shape(rect(40.0, 40.0, 20.0, 20.0), Sense::CLICK, 2, 0, HitLevel::Body, 0),
        ];
        assert_eq!(owner(&near), Some(2));
        let tie = [shape(big(), Sense::CLICK, 1, 0, HitLevel::Body, 0), shape(big(), Sense::CLICK, 2, 0, HitLevel::Body, 0)];
        assert_eq!(owner(&tie), Some(2));
    }
}
