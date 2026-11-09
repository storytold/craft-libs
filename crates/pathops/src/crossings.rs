//! Bounds on the places where two segments cross, none of them ever below the number there are.
//!
//! The check of a sweep ([`crate::MAX_REAL_CROSSINGS`]) is a cap on the crossings the sweep will make, and a count that
//! can fall below the number there are lets an input past it. Two lines cross once or not at all and are tested
//! (in `budget.rs`). A line and a cubic cross at most three times, and two cubics at most nine, and these are bounds
//! on what a pair can have, not on what most do; they are lowered here by arguments that cannot be wrong:
//!
//! - A cubic crosses a line no more often than the signed distances of its control points from the line change sign (the
//!   control polygon crosses it at least as often as the curve does).
//! - Two arcs that cross at two points `a` and `b` each have a tangent along the chord `ab` (the mean value theorem: the
//!   component of the tangent across the chord integrates to nothing along an arc from `a` to `b`). So two arcs whose
//!   tangents never point the same way (as lines, not as directions) cross once at most. The tangents of a piece of a
//!   cubic are convex combinations of the three differences of its control points, so they lie in the cone of those, and
//!   two pieces whose cones do not meet (in direction, or opposite) cross at most once.
//! - Two arcs that share an end `s` and lie in cones that meet only at `s` (the cones of the rays from `s` to their control
//!   points) meet only there: they cross nowhere else. This is what takes the joint of two neighbours in a path, or the
//!   point where the ends of many segments meet, for no crossing.
//! - A cubic whose tangents lie in one cone narrower than a half plane has no loop (its displacement along any arc is
//!   a sum of vectors in the cone, which is not nothing).
//!
//! Where none of these separates a pair, it is counted at the most it can have, which is nine for two cubics. Every
//! angle and box is widened by what the rounding of the points allows (and the boxes of two pieces by the slack of the sweep), so that a doubt is a
//! pair counted higher.

use std::cell::OnceCell;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use kurbo::{CubicBez, Line, ParamCurve, Point, Rect, Vec2};

/// Pieces a curve is cut into, equal in its parameter, where the cones of the whole curve meet.
const PIECES: usize = 8;

/// What two angles may be nearer than and still meet, on top of what the rounding of the points takes.
const ANGLE_SLACK: f64 = 1e-9;

/// A range of directions: the angle of its middle and its half width, in radians. A half width of `PI` is every direction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cone {
    centre: f64,
    half: f64,
}

/// `x` brought to within half a `period` of 0.
fn wrap(x: f64, period: f64) -> f64 {
    x - period * (x / period).round()
}

impl Cone {
    /// Every direction.
    const ALL: Self = Self { centre: 0.0, half: PI };

    /// The cone that holds the directions of `vectors`, each of which is known to within `err` in length: every direction if
    /// one of them could be nothing, or if they do not lie in a cone narrower than a half plane. The cone is widened by the
    /// angle each one's error can turn it through.
    fn of(vectors: &[Vec2], err: f64) -> Self {
        let mut sum = Vec2::ZERO;
        for v in vectors {
            let n = v.hypot();
            if !(n > 2.0 * err && n.is_finite()) {
                return Self::ALL;
            }
            sum += *v / n;
        }
        let norm = sum.hypot();
        if norm.is_nan() || norm <= 1e-9 {
            return Self::ALL;
        }
        let middle = sum.atan2();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for v in vectors {
            let off = wrap(v.atan2() - middle, TAU);
            let turn = (err / v.hypot()).min(1.0).asin();
            lo = lo.min(off - turn);
            hi = hi.max(off + turn);
        }
        let width = hi - lo;
        if width.is_nan() || width >= PI - 1e-6 {
            return Self::ALL;
        }
        Self { centre: middle + 0.5 * (lo + hi), half: 0.5 * (hi - lo) }
    }

    /// Does this cone hold every direction (as far as is known)?
    fn is_all(self) -> bool {
        self.half >= FRAC_PI_2
    }

