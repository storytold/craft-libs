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
    /// A chain of segments, hit within `reach` ([`LINE_REACH`] unless [`HitShape::polyline_within`] says otherwise).
    #[non_exhaustive]
    Polyline {
        /// The corners in order.
        points: Vec<Pos2>,
        /// Whether the last corner joins the first.
        closed: bool,
        /// How far from the chain a point still hits it, in points.
        reach: f32,
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
    /// A shape with discs cut out of it: a widget part over a bounding-box handle leaves the handle's disc to the
    /// handle.
    #[non_exhaustive]
    Excluding {
        /// The shape the discs are cut out of.
        shape: Box<HitShape>,
        /// The discs cut out, each a centre and a radius.
        holes: Vec<(Pos2, f32)>,
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

    /// A chain of segments, closed when `closed`, hit within [`LINE_REACH`].
    pub fn polyline(points: Vec<Pos2>, closed: bool) -> Self {
        HitShape::Polyline { points, closed, reach: LINE_REACH }
    }

    /// A chain of segments, closed when `closed`, hit within `reach` points: a tool whose measured reach differs from
    /// a line's. A negative or non-finite `reach` is [`LINE_REACH`].
    pub fn polyline_within(points: Vec<Pos2>, closed: bool, reach: f32) -> Self {
        let reach = if reach.is_finite() && reach >= 0.0 { reach } else { LINE_REACH };
        HitShape::Polyline { points, closed, reach }
    }

    /// The part of `within` outside `polygon`.
    pub fn outside_polygon(within: Rect, polygon: Vec<Pos2>) -> Self {
        HitShape::OutsidePolygon { within, polygon }
    }

    /// `shape` without the discs `holes` (a centre and a radius each).
    pub fn excluding(shape: HitShape, holes: Vec<(Pos2, f32)>) -> HitShape {
        HitShape::Excluding { shape: Box::new(shape), holes }
    }

    /// The smallest rectangle around the shape.
    pub fn bounds(&self) -> Rect {
        match self {
            HitShape::Rect(rect) | HitShape::OutsidePolygon { within: rect, .. } => *rect,
            HitShape::Excluding { shape, .. } => shape.bounds(),
            HitShape::Circle { center, radius } => Rect::from_center_size(*center, Vec2::splat(radius * 2.0)),
            HitShape::Segment { a, b } => Rect::from_two_pos(*a, *b),
            HitShape::Polyline { points, .. } | HitShape::Polygon(points) => {
                points.iter().fold(Rect::NOTHING, |bounds, point| bounds.union(Rect::from_min_max(*point, *point)))
            }
        }
    }

    /// The shape moved by `by`.
    #[must_use]
    pub fn translated(&self, by: Vec2) -> Self {
        let points = |points: &[Pos2]| points.iter().map(|point| *point + by).collect();
        match self {
            HitShape::Rect(rect) => HitShape::Rect(rect.translate(by)),
            HitShape::Circle { center, radius } => HitShape::Circle { center: *center + by, radius: *radius },
            HitShape::Segment { a, b } => HitShape::Segment { a: *a + by, b: *b + by },
            HitShape::Polyline { points: line, closed, reach } => HitShape::Polyline { points: points(line), closed: *closed, reach: *reach },
            HitShape::Polygon(polygon) => HitShape::Polygon(points(polygon)),
            HitShape::OutsidePolygon { within, polygon } => HitShape::OutsidePolygon { within: within.translate(by), polygon: points(polygon) },
            HitShape::Excluding { shape, holes } => HitShape::Excluding {
                shape: Box::new(shape.translated(by)),
                holes: holes.iter().map(|(center, radius)| (*center + by, *radius)).collect(),
            },
        }
    }

    /// The shape with each coordinate taken from `to`'s where the two lie within `within` points, when both are the
    /// same kind of shape with as many points: two shapes `==` finds equal after it differ by no more than `within`.
    #[must_use]
    pub fn snapped(&self, to: &HitShape, within: f32) -> Self {
        let near = |value: f32, to: f32| if (value - to).abs() <= within { to } else { value };
        let pos = |value: Pos2, to: Pos2| Pos2::new(near(value.x, to.x), near(value.y, to.y));
        let rect = |value: Rect, to: Rect| Rect::from_min_max(pos(value.min, to.min), pos(value.max, to.max));
        let points = |values: &[Pos2], to: &[Pos2]| {
            if values.len() == to.len() { values.iter().zip(to).map(|(value, to)| pos(*value, *to)).collect() } else { values.to_vec() }
        };
        match (self, to) {
            (HitShape::Rect(value), HitShape::Rect(to)) => HitShape::Rect(rect(*value, *to)),
            (HitShape::Circle { center, radius }, HitShape::Circle { center: to_center, radius: to_radius }) => {
                HitShape::Circle { center: pos(*center, *to_center), radius: near(*radius, *to_radius) }
            }
            (HitShape::Segment { a, b }, HitShape::Segment { a: to_a, b: to_b }) => HitShape::Segment { a: pos(*a, *to_a), b: pos(*b, *to_b) },
            (HitShape::Polyline { points: line, closed, reach }, HitShape::Polyline { points: to_line, reach: to_reach, .. }) => {
                HitShape::Polyline { points: points(line, to_line), closed: *closed, reach: near(*reach, *to_reach) }
            }
            (HitShape::Polygon(polygon), HitShape::Polygon(to_polygon)) => HitShape::Polygon(points(polygon, to_polygon)),
            (HitShape::OutsidePolygon { within: area, polygon }, HitShape::OutsidePolygon { within: to_area, polygon: to_polygon }) => {
                HitShape::OutsidePolygon { within: rect(*area, *to_area), polygon: points(polygon, to_polygon) }
            }
            (HitShape::Excluding { shape, holes }, HitShape::Excluding { shape: to_shape, holes: to_holes }) => {
                let holes = if holes.len() == to_holes.len() {
                    holes
                        .iter()
                        .zip(to_holes)
                        .map(|((center, radius), (to_center, to_radius))| (pos(*center, *to_center), near(*radius, *to_radius)))
                        .collect()
                } else {
                    holes.clone()
                };
                HitShape::Excluding { shape: Box::new(shape.snapped(to_shape, within)), holes }
            }
            _ => self.clone(),
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
            HitShape::Polyline { points, closed, reach: own } => {
                let reach = own.max(across / 2.0);
                let closing = closed.then(|| points.first().zip(points.last())).flatten();
                points.windows(2).any(|pair| segment_distance(point, pair[0], pair[1]) <= reach)
                    || closing.is_some_and(|(first, last)| segment_distance(point, *last, *first) <= reach)
            }
            HitShape::OutsidePolygon { within, polygon } => within.contains(point) && !inside_polygon(point, polygon),
            HitShape::Excluding { shape, holes } => shape.hit_across(point, across) && !in_hole(holes, point),
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
            HitShape::Polyline { points, closed, reach } => {
                let closing = closed.then(|| points.first().zip(points.last())).flatten();
                points.windows(2).any(|pair| segment_distance(point, pair[0], pair[1]) <= *reach)
                    || closing.is_some_and(|(first, last)| segment_distance(point, *last, *first) <= *reach)
            }
            HitShape::Polygon(points) => inside_polygon(point, points),
            HitShape::OutsidePolygon { within, polygon } => within.contains(point) && !inside_polygon(point, polygon),
            HitShape::Excluding { shape, holes } => shape.hit(point) && !in_hole(holes, point),
        }
    }
}

/// Whether `point` lies inside one of the discs `holes`.
fn in_hole(holes: &[(Pos2, f32)], point: Pos2) -> bool {
    holes.iter().any(|(center, radius)| center.distance(point) <= *radius)
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
    /// The named stacking order: stacked parts of one priority rank by it before the nearest centre, the higher order
    /// winning.
    pub stacking: u32,
    /// Whether a passed drag targets the arena root instead of the next shape.
    pub drag_to_root: bool,
}

impl<O: Clone> Shape<O> {
    /// The shape moved by `by`.
    #[must_use]
    pub fn translated(&self, by: Vec2) -> Self {
        Shape { shape: self.shape.translated(by), owner: self.owner.clone(), ..*self }
    }

    /// The shape with its geometry and least size across taken from `to`'s where they lie within `within` points
    /// ([`HitShape::snapped`]).
    #[must_use]
    pub fn snapped(&self, to: &Shape<O>, within: f32) -> Self {
        let min_across = if (self.min_across - to.min_across).abs() <= within { to.min_across } else { self.min_across };
        Shape { shape: self.shape.snapped(&to.shape, within), owner: self.owner.clone(), min_across, ..*self }
    }
}

impl<O> Shape<O> {
    /// A shape of `owner` at stack `position`.
    pub const fn new(shape: HitShape, sense: Sense, owner: O, part: Option<PartRef>, priority: HitPriority, position: u32) -> Self {
        Shape { shape, sense, owner, part, priority, position, local: false, min_across: 0.0, stacking: 0, drag_to_root: false }
    }

    /// The hit area is at least `across` points wide and high.
    pub const fn min_across(mut self, across: f32) -> Self {
        self.min_across = across;
        self
    }

    /// The shape's stacking order among shapes of its priority: the higher order wins before the nearest centre.
    pub const fn stacking(mut self, order: u32) -> Self {
        self.stacking = order;
        self
    }

    /// The shape lies in the body layer's space.
    pub const fn local(mut self, local: bool) -> Self {
        self.local = local;
        self
    }

    /// Whether `point` lies in the hit area: the shape grown to be at least its `min_across` wide and high. The
    /// test a resolution keeps a shape by.
    pub fn hits(&self, point: Pos2) -> bool {
        self.shape.hit_across(point, self.min_across)
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
    /// A passed drag targets the arena root.
    pub drag_to_root: bool,
}

/// The winner among `shapes` at `point`: stack position first, then level, sub-level, stacking order, the nearest
/// centre, the last registered. A press considers only shapes that sense a click or a drag.
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
        .filter(|(_, shape)| shape.hits(at(shape)))
        .max_by(|(ia, a), (ib, b)| {
            (a.position, a.priority.level, a.priority.sub, a.stacking)
                .cmp(&(b.position, b.priority.level, b.priority.sub, b.stacking))
                .then_with(|| distance(b).partial_cmp(&distance(a)).unwrap_or(Ordering::Equal))
                .then(ia.cmp(ib))
        })
        .map(|(index, shape)| Winner {
            index,
            owner: shape.owner,
            part: shape.part,
            sense: shape.sense,
            local: shape.local,
            drag_to_root: shape.drag_to_root,
        })
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

    #[test]
    fn stacked_parts_of_one_priority_rank_by_their_order_before_the_nearest_centre() {
        let at = pos2(50.0, 50.0);
        let owner = |shapes: &[Shape<u8>]| resolve(shapes, at, ResolveFor::Press).map(|w| w.owner);
        let near = || shape(rect(40.0, 40.0, 20.0, 20.0), Sense::CLICK, 1, 0, HitLevel::Body, 0);
        let far = || shape(rect(0.0, 0.0, 200.0, 200.0), Sense::CLICK, 2, 0, HitLevel::Body, 0);
        assert_eq!(owner(&[far(), near()]), Some(1), "unstacked, the nearer centre wins");
        assert_eq!(owner(&[far().stacking(1), near()]), Some(2), "the higher stacking order wins though the press is nearer the other's centre");
        assert_eq!(owner(&[near().stacking(1), far().stacking(1)]), Some(1), "one order: the nearest centre decides");
    }

    #[test]
    fn a_polyline_hits_within_its_own_reach_and_a_bad_reach_is_a_lines() {
        let points = vec![pos2(0.0, 0.0), pos2(100.0, 0.0)];
        let line = HitShape::polyline(points.clone(), false);
        assert!(line.hit(pos2(50.0, LINE_REACH)) && !line.hit(pos2(50.0, LINE_REACH + 0.5)));
        let wide = HitShape::polyline_within(points.clone(), false, 5.0);
        assert!(wide.hit(pos2(50.0, 5.0)) && wide.hit(pos2(50.0, -5.0)) && !wide.hit(pos2(50.0, 5.5)));
        let hits = |shape: HitShape, y: f32| Shape::new(shape, Sense::CLICK, 0_u8, None, HitPriority::BODY, 0).min_across(8.0).hits(pos2(50.0, y));
        assert!(hits(wide.clone(), 5.0) && !hits(wide, 5.5), "a target size under twice the reach keeps the reach");
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(HitShape::polyline_within(points.clone(), false, bad), line);
        }
    }
}
