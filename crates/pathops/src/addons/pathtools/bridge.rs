//! Smooth Bridge: two circles joined into one outline by two concave arcs, each touching both
//! circles from outside, so the outline runs on smoothly from each circle into the bridge.

use std::f64::consts::{FRAC_PI_2, PI, SQRT_2, TAU};

use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{CubicBez, ParamCurve, Point, Shape, Vec2};

use super::Circle;

/// How far (as a share of the radius) a path's outline may stray from a circle and still be read
/// as that circle.
const ROUND: f64 = 1e-3;
/// Points [`circle_of`] checks on each segment, its ends included.
const SAMPLES: usize = 9;

/// Why [`smooth_bridge`] makes no outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// One circle is inside the other (or they share a center).
    Inside,
    /// The circles are too far apart for the radius: the two arcs would not reach both circles,
    /// or would cross before they do.
    TooFar,
    /// A radius or circle that is not a finite size above 0, or an outline too large for `f64`.
    Bad,
}

/// The circle `path` draws: one closed subpath whose outline stays within 0.1% of its radius of
/// a circle all the way around, once. `None` for any other path.
pub fn circle_of(path: &PathData) -> Option<Circle> {
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() < 2 {
        return None;
    }
    let b = path.bounds()?;
    let (w, h) = (b.width(), b.height());
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) || (w - h).abs() > ROUND * w.max(h) {
        return None;
    }
    let (center, radius) = (b.center(), (w + h) / 4.0);
    let off = |q: Point| (q.distance(center) - radius).abs() > ROUND * radius;
    for k in 0..sp.segment_count() {
        let seg = sp.segment(k);
        if (0..SAMPLES).any(|i| off(seg.eval(i as f64 / (SAMPLES - 1) as f64))) {
            return None;
        }
    }
    // Around once: a path going around twice, or back on itself, encloses a different area.
    let area = path.to_bezpath().area().abs();
    ((area - PI * radius * radius).abs() <= 4.0 * ROUND * area).then_some(Circle { center, radius })
}

/// The outline of circles `a` and `b` joined by two arcs of radius `radius` (pt), one on each
/// side of the line through their centers, each touching both circles from outside. The outline
/// runs around `a` on its side away from `b`, along one arc, around `b` on its side away from `a`
/// and back along the other arc; every anchor is smooth, as each arc and the circle it meets share
/// a tangent there. Overlapping and touching circles are joined too. A larger radius makes a wider,
/// gentler bridge; farther apart circles make a narrower one, until the arcs would cross.
pub fn smooth_bridge(a: Circle, b: Circle, radius: f64) -> Result<SubPath, BridgeError> {
    let finite = |x: f64| x.is_finite() && x > 0.0;
    let points = [a.center, b.center].iter().all(|c| c.x.is_finite() && c.y.is_finite());
    if !finite(radius) || !finite(a.radius) || !finite(b.radius) || !points {
        return Err(BridgeError::Bad);
    }
    let axis = b.center - a.center;
    // `f64::hypot`: `Vec2::hypot` squares the length, which overflows above about 1e154 pt.
    let d = axis.x.hypot(axis.y);
    if d + a.radius.min(b.radius) <= a.radius.max(b.radius) * (1.0 + 1e-9) {
        return Err(BridgeError::Inside);
    }
    // The arcs' centers are `radius` outside both circles: where circles about the two centers,
    // `radius` larger than each, cross.
    let (ra, rb) = (radius + a.radius, radius + b.radius);
    if !ra.is_finite() || !rb.is_finite() {
        return Err(BridgeError::Bad);
    }
    // Halves and no squares, so no step overflows while the lengths fit in `f64`.
    let half = ra / 2.0 + rb / 2.0;
    if d / 2.0 > half {
        return Err(BridgeError::TooFar);
    }
    let along = d / 2.0 + (ra - rb) / d * half;
    let across = (ra - along).max(0.0).sqrt() * (ra / 2.0 + along / 2.0).max(0.0).sqrt() * SQRT_2;
    if !along.is_finite() || !across.is_finite() {
        return Err(BridgeError::Bad);
    }
    // Each arc runs between its points on `a` and `b`, which lie towards the two centers from the
    // arc's center. Its point nearest the axis is the foot of its center on the axis when that foot
    // is between the two centers, else one of its ends, which are off the axis. So the two arcs
    // cross, leaving no neck, only when that foot is between the centers and nearer to the arc's
    // center than its radius.
    if along > 0.0 && along < d && across <= radius * (1.0 + 1e-9) {
        return Err(BridgeError::TooFar);
    }
    let u = axis / d;
    let n = Vec2::new(-u.y, u.x);
    let mut segs = vec![];
    // Each side's arc center and where its arc touches `a` and `b`.
    let side = |s: f64| {
        let p = a.center + u * along + n * (s * across);
        (p, a.center + (p - a.center) * (a.radius / ra), b.center + (p - b.center) * (b.radius / rb))
    };
    let ((p1, a1, b1), (p2, a2, b2)) = (side(1.0), side(-1.0));
    arc(&mut segs, a.center, a.radius, a1, a2, Some(-u));
    arc(&mut segs, p2, radius, a2, b2, None);
    arc(&mut segs, b.center, b.radius, b2, b1, Some(u));
    arc(&mut segs, p1, radius, b1, a1, None);
    let count = segs.len();
    let anchors: Vec<Anchor> = (0..count)
        .map(|i| {
            let (prev, seg) = (segs[(i + count - 1) % count], segs[i]);
            Anchor { p: seg.p0, h_in: prev.p2, h_out: seg.p1, kind: AnchorKind::Smooth }
        })
        .collect();
    let finite_point = |p: Point| p.x.is_finite() && p.y.is_finite();
    if !anchors.iter().all(|q| finite_point(q.p) && finite_point(q.h_in) && finite_point(q.h_out)) {
        return Err(BridgeError::Bad);
    }
    Ok(SubPath::new(anchors, true))
}