    /// Are there no two directions, one in each, along one line (the same way or opposite)?
    fn lines_apart(self, other: Self) -> bool {
        wrap(self.centre - other.centre, PI).abs() > self.half + other.half + ANGLE_SLACK
    }

    /// Are there no direction in both?
    fn apart(self, other: Self) -> bool {
        wrap(self.centre - other.centre, TAU).abs() > self.half + other.half + ANGLE_SLACK
    }
}

/// The least and the greatest coordinate of four points, as a box.
fn hull(p: &[Point; 4]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for q in p {
        x0 = x0.min(q.x);
        y0 = y0.min(q.y);
        x1 = x1.max(q.x);
        y1 = y1.max(q.y);
    }
    Rect::new(x0, y0, x1, y1)
}

/// The control points of `c`.
fn points(c: &CubicBez) -> [Point; 4] {
    [c.p0, c.p1, c.p2, c.p3]
}

/// The largest coordinate of `p`, which sets how far its differences can be off.
fn largest(p: &[Point; 4]) -> f64 {
    p.iter().fold(0.0_f64, |m, q| m.max(q.x.abs()).max(q.y.abs()))
}

/// The cone of the tangents of the piece of a cubic with control points `p`.
fn tangents(p: &[Point; 4]) -> Cone {
    Cone::of(&[p[1] - p[0], p[2] - p[1], p[3] - p[2]], 8.0 * f64::EPSILON * largest(p))
}

/// The cone of the rays from `apex`, which is one of the control points `p` (or is not, for a line), to the others, and none
/// where all of them are the apex itself. A point at the apex adds nothing (the hull of the points is the apex and the sums of
/// the rays, whatever their weights). The rays of every other point count, however short: a point a hair from the apex is a ray
/// that points where it points, and the cone is that of the rays widened by the rounding of the points alone. (A ray dropped
/// for being short would leave the hull a hair outside the cone, which two cones apart do not exclude where they come close.)
fn rays(p: &[Point; 4], apex: Point) -> Option<Cone> {
    let mut vectors = [Vec2::ZERO; 4];
    let mut n = 0;
    for &q in p {
        let v = q - apex;
        if let Some(slot) = vectors.get_mut(n)
            && v != Vec2::ZERO
        {
            *slot = v;
            n += 1;
        }
    }
    let err = 8.0 * f64::EPSILON * largest(p).max(apex.x.abs()).max(apex.y.abs());
    vectors.get(..n).filter(|v| !v.is_empty()).map(|v| Cone::of(v, err))
}

/// One piece of a curve: its control points, their box and the cone of its tangents.
struct Piece {
    points: [Point; 4],
    hull: Rect,
    tangents: Cone,
}

/// A cubic, as the count of its crossings reads it: the curve, the cone of its tangents, and its pieces (made when a
/// pair of curves needs them).
pub(crate) struct Curve {
    c: CubicBez,
    tangents: Cone,
    pieces: OnceCell<Box<[Piece]>>,
}

impl Curve {
    pub(crate) fn new(c: CubicBez) -> Self {
        let tangents = tangents(&points(&c));
        Self { c, tangents, pieces: OnceCell::new() }
    }

    /// May the curve cross itself? Not if its tangents lie in a cone narrower than a half plane.
    pub(crate) fn may_loop(&self) -> bool {
        self.tangents.is_all()
    }

    fn pieces(&self) -> &[Piece] {
        self.pieces.get_or_init(|| {
            (0..PIECES)
                .map(|k| {
                    let c = self.c.subsegment(k as f64 / PIECES as f64..(k + 1) as f64 / PIECES as f64);
                    let p = points(&c);
                    Piece { points: p, hull: hull(&p), tangents: tangents(&p) }
                })
                .collect()
        })
    }
}

/// Which ends two segments share exactly (the same two numbers), each as whether it is at the end of the first (and not the
/// start) and at the end of the second. Only an exact end is one apex: ends a hair apart make two, and two cones apart from two
/// apexes do not keep the curves from crossing where they meet (straight cubics from points 1e-6 apart whose directions differ
/// by 1e-8 cross 100 away). The segments of a path share exact ends.
fn shared_ends(a: [Point; 2], b: [Point; 2]) -> impl Iterator<Item = (bool, bool)> {
    (0..4).filter_map(move |k| {
        let (a_end, b_end) = (k % 2 == 1, k / 2 == 1);
        (a[usize::from(a_end)] == b[usize::from(b_end)]).then_some((a_end, b_end))
    })
}

