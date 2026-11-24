//! Geometric primitives, like points and lines.

use arrayvec::ArrayVec;
use kurbo::{CubicBez, ParamCurve, ParamCurveExtrema, QuadBez};
use polycool::Quadratic;

use crate::curve::{quadratic_signs, solve_t_for_y, solve_x_for_y};
use crate::num::CheapOrderedFloat;

/// A two-dimensional point.
///
/// Points are sorted by `y` and then by `x`, for the convenience of our sweep-line
/// algorithm (which moves in increasing `y`).
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, PartialEq)]
pub struct Point {
    /// Vertical coordinate.
    ///
    /// Although it isn't important for functionality, the documentation and method naming
    /// assumes that larger values are down.
    pub y: f64,
    /// Horizontal component.
    ///
    /// Although it isn't important for functionality, the documentation and method naming
    /// assumes that larger values are to the right.
    pub x: f64,
}

impl Ord for Point {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            CheapOrderedFloat::from(self.y),
            CheapOrderedFloat::from(self.x),
        )
            .cmp(&(
                CheapOrderedFloat::from(other.y),
                CheapOrderedFloat::from(other.x),
            ))
    }
}

impl PartialOrd for Point {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for Point {}

impl std::fmt::Debug for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:?}, {:?})", self.x, self.y)
    }
}

impl Point {
    /// Create a new point.
    ///
    /// Note that the `x` coordinate comes first. This might be a tiny bit
    /// confusing because we're sorting by `y` coordinate first, but `(x, y)` is
    /// the only sane order (prove me wrong).
    pub fn new(x: f64, y: f64) -> Self {
        debug_assert!(x.is_finite());
        debug_assert!(y.is_finite());
        Point { x, y }
    }

    /// Compute an affine combination between `self` and `other`; that is, `(1 - t) * self + t * other`.
    ///
    /// Panics if a NaN is encountered, which might happen if some input is infinite.
    pub fn affine(&self, other: &Self, t: f64) -> Self {
        Point {
            x: (1.0 - t) * self.x + t * other.x,
            y: (1.0 - t) * self.y + t * other.y,
        }
    }

    /// Compatibility with `kurbo` points.
    pub fn to_kurbo(self) -> kurbo::Point {
        kurbo::Point::new(self.x, self.y)
    }

    /// Compatibility with `kurbo` points.
    pub fn from_kurbo(c: kurbo::Point) -> Self {
        Self::new(c.x, c.y)
    }
}

impl From<(f64, f64)> for Point {
    fn from((x, y): (f64, f64)) -> Self {
        Self { x, y }
    }
}

impl From<Point> for kurbo::Point {
    fn from(p: Point) -> Self {
        kurbo::Point::new(p.x, p.y)
    }
}

impl From<kurbo::Point> for Point {
    fn from(p: kurbo::Point) -> Self {
        Point::new(p.x, p.y)
    }
}

/// A contour segment.
#[derive(Clone, PartialEq, Eq)]
pub struct Segment {
    inner: SegmentInner,
}

#[derive(Clone, PartialEq, Eq)]
enum SegmentInner {
    Line {
        p0: Point,
        p1: Point,
    },
    Quadratic {
        p0: Point,
        p1: Point,
        p2: Point,
    },
    Cubic {
        p0: Point,
        p1: Point,
        p2: Point,
        p3: Point,
    },
}

impl std::fmt::Debug for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner {
            SegmentInner::Line { p0, p1 } => write!(f, "{p0:?} -- {p1:?}"),
            SegmentInner::Quadratic { p0, p1, p2 } => write!(f, "{p0:?} -- {p1:?} -- {p2:?}"),
            SegmentInner::Cubic { p0, p1, p2, p3 } => {
                write!(f, "{p0:?} -- {p1:?} -- {p2:?} -- {p3:?}")
            }
        }
    }
}

/// A y-monotonic piece of a cubic Bezier.
#[derive(Debug)]
pub(crate) struct MonotonicPiece {
    pub start_t: f64,
    pub end_t: f64,
    pub piece: Segment,
    pub positively_oriented: bool,
}

pub(crate) fn monotonic_pieces(s: kurbo::PathSeg) -> ArrayVec<MonotonicPiece, 3> {
    match s {
        kurbo::PathSeg::Line(ell) => {
            let mut ret = ArrayVec::new();
            let p0: Point = ell.p0.into();
            let p1: Point = ell.p1.into();
            let (p0, p1, positively_oriented) = if p0 <= p1 {
                (p0, p1, true)
            } else {
                (p1, p0, false)
            };
            if p0 != p1 {
                ret.push(MonotonicPiece {
                    start_t: 0.0,
                    end_t: 1.0,
                    piece: Segment::straight(p0, p1),
                    positively_oriented,
                });
            }
            ret
        }
        kurbo::PathSeg::Quad(q) => monotonic_quadratic_pieces(q),
        kurbo::PathSeg::Cubic(c) => monotonic_cubic_pieces(c),
    }
}

