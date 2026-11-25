//! The winding number of a path round a point, counted by the crossings of a ray that points left, and
//! the crossings of a cubic with a horizontal line it is made of.
//!
//! `kurbo`'s `BezPath::winding` finds where a monotone piece of a cubic crosses the height of the point by the
//! cubic formula, and the formula loses the root where the cubic term is rounding noise beside the others, as it is
//! for an arch whose handles are level (control points `(0, 0)`, `(100/3, h)`, `(200/3, h)`, `(100, 0)`: the
//! coefficient of `t^3` is about 1e-15, and the root of 0.1732 comes out as 0.0578). A point is then counted on the
//! wrong side of the curve, and what is inside is told from what is outside wrongly over a good part of the arch. The
//! planar map reads what covers each face from such windings, so a face got the wrong inputs.
//!
//! Here each cubic is cut where its height turns (a quadratic, solved without subtracting nearly equal numbers),
//! and the pieces, which rise or fall the whole way, are solved for the height of the point by Newton's method held
//! inside the range the heights seen leave, and then bisected: never by the cubic formula. The crossing found is as
//! exact as the arithmetic of the curve's own evaluation, so a point farther from the path than the rounding of its
//! coordinates is counted on the right side. The work is bounded: a segment whose box the point is outside is turned
//! away at once, and a cubic takes at most [`NEWTON_STEPS`] and [`MAX_BISECTIONS`] evaluations of its height for each
//! of its (at most three) pieces. The rules are `kurbo`'s, so that nothing changes where it was right: a piece takes
//! its lower end and not its upper (the crossings at a vertex are counted once), a crossing at the point's own `x` is
//! to its left, a rising piece counts -1 and a falling one +1, and a line is tested as `kurbo` tests it.
//!
//! # The numeric domain
//!
//! - **Which numbers.** Any finite coordinates, of any size the doubles hold, and any scale, uniform or not. A segment with a coordinate that is not
//!   finite winds nothing, and so does a point with a coordinate that is not a number; a point whose `x` is infinite is beyond every finite one, and one
//!   whose `y` is infinite is level with none.
//! - **Scale.** The winding does not change when an axis is multiplied by a positive number, so each segment is wound in a frame ([`Frame`]): each axis is
//!   multiplied by the power of two that brings the greatest magnitude of that axis among the control points to between 1 and 2, and the point has the same
//!   factors. A power of two scales every operation exactly, so the result is the same to the bit as it is at unit size on both axes (a segment 2^1000
//!   across in `x` and 2^-1000 in `y` winds as it does at unit size, and a quadratic from 1e308 to -1e308 is raised to a cubic of numbers), and no
//!   sum or product below overflows: each is under 16 in the frame. A slope of nothing, or a step to an infinity, ends the Newton method as it did.
//!   A point that is too small for the frame is the least number of its sign and not zero, so that it stays on its own side of a control point of exactly 0.
//! - **Range.** What the doubles cannot hold at once is lost as they lose it: in a segment whose greatest height is 1 (the greatest magnitude of an axis
//!   is always 1 to 2 in the frame) a control point under 2^-1022 on that axis is 0, and a point's coordinate under it is the least number of its sign.
//! - **Accuracy.** The heights are compared exactly: a point is over, at or under the end of a piece to the bit. The crossing of a piece with the height of
//!   a point is found at a parameter where the curve's height is within 8 units in the last place of the segment's greatest height of the point's (the
//!   greatest residual measured over 80,000 pieces and heights, curves 1, 1e-5 and 3e13 across, level handles, flat ends and heights within 1e-40 of the
//!   ends of a piece, was 5): the `x` of it is as good as the heights can tell, which is the piece's own extent where the curve is flat to within that
//!   rounding (a height within a few units of a turn's), and a point nearer to the crossing than that is counted on either side. Newton's method is
//!   ended by a step of 1e-13 of the piece's range of the parameter and not of the parameter's (a curve 2^45 across, whose crossing is 0.29 from its start,
//!   has a piece of 3e-14 of the parameter), and the bisection that follows a method that leaves its range halves that range 64 times.
//!
//! [`y_pieces`] and [`crossing_x`], which [`crate::offset_path`] reads the spans of a path from, are in the same frame: they bring their cubic to it and give
//! the results back in the cubic's own numbers.

use kurbo::{BezPath, CubicBez, PathSeg, Point, QuadBez};

use crate::work::{self, Work};

/// Most halvings of the parameter range of one piece: 2^-64 of it, past what `f64` tells apart over a unit.
pub(crate) const MAX_BISECTIONS: usize = 64;

/// Most steps of Newton's method for the crossing of one piece, before the range it has narrowed is bisected.
pub(crate) const NEWTON_STEPS: usize = 12;

/// A Newton step this short beside the parameter range of the piece (which runs from `lo` to `hi`, of at most 1) is the last: the method converges
/// quadratically, so the error left is the square of it, under what rounding can tell. It is beside the range and not absolute, so that a piece that
/// is shorter than the step (a curve 2^45 across whose first quarter unit is a tiny piece of the parameter) is not taken for found at the first step.
const NEWTON_DONE: f64 = 1e-13;

/// `2^e` as a number, for `e` from -1022 to 1023.
fn pow2(e: i32) -> f64 {
    f64::from_bits(u64::try_from((e + 1023).clamp(1, 2046)).unwrap_or(1) << 52)
}

/// `v` times `2^e`, in as many steps as it takes (a power of two is a number only from -1022 to 1023): exact where the result is a normal number.
fn times_pow2(mut v: f64, mut e: i32) -> f64 {
    while e > 1000 {
        v *= pow2(1000);
        e -= 1000;
    }
    while e < -1000 {
        v *= pow2(-1000);
        e += 1000;
    }
    v * pow2(e)
}

/// The exponent of the finite `m`: `e` where `2^e <= |m| < 2^(e + 1)`; 0 for 0.
fn exponent_of(m: f64) -> i32 {
    let m = m.abs();
    if m == 0.0 {
        return 0;
    }
    if m < f64::MIN_POSITIVE {
        // A subnormal number is lifted by 2^200 to read it, and the lift is taken off.
        return exponent_of(m * pow2(200)) - 200;
    }
    i32::try_from((m.to_bits() >> 52) & 0x7ff).unwrap_or(1023) - 1023
}

/// The scale a segment is wound at: a power of two for each axis, which brings the greatest magnitude of that axis among the control points to between
/// 1 and 2 (see the module). The winding of a segment round a point is the same in it as in the numbers they came in, and the arithmetic is the same to
/// the bit, scaled, where it does not underflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    /// The `x` of a point is multiplied by 2 to the power of this.
    x: i32,
    /// The same for `y`.
    y: i32,
}

impl Frame {
    /// The frame in which the numbers are as they came.
    const UNIT: Self = Self { x: 0, y: 0 };

    /// The frame of control points that are all finite; `None` where one is not (a NaN or an infinity).
    fn of(points: &[Point]) -> Option<Self> {
        let (mut mx, mut my) = (0.0_f64, 0.0_f64);
        for p in points {
            if !(p.x.is_finite() && p.y.is_finite()) {
                return None;
            }
            mx = mx.max(p.x.abs());
            my = my.max(p.y.abs());
        }
        Some(Self { x: -exponent_of(mx), y: -exponent_of(my) })
    }

    /// A control point in the frame.
    fn control(self, p: Point) -> Point {
        Point::new(times_pow2(p.x, self.x), times_pow2(p.y, self.y))
    }

    /// `c` in the frame.
    fn cubic(self, c: &CubicBez) -> CubicBez {
        CubicBez::new(self.control(c.p0), self.control(c.p1), self.control(c.p2), self.control(c.p3))
    }

    /// A coordinate of the point that is wound round, in the frame: infinite where it is further from the segment than the doubles reach (it compares as it
    /// should), and where it is a number too small for the frame it is the least number of its sign (so that a height under 2^-1022 of the segment's, just
    /// under a piece's end of 0, is not taken for 0: `-0.0` is not less than `0.0`).
    fn query(v: f64, e: i32) -> f64 {
        let r = times_pow2(v, e);
        if r == 0.0 && v != 0.0 { f64::from_bits(1).copysign(v) } else { r }
    }

    /// The point that is wound round, in the frame.
    fn point(self, p: Point) -> Point {
        Point::new(Self::query(p.x, self.x), Self::query(p.y, self.y))
    }

    /// An `x` of the frame in the numbers it came in.
    fn unscale_x(self, x: f64) -> f64 {
        times_pow2(x, -self.x)
    }

    /// A `y` of the frame in the numbers it came in.
    fn unscale_y(self, y: f64) -> f64 {
        times_pow2(y, -self.y)
    }
}

/// The winding number of the closed subpaths of `path` round `p` (the segments are those of
/// [`BezPath::segments`], a `ClosePath` giving the line back to the start): each segment's crossing with a ray
/// from `p` to the left counts -1 where the segment rises and +1 where it falls. It is `kurbo::Shape::winding`,
/// made right for cubics (see the module).
pub(crate) fn winding(path: &BezPath, p: Point) -> i32 {
    // `Region::contains` takes a path a caller made, and kurbo's `segments` panics on one that begins with a close.
    crate::path::without_leading_closes(path).segments().fold(0_i32, |w, seg| w.saturating_add(segment_winding(seg, p)))
}