fn overlap(a: &Rect, b: &Rect, slack: f64) -> bool {
    a.x0 <= b.x1 + slack && b.x0 <= a.x1 + slack && a.y0 <= b.y1 + slack && b.y0 <= a.y1 + slack
}

/// The most places two cubics can cross at, away from an end they share exactly (nine at the most, which is what two cubics that
/// are not the same curve can have), taken from the cones of their tangents: the pair is counted at once if the cones of the
/// whole curves do not meet, and by pieces if they do (each pair of pieces whose boxes overlap counts one if their cones do not
/// meet, and nine if they do), none where the pair meets only at an end they share exactly and leave in cones that do not meet.
/// Ends a hair apart are two points and not that: the pair is counted as any other.
pub(crate) fn cubics_cross_at_most(a: &Curve, b: &Curve, slack: f64) -> u64 {
    let mut ends = [(false, false); 4];
    let mut n = 0;
    for e in shared_ends([a.c.p0, a.c.p3], [b.c.p0, b.c.p3]) {
        if let Some(slot) = ends.get_mut(n) {
            *slot = e;
            n += 1;
        }
    }
    let shared = ends.get(..n).unwrap_or(&[]);
    let end = |c: &CubicBez, at_end: bool| if at_end { c.p3 } else { c.p0 };
    if let &[(a_end, b_end)] = shared {
        let (apex_a, apex_b) = (end(&a.c, a_end), end(&b.c, b_end));
        if let (Some(ca), Some(cb)) = (rays(&points(&a.c), apex_a), rays(&points(&b.c), apex_b))
            && ca.apart(cb)
        {
            return 0;
        }
    }
    if a.tangents.lines_apart(b.tangents) {
        return 1;
    }
    let (pa, pb) = (a.pieces(), b.pieces());
    let mut total = 0u64;
    for (i, p) in pa.iter().enumerate() {
        for (j, q) in pb.iter().enumerate() {
            if !overlap(&p.hull, &q.hull, slack) {
                continue;
            }
            // Where they meet only at an end that both pieces have, nowhere else.
            let at_shared_end = shared.iter().any(|&(a_end, b_end)| {
                let (first_a, first_b) = (if a_end { PIECES - 1 } else { 0 }, if b_end { PIECES - 1 } else { 0 });
                i == first_a && j == first_b && {
                    let (apex_a, apex_b) = (end(&a.c, a_end), end(&b.c, b_end));
                    matches!((rays(&p.points, apex_a), rays(&q.points, apex_b)), (Some(ca), Some(cb)) if ca.apart(cb))
                }
            });
            if at_shared_end {
                continue;
            }
            total += leaf_charge(p, q);
        }
    }
    total.min(9)
}

/// What a pair of pieces whose boxes overlap counts: one if the directions of their tangents (as lines) are apart, since two arcs
/// that cross twice both have a tangent along the chord between the crossings, and nine otherwise.
fn leaf_charge(p: &Piece, q: &Piece) -> u64 {
    if p.tangents.lines_apart(q.tangents) { 1 } else { 9 }
}

/// The most sign changes the numbers `d` can have when each that is within `eps` of nothing is taken as whichever sign
/// makes more.
fn most_sign_changes(d: [f64; 4], eps: f64) -> u64 {
    // For each number, the most changes so far ending in a positive sign and in a negative one (none where it cannot be).
    let mut best: [Option<u64>; 2] = [Some(0), Some(0)];
    for (k, &v) in d.iter().enumerate() {
        let allowed = [v > -eps, v < eps];
        let mut next: [Option<u64>; 2] = [None, None];
        for (sign, &ok) in allowed.iter().enumerate() {
            if !ok {
                continue;
            }
            for (before, &changes) in best.iter().enumerate() {
                if let Some(n) = changes {
                    let n = n + u64::from(k > 0 && before != sign);
                    next[sign] = Some(next[sign].map_or(n, |m| m.max(n)));
                }
            }
        }
        best = next;
    }
    best.iter().flatten().copied().max().unwrap_or(0)
}