/// Append to `out` the arc of the circle about `center` with radius `r` from `from` to `to` (both
/// on it): through the direction `through` from the center when given, else the shorter way. In
/// cubics of at most a quarter turn each, `from` exactly at the start and `to` at the end.
fn arc(out: &mut Vec<CubicBez>, center: Point, r: f64, from: Point, to: Point, through: Option<Vec2>) {
    let angle = |q: Vec2| q.y.atan2(q.x);
    let start = angle(from - center);
    // The turn from `from` to `to` counter-clockwise (in the y-up sense), 0 to 2π.
    let ccw = (angle(to - center) - start).rem_euclid(TAU);
    let sweep = match through {
        Some(dir) if (angle(dir) - start).rem_euclid(TAU) <= ccw => ccw,
        Some(_) => ccw - TAU,
        None if ccw > PI => ccw - TAU,
        None => ccw,
    };
    let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / pieces as f64;
    // Handle length that makes a cubic follow the arc of `step`, signed with its direction.
    let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
    let on = |t: f64| center + Vec2::new(t.cos(), t.sin()) * r;
    let tangent = |t: f64| Vec2::new(-t.sin(), t.cos());
    for i in 0..pieces {
        let (t0, t1) = (start + step * i as f64, start + step * (i + 1) as f64);
        let p0 = if i == 0 { from } else { on(t0) };
        let p3 = if i + 1 == pieces { to } else { on(t1) };
        out.push(CubicBez::new(p0, p0 + tangent(t0) * k, p3 - tangent(t1) * k, p3));
    }
}

#[cfg(test)]
mod tests {
    use craft_geom::shapes;

    use super::*;

    fn circle(x: f64, y: f64, r: f64) -> Circle {
        Circle { center: Point::new(x, y), radius: r }
    }

    /// Every point the outline passes through, 32 to a segment.
    fn outline(sp: &SubPath) -> Vec<Point> {
        (0..sp.segment_count()).flat_map(|k| (0..32).map(move |i| sp.segment(k).eval(i as f64 / 32.0))).collect()
    }