/// The winding of one segment round `p`: the crossing of the ray to the left of `p` with it, -1, 0 or +1 for each
/// monotone piece of it.
///
/// A segment with a coordinate that is not finite winds nothing, and neither does a point that is not a number (a point with an infinite `x` is beyond
/// every finite one, and with an infinite `y` is level with none): the box of the control points drops a NaN (`f64::min` and `f64::max` return the other
/// operand), so a cubic whose finite ends straddle the point's height would count a crossing it has no way to find. Everything the crate sweeps has been
/// refused for a coordinate that is not finite first ([`crate::PathOpsError::NonFinite`]); [`crate::Region::contains`] takes a path a caller made.
pub(crate) fn segment_winding(seg: PathSeg, p: Point) -> i32 {
    if p.x.is_nan() || p.y.is_nan() {
        return 0;
    }
    match seg {
        PathSeg::Line(l) => line_winding(l.p0, l.p1, p),
        PathSeg::Quad(q) => quad_winding(q, p),
        PathSeg::Cubic(c) => cubic_winding(&c, p),
    }
}

/// A line's crossing with the ray, as `kurbo` tests it: the end with the smaller `y` is the line's and the other is
/// not, and a point on the line (as the arithmetic finds it) is to the right of it.
fn line_winding(start: Point, end: Point, p: Point) -> i32 {
    if !(start.x.is_finite() && start.y.is_finite() && end.x.is_finite() && end.y.is_finite()) {
        return 0;
    }
    let sign = if end.y > start.y {
        if !(p.y >= start.y && p.y < end.y) {
            return 0;
        }
        -1
    } else if end.y < start.y {
        if !(p.y >= end.y && p.y < start.y) {
            return 0;
        }
        1
    } else {
        return 0;
    };
    if p.x < start.x.min(end.x) {
        return 0;
    }
    if p.x >= start.x.max(end.x) {
        return sign;
    }
    // The point is inside the line's box. The line `a x + b y = c`, and which side of it the point is on, in the frame: every product and sum is under 16, and
    // each of the two terms of the side is the kurbo's scaled by 2^(fx + fy), so the sign is the same where kurbo's arithmetic works.
    let Some(frame) = Frame::of(&[start, end]) else { return 0 };
    let (s, e, q) = (frame.control(start), frame.control(end), frame.point(p));
    let a = e.y - s.y;
    let b = s.x - e.x;
    let c = a * s.x + b * s.y;
    if (a * q.x + b * q.y - c) * f64::from(sign) <= 0.0 { sign } else { 0 }
}

/// A quadratic's crossing with the ray: the quadratic raised to a cubic in the frame, where the differences that `QuadBez::raise` takes cannot overflow.
fn quad_winding(q: QuadBez, p: Point) -> i32 {
    // The box of the control points turns most segments away, before the frame is found.
    if outside_box(&[q.p0.x, q.p1.x, q.p2.x], &[q.p0.y, q.p1.y, q.p2.y], p) {
        return 0;
    }
    let Some(frame) = Frame::of(&[q.p0, q.p1, q.p2]) else { return 0 };
    let scaled = QuadBez::new(frame.control(q.p0), frame.control(q.p1), frame.control(q.p2));
    framed_cubic_winding(&scaled.raise(), frame.point(p))
}

/// A cubic's crossings with the ray, in the frame of its control points.
fn cubic_winding(c: &CubicBez, p: Point) -> i32 {
    // The box of the control points turns most segments away, before the frame is found.
    let (xs, ys) = coordinates(c);
    if outside_box(&xs, &ys, p) {
        return 0;
    }
    let Some(frame) = Frame::of(&[c.p0, c.p1, c.p2, c.p3]) else { return 0 };
    framed_cubic_winding(&frame.cubic(c), frame.point(p))
}

/// A cubic's crossings with the ray, the cubic and the point in the frame: its pieces whose height rises or falls the whole way, each of which crosses the
/// height of `p` once or not at all, and the crossing's `x` against the point's.
fn framed_cubic_winding(c: &CubicBez, p: Point) -> i32 {
    let (xs, ys) = coordinates(c);
    if outside_box(&xs, &ys, p) {
        return 0;
    }
    let x_max = bounds(&xs).1;
    let mut winding = 0;
    for piece in framed_pieces(c).iter() {
        if (piece.y_lo <= p.y) == (piece.y_hi <= p.y) {
            continue;
        }
        let sign = if piece.y_hi > piece.y_lo { -1 } else { 1 };
        // A point right of the whole box is right of the crossing, and the root need not be found.
        if p.x >= x_max || p.x >= framed_crossing_x(c, &piece, p.y) {
            winding += sign;
        }
    }
    winding
}

/// A piece of a cubic whose height rises or falls the whole way: the range `lo` to `hi` of its parameter, and the
/// height at each end.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Piece {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) y_lo: f64,
    pub(crate) y_hi: f64,
}

/// The pieces of a cubic ([`y_pieces`]): one to three.
pub(crate) struct Pieces {
    /// The parameter and the height at the start, at each turn of the height, and at the end.
    cuts: [(f64, f64); 4],
    len: usize,
}

impl Pieces {
    /// The pieces, from the start of the curve to its end.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Piece> + '_ {
        let cuts = self.cuts.get(..self.len).unwrap_or(&[]);
        cuts.windows(2).filter_map(|w| match w {
            [(lo, y_lo), (hi, y_hi)] => Some(Piece { lo: *lo, hi: *hi, y_lo: *y_lo, y_hi: *y_hi }),
            _ => None,
        })
    }
}

/// `c` cut where its height turns, so that each piece rises or falls the whole way. The turns are the roots of
/// `y'(t) / 3 = a (1 - t)^2 + 2 b (1 - t) t + d t^2` between 0 and 1 (`a`, `b` and `d` the steps of the height from
/// one control point to the next), found by [`quadratic_roots`]: the cubic's own coefficient `a - 2b + d` is nothing
/// but rounding for a curve whose handles are level, and a formula that divides by it finds the other root where it
/// is. The ends of the curve are its own ends exactly. The cubic is cut in the frame of its control points (a cubic that is not finite as it came), and the heights are given back
/// in its own numbers.
pub(crate) fn y_pieces(c: &CubicBez) -> Pieces {
    let frame = Frame::of(&[c.p0, c.p1, c.p2, c.p3]).unwrap_or(Frame::UNIT);
    let mut pieces = framed_pieces(&frame.cubic(c));
    for cut in pieces.cuts.iter_mut().take(pieces.len) {
        cut.1 = frame.unscale_y(cut.1);
    }
    if let Some(first) = pieces.cuts.first_mut() {
        first.1 = c.p0.y;
    }
    if let Some(last) = pieces.cuts.get_mut(pieces.len.saturating_sub(1)) {
        last.1 = c.p3.y;
    }
    pieces
}

/// [`y_pieces`] of a cubic in the frame, in the frame's numbers.
fn framed_pieces(c: &CubicBez) -> Pieces {
    let [y0, y1, y2, y3] = [c.p0.y, c.p1.y, c.p2.y, c.p3.y];
    let (a, b, d) = (y1 - y0, y2 - y1, y3 - y2);
    let mut turns = [0.0_f64; 2];
    let mut n = 0;
    for t in quadratic_roots(a - 2.0 * b + d, 2.0 * (b - a), a).into_iter().flatten() {
        if t > 0.0
            && t < 1.0
            && let Some(slot) = turns.get_mut(n)
        {
            *slot = t;
            n += 1;
        }
    }
    let [mut t0, mut t1] = turns;
    if n == 2 && t0 > t1 {
        std::mem::swap(&mut t0, &mut t1);
    }
    // Two equal roots are one turn.
    if n == 2 && t0 == t1 {
        n = 1;
    }
    let ys = [y0, y1, y2, y3];
    let mut cuts = [(0.0, y0), (1.0, y3), (1.0, y3), (1.0, y3)];
    let mut len = 1;
    for t in [t0, t1].into_iter().take(n) {
        if let Some(slot) = cuts.get_mut(len) {
            *slot = (t, bernstein(ys, t));
            len += 1;
        }
    }
    if let Some(slot) = cuts.get_mut(len) {
        *slot = (1.0, y3);
        len += 1;
    }
    Pieces { cuts, len }
}

/// The `x` at which the piece of `c` crosses the height `y`, which it does, in the numbers they came in: found in the frame of the control points ([`framed_crossing_x`]).
pub(crate) fn crossing_x(c: &CubicBez, piece: &Piece, y: f64) -> f64 {
    let frame = Frame::of(&[c.p0, c.p1, c.p2, c.p3]).unwrap_or(Frame::UNIT);
    let scaled = Piece { lo: piece.lo, hi: piece.hi, y_lo: times_pow2(piece.y_lo, frame.y), y_hi: times_pow2(piece.y_hi, frame.y) };
    frame.unscale_x(framed_crossing_x(&frame.cubic(c), &scaled, Frame::query(y, frame.y)))
}

/// The `x` at which the piece of `c` crosses the height `y`, which it does, found on the piece's parameter range by
/// Newton's method from where the straight line between the piece's ends crosses, held inside the range the heights
/// seen so far leave (a step that leaves it, or whose slope is nothing, ends the method), and then by bisection of
/// that range: the side of the height the piece's lower end is on is kept at one end and the other at the other, for
/// [`MAX_BISECTIONS`] halvings at most (a range halved to where it can no longer be is done). Each step is one
/// evaluation of the height, at most [`NEWTON_STEPS`] and [`MAX_BISECTIONS`] of them in all, and never the cubic
/// formula, so the rounding of coefficients that is the fault of that formula does not come into it. A height that is
/// one of the piece's own ends is crossed at that end, and the search is not made (see the body). Everything is in the frame.
fn framed_crossing_x(c: &CubicBez, piece: &Piece, y: f64) -> f64 {
    bernstein(coordinates(c).0, framed_crossing_t(c, piece, y))
}