/// The most places a line and a cubic can cross at (three at the most): none where they meet only at an end they share
/// and the line leaves it outside the cone of the curve's control points, and otherwise the sign changes of the signed
/// distances of the control points from the line.
pub(crate) fn line_cubic_cross_at_most(line: &Line, curve: &Curve) -> u64 {
    let p = points(&curve.c);
    let toward = line.p1 - line.p0;
    for (line_end, curve_end) in shared_ends([line.p0, line.p1], [curve.c.p0, curve.c.p3]) {
        let (apex, away) = if line_end { (line.p1, -toward) } else { (line.p0, toward) };
        let apex_of_curve = if curve_end { curve.c.p3 } else { curve.c.p0 };
        let err = 8.0 * f64::EPSILON * largest(&p).max(apex.x.abs()).max(apex.y.abs());
        if let (Some(rays_of_curve), true) = (rays(&p, apex_of_curve), away.hypot() > 2.0 * err)
            && rays_of_curve.apart(Cone::of(&[away], err))
        {
            return 0;
        }
    }
    let length = toward.hypot();
    let d = p.map(|q| toward.cross(q - line.p0));
    let reach = p.iter().fold(0.0_f64, |m, q| m.max((*q - line.p0).hypot()));
    let eps = 32.0 * f64::EPSILON * length * (reach + length).max(largest(&p));
    most_sign_changes(d, eps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    fn flat(c: &CubicBez, n: u32) -> Vec<Point> {
        (0..=n).map(|k| c.eval(f64::from(k) / f64::from(n))).collect()
    }

    fn random_cubic(rng: &mut Rng, spread: f64) -> CubicBez {
        let mut p = || Point::new(rng.range(0.0, spread), rng.range(0.0, spread));
        CubicBez::new(p(), p(), p(), p())
    }

    /// How many times the polylines `a` and `b` cross, which is how often the curves they are fine flattenings of do.
    fn polyline_crossings(a: &[Point], b: &[Point]) -> u64 {
        let side = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
        let mut n = 0;
        for wa in a.windows(2) {
            let ra = Rect::from_points(wa[0], wa[1]);
            for wb in b.windows(2) {
                if !overlap(&ra, &Rect::from_points(wb[0], wb[1]), 0.0) {
                    continue;
                }
                let (d1, d2) = (side(wa[0], wa[1], wb[0]), side(wa[0], wa[1], wb[1]));
                let (d3, d4) = (side(wb[0], wb[1], wa[0]), side(wb[0], wb[1], wa[1]));
                n += u64::from(d1 * d2 < 0.0 && d3 * d4 < 0.0);
            }
        }
        n
    }

    #[test]
    fn a_cone_holds_the_directions_it_is_made_of_and_no_others() {
        let c = Cone::of(&[Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.2), Vec2::new(1.0, -0.1)], 0.0);
        assert!(c.half > 0.0 && c.half < 0.2 && c.centre.abs() < 0.2);
        // Opposite vectors, nothing, and a ring of them are every direction.
        assert!(Cone::of(&[Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)], 0.0).is_all());
        assert!(Cone::of(&[Vec2::new(1.0, 0.0), Vec2::ZERO], 0.0).is_all());
        assert!(Cone::of(&[Vec2::new(1.0, 0.0), Vec2::new(-0.5, 0.9), Vec2::new(-0.5, -0.9)], 0.0).is_all());
        // A vector no longer than its error could point anywhere.
        assert!(Cone::of(&[Vec2::new(1.0, 0.0), Vec2::new(0.0, 1e-9)], 1e-9).is_all());
        // As directions of lines, a cone and the one opposite are the same, and as directions they are apart.
        let right = Cone::of(&[Vec2::new(1.0, 0.1)], 0.0);
        let left = Cone::of(&[Vec2::new(-1.0, -0.1)], 0.0);
        assert!(!right.lines_apart(left) && right.apart(left));
        assert!(right.lines_apart(Cone::of(&[Vec2::new(0.0, 1.0)], 0.0)));
    }

    /// Wiggles between the samples: a straight cubic along the x axis and one whose height is `K (t - .30)(t - .33)(t - .36)`
    /// cross three times, between 0.30 and 0.36, inside one of the eight pieces of each, where the chords of that piece
    /// cross once (the heights at the ends of it, 0.25 and 0.375, are of opposite signs): the count by chords was one. The
    /// bound is not below three.
    #[test]
    fn a_wiggle_inside_a_piece_is_not_counted_by_its_chord() {
        // The height in the Bernstein form: the polynomial `K (t^3 - 0.99 t^2 + 0.3258 t - 0.03564)`.
        let k = 400.0;
        let (a0, a1, a2, a3) = (-0.03564 * k, 0.3258 * k, -0.99 * k, k);
        let y = [a0, a0 + a1 / 3.0, a0 + 2.0 * a1 / 3.0 + a2 / 3.0, a0 + a1 + a2 + a3];
        let wiggle = CubicBez::new((0.0, y[0]), (100.0 / 3.0, y[1]), (200.0 / 3.0, y[2]), (100.0, y[3]));
        let straight = CubicBez::new((0.0, 0.0), (100.0 / 3.0, 0.0), (200.0 / 3.0, 0.0), (100.0, 0.0));
        assert_eq!(polyline_crossings(&flat(&wiggle, 4000), &flat(&straight, 4000)), 3, "the curves cross three times");
        // The chords of the pieces cross once in the piece that holds the three.
        let chord_crossings: u64 = (0..PIECES)
            .map(|i| {
                let (s0, s1) = (i as f64 / 8.0, (i + 1) as f64 / 8.0);
                let side = |t: f64| wiggle.eval(t).y;
                u64::from(side(s0) * side(s1) < 0.0)
            })
            .sum();
        assert_eq!(chord_crossings, 1, "by the chords of eight pieces there is one");
        let (wiggle, straight) = (Curve::new(wiggle), Curve::new(straight));
        let bound = cubics_cross_at_most(&wiggle, &straight, 1e-6);
        assert!((3..=9).contains(&bound), "{bound}");
        // The piece that holds the three has tangents along the straight one's, and is counted at nine, not one: a pair counted one
        // for each pair of pieces whose boxes overlap would be as many as there are of those, which is no bound.
        assert_eq!(leaf_charge(&wiggle.pieces()[2], &straight.pieces()[2]), 9);
        // Pieces whose tangents run apart count one.
        let (rising, falling) = (
            Curve::new(CubicBez::new((0.0, 0.0), (3.0, 4.0), (7.0, 4.0), (10.0, 0.0))),
            Curve::new(CubicBez::new((0.0, 2.0), (3.0, -2.0), (7.0, -2.0), (10.0, 2.0))),
        );
        assert_eq!(leaf_charge(&rising.pieces()[1], &falling.pieces()[1]), 1);
    }

    /// The bound is not below the number of crossings, whatever the cubics: pairs of random cubics (the handles spread
    /// widely, so that they wiggle and loop, and others that are nearly straight or nearly the same curve), against the
    /// crossings of fine flattenings.
    #[test]
    fn the_bound_is_never_below_the_crossings_of_random_cubics() {
        let mut rng = Rng(0x1357_9bdf_2468_ace1);
        let (mut tested, mut tight, mut wiggly) = (0u64, 0u64, 0u64);
        for case in 0..1200 {
            let a = random_cubic(&mut rng, 100.0);
            let b = match case % 3 {
                0 => random_cubic(&mut rng, 100.0),
                // Nearly the same curve: its control points moved a little.
                1 => CubicBez::new(
                    a.p0 + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)),
                    a.p1 + Vec2::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0)),
                    a.p2 + Vec2::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0)),
                    a.p3 + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)),
                ),
                // Heights that wiggle: the same x, heights drawn by turns up and down.
                _ => CubicBez::new(
                    (a.p0.x, rng.range(0.0, 100.0)),
                    (a.p1.x, rng.range(0.0, 100.0)),
                    (a.p2.x, rng.range(0.0, 100.0)),
                    (a.p3.x, rng.range(0.0, 100.0)),
                ),
            };
            let truth = polyline_crossings(&flat(&a, 400), &flat(&b, 400));
            let bound = cubics_cross_at_most(&Curve::new(a), &Curve::new(b), 1e-6);
            // A crossing at a shared end is not one; the random curves share none.
            assert!(bound >= truth.min(9), "case {case}: {truth} crossings, bound {bound}: {a:?} {b:?}");
            tested += 1;
            tight += u64::from(bound == truth);
            wiggly += u64::from(truth >= 3);
        }
        assert!(tested == 1200 && wiggly > 50, "{wiggly} pairs of cubics cross three times or more");
        assert!(tight > 200, "the bound is the number of crossings for {tight} pairs");
    }

    /// A line crossed by a cubic, and the cubic's end at the line's: the bound is the sign changes of the control polygon,
    /// and nothing where the two meet only at the end they share.
    #[test]
    fn a_line_and_a_cubic_are_bounded_by_the_control_polygon_and_by_the_end_they_share() {
        let s = CubicBez::new((0.0, -6.0), (8.0 / 3.0, 26.0 / 3.0), (16.0 / 3.0, -26.0 / 3.0), (8.0, 6.0));
        let across = Line::new((-1.0, 0.0), (9.0, 0.0));
        assert_eq!(line_cubic_cross_at_most(&across, &Curve::new(s)), 3);
        let above = Line::new((-1.0, 100.0), (9.0, 100.0));
        assert_eq!(line_cubic_cross_at_most(&above, &Curve::new(s)), 0);
        // A line that ends where a smooth curve starts, and goes the way the curve comes from: they meet there only.
        let arc = CubicBez::new((0.0, 0.0), (10.0, 5.0), (20.0, 5.0), (30.0, 0.0));
        let behind = Line::new((-10.0, -5.0), (0.0, 0.0));
        assert_eq!(line_cubic_cross_at_most(&behind, &Curve::new(arc)), 0);
        // One that ends there and goes back over the curve is not separated from it.
        let over = Line::new((0.0, 0.0), (15.0, 4.0));
        assert!(line_cubic_cross_at_most(&over, &Curve::new(arc)) <= 3);
        // Signs: zeros are whichever makes more changes.
        assert_eq!(most_sign_changes([1.0, 0.0, 1.0, 1.0], 1e-9), 2);
        assert_eq!(most_sign_changes([1.0, -1.0, 1.0, -1.0], 1e-9), 3);
        assert_eq!(most_sign_changes([1.0, 1.0, 1.0, 1.0], 1e-9), 0);
    }

    /// Neighbours in a path, which share an end and are smooth there, cross nowhere; ones that fold back on one another
    /// are not separated.
    #[test]
    fn cubics_that_share_an_end_and_leave_it_apart_cross_nowhere_else() {
        let first = CubicBez::new((0.0, 0.0), (10.0, 5.0), (20.0, 5.0), (30.0, 0.0));
        let smooth = CubicBez::new((30.0, 0.0), (40.0, -5.0), (50.0, -5.0), (60.0, 0.0));
        assert_eq!(cubics_cross_at_most(&Curve::new(first), &Curve::new(smooth), 1e-6), 0);
        // A corner, the second leaving at a right angle.
        let corner = CubicBez::new((30.0, 0.0), (30.0, 10.0), (35.0, 20.0), (45.0, 25.0));
        assert_eq!(cubics_cross_at_most(&Curve::new(first), &Curve::new(corner), 1e-6), 0);
        // A cusp: the second goes straight back over the first.
        let back = CubicBez::new((30.0, 0.0), (20.0, 5.0), (10.0, 5.0), (0.5, 0.1));
        assert!(cubics_cross_at_most(&Curve::new(first), &Curve::new(back), 1e-6) > 0);
    }

    /// A cubic whose tangents turn through less than a half plane has no loop, and one that curls into one may.
    #[test]
    fn a_cubic_that_may_loop_is_told_from_one_that_cannot() {
        assert!(!Curve::new(CubicBez::new((0.0, 0.0), (10.0, 5.0), (20.0, 5.0), (30.0, 0.0))).may_loop());
        assert!(Curve::new(CubicBez::new((0.0, 0.0), (30.0, 30.0), (-10.0, 30.0), (20.0, 0.0))).may_loop());
    }

    /// The flatness of a cubic: how far its inner control points are from its chord.
    fn flatness(c: &CubicBez) -> f64 {
        let d = c.p3 - c.p0;
        let n = d.hypot();
        let off = |p: Point| if n > 0.0 { (d.cross(p - c.p0) / n).abs() } else { (p - c.p0).hypot() };
        off(c.p1).max(off(c.p2))
    }

    /// Where the cubics `a` and `b` cross, by subdividing the one that is less flat where the boxes of the two overlap until
    /// both are within `tol` of their chords and taking the crossings of the chords: `None` where `budget` runs out.
    fn oracle(a: &CubicBez, b: &CubicBez, tol: f64, budget: &mut u64, out: &mut u64) -> Option<()> {
        *budget = budget.checked_sub(1)?;
        let (ba, bb) = (hull(&points(a)), hull(&points(b)));
        if !overlap(&ba, &bb, 0.0) {
            return Some(());
        }
        let (fa, fb) = (flatness(a), flatness(b));
        if fa <= tol && fb <= tol {
            let (r, s, w) = (a.p3 - a.p0, b.p3 - b.p0, b.p0 - a.p0);
            let den = r.cross(s);
            if den != 0.0 {
                let (t, u) = (w.cross(s) / den, w.cross(r) / den);
                *out += u64::from((0.0..1.0).contains(&t) && (0.0..1.0).contains(&u));
            }
            return Some(());
        }
        if fa >= fb {
            let (a1, a2) = a.subdivide();
            oracle(&a1, b, tol, budget, out)?;
            oracle(&a2, b, tol, budget, out)
        } else {
            let (b1, b2) = b.subdivide();
            oracle(a, &b1, tol, budget, out)?;
            oracle(a, &b2, tol, budget, out)
        }
    }

    /// The crossings of two cubics, from the oracle at two tolerances, which must agree: `None` where it cannot tell.
    fn truth(a: &CubicBez, b: &CubicBez) -> Option<u64> {
        let scale = largest(&points(a)).max(largest(&points(b)));
        let count = |tol: f64| {
            let (mut budget, mut n) = (200_000u64, 0u64);
            oracle(a, b, tol, &mut budget, &mut n).map(|()| n)
        };
        let (c1, c2) = (count(16.0 * f64::EPSILON * scale + 1e-9)?, count(1e-12 * scale.max(1.0))?);
        (c1 == c2).then_some(c1)
    }

    /// The sweep's tolerance at the extent of the two, which is how far apart ends may be and be one.
    fn slack_of(a: &CubicBez, b: &CubicBez) -> f64 {
        crate::boolean::sweep_tolerance(largest(&points(a)).max(largest(&points(b))))
    }

    /// Ends that match to within the slack but not exactly are two points, and two cones apart from two points are not
    /// curves that cross nowhere: straight cubics from points 0.9e-6 apart (the slack is 1e-6) whose directions differ by 3e-9
    /// to 1e-4 cross 9e-3 to 300 along, and were bounded 0 (6,372 of 20,000 crossed and were). Straight ones are
    /// told by the crossing of their chords, bent ones by the oracle by subdivision.
    #[test]
    fn ends_a_hair_apart_are_counted_as_two_points() {
        let mut rng = Rng(0x00ba_dfee_d123_4567);
        let dir = |a: f64| Vec2::new(a.cos(), a.sin());
        let (mut crossing, mut bad) = (0u64, 0u64);
        for k in 0..2000 {
            let s = Point::new(rng.range(-100.0, 100.0), rng.range(-100.0, 100.0));
            let th = rng.range(0.0, std::f64::consts::TAU);
            let gap = 10f64.powf(rng.range(-8.5, -4.0));
            let len = rng.range(10.0, 100.0);
            let off = dir(th + std::f64::consts::FRAC_PI_2) * 0.9e-6 * if k % 2 == 0 { 1.0 } else { -1.0 };
            let straight = |from: Point, a: f64| CubicBez::new(from, from + dir(a) * len / 3.0, from + dir(a) * 2.0 * len / 3.0, from + dir(a) * len);
            let (a, b) = (straight(s, th), straight(s + off, th + gap));
            let (r, t, w) = (a.p3 - a.p0, b.p3 - b.p0, b.p0 - a.p0);
            let den = r.cross(t);
            let (u, v) = (w.cross(t) / den, w.cross(r) / den);
            let crosses = (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v);
            let bound = cubics_cross_at_most(&Curve::new(a), &Curve::new(b), slack_of(&a, &b));
            crossing += u64::from(crosses);
            bad += u64::from(crosses && bound == 0);
        }
        assert!(crossing > 300, "{crossing} of the pairs cross");
        assert_eq!(bad, 0, "pairs that cross were bounded 0");
        // And bent ones, from near ends, against the oracle.
        let (mut cases, mut bad) = (0u64, 0u64);
        for k in 0..300 {
            let s = Point::new(rng.range(-100.0, 100.0), rng.range(-100.0, 100.0));
            let th = rng.range(0.0, std::f64::consts::TAU);
            let gap = 10f64.powf(rng.range(-6.0, -2.0));
            let off = dir(th + std::f64::consts::FRAC_PI_2) * 0.9e-6 * if k % 2 == 0 { 1.0 } else { -1.0 };
            let bend = |from: Point, a: f64, side: f64| {
                let len = 50.0;
                CubicBez::new(
                    from,
                    from + dir(a) * len / 3.0 + dir(a + 1.5) * side,
                    from + dir(a) * 2.0 * len / 3.0 - dir(a + 1.5) * side,
                    from + dir(a) * len,
                )
            };
            let (side_a, side_b) = (rng.range(-5.0, 5.0), rng.range(-5.0, 5.0));
            let (a, b) = (bend(s, th, side_a), bend(s + off, th + gap, side_b));
            let Some(t) = truth(&a, &b) else { continue };
            cases += 1;
            bad += u64::from(t.min(9) > cubics_cross_at_most(&Curve::new(a), &Curve::new(b), slack_of(&a, &b)));
        }
        assert!(cases > 150, "{cases} decided");
        assert_eq!(bad, 0, "pairs that cross were bounded below the crossings");
    }

    /// The segments of a path share exact ends, and neighbours cost nothing, whatever the place and size of the path: a circle
    /// of radius 1,000 at (1e12, 1e12) cut into 100,000 cubics has handles of 0.021 against a slack of 0.014, which the rays from
    /// a joint dropped for being within the slack left as every direction (18 for each joint, 1.8 million: refused); and at the
    /// origin handles under 2e-6 did the same.
    #[test]
    fn the_joints_of_a_circle_of_many_cubics_cost_nothing_wherever_it_is() {
        use crate::testutil::cubic_circle;
        use kurbo::PathSeg;
        for (r, c) in [(1000.0, 1e12), (1000.0, 1e9), (1000.0, 0.0), (0.09, 0.0), (1e-3, 1e6)] {
            let circle = cubic_circle(100_000, r, c, c);
            let cubics: Vec<CubicBez> = circle.segments().filter_map(|s| if let PathSeg::Cubic(c) = s { Some(c) } else { None }).collect();
            assert_eq!(cubics.len(), 100_000);
            let slack = crate::boolean::sweep_tolerance(c + r);
            let mut at = 0u64;
            for k in (0..cubics.len()).step_by(97) {
                let (a, b) = (&cubics[k], &cubics[(k + 1) % cubics.len()]);
                at += cubics_cross_at_most(&Curve::new(*a), &Curve::new(*b), slack);
            }
            assert_eq!(at, 0, "radius {r} at {c:e}");
        }
    }
}