/// This could return an `ArrayVec<_, 2>`, but by returning `ArrayVec<_, 3>` we
/// make it the same return type as `monotonic_cubic_pieces`.
fn monotonic_quadratic_pieces(q: QuadBez) -> ArrayVec<MonotonicPiece, 3> {
    fn to_seg(q: QuadBez) -> Option<(Segment, bool)> {
        let (p0, mut p1, p2, orient) = if (q.p0.y, q.p0.x) <= (q.p2.y, q.p2.x) {
            (q.p0, q.p1, q.p2, true)
        } else {
            (q.p2, q.p1, q.p0, false)
        };
        if p0 != p2 {
            // Our derivative-solving should have made this cubic monotonic, but
            // there might be some numerical issues with the starting and ending
            // tangents. Fix them up.
            p1.y = p0.y.max(p1.y).min(p2.y);
            let seg = Segment::monotonic_quadratic(p0.into(), p1.into(), p2.into());
            Some((seg, orient))
        } else {
            None
        }
    }

    // The derivative is 2(1 - t) (p1 - p0) + 2t (p2 - p1), and we want to solve
    // for when the y coordinate of this derivative is zero.
    let t = (q.p0.y - q.p1.y) / (q.p2.y - 2.0 * q.p1.y + q.p0.y);
    if 0.0 < t && t < 1.0 {
        let q0 = q.subsegment(0.0..t);
        let q1 = q.subsegment(t..1.0);
        if let (Some((piece0, orient0)), Some((piece1, orient1))) = (to_seg(q0), to_seg(q1)) {
            let mut ret = ArrayVec::new();
            ret.push(MonotonicPiece {
                start_t: 0.0,
                end_t: t,
                piece: piece0,
                positively_oriented: orient0,
            });
            ret.push(MonotonicPiece {
                start_t: t,
                end_t: 1.0,
                piece: piece1,
                positively_oriented: orient1,
            });
            return ret;
        }
    }

    let mut ret = ArrayVec::new();
    if let Some((piece, positively_oriented)) = to_seg(q) {
        ret.push(MonotonicPiece {
            start_t: 0.0,
            end_t: 1.0,
            piece,
            positively_oriented,
        });
    }
    ret
}

pub(crate) fn monotonic_cubic_pieces(cub: CubicBez) -> ArrayVec<MonotonicPiece, 3> {
    fn to_seg(c: CubicBez) -> Option<(Segment, bool)> {
        let (p0, mut p1, mut p2, p3, orient) = if (c.p0.y, c.p0.x) <= (c.p3.y, c.p3.x) {
            (c.p0, c.p1, c.p2, c.p3, true)
        } else {
            (c.p3, c.p2, c.p1, c.p0, false)
        };
        if p0 != p3 {
            // Our derivative-solving should have made this cubic monotonic, but
            // there might be some numerical issues with the starting and ending
            // tangents. Fix them up.
            p1.y = p0.y.max(p1.y);
            p2.y = p3.y.min(p2.y);
            let seg = Segment::monotonic_cubic(p0.into(), p1.into(), p2.into(), p3.into());
            Some((seg, orient))
        } else {
            None
        }
    }

    let mut ret = ArrayVec::new();
    let q0 = cub.p1.y - cub.p0.y;
    let q1 = cub.p2.y - cub.p1.y;
    let q2 = cub.p3.y - cub.p2.y;

    // Convert to the representation a t^2 + b t + c.
    let c = q0;
    let b = (q1 - q0) * 2.0;
    let a = q0 - q1 * 2.0 + q2;
    let q = Quadratic::new([c, b, a]);
    let (_, r0, r1) = quadratic_signs(q);

    let mut roots: ArrayVec<f64, 2> = ArrayVec::new();
    if r0 > 0.0 && r0 < 1.0 {
        roots.push(r0);
    }
    if r1 > r0 && r1 > 0.0 && r1 < 1.0 {
        roots.push(r1);
    }

    let mut last_r = 0.0;
    // We could try to remove very short segments, which could easily happen
    // if the input curve has an initial or final tangent that's very close
    // to horizontal. For now, we only remove segments that are exactly
    // zero-length.
    for r in roots {
        let piece_before = cub.subsegment(last_r..r);
        if let Some((piece, positively_oriented)) = to_seg(piece_before) {
            ret.push(MonotonicPiece {
                piece,
                positively_oriented,
                start_t: last_r,
                end_t: r,
            });
            last_r = r;
        }
    }

    let piece_before = cub.subsegment(last_r..1.0);
    if let Some((piece, positively_oriented)) = to_seg(piece_before) {
        ret.push(MonotonicPiece {
            piece,
            positively_oriented,
            start_t: last_r,
            end_t: 1.0,
        });
    } else if let Some(last_piece) = ret.last_mut() {
        last_piece.end_t = 1.0;
    }

    ret
}