    /// The outline crosses the vertical line at `x` at these y values, top to bottom.
    fn crossings_at(sp: &SubPath, x: f64) -> Vec<f64> {
        let pts = outline(sp);
        let mut ys: Vec<f64> = (0..pts.len())
            .filter_map(|i| {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                ((p.x - x) * (q.x - x) <= 0.0 && p.x != q.x).then(|| p.y + (q.y - p.y) * (x - p.x) / (q.x - p.x))
            })
            .collect();
        ys.sort_by(f64::total_cmp);
        ys
    }

    /// The bridge's width at `x`, from the outline's crossings there.
    fn width_at(sp: &SubPath, x: f64) -> f64 {
        let ys = crossings_at(sp, x);
        ys.last().unwrap() - ys.first().unwrap()
    }

    fn assert_smooth(sp: &SubPath) {
        for a in sp.anchors.iter() {
            let (i, o) = (a.p - a.h_in, a.h_out - a.p);
            assert!(i.hypot() > 1e-6 && o.hypot() > 1e-6, "both handles at {a:?}");
            assert!(i.cross(o).abs() <= 1e-9 * i.hypot() * o.hypot() && i.dot(o) > 0.0, "the outline turns at {a:?}");
            assert_eq!(a.kind, AnchorKind::Smooth);
        }
    }

    /// Each point of the outline is on circle `c` or outside it.
    fn outside(sp: &SubPath, c: Circle) -> bool {
        outline(sp).iter().all(|p| p.distance(c.center) >= c.radius - 1e-3)
    }

    #[test]
    fn two_equal_circles_join_with_a_neck_narrower_than_the_circles() {
        let (a, b) = (circle(0.0, 0.0, 20.0), circle(60.0, 0.0, 20.0));
        let sp = smooth_bridge(a, b, 20.0).unwrap();
        assert!(sp.closed);
        assert_smooth(&sp);
        // The arcs' centers are 40 from each circle's center: (30, ±√700), so the neck at x = 30 is
        // 2 × (√700 − 20) wide.
        let neck = 2.0 * (700f64.sqrt() - 20.0);
        assert!((width_at(&sp, 30.0) - neck).abs() < 1e-2, "{}", width_at(&sp, 30.0));
        assert!(width_at(&sp, 30.0) < width_at(&sp, 10.0));
        // Around each circle on its far side, and symmetric.
        let ys = crossings_at(&sp, -10.0);
        assert!((ys[0] + 300f64.sqrt()).abs() < 1e-2 && (ys[1] - 300f64.sqrt()).abs() < 1e-2, "{ys:?}");
        assert!((width_at(&sp, 50.0) - width_at(&sp, 10.0)).abs() < 1e-2);
        let b = PathData::single(sp).bounds().unwrap();
        // Within the cubics' own error from a true circle (about 0.03% of the radius).
        assert!((b.x0 + 20.0).abs() < 1e-2 && (b.x1 - 80.0).abs() < 1e-2 && (b.y1 - 20.0).abs() < 1e-2, "{b:?}");
    }

    #[test]
    fn the_arcs_touch_both_circles_and_run_on_from_them() {
        for (a, b, r) in [
            (circle(0.0, 0.0, 20.0), circle(60.0, 0.0, 20.0), 20.0),
            (circle(10.0, 5.0, 30.0), circle(70.0, 40.0, 8.0), 40.0),
            (circle(0.0, 0.0, 20.0), circle(30.0, 0.0, 15.0), 10.0),
        ] {
            let sp = smooth_bridge(a, b, r).unwrap();
            assert_smooth(&sp);
            // The outline goes around the outside of both circles, never into either.
            assert!(outside(&sp, a) && outside(&sp, b));
            // On each circle the outline leaves along the circle's tangent, where an arc takes over
            // as well: two anchors on each circle are where an arc meets it.
            for c in [a, b] {
                let on: Vec<_> = sp.anchors.iter().filter(|q| (q.p.distance(c.center) - c.radius).abs() < 1e-6).collect();
                assert!(on.len() >= 2, "{}", on.len());
                for q in on {
                    let radial = q.p - c.center;
                    assert!(radial.dot(q.h_out - q.p).abs() <= 1e-6 * radial.hypot() * (q.h_out - q.p).hypot(), "{q:?}");
                }
            }
            assert!(sp.anchors.len() <= 12, "{}", sp.anchors.len());
            // Every segment off the two circles is part of an arc of radius `r` that touches both
            // circles from outside: its center is `r` farther than each circle's radius from it.
            let on = |q: Point, c: Circle| (q.distance(c.center) - c.radius).abs() < 1e-3;
            let mut arcs = 0;
            for k in 0..sp.segment_count() {
                let seg = sp.segment(k);
                let (p0, pm, p1) = (seg.eval(0.0), seg.eval(0.5), seg.eval(1.0));
                if on(pm, a) || on(pm, b) {
                    continue;
                }
                arcs += 1;
                let (center, radius) = circumcircle(p0, pm, p1);
                assert!((radius - r).abs() < 1e-2 * r, "radius {radius} for {r}");
                assert!((center.distance(a.center) - (r + a.radius)).abs() < 1e-2 * r, "touches {a:?}");
                assert!((center.distance(b.center) - (r + b.radius)).abs() < 1e-2 * r, "touches {b:?}");
            }
            assert!((2..=4).contains(&arcs), "{arcs}");
        }
    }

