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

use kurbo::{BezPath, CubicBez, PathSeg, Point};

use crate::work::{self, Work};

/// Most halvings of the parameter range of one piece: 2^-64 of it, past what `f64` tells apart over a unit.
pub(crate) const MAX_BISECTIONS: usize = 64;

/// Most steps of Newton's method for the crossing of one piece, before the range it has narrowed is bisected.
pub(crate) const NEWTON_STEPS: usize = 12;

/// A Newton step this short (in the parameter, which runs from 0 to 1) is the last: the method converges
/// quadratically, so the error left is the square of it, under what rounding can tell.
const NEWTON_DONE: f64 = 1e-13;

/// The winding number of the closed subpaths of `path` round `p` (the segments are those of
/// [`BezPath::segments`], a `ClosePath` giving the line back to the start): each segment's crossing with a ray
/// from `p` to the left counts -1 where the segment rises and +1 where it falls. It is `kurbo::Shape::winding`,
/// made right for cubics (see the module).
pub(crate) fn winding(path: &BezPath, p: Point) -> i32 {
    path.segments().fold(0_i32, |w, seg| w.saturating_add(segment_winding(seg, p)))
}

/// The winding of one segment round `p`: the crossing of the ray to the left of `p` with it, -1, 0 or +1 for each
/// monotone piece of it.
///
/// A segment with a coordinate that is not a number winds nothing, and neither does a point that is not one: the box of the control points
/// drops a NaN (`f64::min` and `f64::max` return the other operand), so a cubic whose finite ends straddle the point's height would count a
/// crossing it has no way to find. Everything the crate sweeps has been refused for a coordinate that is not a number first
/// ([`crate::PathOpsError::NonFinite`]); [`crate::Region::contains`] takes a path a caller made.
pub(crate) fn segment_winding(seg: PathSeg, p: Point) -> i32 {
    if has_nan(seg) || p.x.is_nan() || p.y.is_nan() {
        return 0;
    }
    match seg {
        PathSeg::Line(l) => line_winding(l.p0, l.p1, p),
        PathSeg::Quad(q) => cubic_winding(&q.raise(), p),
        PathSeg::Cubic(c) => cubic_winding(&c, p),
    }
}

/// Does a control point of `seg` have a coordinate that is not a number?
fn has_nan(seg: PathSeg) -> bool {
    let nan = |p: Point| p.x.is_nan() || p.y.is_nan();
    match seg {
        PathSeg::Line(l) => nan(l.p0) || nan(l.p1),
        PathSeg::Quad(q) => nan(q.p0) || nan(q.p1) || nan(q.p2),
        PathSeg::Cubic(c) => nan(c.p0) || nan(c.p1) || nan(c.p2) || nan(c.p3),
    }
}

/// A line's crossing with the ray, as `kurbo` tests it: the end with the smaller `y` is the line's and the other is
/// not, and a point on the line (as the arithmetic finds it) is to the right of it.
fn line_winding(start: Point, end: Point, p: Point) -> i32 {
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
    // The line `a x + b y = c`: which side of it the point is on.
    let a = end.y - start.y;
    let b = start.x - end.x;
    let c = a * start.x + b * start.y;
    if (a * p.x + b * p.y - c) * f64::from(sign) <= 0.0 { sign } else { 0 }
}

/// A cubic's crossings with the ray: its pieces whose height rises or falls the whole way, each of which crosses the
/// height of `p` once or not at all, and the crossing's `x` against the point's.
fn cubic_winding(c: &CubicBez, p: Point) -> i32 {
    let (xs, ys) = coordinates(c);
    let ((x_min, x_max), (y_min, y_max)) = (bounds(xs), bounds(ys));
    // The curve is in the box of its control points. (A NaN fails each comparison and is no crossing.)
    if !(p.y >= y_min && p.y <= y_max && p.x >= x_min) {
        return 0;
    }
    let mut winding = 0;
    for piece in y_pieces(c).iter() {
        if (piece.y_lo <= p.y) == (piece.y_hi <= p.y) {
            continue;
        }
        let sign = if piece.y_hi > piece.y_lo { -1 } else { 1 };
        // A point right of the whole box is right of the crossing, and the root need not be found.
        if p.x >= x_max || p.x >= crossing_x(c, &piece, p.y) {
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
/// is. The ends of the curve are its own ends exactly.
pub(crate) fn y_pieces(c: &CubicBez) -> Pieces {
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

/// The `x` at which the piece of `c` crosses the height `y`, which it does, found on the piece's parameter range by
/// Newton's method from where the straight line between the piece's ends crosses, held inside the range the heights
/// seen so far leave (a step that leaves it, or whose slope is nothing, ends the method), and then by bisection of
/// that range: the side of the height the piece's lower end is on is kept at one end and the other at the other, for
/// [`MAX_BISECTIONS`] halvings at most (a range halved to where it can no longer be is done). Each step is one
/// evaluation of the height, at most [`NEWTON_STEPS`] and [`MAX_BISECTIONS`] of them in all, and never the cubic
/// formula, so the rounding of coefficients that is the fault of that formula does not come into it.
pub(crate) fn crossing_x(c: &CubicBez, piece: &Piece, y: f64) -> f64 {
    let (xs, ys) = coordinates(c);
    let [y0, y1, y2, y3] = ys;
    let (a, b, d) = (y1 - y0, y2 - y1, y3 - y2);
    let at_or_below = piece.y_lo <= y;
    let (mut lo, mut hi) = (piece.lo, piece.hi);
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
            return bernstein(xs, t);
        }
        // The slope of the height, `y'(t) = 3 (a (1 - t)^2 + 2 b (1 - t) t + d t^2)`.
        let mt = 1.0 - t;
        let next = t - (h - y) / (3.0 * (a * (mt * mt) + 2.0 * b * mt * t + d * (t * t)));
        if !(next > lo && next < hi) {
            break;
        }
        if (next - t).abs() <= NEWTON_DONE {
            return bernstein(xs, next);
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
    bernstein(xs, 0.5 * (lo + hi))
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

/// The least and the greatest of four numbers.
fn bounds(v: [f64; 4]) -> (f64, f64) {
    (v.into_iter().fold(f64::INFINITY, f64::min), v.into_iter().fold(f64::NEG_INFINITY, f64::max))
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
    use kurbo::ParamCurve;
    use kurbo::Shape;

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
                assert_eq!(winding(&circle, p), circle.winding(p), "{p:?}");
            }
        }
        assert_eq!(on_the_circle, 4, "the four vertices are among the points");
        // The heights of the turns: inside and outside, at the top and bottom of the circle exactly.
        for p in [(4.0, 0.0), (4.0, 8.0), (3.0, 0.0), (3.0, 8.0), (0.0, 4.0), (8.0, 4.0), (4.0, 4.0)] {
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
}