impl Segment {
    /// Create a new cubic segment that must be increasing in `y`.
    pub fn monotonic_cubic(p0: Point, p1: Point, p2: Point, p3: Point) -> Self {
        if p3.y == p0.y {
            // Ensure that horizontal segments are just represented by straight lines.
            // TODO: maybe we should do something about degenerate S-shaped curves?
            Self::straight(p0, p3)
        } else {
            Self {
                inner: SegmentInner::Cubic { p0, p1, p2, p3 },
            }
        }
    }

    /// Create a new quadratic segment that must be increasing in `y`.
    pub fn monotonic_quadratic(p0: Point, p1: Point, p2: Point) -> Self {
        if p2.y == p0.y {
            // Ensure that horizontal segments are just represented by straight lines.
            // TODO: maybe we should do something about degenerate C-shaped curves?
            Self::straight(p0, p2)
        } else {
            Self {
                inner: SegmentInner::Quadratic { p0, p1, p2 },
            }
        }
    }

    /// Create a new segment that's just a straight line.
    ///
    /// `start` must be less than `end`.
    pub fn straight(start: Point, end: Point) -> Self {
        debug_assert!(start <= end);
        Self {
            inner: SegmentInner::Line { p0: start, p1: end },
        }
    }

    /// The starting point (smallest in the sweep-line order) of this segment.
    pub fn start(&self) -> Point {
        match self.inner {
            SegmentInner::Line { p0, .. }
            | SegmentInner::Quadratic { p0, .. }
            | SegmentInner::Cubic { p0, .. } => p0,
        }
    }

    /// The ending point (largest in the sweep-line order) of this segment.
    pub fn end(&self) -> Point {
        match self.inner {
            SegmentInner::Line { p1, .. } => p1,
            SegmentInner::Quadratic { p2, .. } => p2,
            SegmentInner::Cubic { p3, .. } => p3,
        }
    }

    /// Compatibility with `kurbo`'s cubics.
    pub fn to_kurbo_cubic(&self) -> kurbo::CubicBez {
        match self.inner {
            SegmentInner::Line { p0: q0, p1: q1 } => {
                let p0 = q0.to_kurbo();
                let p3 = q1.to_kurbo();
                let p1 = p0 + (1. / 3.) * (p3 - p0);
                let p2 = p0 + (2. / 3.) * (p3 - p0);
                kurbo::CubicBez { p0, p1, p2, p3 }
            }
            SegmentInner::Quadratic {
                p0: q0,
                p1: q1,
                p2: q2,
            } => {
                let p0 = q0.to_kurbo();
                let p3 = q2.to_kurbo();
                let v1 = q1.to_kurbo().to_vec2();
                let p1 = ((p0 + 2.0 * v1).to_vec2() / 3.0).to_point();
                let p2 = ((p3 + 2.0 * v1).to_vec2() / 3.0).to_point();
                kurbo::CubicBez { p0, p1, p2, p3 }
            }
            SegmentInner::Cubic { p0, p1, p2, p3 } => kurbo::CubicBez {
                p0: p0.to_kurbo(),
                p1: p1.to_kurbo(),
                p2: p2.to_kurbo(),
                p3: p3.to_kurbo(),
            },
        }
    }

    /// Compatibility with kurbo.
    pub fn to_kurbo_seg(&self) -> kurbo::PathSeg {
        match self.inner {
            SegmentInner::Line { p0, p1 } => kurbo::Line {
                p0: p0.to_kurbo(),
                p1: p1.to_kurbo(),
            }
            .into(),
            SegmentInner::Quadratic { p0, p1, p2 } => kurbo::QuadBez {
                p0: p0.to_kurbo(),
                p1: p1.to_kurbo(),
                p2: p2.to_kurbo(),
            }
            .into(),
            SegmentInner::Cubic { p0, p1, p2, p3 } => kurbo::CubicBez {
                p0: p0.to_kurbo(),
                p1: p1.to_kurbo(),
                p2: p2.to_kurbo(),
                p3: p3.to_kurbo(),
            }
            .into(),
        }
    }