    /// The circle through three points: its center and radius.
    fn circumcircle(p: Point, q: Point, s: Point) -> (Point, f64) {
        let d = 2.0 * (p.x * (q.y - s.y) + q.x * (s.y - p.y) + s.x * (p.y - q.y));
        let sq = |v: Point| v.x * v.x + v.y * v.y;
        let x = (sq(p) * (q.y - s.y) + sq(q) * (s.y - p.y) + sq(s) * (p.y - q.y)) / d;
        let y = (sq(p) * (s.x - q.x) + sq(q) * (p.x - s.x) + sq(s) * (q.x - p.x)) / d;
        let c = Point::new(x, y);
        (c, c.distance(p))
    }

    #[test]
    fn unequal_circles_join_round_both() {
        let (a, b) = (circle(0.0, 0.0, 30.0), circle(70.0, 0.0, 10.0));
        let sp = smooth_bridge(a, b, 40.0).unwrap();
        assert_smooth(&sp);
        let bounds = PathData::single(sp.clone()).bounds().unwrap();
        assert!((bounds.x0 + 30.0).abs() < 1e-2 && (bounds.x1 - 80.0).abs() < 1e-2 && (bounds.y0 + 30.0).abs() < 1e-2, "{bounds:?}");
        // The neck narrows from the big circle towards the small one.
        assert!(width_at(&sp, 35.0) > width_at(&sp, 50.0));
        assert!(width_at(&sp, 50.0) < 20.0);
    }

    #[test]
    fn unequal_circles_join_at_large_radii_overlapping_or_not() {
        // With a large radius the arcs' centers lie beyond the smaller circle, so the arcs never
        // reach the axis even though their centers are nearer to it than their radius.
        for (a, b, r) in [
            (circle(0.0, 0.0, 10.0), circle(25.0, 0.0, 30.0), 100.0),
            (circle(0.0, 0.0, 30.0), circle(25.0, 0.0, 10.0), 30.0),
            (circle(0.0, 0.0, 30.0), circle(25.0, 0.0, 10.0), 50.0),
            (circle(0.0, 0.0, 30.0), circle(60.0, 0.0, 8.0), 300.0),
        ] {
            let sp = smooth_bridge(a, b, r).unwrap_or_else(|e| panic!("{a:?} {b:?} {r}: {e:?}"));
            assert_smooth(&sp);
            assert!(outside(&sp, a) && outside(&sp, b));
            // The outline crosses the axis only at the two far ends: it does not cross itself.
            let pts = outline(&sp);
            let crossings = (0..pts.len()).filter(|&i| (pts[i].y > 0.0) != (pts[(i + 1) % pts.len()].y > 0.0)).count();
            assert_eq!(crossings, 2, "{a:?} {b:?} {r}");
            // It encloses more than the larger circle.
            let area = PathData::single(sp).to_bezpath().area().abs();
            assert!(area > PI * a.radius.max(b.radius).powi(2), "{area}");
        }
    }