/// The parameter at which the piece of `c` crosses the height `y` ([`framed_crossing_x`]).
fn framed_crossing_t(c: &CubicBez, piece: &Piece, y: f64) -> f64 {
    let ys = coordinates(c).1;
    // The heights of a piece's ends are what [`bernstein`] finds at its ends (the curve's own ends exactly, and a turn is
    // cut by it), so a height that is one of them is crossed at that end, to the bit. The search cannot say so: the guess is
    // then an end of the range, which it leaves for the middle, and it closes in on that end from one side, within 2^-64
    // of it and not on it, so that a point on the curve at the end of a piece could be told from the crossing by a hair and
    // counted on the wrong side of it (a rising straight cubic wound 0 where the line it lies on winds -1).
    if y == piece.y_lo {
        return piece.lo;
    }
    if y == piece.y_hi {
        return piece.hi;
    }
    let [y0, y1, y2, y3] = ys;
    let (a, b, d) = (y1 - y0, y2 - y1, y3 - y2);
    let at_or_below = piece.y_lo <= y;
    let (mut lo, mut hi) = (piece.lo, piece.hi);
    let done = NEWTON_DONE * (hi - lo);
    let guess = lo + (hi - lo) * ((y - piece.y_lo) / (piece.y_hi - piece.y_lo));
    let mut t = if guess > lo && guess < hi { guess } else { 0.5 * (lo + hi) };
    for _ in 0..NEWTON_STEPS {
        work::add(Work::WindingSteps, 1);
        let h = bernstein(ys, t);
        if (h <= y) == at_or_below {
            lo = t;
        } else {
            hi = t;
        }
        if h == y {
            return t;
        }
        // The slope of the height, `y'(t) = 3 (a (1 - t)^2 + 2 b (1 - t) t + d t^2)`.
        let mt = 1.0 - t;
        let next = t - (h - y) / (3.0 * (a * (mt * mt) + 2.0 * b * mt * t + d * (t * t)));
        if !(next > lo && next < hi) {
            break;
        }
        if (next - t).abs() <= done {
            return next;
        }
        t = next;
    }
    for _ in 0..MAX_BISECTIONS {
        let mid = 0.5 * (lo + hi);
        if !(mid > lo && mid < hi) {
            break;
        }
        work::add(Work::WindingSteps, 1);
        if (bernstein(ys, mid) <= y) == at_or_below {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The real roots of `a t^2 + b t + c` (none where it has none), found without the loss of the textbook formula
/// where `a` is small beside `b`, as it is for a curve whose height turns at the middle.
fn quadratic_roots(a: f64, b: f64, c: f64) -> [Option<f64>; 2] {
    if !(a.is_finite() && b.is_finite() && c.is_finite()) {
        return [None, None];
    }
    if a == 0.0 {
        return if b == 0.0 { [None, None] } else { [Some(-c / b), None] };
    }
    // The roots are those of the equation multiplied by any number, so the coefficients are brought to where `b b` and `4 a c`
    // can neither overflow nor underflow: at 1e155 both are infinite and their difference is not a number, at 1e-165 both are
    // nothing and the quadratic has a double root, and the turns of the curve were lost, or found where they are not. The
    // number is the power of two below the greatest coefficient (a subnormal one is lifted to the least normal), which scales
    // exactly: where the formula worked the roots are the same to the bit. (The steps of a height in the frame are under 4, so this is for the steps
    // that are small beside the greatest height of the curve.)
    const EXPONENT: u64 = 0x7ff0_0000_0000_0000;
    let scale = f64::from_bits(a.abs().max(b.abs()).max(c.abs()).to_bits() & EXPONENT).max(f64::MIN_POSITIVE);
    let (a, b, c) = (a / scale, b / scale, c / scale);
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 || !disc.is_finite() {
        return [None, None];
    }
    // The root that does not subtract nearly equal numbers, and the other from their product.
    let q = -0.5 * (b + disc.sqrt().copysign(b));
    if q == 0.0 { [Some(0.0), None] } else { [Some(q / a), Some(c / q)] }
}

/// The x and y of the four control points of `c`.
fn coordinates(c: &CubicBez) -> ([f64; 4], [f64; 4]) {
    ([c.p0.x, c.p1.x, c.p2.x, c.p3.x], [c.p0.y, c.p1.y, c.p2.y, c.p3.y])
}

/// The least and the greatest of some numbers.
fn bounds(v: &[f64]) -> (f64, f64) {
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

/// Is `p` outside the box of the control points `xs` and `ys` of a curve, or to the left of it (the curve is in its box, and the ray from `p` to the left
/// meets nothing there)? A NaN fails each comparison and is no crossing.
fn outside_box(xs: &[f64], ys: &[f64], p: Point) -> bool {
    let ((x_min, _), (y_min, y_max)) = (bounds(xs), bounds(ys));
    !(p.y >= y_min && p.y <= y_max && p.x >= x_min)
}

/// One coordinate of the curve with control values `v` at `t`, added in the order [`CubicBez::eval`] adds them.
fn bernstein(v: [f64; 4], t: f64) -> f64 {
    let [v0, v1, v2, v3] = v;
    let mt = 1.0 - t;
    v0 * (mt * mt * mt) + (v1 * (mt * mt * 3.0) + (v2 * (mt * 3.0) + v3 * t) * t) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Flattened, Rng, distance_to_path, ellipse, polyline, rect};
    use kurbo::Shape;
    use kurbo::{Affine, ParamCurve};

    #[test]
    fn the_coordinates_evaluate_as_the_curve_does() {
        let c = CubicBez::new((1.0, -2.0), (30.5, 10.25), (66.0, 3.5), (100.0, 0.125));
        let (xs, ys) = coordinates(&c);
        for k in 0..=20 {
            let t = f64::from(k) / 20.0;
            let p = c.eval(t);
            assert_eq!((bernstein(xs, t), bernstein(ys, t)), (p.x, p.y), "t = {t}");
        }
    }

    #[test]
    fn quadratic_roots_are_found_stably() {
        let roots = |a, b, c| quadratic_roots(a, b, c).into_iter().flatten().collect::<Vec<f64>>();
        let near_half = roots(1e-20, 2.0, -1.0);
        assert!(near_half.iter().any(|t| (t - 0.5).abs() < 1e-15), "{near_half:?}");
        assert_eq!(roots(0.0, 2.0, -1.0), vec![0.5]);
        assert!(roots(1.0, 0.0, 1.0).is_empty() && roots(0.0, 0.0, 1.0).is_empty());
        assert!(roots(f64::NAN, 1.0, 1.0).is_empty());
        let mut two = roots(1.0, -3.0, 2.0);
        two.sort_by(f64::total_cmp);
        assert_eq!(two, vec![1.0, 2.0]);
    }

    #[test]
    fn a_cubic_is_cut_where_its_height_turns() {
        // Level handles: one turn, at the middle, though the cubic coefficient is noise.
        let arch = CubicBez::new((0.0, 0.0), (100.0 / 3.0, 10.05), (200.0 / 3.0, 10.05), (100.0, 0.0));
        let pieces: Vec<Piece> = y_pieces(&arch).iter().collect();
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert!((pieces[0].hi - 0.5).abs() < 1e-15 && pieces[0].hi == pieces[1].lo, "{pieces:?}");
        assert_eq!((pieces[0].lo, pieces[1].hi), (0.0, 1.0));
        assert_eq!((pieces[0].y_lo, pieces[1].y_hi), (0.0, 0.0));
        assert!((pieces[0].y_hi - 7.5375).abs() < 1e-12 && pieces[0].y_hi == pieces[1].y_lo, "{pieces:?}");
        // An S: two turns, in order. A rising curve: none. A point: one piece that rises nowhere.
        let s = CubicBez::new((0.0, 0.0), (10.0, 10.0), (20.0, -10.0), (30.0, 0.0));
        let turns: Vec<f64> = y_pieces(&s).iter().map(|p| p.hi).collect();
        assert_eq!(turns.len(), 3);
        assert!(turns[0] < turns[1] && turns[1] < 1.0 && turns[2] == 1.0, "{turns:?}");
        let rising = CubicBez::new((0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 5.0));
        assert_eq!(y_pieces(&rising).iter().count(), 1);
        let point = CubicBez::new((4.0, 4.0), (4.0, 4.0), (4.0, 4.0), (4.0, 4.0));
        assert_eq!(y_pieces(&point).iter().count(), 1);
        // A horizontal tangent at an inflection (y = t^3): the two equal roots are one turn at most.
        let flat = CubicBez::new((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 1.0));
        assert!(y_pieces(&flat).iter().count() <= 2);
    }

    /// The closed arch of the level control points of the fault: from `(0, 0)` to `(100, 0)` through the handles
    /// `(100/3, h)` and `(200/3, h)`, closed by its chord. Its `x` is `100 t` and its height `3h t (1 - t)`, so it is
    /// the parabola `y = 3h u (1 - u)` for `u = x / 100`, and a point is inside where it is under that and over the chord.
    fn arch(h: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((100.0 / 3.0, h), (200.0 / 3.0, h), (100.0, 0.0));
        p.close_path();
        p
    }

    /// The winding of the arch of height `h` at a point, from the parabola: -1 inside (the ray to the left crosses the
    /// rising side) and 0 outside, or `None` within a hair of the curve or of the chord.
    fn arch_truth(h: f64, p: Point) -> Option<i32> {
        let u = p.x / 100.0;
        let top = 3.0 * h * u * (1.0 - u);
        if (0.0..=100.0).contains(&p.x) {
            if (p.y - top).abs() < 1e-6 || p.y.abs() < 1e-6 {
                return None;
            }
            return Some(if p.y > 0.0 && p.y < top { -1 } else { 0 });
        }
        Some(0)
    }

    /// The fault: an arch of handles at 10.05 was told wrongly in about a tenth of the points of a grid of 160 by 100
    /// (1,440 of 16,000, and 2,000 for others), and right with the handles at 10.0, 9.0 or 7.0.
    #[test]
    fn an_arch_whose_handles_are_level_is_wound_right_where_the_cubic_formula_fails() {
        for h in [10.05, 10.0, 9.0, 7.0, 10.1, 3.3, 0.05, 1e-3] {
            let path = arch(h);
            let flat = Flattened::new(&path, 1e-7);
            let mut wrong = 0;
            let mut tested = 0;
            for i in 0..160 {
                for j in 0..100 {
                    let p = Point::new(-5.0 + 110.0 * (f64::from(i) + 0.5) / 160.0, -1.0 + (3.0 * h + 2.0) * (f64::from(j) + 0.5) / 100.0);
                    let Some(truth) = arch_truth(h, p) else { continue };
                    tested += 1;
                    wrong += usize::from(winding(&path, p) != truth);
                    // The oracles agree with one another (on every seventh point, which is enough to say).
                    if (i + j) % 7 == 0 {
                        assert!(distance_to_path(&path, p) < 1e-5 || flat.winding(p) == truth, "h = {h}, {p:?}: {} against {truth}", flat.winding(p));
                    }
                }
            }
            assert!(tested > 15_000, "h = {h}: {tested} points tested");
            assert_eq!(wrong, 0, "h = {h}: {wrong} of {tested} points are wound wrongly");
        }
        // The point of the fault: (2.929, 2.385) is outside the arch (which is 0.857 high there).
        assert_eq!(winding(&arch(10.05), Point::new(2.929, 2.385)), 0);
        assert_eq!(winding(&arch(10.05), Point::new(2.929, 0.5)), -1);
    }

    /// A closed path of one cubic with the handles drawn from `rng`, and the heights of its ends and of its handles level
    /// as asked: level handles (`level`), and ends at one height (`arch`).
    fn random_cubic(rng: &mut Rng, level: bool, arch: bool) -> BezPath {
        let x = |rng: &mut Rng| rng.range(-50.0, 150.0);
        let (y0, mut y3) = (rng.range(-10.0, 10.0), rng.range(-10.0, 10.0));
        if arch {
            y3 = y0;
        }
        let (mut h1, mut h2) = (rng.range(-40.0, 40.0), rng.range(-40.0, 40.0));
        if level {
            h2 = h1;
        }
        if rng.unit() < 0.1 {
            h1 = y0;
            h2 = y3;
        }
        let mut p = BezPath::new();
        p.move_to((0.0, y0));
        p.curve_to((x(rng), h1), (x(rng), h2), (100.0, y3));
        p.close_path();
        p
    }

    /// Cubics with level handles are wound as a fine flattening winds them, at points as near the path as 1e-5 of its size
    /// and no nearer, so are the others: arches, general cubics and quadratics.
    #[test]
    fn random_cubics_are_wound_as_their_flattening_is_wound() {
        let mut rng = Rng(0x00c0_ffee_1234_5679);
        let mut tested = 0;
        for case in 0..400 {
            let path = match case % 4 {
                0 => random_cubic(&mut rng, true, true),
                1 => random_cubic(&mut rng, true, false),
                2 => random_cubic(&mut rng, false, false),
                _ => {
                    let mut q = BezPath::new();
                    q.move_to((0.0, rng.range(-10.0, 10.0)));
                    q.quad_to((rng.range(-50.0, 150.0), rng.range(-40.0, 40.0)), (100.0, rng.range(-10.0, 10.0)));
                    q.close_path();
                    q
                }
            };
            let flat = Flattened::new(&path, 1e-8);
            for _ in 0..100 {
                let p = Point::new(rng.range(-60.0, 160.0), rng.range(-50.0, 50.0));
                if distance_to_path(&path, p) < 1e-5 {
                    continue;
                }
                tested += 1;
                assert_eq!(winding(&path, p), flat.winding(p), "case {case}, {p:?}: {}", path.to_svg());
            }
        }
        assert!(tested > 39_000, "{tested}");
    }

    /// Lines are wound as `kurbo` winds them, a point on an edge or at a vertex included: the polygons are on a grid
    /// and so are the points, to the half unit.
    #[test]
    fn lines_are_wound_as_kurbo_winds_them() {
        let mut rng = Rng(0xfeed_beef_0001);
        for case in 0..300 {
            let n = 3 + (rng.unit() * 5.0) as usize;
            let pts: Vec<(f64, f64)> = (0..n).map(|_| ((rng.unit() * 8.0).floor(), (rng.unit() * 8.0).floor())).collect();
            let path = polyline(&pts, true);
            for i in 0..=18 {
                for j in 0..=18 {
                    let p = Point::new(f64::from(i) * 0.5 - 0.5, f64::from(j) * 0.5 - 0.5);
                    assert_eq!(winding(&path, p), path.winding(p), "case {case}, {p:?}: {}", path.to_svg());
                }
            }
        }
    }

    /// A curve is wound as `kurbo` winds it where `kurbo` is right, at the heights of its vertices and of its turns too
    /// (a piece takes its lower end and not its upper, so a vertex is counted once): the four quarter arcs of a circle
    /// on a grid of half units, among them the points of its four vertices, and a rectangle through them.
    #[test]
    fn a_vertex_at_the_height_of_the_point_is_counted_once() {
        let circle = ellipse(kurbo::Rect::new(0.0, 0.0, 8.0, 8.0));
        let mut on_the_circle = 0;
        for i in -2..=18 {
            for j in -2..=18 {
                let p = Point::new(f64::from(i) * 0.5, f64::from(j) * 0.5);
                if (p - Point::new(4.0, 4.0)).hypot() == 4.0 {
                    on_the_circle += 1;
                }
                // The left vertex is on the circle, at the lower end of the arc that falls into it, and the crossing there is at
                // the point's own `x`, which is to its left: it winds as a point on the left edge of a square does (+1, the
                // same in `kurbo`). `kurbo`'s solver puts the root a hair right of the vertex and winds 0, and so did this
                // crate before a height that is a piece's own end was crossed at that end exactly.
                let want = if p == Point::new(0.0, 4.0) { 1 } else { circle.winding(p) };
                assert_eq!(winding(&circle, p), want, "{p:?}");
            }
        }
        let (square, left_edge) = (rect(0.0, 0.0, 8.0, 8.0), Point::new(0.0, 4.0));
        assert_eq!((winding(&square, left_edge), square.winding(left_edge)), (1, 1), "a point on the left edge of a square");
        assert_eq!(on_the_circle, 4, "the four vertices are among the points");
        // The heights of the turns: inside and outside, at the top and bottom of the circle exactly (the left vertex is above).
        for p in [(4.0, 0.0), (4.0, 8.0), (3.0, 0.0), (3.0, 8.0), (8.0, 4.0), (4.0, 4.0)] {
            let p = Point::from(p);
            assert_eq!(winding(&circle, p), circle.winding(p), "{p:?}");
        }
        // A rising and a falling piece meeting at the height of a point count once between them (one -1 and one +1 at the
        // top, and a falling piece from the top vertex of the diamond).
        let diamond = polyline(&[(4.0, 0.0), (8.0, 4.0), (4.0, 8.0), (0.0, 4.0)], true);
        assert_eq!(winding(&diamond, Point::new(4.0, 4.0)), 1);
        assert_eq!(winding(&diamond, Point::new(4.0, 8.0)), 0, "the top vertex is the upper end of both its edges");
        assert_eq!(winding(&diamond, Point::new(2.0, 8.0)), 0);
        assert_eq!(winding(&diamond, Point::new(9.0, 4.0)), 0, "right of the right vertex, where a rising edge ends and a falling one starts");
        assert_eq!(winding(&diamond, Point::new(4.0, 0.0)), diamond.winding(Point::new(4.0, 0.0)));
        assert_eq!(winding(&rect(0.0, 0.0, 8.0, 8.0), Point::new(3.0, 8.0)), 0, "the upper edge of a rectangle is out");
    }

    /// The winding of a path of several subpaths is the sum of theirs, whichever way each goes (a hole is wound against
    /// its outline), and an open subpath is wound as the segments it has, none closing it.
    #[test]
    fn subpaths_add_up() {
        let mut ring = rect(0.0, 0.0, 10.0, 10.0);
        ring.extend(rect(3.0, 3.0, 7.0, 7.0).reverse_subpaths().iter());
        assert_eq!(winding(&ring, Point::new(1.0, 5.0)), 1);
        assert_eq!(winding(&ring, Point::new(5.0, 5.0)), 0);
        let mut twice = rect(0.0, 0.0, 10.0, 10.0);
        twice.extend(rect(3.0, 3.0, 7.0, 7.0).iter());
        assert_eq!(winding(&twice, Point::new(5.0, 5.0)), 2);
        let open = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], false);
        // No closing edge, so no left side: the ray from inside crosses nothing, and from the right the right side only.
        assert_eq!(winding(&open, Point::new(5.0, 5.0)), 0);
        assert_eq!(winding(&open, Point::new(11.0, 5.0)), -1);
        assert_eq!(winding(&BezPath::new(), Point::new(1.0, 1.0)), 0);
    }

    /// A cubic whose control points lie on a line, as that line is wound (`line_winding`), at the heights of its ends and between them, to
    /// the left of the line, on it, and to the right.
    fn assert_wound_as_its_line(c: CubicBez) {
        let line = kurbo::Line::new(c.p0, c.p3);
        for y in [0.0, 1.5, 3.0, -1.0, 4.0] {
            for x in [-1.0, 0.0, 0.5, 1.5, 2.0, 3.0, 4.0] {
                let at = Point::new(x, y);
                assert_eq!(segment_winding(PathSeg::Cubic(c), at), segment_winding(PathSeg::Line(line), at), "{c:?} at {at:?}");
            }
        }
    }

    /// A point at the height of the lower end of a rising piece, on the curve there, is wound as it is on the straight line the control
    /// points lie on: the crossing is at the end, exactly (`(0, 0)`, `(1, 1)`, `(2, 2)`, `(3, 3)` at `(0, 0)` was 0 for -1, for the guess
    /// of the crossing is the end itself, which the search leaves for the middle and then closes in on from above).
    #[test]
    fn a_point_at_the_lower_end_of_a_rising_cubic_is_wound_as_its_line() {
        let straight = CubicBez::new((0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0));
        assert_eq!(segment_winding(PathSeg::Cubic(straight), Point::new(0.0, 0.0)), -1);
        assert_wound_as_its_line(straight);
    }

    /// The same for a falling piece, whose lower end is its last point: the guess is the upper end of the range, and the search closes in on
    /// it from below.
    #[test]
    fn a_point_at_the_lower_end_of_a_falling_cubic_is_wound_as_its_line() {
        let straight = CubicBez::new((3.0, 3.0), (2.0, 2.0), (1.0, 1.0), (0.0, 0.0));
        assert_eq!(segment_winding(PathSeg::Cubic(straight), Point::new(0.0, 0.0)), 1);
        assert_wound_as_its_line(straight);
    }

    /// The crossing of a piece at the height of its lower end is the `x` of that end, to the bit, for every piece of a cut cubic (the heights at
    /// a cut are found by the same arithmetic as the crossing's): a point on the curve at its vertex is wound by that, and not by a crossing
    /// that the search has closed in on to within 2^-64 of the end.
    #[test]
    fn the_crossing_at_a_pieces_own_end_is_the_x_of_that_end() {
        let curves = [
            CubicBez::new((0.0, 0.0), (100.0 / 3.0, 10.05), (200.0 / 3.0, 10.05), (100.0, 0.0)),
            CubicBez::new((0.0, 0.0), (10.0, 10.0), (20.0, -10.0), (30.0, 0.0)),
            CubicBez::new((0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)),
            CubicBez::new((3.0, 3.0), (2.0, 2.0), (1.0, 1.0), (0.0, 0.0)),
            CubicBez::new((1.0, -2.0), (30.5, 10.25), (66.0, 3.5), (100.0, 0.125)),
            CubicBez::new((1.0, 1.0), (7.0, 9.0), (-3.0, 4.0), (5.0, -6.0)),
        ];
        let mut pieces = 0;
        for c in curves {
            let (xs, _) = coordinates(&c);
            for piece in y_pieces(&c).iter() {
                // The end the piece takes is its lower, in height: the first of a rising piece and the last of a falling one.
                let (end, y) = if piece.y_hi > piece.y_lo { (piece.lo, piece.y_lo) } else { (piece.hi, piece.y_hi) };
                assert_eq!(crossing_x(&c, &piece, y).to_bits(), bernstein(xs, end).to_bits(), "{c:?}, {piece:?}");
                pieces += 1;
            }
        }
        assert!(pieces >= 10, "{pieces}");
    }

    /// The S of `(0, 0)`, `(s, s)`, `(2 s, -s)`, `(3 s, 0)` closed by its chord, which is level: the curve winds +1 round the points of its
    /// lower lobe and -1 round those of its upper, at every scale `s`.
    fn s_curve(s: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((s, s), (2.0 * s, -s), (3.0 * s, 0.0));
        p.close_path();
        p
    }

    /// `2^k` exactly, for `k` from -1022 to 1023.
    fn power_of_two(k: i32) -> f64 {
        f64::from_bits(u64::try_from(k + 1023).expect("an exponent of a normal number") << 52)
    }

    /// The windings of the S at its own scale on a grid, away from the curve: the lower lobe is +1, the upper -1, the rest 0.
    fn s_windings() -> Vec<(Point, i32)> {
        let path = s_curve(1.0);
        let flat = Flattened::new(&path, 1e-9);
        let mut grid = Vec::new();
        for i in 0..=16 {
            for j in 0..12 {
                let p = Point::new(f64::from(i) * 0.25 - 0.5, (f64::from(j) + 0.5) * 0.1 - 0.6);
                if distance_to_path(&path, p) < 1e-3 {
                    continue;
                }
                let w = winding(&path, p);
                assert_eq!(w, flat.winding(p), "{p:?}");
                grid.push((p, w));
            }
        }
        grid
    }

    /// The point of the finding: a cubic at a scale where the discriminant of the quadratic that finds its turns overflows (`s = 1e155`,
    /// the products `b b` and `4 a c` of its coefficients, 36 s^2 and 24 s^2, are infinite and their difference is not a number) was cut
    /// nowhere: one piece whose ends are level, and the lower lobe wound 0 where it winds +1.
    #[test]
    fn a_cubic_of_huge_coordinates_keeps_its_turns() {
        let s = 1e155;
        let lobe = Point::new(2.25 * s, -0.1 * s);
        assert_eq!(winding(&s_curve(1.0), Point::new(2.25, -0.1)), 1, "the lower lobe, at unit scale");
        assert_eq!(winding(&s_curve(s), lobe), 1, "the lower lobe, at {s}");
        assert_eq!(winding(&s_curve(s), Point::new(0.75 * s, 0.1 * s)), -1, "the upper lobe");
        assert_eq!(winding(&s_curve(s), Point::new(3.5 * s, -0.1 * s)), 0, "right of the whole");
        let c = CubicBez::new((0.0, 0.0), (s, s), (2.0 * s, -s), (3.0 * s, 0.0));
        assert_eq!(y_pieces(&c).iter().count(), 3, "two turns, three pieces");
    }

    /// The other end of the range: at 2^-560 (1e-169) the products are under the smallest number there is, and the discriminant is nothing,
    /// so the quadratic had a double root where it has two (the cuts were at 1/2 and 1/3 and not at 0.2113 and 0.7887), and the lower lobe
    /// wound nothing; at 1e-165 the same.
    #[test]
    fn a_cubic_of_tiny_coordinates_keeps_its_turns() {
        for s in [power_of_two(-560), 1e-165, 3e-200] {
            let c = CubicBez::new((0.0, 0.0), (s, s), (2.0 * s, -s), (3.0 * s, 0.0));
            let turns: Vec<f64> = y_pieces(&c).iter().map(|p| p.hi).collect();
            assert_eq!(turns.len(), 3, "{s}: {turns:?}");
            assert!((turns[0] - 0.211_324_865_405_187).abs() < 1e-12 && (turns[1] - 0.788_675_134_594_813).abs() < 1e-12, "{s}: {turns:?}");
            assert_eq!(winding(&s_curve(s), Point::new(2.25 * s, -0.1 * s)), 1, "{s}");
        }
    }

    /// At the top of the range (1e300 and past it) the steps of the heights and their second difference are not numbers: for a handle that goes up
    /// by 3e307 and a handle that goes down by as much the second difference is 1.8e308, and the quadratic found no turns, so that the lower lobe
    /// of the curve wound nothing.
    #[test]
    fn a_cubic_at_the_top_of_the_range_keeps_its_turns() {
        for s in [3e307, 2.9e307, -3e307, 1e300, 1.5e308] {
            let c = CubicBez::new((0.0, 0.0), (s, s), (2.0 * s, -s), (3.0 * s, 0.0));
            if !(c.p3.x.is_finite() && c.p2.x.is_finite()) {
                continue;
            }
            let turns: Vec<f64> = y_pieces(&c).iter().map(|p| p.hi).collect();
            assert_eq!(turns.len(), 3, "{s}: {turns:?}");
            assert!((turns[0] - 0.211_324_865_405_187).abs() < 1e-12 && (turns[1] - 0.788_675_134_594_813).abs() < 1e-12, "{s}: {turns:?}");
        }
        // The two lobes wind as they do at unit scale (the points are well inside them).
        let lobes = |s: f64| [Point::new(2.25 * s, -0.1 * s), Point::new(0.75 * s, 0.1 * s)].map(|p| winding(&s_curve(s), p));
        let unit = lobes(1.0);
        assert!(unit[0] == 1 && unit[1] != 0, "{unit:?}");
        for s in [3e307, 1e300] {
            assert_eq!(lobes(s), unit, "at {s}");
        }
    }

    /// A path and its points scaled by a power of two wind as at unit scale to the last (the arithmetic scales exactly), from 2^-560 to 2^515,
    /// and by scales that are not powers of two (the points are away from the curve) from 1e-165 to 1e155.
    #[test]
    fn the_winding_of_a_cubic_does_not_depend_on_its_scale() {
        let grid = s_windings();
        assert!(grid.iter().any(|&(_, w)| w == 1) && grid.iter().any(|&(_, w)| w == -1) && grid.len() > 150, "{} points", grid.len());
        for s in [
            power_of_two(-560),
            power_of_two(-300),
            power_of_two(-60),
            power_of_two(60),
            power_of_two(300),
            power_of_two(515),
            1e-165,
            1e-100,
            1e100,
            1e155,
        ] {
            let path = s_curve(s);
            for &(p, w) in &grid {
                assert_eq!(winding(&path, Point::new(p.x * s, p.y * s)), w, "scale {s}, {p:?}");
            }
        }
    }

    /// The roots of the quadratic are the same at every scale of its coefficients, to the bit, wherever the scaled coefficients are numbers
    /// (a root does not change when the equation is multiplied by a number, and a power of two multiplies exactly).
    #[test]
    fn quadratic_roots_do_not_depend_on_the_scale_of_the_coefficients() {
        for (a, b, c) in [(6.0, -6.0, 1.0), (1.0, -3.0, 2.0), (1e-20, 2.0, -1.0), (3.0, 1.0, -1.0), (-0.75, 5.0, 0.25)] {
            let unit = quadratic_roots(a, b, c);
            assert!(unit.iter().flatten().count() > 0, "{a} {b} {c}");
            for k in (-900..=900).step_by(25) {
                let m = power_of_two(k);
                let scaled = quadratic_roots(a * m, b * m, c * m);
                let bits = |r: [Option<f64>; 2]| r.map(|t| t.map(f64::to_bits));
                assert_eq!(bits(scaled), bits(unit), "({a}, {b}, {c}) times 2^{k}");
            }
        }
    }

    /// A curve whose handles are 1e308 out (the sums of the Bernstein form, 1.5e308 and 0.5e308, overflow though the curve does not: its x at the
    /// middle is 5e307) winds the points beside it as the same curve at a hundredth of the size does: a point to the right of the crossing at height
    /// 1.5 is wound -1 (a rising curve), and the sum overflowed to a crossing that is nowhere.
    #[test]
    fn a_cubic_with_handles_at_the_top_of_the_range_winds_as_a_smaller_one_does() {
        let segment = |s: f64| PathSeg::Cubic(CubicBez::new((0.0, 0.0), (0.0, 1.0), (s, 2.0), (s, 3.0)));
        let big = 1e308;
        for (x, y) in [(0.75, 1.5), (0.6, 1.4), (0.4, 1.6), (0.95, 2.5), (0.05, 0.4), (0.3, 1.5), (0.99, 0.5)] {
            let small = segment_winding(segment(0.01 * big), Point::new(x * 0.01 * big, y));
            assert_eq!(segment_winding(segment(big), Point::new(x * big, y)), small, "({x}, {y})");
        }
        assert_eq!(segment_winding(segment(big), Point::new(0.75 * big, 1.5)), -1);
    }

    /// A quadratic from 1e308 to -1e308 and back is raised to a cubic whose controls are numbers (the difference of its x is 2e308): its crossing
    /// of the height 1 is at x = 0, so a point to the left of it is wound 0 (the controls were -infinity and the crossing was nowhere), and the other
    /// points are wound as they are by the same quadratic at unit scale.
    #[test]
    fn a_quadratic_at_the_top_of_the_range_is_raised_to_a_cubic_of_numbers() {
        let quad = |s: f64| PathSeg::Quad(QuadBez::new((s, 0.0), (-s, 1.0), (s, 2.0)));
        assert_eq!(segment_winding(quad(1e308), Point::new(-1.0, 1.0)), 0);
        for (x, y) in [(0.25, 1.0), (0.9, 0.5), (-0.5, 1.5), (0.5, 1.9)] {
            assert_eq!(segment_winding(quad(1e308), Point::new(x * 1e308, y)), segment_winding(quad(1.0), Point::new(x, y)), "({x}, {y})");
        }
    }

    /// Where the coordinates are past 2^1021 the products of the line's equation overflow though the line's own steps do not (the slope 1.5
    /// times a point's x of 2.75 x 2^1022 is 4.1 x 2^1022): the line wound its point as if it were left of the crossing, for a crossing to its right.
    /// At every scale up to the top the line and its points wind as at unit scale.
    #[test]
    fn a_line_at_the_top_of_the_range_winds_as_at_unit_scale() {
        let line = |s: f64| PathSeg::Line(kurbo::Line::new((2.0 * s, 2.0 * s), (3.0 * s, 3.5 * s)));
        let grid: Vec<(f64, f64)> = (0..=40).flat_map(|i| (0..=40).map(move |j| (1.5 + f64::from(i) * 0.05, 1.5 + f64::from(j) * 0.1))).collect();
        let unit: Vec<i32> = grid.iter().map(|&(x, y)| segment_winding(line(1.0), Point::new(x, y))).collect();
        assert!(unit.iter().any(|&w| w != 0) && unit.contains(&0));
        for k in [1020, 1021, 1022] {
            let s = power_of_two(k);
            for (&(x, y), &w) in grid.iter().zip(&unit) {
                // Not the points the line passes within a thousandth of a unit of: there the sides are the arithmetic's.
                let off = ((x - 2.0) * 1.5 - (y - 2.0)).abs() / 1.8;
                if off > 1e-3 && 3.5 * s < f64::MAX {
                    assert_eq!(segment_winding(line(s), Point::new(x * s, y * s)), w, "2^{k}, ({x}, {y})");
                }
            }
        }
        let s = power_of_two(1022);
        assert_eq!(segment_winding(line(s), Point::new(2.75 * s, 3.25 * s)), segment_winding(line(1.0), Point::new(2.75, 3.25)));
    }

    /// A line and its points scaled by a power of two wind as at unit scale to the last (the arithmetic scales exactly): from 2^-560, where
    /// every product of the line's equation is nothing and every point is on it, to 2^515, where the product of its slope and the start's
    /// coordinate is infinite and the side of the point is not a number, and at 2^1022, where the line is wider than a double holds.
    #[test]
    fn the_winding_of_a_line_does_not_depend_on_its_scale() {
        let line = |s: f64, a: (f64, f64), b: (f64, f64)| PathSeg::Line(kurbo::Line::new((a.0 * s, a.1 * s), (b.0 * s, b.1 * s)));
        // The points a half apart, on the line (3, 5) among them, from outside it on either side to inside.
        let grid: Vec<(f64, f64)> = (-4..=20).flat_map(|i| (-4..=24).map(move |j| (f64::from(i) * 0.5, f64::from(j) * 0.5))).collect();
        for (a, b) in [((1.0, 1.0), (5.0, 9.0)), ((5.0, 9.0), (1.0, 1.0)), ((1.0, 9.0), (5.0, 1.0)), ((2.0, 1.0), (3.0, 11.0))] {
            let unit = line(1.0, a, b);
            let want: Vec<i32> = grid.iter().map(|&p| segment_winding(unit, Point::from(p))).collect();
            assert!(want.iter().any(|&w| w != 0) && want.contains(&0), "{a:?} to {b:?}");
            for k in [-560, -300, -60, 60, 300, 515] {
                let s = power_of_two(k);
                let scaled = line(s, a, b);
                for (&p, &w) in grid.iter().zip(&want) {
                    assert_eq!(segment_winding(scaled, Point::new(p.0 * s, p.1 * s)), w, "{a:?} to {b:?} at 2^{k}, {p:?}");
                }
            }
            // Not a power of two, the points away from the line (a thousandth of its length at the least): the same.
            for s in [1e-165, 1e-100, 1e100, 1e155] {
                let scaled = line(s, a, b);
                for (&p, &w) in grid.iter().zip(&want) {
                    let (d, e) = ((b.0 - a.0, b.1 - a.1), (p.0 - a.0, p.1 - a.1));
                    if (d.0 * e.1 - d.1 * e.0).abs() > 1e-3 * d.0.hypot(d.1) {
                        assert_eq!(segment_winding(scaled, Point::new(p.0 * s, p.1 * s)), w, "{a:?} to {b:?} at {s}, {p:?}");
                    }
                }
            }
        }
        // A line wider than a double: -3 to 3 across and -2 to 2 up, times 2^1022, is 2^1024 up and more across.
        let s = power_of_two(1022);
        let wide = line(s, (-3.0, -2.0), (3.0, 2.0));
        let unit = line(1.0, (-3.0, -2.0), (3.0, 2.0));
        let mut seen = [0usize; 3];
        for i in -6..=6 {
            for j in -6..=6 {
                let p = (f64::from(i) * 0.5, f64::from(j) * 0.5);
                let w = segment_winding(unit, Point::from(p));
                assert_eq!(segment_winding(wide, Point::new(p.0 * s, p.1 * s)), w, "{p:?}");
                seen[(w + 1) as usize] += 1;
            }
        }
        // The line rises, so a point to its right and under it winds -1, and the rest 0.
        assert!(seen[0] > 0 && seen[1] > 0 && seen[2] == 0, "{seen:?}");
        // An infinite coordinate is no line, and no loop.
        let endless = PathSeg::Line(kurbo::Line::new((f64::NEG_INFINITY, 0.0), (f64::INFINITY, 5.0)));
        assert_eq!(segment_winding(endless, Point::new(0.0, 1.0)), 0);
    }

    /// A segment that is a point, or a line, or a cubic that is no more than one, adds no crossing it should not.
    #[test]
    fn degenerate_segments_wind_nothing_or_what_a_line_does() {
        let at = Point::new(5.0, 5.0);
        let dot = CubicBez::new((3.0, 5.0), (3.0, 5.0), (3.0, 5.0), (3.0, 5.0));
        assert_eq!(segment_winding(PathSeg::Cubic(dot), at), 0);
        assert_eq!(segment_winding(PathSeg::Line(kurbo::Line::new((3.0, 5.0), (3.0, 5.0))), at), 0);
        // A cubic straight up on the line x = 3: the ray from (5, 5) crosses it once, as the line does.
        let straight = CubicBez::new((3.0, 0.0), (3.0, 10.0 / 3.0), (3.0, 20.0 / 3.0), (3.0, 10.0));
        assert_eq!(segment_winding(PathSeg::Cubic(straight), at), -1);
        assert_eq!(segment_winding(PathSeg::Line(kurbo::Line::new((3.0, 0.0), (3.0, 10.0))), at), -1);
        assert_eq!(segment_winding(PathSeg::Cubic(straight), Point::new(2.0, 5.0)), 0);
        // A quadratic is the cubic that raises it.
        let q = kurbo::QuadBez::new((3.0, 0.0), (9.0, 5.0), (3.0, 10.0));
        assert_eq!(segment_winding(PathSeg::Quad(q), Point::new(7.0, 5.0)), -1, "right of the bulge, which is 6 at the middle");
        assert_eq!(segment_winding(PathSeg::Quad(q), Point::new(5.0, 5.0)), 0, "left of it");
        assert_eq!(segment_winding(PathSeg::Quad(q), Point::new(7.0, 20.0)), 0);
    }

    /// A segment with a coordinate that is not a number winds nothing: `f64::min` and `f64::max` return the other operand for a NaN, so the
    /// box of the control points left it out, and a cubic whose finite ends straddle the height of the point (and whose box the point is
    /// right of) counted a crossing it had no way to find. Asserted on the segments alone, which a closing segment cannot cancel, and on an
    /// open path made of them.
    #[test]
    fn a_segment_with_a_coordinate_that_is_not_a_number_winds_nothing() {
        let nan = f64::NAN;
        let at = Point::new(5.0, 5.0);
        // Ends (0, 0) and (0, 10) straddle the height 5, and the finite control points have x of 0 to 2: the point is right of the box.
        let handle_x = CubicBez::new((0.0, 0.0), (nan, 3.0), (2.0, 7.0), (0.0, 10.0));
        let handle_y = CubicBez::new((0.0, 0.0), (1.0, nan), (2.0, 7.0), (0.0, 10.0));
        let both = CubicBez::new((0.0, 0.0), (nan, nan), (nan, nan), (0.0, 10.0));
        let finite = CubicBez::new((0.0, 0.0), (1.0, 3.0), (2.0, 7.0), (0.0, 10.0));
        assert_eq!(segment_winding(PathSeg::Cubic(finite), at), -1, "the same cubic with finite handles crosses the ray");
        for (name, c) in [("a handle's x", handle_x), ("a handle's y", handle_y), ("both handles", both)] {
            assert_eq!(segment_winding(PathSeg::Cubic(c), at), 0, "{name}");
            let mut open = BezPath::new();
            open.extend([kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)]);
            assert_eq!(winding(&open, at), 0, "{name}, an open path of it");
        }
        let quad = kurbo::QuadBez::new((0.0, 0.0), (nan, 5.0), (0.0, 10.0));
        assert_eq!(segment_winding(PathSeg::Quad(quad), at), 0, "a quadratic");
        // A line from (0, 0) to (NaN, 10): its x ends are 0 and nothing as far as the box goes.
        assert_eq!(segment_winding(PathSeg::Line(kurbo::Line::new((0.0, 0.0), (nan, 10.0))), at), 0, "a line with an x that is not a number");
        assert_eq!(segment_winding(PathSeg::Line(kurbo::Line::new((0.0, 0.0), (0.0, nan))), at), 0, "a line with a y that is not a number");
        assert_eq!(segment_winding(PathSeg::Line(kurbo::Line::new((0.0, 0.0), (0.0, 10.0))), at), -1, "and the finite line crosses");
        // The one public way to it: a `Region` a caller made, whose `path` is its own (every other entry point refuses a coordinate that is not
        // a number before it sweeps). An open path, so that no closing segment cancels the crossing.
        let mut open = BezPath::new();
        open.extend([kurbo::PathEl::MoveTo(handle_x.p0), kurbo::PathEl::CurveTo(handle_x.p1, handle_x.p2, handle_x.p3)]);
        assert!(!crate::Region { path: open, sources: vec![] }.contains(at), "Region::contains");
        // A point that is not a number has nothing wound round it.
        assert_eq!(segment_winding(PathSeg::Cubic(finite), Point::new(nan, 5.0)), 0);
        assert_eq!(segment_winding(PathSeg::Cubic(finite), Point::new(5.0, nan)), 0);
    }

    /// Where a piece's height has a flat spot (`y = t^3`, whose slope is nothing at the end) Newton's method is not
    /// fit for the crossing, and what follows it is: the crossing is where the cube root puts it for heights from 1e-30
    /// to a hair under 1, and no more steps are made than the two bounds allow.
    #[test]
    fn a_crossing_near_a_flat_spot_is_found_by_the_bisection_that_follows_newton() {
        let c = CubicBez::new((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 1.0));
        let piece = y_pieces(&c).iter().next().expect("a piece");
        let mut most = 0;
        for y in [1e-30, 1e-20, 1e-12, 1e-6, 0.001, 0.1, 0.5, 0.9, 0.999999, 1.0 - 1e-12] {
            work::reset();
            let x = crossing_x(&c, &piece, y);
            most = most.max(work::take(Work::WindingSteps));
            let exact = 3.0 * y.cbrt();
            assert!((x - exact).abs() <= 1e-12 * exact.max(1e-3) + 1e-15, "y = {y}: {x} against {exact}");
        }
        assert!(most > NEWTON_STEPS, "some of them went on to the bisection: {most}");
        assert!(most <= NEWTON_STEPS + MAX_BISECTIONS, "{most} steps");
    }

    /// The work of a winding is bounded by its segments: at most [`NEWTON_STEPS`] and [`MAX_BISECTIONS`] steps for each of
    /// the (at most three) pieces of a cubic, and none for a line or for a cubic whose box the point is outside.
    #[test]
    fn the_steps_of_a_winding_are_bounded_by_its_segments() {
        let mut rng = Rng(0x5eed_0b0e);
        let per_piece = NEWTON_STEPS + MAX_BISECTIONS;
        for case in 0..200 {
            let path = random_cubic(&mut rng, case % 2 == 0, false);
            let segments = path.segments().count();
            for _ in 0..50 {
                let p = Point::new(rng.range(-60.0, 160.0), rng.range(-50.0, 50.0));
                work::reset();
                let _ = winding(&path, p);
                let steps = work::take(Work::WindingSteps);
                assert!(steps <= 3 * per_piece * segments, "{steps} steps for {segments} segments");
            }
        }
        // A point outside the box of the controls, or right of the box of a curve with a crossing of its height, takes none.
        let arch = arch(10.05);
        work::reset();
        for p in [(-1.0, 3.0), (50.0, 11.0), (50.0, -1.0), (101.0, 3.0)] {
            let _ = winding(&arch, Point::from(p));
        }
        assert_eq!(work::take(Work::WindingSteps), 0);
        work::reset();
        let _ = winding(&arch, Point::new(30.0, 3.0));
        let found = work::take(Work::WindingSteps);
        assert!(found > 0 && found <= per_piece, "{found}");
    }

    /// A path that is not made of numbers is no crash and no endless search: every comparison with a NaN fails, so it
    /// winds nothing, and an infinite coordinate or a huge one has the loops end at their bounds all the same.
    #[test]
    fn hostile_numbers_are_wound_without_a_crash_or_an_endless_search() {
        let nan = f64::NAN;
        let mut paths = Vec::new();
        for bad in [nan, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300, f64::MAX, f64::MIN_POSITIVE, 5e-324] {
            for slot in 0..8 {
                let mut v = [0.0, 0.0, 30.0, 10.0, 60.0, 10.0, 90.0, 0.0];
                v[slot] = bad;
                let mut p = BezPath::new();
                p.move_to((v[0], v[1]));
                p.curve_to((v[2], v[3]), (v[4], v[5]), (v[6], v[7]));
                p.close_path();
                paths.push(p);
            }
        }
        let mut sink = 0;
        for path in &paths {
            for p in [(45.0, 3.0), (nan, 3.0), (45.0, nan), (f64::INFINITY, 3.0), (45.0, f64::NEG_INFINITY), (1e300, 1e300)] {
                work::reset();
                sink += winding(path, Point::from(p));
                assert!(work::take(Work::WindingSteps) <= 3 * (NEWTON_STEPS + MAX_BISECTIONS) * 2);
            }
        }
        // A NaN point or a NaN curve winds nothing.
        assert_eq!(winding(&arch(10.05), Point::new(f64::NAN, 3.0)), 0);
        assert_eq!(winding(&arch(10.05), Point::new(30.0, f64::NAN)), 0);
        assert!(sink.abs() < 100_000);
    }

    /// The frame: a power of two for each axis that brings the greatest magnitude of the axis to between 1 and 2, and takes it back, exactly, for every
    /// exponent a double has, the subnormal ones too.
    #[test]
    fn the_frame_scales_each_axis_to_between_one_and_two_and_back_exactly() {
        for m in [1.0, 1.999, 2.0, 0.75, 3e300, 1.7e308, 1e-300, 5e-324, 1e-310, 2.2250738585072014e-308, 123.456] {
            let frame = Frame::of(&[Point::new(m, -m / 3.0), Point::new(-m / 7.0, 0.5 * m)]).expect("finite");
            let scaled = frame.control(Point::new(m, 0.5 * m));
            assert!((1.0..2.0).contains(&scaled.x), "{m:e}: x is {}", scaled.x);
            let down = frame.control(Point::new(-m / 7.0, 0.5 * m));
            assert_eq!((frame.unscale_x(scaled.x), frame.unscale_y(down.y)), (m, 0.5 * m), "{m:e}");
        }
        // Per axis: a tall thin segment is scaled by one factor on `x` and by another on `y`.
        let tall = Frame::of(&[Point::new(1e-200, 0.0), Point::new(0.0, 1e200)]).expect("finite");
        assert!(tall.x > 600 && tall.y < -600, "{tall:?}");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Frame::of(&[Point::new(1.0, 2.0), Point::new(bad, 0.0)]), None);
            assert_eq!(Frame::of(&[Point::new(1.0, 2.0), Point::new(0.0, bad)]), None);
        }
        assert_eq!(Frame::of(&[Point::ORIGIN, Point::ORIGIN]), Some(Frame::UNIT));
        assert_eq!(times_pow2(1.0, 1023), pow2(1023));
        assert_eq!(times_pow2(pow2(1023), -2045), pow2(-1022));
        assert_eq!(times_pow2(5e-324, 2000), pow2(-1074 + 2000));
    }

    /// The winding does not change when an axis is multiplied by a power of two, from 2^-1000 to 2^1000 on each axis separately (a thin segment, a
    /// tall one, one 2^1000 across and 2^-1000 high), to the bit: a line, a quadratic and a cubic, at every point of a grid, near the curve or not.
    /// At the top of the range, a sum of two of the control points does not overflow in the frame, which it does in the numbers the segment came in.
    #[test]
    fn the_winding_of_a_segment_does_not_depend_on_the_scale_of_either_axis() {
        let unit_points: Vec<Point> =
            (-4..=26).flat_map(|i| (-8..=26).map(move |j| Point::new(f64::from(i) * 0.125, f64::from(j) * 0.125))).collect();
        let line = |k: Affine| PathSeg::Line(k * kurbo::Line::new((0.5, 0.25), (2.5, 2.75)));
        let quad = |k: Affine| PathSeg::Quad(k * QuadBez::new((0.25, 0.0), (3.0, 1.5), (0.75, 3.0)));
        let cubic = |k: Affine| PathSeg::Cubic(k * CubicBez::new((0.0, 0.0), (1.0, 3.0), (2.0, -1.0), (3.0, 1.0)));
        let makers: [(&str, &dyn Fn(Affine) -> PathSeg); 3] = [("line", &line), ("quad", &quad), ("cubic", &cubic)];
        for (name, make) in makers {
            let unit: Vec<i32> = unit_points.iter().map(|&p| segment_winding(make(Affine::IDENTITY), p)).collect();
            assert!(unit.iter().any(|&w| w != 0) && unit.contains(&0), "{name}");
            for (ex, ey) in [
                (0, 0),
                (-1000, -1000),
                (1000, 1000),
                (-1000, 1000),
                (1000, -1000),
                (45, -45),
                (-300, 700),
                (700, -300),
                (1000, 0),
                (0, -1000),
                (1, 1000),
            ] {
                let (fx, fy) = (pow2(ex), pow2(ey));
                let k = Affine::scale_non_uniform(fx, fy);
                for (&p, &w) in unit_points.iter().zip(&unit) {
                    let got = segment_winding(make(k), Point::new(p.x * fx, p.y * fy));
                    assert_eq!(got, w, "{name} at 2^{ex} by 2^{ey}, {p:?}");
                }
            }
        }
    }

    /// A curve 2^45 across whose crossing lies 0.29 from its start (the point, 0.27 from it, is well under 1e-13 of the curve's size, and the first
    /// Newton step is shorter than the 1e-13 in the parameter that the end of the method was once taken at): `y = 6x - 3x^2` is at 1.5 where `x = 0.2929`
    /// and 1.7071, both right of the point, which is wound 0.
    #[test]
    fn a_crossing_close_to_the_start_of_a_curve_that_is_very_wide_is_found() {
        let s = pow2(45);
        let c = CubicBez::new((0.0, 0.0), (s / 3.0, 2.0 * s), (2.0 * s / 3.0, 4.0 * s - s * s), (s, 6.0 * s - 3.0 * s * s));
        let seg = PathSeg::Cubic(c);
        assert_eq!(segment_winding(seg, Point::new(0.27, 1.5)), 0);
        assert_eq!(segment_winding(seg, Point::new(0.31, 1.5)), -1);
        assert_eq!(segment_winding(seg, Point::new(1.6, 1.5)), -1);
        assert_eq!(segment_winding(seg, Point::new(1.8, 1.5)), -1 + 1);
    }

    /// A height under the unit in the last place of the segment's greatest is no height to it: the curve `(0, 0), (1, 0), (2, 0), (3, 1)` rises
    /// as `t^3`, its crossing at 1e-90 is where the doubles of its heights cannot tell it from the start's (3e-30 across), and a point 0.5 to its
    /// right is wound -1 (the crossing is found to the heights' rounding, the point is far beyond that).
    #[test]
    fn a_height_under_the_rounding_of_the_curves_heights_is_found_within_that_rounding() {
        let c = CubicBez::new((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 1.0));
        for y in [1e-90, 1e-60, 1e-30, 1e-20] {
            assert_eq!(segment_winding(PathSeg::Cubic(c), Point::new(0.5, y)), -1, "{y:e}");
            assert_eq!(segment_winding(PathSeg::Cubic(c), Point::new(-0.5, y)), 0, "{y:e}");
        }
    }

    /// The point is in the frame as a number too small for it is the least of its sign, not zero: a height just under a piece's end of exactly 0 is
    /// under it (`-1e-300` scaled by 2^-996 is `-0.0`, which is not under `0.0`), and a point 1e-300 to the left of an end at `x = 0` is left of it.
    #[test]
    fn a_point_too_small_for_the_frame_is_on_its_own_side_of_a_zero() {
        let c = CubicBez::new((0.0, 0.0), (1.0, 3e299), (2.0, 6e299), (3.0, 1e300));
        assert_eq!(segment_winding(PathSeg::Cubic(c), Point::new(10.0, -1e-300)), 0, "under the start");
        assert_eq!(segment_winding(PathSeg::Cubic(c), Point::new(10.0, 1e-300)), -1, "over the start");
        let level = CubicBez::new((0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (0.0, 1e300));
        assert_eq!(segment_winding(PathSeg::Cubic(level), Point::new(-1e-300, 0.5)), 0, "left of a curve along x = 0");
        assert_eq!(segment_winding(PathSeg::Cubic(level), Point::new(1e-300, 0.5)), -1, "right of it");
    }

    /// A segment with a coordinate that is not finite winds nothing, as one with a NaN does; a point at the far left or right is beyond every finite
    /// crossing, and one at an infinite height is level with none.
    #[test]
    fn infinite_coordinates_wind_nothing_and_an_infinite_point_is_beyond_every_crossing() {
        let finite = CubicBez::new((0.0, 0.0), (1.0, 3.0), (2.0, -1.0), (3.0, 1.0));
        for bad in [f64::INFINITY, f64::NEG_INFINITY] {
            for slot in 0..8 {
                let mut v = [0.0, 0.0, 1.0, 3.0, 2.0, -1.0, 3.0, 1.0];
                v[slot] = bad;
                let seg = PathSeg::Cubic(CubicBez::new((v[0], v[1]), (v[2], v[3]), (v[4], v[5]), (v[6], v[7])));
                assert_eq!(segment_winding(seg, Point::new(1.5, 0.5)), 0, "slot {slot}");
                // A line has two ends and a quadratic three: the slots they have.
                let line = PathSeg::Line(kurbo::Line::new((v[0], v[1]), (v[2], v[3])));
                assert!(slot >= 4 || segment_winding(line, Point::new(1.5, 0.5)) == 0, "line, slot {slot}");
                let quad = PathSeg::Quad(QuadBez::new((v[0], v[1]), (v[2], v[3]), (v[4], v[5])));
                assert!(slot >= 6 || segment_winding(quad, Point::new(1.5, 0.5)) == 0, "quadratic, slot {slot}");
            }
        }
        let inside = Point::new(1.5, 0.5);
        let at = |x: f64, y: f64| segment_winding(PathSeg::Cubic(finite), Point::new(x, y));
        assert_eq!(at(f64::INFINITY, inside.y), at(1e300, inside.y));
        assert_eq!(at(f64::NEG_INFINITY, inside.y), 0);
        assert_eq!((at(1.5, f64::INFINITY), at(1.5, f64::NEG_INFINITY)), (0, 0));
    }

    /// The crossing is found at a parameter where the height is within a few units in the last place of the curve's greatest height of the height asked
    /// for: the curves are those that are 1 across and those 3e13 and 1e-5 across, arches, level handles, flat ends (`y = t^3`) and handles that go nearly
    /// level, and the heights are spread over each piece and within 1e-40 of its ends. Measured over 80,000 of them, the greatest was 5 units.
    #[test]
    fn the_crossing_is_found_within_a_few_units_of_the_rounding_of_the_heights() {
        let mut rng = Rng(0x1234_5678_9abc);
        let mut worst = 0.0_f64;
        for case in 0..20_000_u32 {
            let kind = case % 6;
            let sx = [1.0, 1.0, 1.0, 3e13, 1e-5, 1.0][kind as usize];
            let x = |r: &mut Rng| r.range(-1.0, 1.0) * sx;
            let (y0, y1, y2, y3) = match kind {
                1 => {
                    let h = rng.range(-1.0, 1.0);
                    (0.0, h, h, 0.0)
                }
                2 => (0.0, 0.0, 0.0, 1.0),
                4 => {
                    let h = rng.range(0.1, 1.0);
                    (0.0, h, h * 0.999_999_9, 0.0)
                }
                _ => (rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)),
            };
            let source = CubicBez::new((x(&mut rng), y0), (x(&mut rng), y1), (x(&mut rng), y2), (x(&mut rng), y3));
            let Some(frame) = Frame::of(&[source.p0, source.p1, source.p2, source.p3]) else { continue };
            let c = frame.cubic(&source);
            let ys = coordinates(&c).1;
            let greatest = ys.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
            for piece in framed_pieces(&c).iter().filter(|p| p.y_lo != p.y_hi) {
                let (lo, hi) = (piece.y_lo.min(piece.y_hi), piece.y_lo.max(piece.y_hi));
                for f in [0.5, 0.3, 0.01, 1e-4, 1e-8, 1e-12, 1e-16, 1e-24, 1e-40, 1.0 - 1e-8, 1.0 - 1e-12] {
                    let y = lo + (hi - lo) * f;
                    if !(y > lo && y < hi) {
                        continue;
                    }
                    let t = framed_crossing_t(&c, &piece, y);
                    assert!(t >= piece.lo && t <= piece.hi, "{t} outside {piece:?}");
                    worst = worst.max((bernstein(ys, t) - y).abs() / (greatest * f64::EPSILON));
                }
            }
        }
        assert!(worst <= 8.0, "a crossing's height is {worst} units of rounding from the height asked for");
    }
}