    /// TODO: maybe From instead?
    pub fn from_kurbo_cubic(c: kurbo::CubicBez) -> Self {
        Self::monotonic_cubic(
            Point::from_kurbo(c.p0),
            Point::from_kurbo(c.p1),
            Point::from_kurbo(c.p2),
            Point::from_kurbo(c.p3),
        )
    }

    /// Is this segment just a straight line?
    pub fn is_line(&self) -> bool {
        matches!(self.inner, SegmentInner::Line { .. })
    }

    /// If this segment is a line, returns it.
    pub fn as_kurbo_line(&self) -> Option<kurbo::Line> {
        match self.inner {
            SegmentInner::Line { p0, p1 } => Some(kurbo::Line::new(p0.to_kurbo(), p1.to_kurbo())),
            _ => None,
        }
    }

    /// A crude lower bound on our minimum horizontal position.
    pub fn min_x(&self) -> f64 {
        match self.inner {
            SegmentInner::Line { p0, p1 } => p0.x.min(p1.x),
            SegmentInner::Quadratic { p0, p1, p2 } => p0.x.min(p1.x).min(p2.x),
            SegmentInner::Cubic { p0, p1, p2, p3 } => p0.x.min(p1.x).min(p2.x).min(p3.x),
        }
    }

    /// A crude upper bound on our maximum horizontal position.
    pub fn max_x(&self) -> f64 {
        match self.inner {
            SegmentInner::Line { p0, p1 } => p0.x.max(p1.x),
            SegmentInner::Quadratic { p0, p1, p2 } => p0.x.max(p1.x).max(p2.x),
            SegmentInner::Cubic { p0, p1, p2, p3 } => p0.x.max(p1.x).max(p2.x).max(p3.x),
        }
    }

    /// Our `x` coordinate at the given `y` coordinate.
    ///
    /// Horizontal segments will return their largest `x` coordinate.
    ///
    /// # Panics
    ///
    /// Panics if `y` is outside the `y` range of this segment.
    pub fn at_y(&self, y: f64) -> f64 {
        debug_assert!(
            (self.start().y..=self.end().y).contains(&y),
            "segment {self:?}, y={y:?}"
        );

        match self.inner {
            SegmentInner::Line { p0, p1 } => {
                if self.is_horizontal() {
                    p1.x
                } else {
                    let t = (y - p0.y) / (p1.y - p0.y);
                    p0.x + t * (p1.x - p0.x)
                }
            }
            // We could have a more optimized implementation for qudratics.
            SegmentInner::Quadratic { .. } | SegmentInner::Cubic { .. } => {
                solve_x_for_y(self.to_kurbo_cubic(), y)
            }
        }
    }

    fn local_bbox(&self, y: f64, eps: f64) -> kurbo::Rect {
        let start_y = (y - eps).max(self.start().y);
        let end_y = (y + eps).min(self.end().y);

        match self.inner {
            SegmentInner::Line { .. } => {
                let start_x = self.at_y(start_y);
                let end_x = self.at_y(end_y);
                kurbo::Rect::from_points((start_x, start_y), (end_x, end_y))
            }
            SegmentInner::Quadratic { .. } | SegmentInner::Cubic { .. } => {
                let c = self.to_kurbo_cubic();
                let t_min = solve_t_for_y(c, start_y);
                let t_max = solve_t_for_y(c, end_y);

                c.subsegment(t_min..t_max).bounding_box()
            }
        }
    }

    /// Returns a lower bound on the `x` coordinate near `y`.
    ///
    /// More precisely, take a Minkowski sum between this curve
    /// and a small square. This returns a lower bound on the `x`
    /// coordinate of the resulting set at height `y`.
    pub fn lower(&self, y: f64, eps: f64) -> f64 {
        self.local_bbox(y, eps).min_x() - eps
    }

    /// Returns an upper bound on the `x` coordinate near `y`.
    ///
    /// More precisely, take a Minkowski sum between this curve
    /// and a small square. This returns a upper bound on the `x`
    /// coordinate of the resulting set at height `y`.
    pub fn upper(&self, y: f64, eps: f64) -> f64 {
        self.local_bbox(y, eps).max_x() + eps
    }

    /// Returns true if this segment is exactly horizontal.
    pub fn is_horizontal(&self) -> bool {
        match self.inner {
            SegmentInner::Line { p0, p1 } => p0.y == p1.y,
            SegmentInner::Cubic { .. } | SegmentInner::Quadratic { .. } => {
                debug_assert_ne!(
                    self.start().y,
                    self.end().y,
                    "horizontal segments should be represented by lines"
                );
                false
            }
        }
    }
}