    #[test]
    fn overlapping_and_touching_circles_are_joined() {
        let (a, b) = (circle(0.0, 0.0, 20.0), circle(30.0, 0.0, 20.0));
        let sp = smooth_bridge(a, b, 10.0).unwrap();
        assert_smooth(&sp);
        // The circles cross at x = 15, y = ±√175; the bridge fills in the notch there.
        assert!(width_at(&sp, 15.0) > 2.0 * 175f64.sqrt());
        assert!(width_at(&sp, 15.0) < 40.0);
        let touching = smooth_bridge(circle(0.0, 0.0, 20.0), circle(40.0, 0.0, 20.0), 10.0).unwrap();
        assert_smooth(&touching);
        assert!(width_at(&touching, 20.0) > 1.0);
    }

    #[test]
    fn far_apart_circles_and_nested_ones_make_no_bridge() {
        let (a, b) = (circle(0.0, 0.0, 10.0), circle(100.0, 0.0, 10.0));
        assert_eq!(smooth_bridge(a, b, 5.0), Err(BridgeError::TooFar), "arcs don't reach");
        // The arcs reach both circles (gap 80 < 2 × 45) but cross before they do.
        assert_eq!(smooth_bridge(a, b, 45.0), Err(BridgeError::TooFar), "no neck left");
        // A large enough radius bridges them: the neck stays open above (d/2)² − r² over 2r = 120.
        assert!(smooth_bridge(a, b, 130.0).is_ok());
        assert_eq!(smooth_bridge(circle(0.0, 0.0, 30.0), circle(5.0, 0.0, 10.0), 10.0), Err(BridgeError::Inside));
        assert_eq!(smooth_bridge(circle(0.0, 0.0, 30.0), circle(0.0, 0.0, 30.0), 10.0), Err(BridgeError::Inside));
        assert_eq!(smooth_bridge(a, b, 0.0), Err(BridgeError::Bad));
        assert_eq!(smooth_bridge(a, b, f64::NAN), Err(BridgeError::Bad));
        assert_eq!(smooth_bridge(circle(f64::INFINITY, 0.0, 10.0), b, 10.0), Err(BridgeError::Bad));
    }

    #[test]
    fn huge_circles_bridge_with_finite_anchors_or_are_refused() {
        // Squaring these lengths overflows f64.
        let (a, b) = (circle(0.0, 0.0, 5e153), circle(1.5e154, 0.0, 5e153));
        let sp = smooth_bridge(a, b, 5e153).unwrap();
        assert!(sp.anchors.iter().all(|q| [q.p, q.h_in, q.h_out].iter().all(|p| p.x.is_finite() && p.y.is_finite())));
        assert_smooth(&sp);
        // The two radius sums fit in f64, but not their total.
        let (a, b) = (circle(-5e307, 0.0, 1e307), circle(5e307, 0.0, 1e307));
        let sp = smooth_bridge(a, b, 1.5e308).unwrap();
        assert!(sp.anchors.iter().all(|q| [q.p, q.h_in, q.h_out].iter().all(|p| p.x.is_finite() && p.y.is_finite())));
        // The arcs' centers would lie beyond the largest f64.
        let (a, b) = (circle(0.0, 0.0, 1e308), circle(1.5e308, 0.0, 1e308));
        assert_eq!(smooth_bridge(a, b, 1e308), Err(BridgeError::Bad));
    }

    #[test]
    fn circles_are_told_from_other_paths() {
        let round = shapes::ellipse(kurbo::Rect::new(10.0, 20.0, 50.0, 60.0));
        let c = circle_of(&round).unwrap();
        assert!(c.center.distance(Point::new(30.0, 40.0)) < 1e-9 && (c.radius - 20.0).abs() < 1e-9);
        let oval = shapes::ellipse(kurbo::Rect::new(0.0, 0.0, 50.0, 40.0));
        assert_eq!(circle_of(&oval), None);
        let square = PathData::single(SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)], true));
        assert_eq!(circle_of(&square), None);
        let mut open = round.clone();
        open.subpaths[0].closed = false;
        assert_eq!(circle_of(&open), None);
        let two = PathData::new(vec![round.subpaths[0].clone(), round.subpaths[0].clone()]);
        assert_eq!(circle_of(&two), None);
    }
}
