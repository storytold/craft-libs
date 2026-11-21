//! Live Corners on any path: the anchors where two sides, straight or curved, meet at an angle,
//! and the path with those corners cut round, inverted round or chamfered (a curved side trimmed
//! to what the cut leaves of it). Rectangles cut theirs the same way.

use kurbo::{CubicBez, ParamCurve, ParamCurveArclen, ParamCurveDeriv, Point, Vec2};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};
use crate::shapes::{CornerKind, KAPPA};

/// Lengths and radii at or below this are none at all.
const EPS: f64 = 1e-9;
/// Sides closer than this to straight on (or folded back), as the sine of their angle, make no corner.
const MIN_SINE: f64 = 1e-6;

/// How closely arc lengths along curved sides are measured (points).
const ARCLEN_ACCURACY: f64 = 1e-6;

/// A side of a corner: the segment from the corner's anchor to its neighbour, as a cubic leaving
/// the corner (a straight side has its handles at its ends).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Side {
    pub curve: CubicBez,
}

impl Side {
    /// Is the side straight, its handles on its ends?
    fn straight(&self) -> bool {
        self.curve.p1.distance(self.curve.p0) <= EPS && self.curve.p2.distance(self.curve.p3) <= EPS
    }

    /// The unit direction the side leaves the corner in: towards its first control point not on
    /// the corner. `None` when the side has no length.
    fn direction(&self) -> Option<Vec2> {
        let c = self.curve;
        let d = [c.p1, c.p2, c.p3].into_iter().map(|q| q - c.p0).find(|d| d.hypot() > EPS)?;
        let d = d / d.hypot();
        d.x.is_finite().then_some(d)
    }

    /// The side's length.
    fn length(&self) -> f64 {
        if self.straight() {
            self.curve.p0.distance(self.curve.p3)
        } else {
            crate::path::measured_arclen(&kurbo::PathSeg::Cubic(self.curve), ARCLEN_ACCURACY)
        }
    }

    /// The parameter `s` along the side from the corner, the point there and the direction the
    /// side runs on in (away from the corner).
    fn at_length(&self, s: f64, u: Vec2) -> (f64, Point, Vec2) {
        let c = self.curve;
        if self.straight() {
            let len = c.p0.distance(c.p3);
            let t = if len > EPS { (s / len).clamp(0.0, 1.0) } else { 0.0 };
            return (t, c.p0 + u * s, u);
        }
        let t = c.inv_arclen(s, ARCLEN_ACCURACY).clamp(0.0, 1.0);
        let d = c.deriv().eval(t).to_vec2();
        let dir = if d.hypot() > EPS { d / d.hypot() } else { u };
        (t, c.eval(t), dir)
    }
}

/// A corner Live Corners can cut: an anchor that isn't smooth, where two sides (straight or
/// curved) meet at an angle (#1144).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// The anchor's index in its path, counting the anchors of every subpath in order.
    pub index: usize,
    /// Where the sides meet.
    pub at: Point,
    /// Unit directions the sides leave the corner in: towards the previous anchor, then towards
    /// the next one.
    pub u: Vec2,
    pub v: Vec2,
    /// The sides' lengths (along curves), in the same order.
    pub lu: f64,
    pub lv: f64,
    /// The sides themselves, leaving the corner, in the same order.
    pub side_u: Side,
    pub side_v: Side,
    /// The anchor indices (in the path) of the anchors at the other ends of the sides, in the
    /// same order.
    pub before: usize,
    pub after: usize,
}

impl Corner {
    /// The corner of anchor `i` of `sp`, whose index in the path is `index`; `None` when the
    /// anchor is an end of an open subpath or is smooth, when a side has no length, or when the
    /// sides leave it straight on.
    fn of(sp: &SubPath, i: usize, index: usize) -> Option<Self> {
        let n = sp.anchors.len();
        let (prev_i, next_i) = if sp.closed { ((i + n).checked_sub(1)? % n, (i + 1) % n) } else { (i.checked_sub(1)?, i + 1) };
        let (a, prev, next) = (sp.anchors.get(i)?, sp.anchors.get(prev_i)?, sp.anchors.get(next_i)?);
        let first = index.checked_sub(i)?;
        let (before, after) = (first + prev_i, first + next_i);
        if a.kind == AnchorKind::Smooth {
            return None;
        }
        let side_u = Side { curve: CubicBez::new(a.p, a.h_in, prev.h_out, prev.p) };
        let side_v = Side { curve: CubicBez::new(a.p, a.h_out, next.h_in, next.p) };
        let (u, v) = (side_u.direction()?, side_v.direction()?);
        let (lu, lv) = (side_u.length(), side_v.length());
        if !(lu > EPS && lv > EPS && lu.is_finite() && lv.is_finite()) {
            return None;
        }
        let c = Self { index, at: a.p, u, v, lu, lv, side_u, side_v, before, after };
        (c.sin() > MIN_SINE).then_some(c)
    }

    /// The cosine of the angle between the sides.
    pub fn cos(&self) -> f64 {
        self.u.dot(self.v)
    }

    /// The sine of the angle between the sides.
    pub fn sin(&self) -> f64 {
        self.u.cross(self.v).abs()
    }

    /// The angle between the sides, in degrees (90 at a rectangle's corners).
    pub fn angle_deg(&self) -> f64 {
        self.sin().atan2(self.cos()).to_degrees()
    }

    /// How far from the corner a round cut of radius `r` meets each side: r / tan(angle / 2).
    pub fn setback(&self, r: f64) -> f64 {
        r * (1.0 + self.cos()) / self.sin()
    }

    /// The radius of a round cut that meets each side `s` from the corner (the inverse of
    /// [`Corner::setback`]).
    pub fn radius_at_setback(&self, s: f64) -> f64 {
        s * self.sin() / (1.0 + self.cos())
    }

    /// The largest radius the corner draws whatever its neighbours do: its cut then reaches
    /// halfway along the shorter side, so neighbouring corners at their largest meet without
    /// overlapping (half the shorter side at a rectangle's corners).
    pub fn max_radius(&self) -> f64 {
        self.radius_at_setback(self.lu.min(self.lv) / 2.0)
    }

    /// The largest radius the corner draws when the cuts of the corners at the other ends of its
    /// sides reach `before` along side u and `after` along side v: its cut reaches halfway along
    /// a side, or further, up to where the other end's cut begins, so all of a side whose other
    /// end isn't cut (#2332).
    pub fn max_radius_beside(&self, before: f64, after: f64) -> f64 {
        let room = |len: f64, other: f64| (len - other.max(0.0)).max(len / 2.0);
        self.radius_at_setback(room(self.lu, before).min(room(self.lv, after)))
    }

    /// The largest radius the corner draws when it and the corners for which `edited` holds are
    /// set to one radius together, the other corners of `corners` keeping their `radii` (by
    /// anchor index). A side shared with an edited corner takes both cuts as far as they fit
    /// side by side, or halfway each; on a side to any other corner the cut reaches up to where
    /// that corner's cut begins ([`Corner::max_radius_beside`]).
    pub fn max_shared_radius(&self, corners: &[Corner], radii: &[f64], edited: impl Fn(usize) -> bool) -> f64 {
        let side = |len: f64, other: usize| match find(corners, other) {
            Some(c) if other == self.index || edited(other) => {
                let both = self.setback(1.0) + c.setback(1.0);
                let apart = if both > EPS { len / both } else { f64::INFINITY };
                apart.max(self.radius_at_setback(len / 2.0))
            }
            _ => self.radius_at_setback((len - requested_setback(corners, radii, other)).max(len / 2.0)),
        };
        side(self.lu, self.before).min(side(self.lv, self.after))
    }

    /// The point `k` along the bisector, measured in (u + v): the centre of a round cut of radius
    /// `r` is at `k = r / sin`.
    pub fn on_bisector(&self, k: f64) -> Point {
        self.at + (self.u + self.v) * k
    }

    /// How much the radius grows when the centre of the round cut moves by `d`: `d`'s part along
    /// the bisector, in radius units.
    pub fn radius_change(&self, d: Vec2) -> f64 {
        d.dot(self.u + self.v) * self.sin() / (2.0 + 2.0 * self.cos())
    }

    /// The cut of radius `radius` (> 0, fitted): where it meets each side, `setback` along it,
    /// and the cut's own curve between them, its handles leaving the sides smoothly.
    fn cut(&self, radius: f64, kind: CornerKind) -> Cut {
        let s = self.setback(radius);
        let (tu, start, du) = self.side_u.at_length(s, self.u);
        let (tv, end, dv) = self.side_v.at_length(s, self.v);
        let cos = du.dot(dv);
        let (h_out, h_in) = match kind {
            // An arc tangent to both sides, turning by 180° less the angle between them there.
            CornerKind::Round => {
                let (hu, hv) = self.round_handles(radius, (start, du), (end, dv));
                (start - du * hu, end - dv * hv)
            }
            // An arc centred on the corner through both ends, spanning the angle.
            CornerKind::InvertedRound => {
                let h = s * arc_handle(cos);
                (start + square_to(du, dv) * h, end + square_to(dv, du) * h)
            }
            CornerKind::Chamfer => (start, end),
        };
        Cut { tu, tv, start, h_out, h_in, end }
    }
}

impl Corner {
    /// The handle lengths of a round cut of `radius` between `start` and `end` (each a point and
    /// the direction its side runs on in there). Between straight sides they are the arc's. A
    /// curved side has turned by the time the cut meets it, so the arc's handles would reach past
    /// where the tangents at the cut's ends cross and the cut would stick out of the corner
    /// (#1512): there each handle is the share an arc takes of its tangent, up to that crossing
    /// (a third of the way to the other end where the tangents don't cross between them).
    fn round_handles(&self, radius: f64, (start, du): (Point, Vec2), (end, dv): (Point, Vec2)) -> (f64, f64) {
        let cos = du.dot(dv);
        let arc = arc_handle(-cos);
        if self.side_u.straight() && self.side_v.straight() {
            return (radius * arc, radius * arc);
        }
        // start − du·a = end − dv·b is where the tangents cross.
        let (cross, w) = (du.cross(dv), start - end);
        let (a, b) = (w.cross(dv) / cross, w.cross(du) / cross);
        if cross.abs() > MIN_SINE && a > EPS && b > EPS && a.is_finite() && b.is_finite() {
            // An arc's handle over its tangent's length, radius · tan(turn / 2).
            let share = arc * (1.0 - cos) / cross.abs();
            (a * share, b * share)
        } else {
            let third = start.distance(end) / 3.0;
            (third, third)
        }
    }
}

/// A corner cut: the parameters where it meets its sides (from the corner), its two new anchors
/// and the handles between them.
#[derive(Clone, Copy, Debug)]
struct Cut {
    tu: f64,
    tv: f64,
    start: Point,
    h_out: Point,
    h_in: Point,
    end: Point,
}

/// The unit direction square to unit `a`, on `b`'s side.
fn square_to(a: Vec2, b: Vec2) -> Vec2 {
    let w = b - a * a.dot(b);
    let l = w.hypot();
    if l > EPS { w / l } else { Vec2::ZERO }
}

/// The handle length, per unit of radius, of a cubic drawing a circular arc that spans an angle
/// whose cosine is `cos`: 4/3 · tan(angle / 4) ([`KAPPA`] for a quarter circle).
fn arc_handle(cos: f64) -> f64 {
    if cos.abs() < 1e-12 {
        return KAPPA;
    }
    let cos = cos.clamp(-1.0, 1.0);
    let (sin_half, cos_half) = (((1.0 - cos) / 2.0).max(0.0).sqrt(), ((1.0 + cos) / 2.0).max(0.0).sqrt());
    4.0 / 3.0 * sin_half / (1.0 + cos_half)
}

/// The corners of `path` Live Corners can cut, in anchor order.
pub fn path_corners(path: &PathData) -> Vec<Corner> {
    let mut out = vec![];
    let mut first = 0;
    for sp in &path.subpaths {
        out.extend((0..sp.anchors.len()).filter_map(|i| Corner::of(sp, i, first + i)));
        first += sp.anchors.len();
    }
    out
}

/// The corner at anchor `index` among `corners` (in anchor order).
fn find(corners: &[Corner], index: usize) -> Option<&Corner> {
    corners.binary_search_by_key(&index, |c| c.index).ok().and_then(|i| corners.get(i))
}

/// How far along each side the cut of the corner at anchor `index` asks to reach, with the radius
/// in `radii` there (0 for an anchor that is no corner, or a radius that is negative or not a
/// number).
fn requested_setback(corners: &[Corner], radii: &[f64], index: usize) -> f64 {
    find(corners, index).map_or(0.0, |c| c.setback(radii.get(index).copied().unwrap_or(0.0).max(0.0)))
}

/// The radius each of `corners` (in anchor order) is drawn with, given the radii by anchor index:
/// its own, no larger than leaves room for the cuts of its neighbours ([`Corner::max_radius_beside`]
/// with the setbacks their radii ask for), so a corner whose neighbours are square rounds until
/// its cut reaches them (#2332) and two neighbours never cut past each other.
pub fn fitted_radii(corners: &[Corner], radii: &[f64]) -> Vec<f64> {
    corners
        .iter()
        .map(|c| {
            let own = radii.get(c.index).copied().unwrap_or(0.0).max(0.0);
            let max = c.max_radius_beside(requested_setback(corners, radii, c.before), requested_setback(corners, radii, c.after));
            own.min(max)
        })
        .collect()
}

/// `path` with each of its corners ([`path_corners`]) cut by the radius in `radii` and the kind in
/// `kinds` at its anchor index (missing: 0 and round), each radius no larger than fits
/// ([`fitted_radii`]). A cut corner becomes two anchors, and a curved side between corners is
/// trimmed to what is left of it; a closed subpath whose first corner is cut starts where that
/// cut ends, so the cut closes it. Also returns, per subpath, the anchor index in `path` each
/// anchor comes from.
pub fn cut_corners(path: &PathData, radii: &[f64], kinds: &[CornerKind]) -> (PathData, Vec<Vec<usize>>) {
    let corners = path_corners(path);
    let fitted = fitted_radii(&corners, radii);
    let mut corners = corners.iter().zip(fitted).peekable();
    let (mut subpaths, mut sources) = (Vec::with_capacity(path.subpaths.len()), Vec::with_capacity(path.subpaths.len()));
    let mut first = 0;
    for sp in &path.subpaths {
        let n = sp.anchors.len();
        // The cut of each anchor, if any.
        let cuts: Vec<Option<Cut>> = (0..n)
            .map(|i| {
                let index = first + i;
                let (c, radius) = corners.next_if(|(c, _)| c.index == index)?;
                (radius > EPS).then(|| c.cut(radius, kinds.get(index).copied().unwrap_or_default()))
            })
            .collect();
        // Each segment from anchor j, trimmed to the part the cuts at its ends leave: its handles
        // (out of its first anchor, into its last), `None` when it is untouched.
        let trimmed: Vec<Option<(Point, Point)>> = (0..n)
            .map(|j| {
                let k = (j + 1) % n;
                let (a, b) = (sp.anchors.get(j)?, sp.anchors.get(k)?);
                let (from, to) = (cuts.get(j).copied().flatten(), cuts.get(k).copied().flatten());
                if (from.is_none() && to.is_none()) || (k == 0 && !sp.closed) {
                    return None;
                }
                // A straight segment stays straight, its handles on its ends.
                if !a.has_out() && !b.has_in() {
                    return Some((from.map_or(a.p, |c| c.end), to.map_or(b.p, |c| c.start)));
                }
                let (s0, s1) = (from.map_or(0.0, |c| c.tv), to.map_or(1.0, |c| 1.0 - c.tu));
                let sub = CubicBez::new(a.p, a.h_out, b.h_in, b.p).subsegment(s0..s1.max(s0));
                Some((sub.p1, sub.p2))
            })
            .collect();
        let (mut anchors, mut from) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut closing = None;
        for (i, a) in sp.anchors.iter().enumerate() {
            let index = first + i;
            let h_in = trimmed.get((i + n - 1) % n).copied().flatten().map_or(a.h_in, |(_, h)| h);
            let h_out = trimmed.get(i).copied().flatten().map_or(a.h_out, |(h, _)| h);
            match cuts.get(i).copied().flatten() {
                Some(c) => {
                    let start = Anchor { p: c.start, h_in, h_out: c.h_out, kind: AnchorKind::Corner };
                    let end = Anchor { p: c.end, h_in: c.h_in, h_out, kind: AnchorKind::Corner };
                    if i == 0 && sp.closed {
                        closing = Some(start);
                    } else {
                        anchors.push(start);
                        from.push(index);
                    }
                    anchors.push(end);
                    from.push(index);
                }
                None => {
                    anchors.push(Anchor { h_in, h_out, ..*a });
                    from.push(index);
                }
            }
        }
        if let Some(a) = closing {
            anchors.push(a);
            from.push(first);
        }
        first += n;
        subpaths.push(SubPath::new(anchors, sp.closed));
        sources.push(from);
    }
    (PathData::new(subpaths), sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;
    use kurbo::Rect;

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    /// A five-pointed star's tips are acute corners, its inner corners obtuse: all ten cut, each
    /// a circle of the radius tangent to both sides.
    #[test]
    fn a_star_has_ten_corners_cut_as_circles() {
        let star = shapes::star(Point::new(100.0, 100.0), 50.0, 25.0, 5, 0.0);
        let corners = path_corners(&star);
        assert_eq!(corners.len(), 10);
        let angles: Vec<_> = corners.iter().map(|c| c.angle_deg()).collect();
        assert!(angles.iter().step_by(2).all(|a| *a < 90.0), "tips are acute: {angles:?}");
        assert!(angles.iter().skip(1).step_by(2).all(|a| *a > 90.0), "inner corners are obtuse: {angles:?}");
        let (cut, from) = cut_corners(&star, &[4.0; 10], &[]);
        assert_eq!(cut.anchor_count(), 20);
        assert_eq!(from, [vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 0]]);
        for c in &corners {
            // Both ends of the cut lie `setback` along the sides, as far from the arc's centre as
            // the radius, and that centre is `radius` from either side.
            let t = c.setback(4.0);
            let o = c.on_bisector(4.0 / c.sin());
            for p in [c.at + c.u * t, c.at + c.v * t] {
                assert!((p.distance(o) - 4.0).abs() < 1e-9);
                assert!(cut.anchors().any(|(_, _, a)| near(a.p, p)), "{p:?} is an anchor");
            }
        }
    }

    #[test]
    fn radii_stop_at_half_the_shorter_side_per_corner() {
        let tri = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 40.0)], true));
        let corners = path_corners(&tri);
        assert_eq!(corners.len(), 3);
        for c in &corners {
            let max = c.max_radius();
            assert!((c.setback(max) - c.lu.min(c.lv) / 2.0).abs() < 1e-9);
        }
        // The right angle at (0, 0) takes up to 20, half its shorter side.
        assert!((corners[0].max_radius() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn ends_curves_smooth_and_straight_anchors_are_no_corners() {
        // An open zig-zag: its ends aren't corners, nor is the anchor its sides run straight on through.
        let open = PathData::single(SubPath::polyline(
            &[Point::new(0.0, 0.0), Point::new(50.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 50.0), Point::new(150.0, 80.0)],
            false,
        ));
        assert_eq!(path_corners(&open).iter().map(|c| c.index).collect::<Vec<_>>(), [2, 3]);
        // Smooth anchors aren't corners.
        assert!(path_corners(&shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0))).is_empty());
        // A second subpath counts on from the first one's anchors.
        let two = PathData::new(vec![
            SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true),
            SubPath::polyline(&[Point::new(50.0, 0.0), Point::new(60.0, 0.0), Point::new(50.0, 10.0)], true),
        ]);
        let (cut, from) = cut_corners(&two, &[0.0, 0.0, 0.0, 1.0], &[]);
        assert_eq!(from, [vec![0, 1, 2], vec![3, 4, 5, 3]]);
        assert_eq!(cut.anchor_count(), 7);
    }

    /// The square `0..100` with its right side bulging out as a curve (#1144).
    fn bulging_square() -> PathData {
        let mut sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)], true);
        sp.anchors[1].h_out = Point::new(140.0, 30.0);
        sp.anchors[2].h_in = Point::new(140.0, 70.0);
        PathData::single(sp)
    }

    /// Corners where a curve meets a line are cut too: the cut ends on the path, leaves both
    /// sides smoothly, and what is left of the curve still follows it (#1144).
    #[test]
    fn corners_next_to_curves_are_cut_along_them() {
        let path = bulging_square();
        let corners = path_corners(&path);
        assert_eq!(corners.iter().map(|c| c.index).collect::<Vec<_>>(), [0, 1, 2, 3]);
        let side = kurbo::CubicBez::new(Point::new(100.0, 0.0), Point::new(140.0, 30.0), Point::new(140.0, 70.0), Point::new(100.0, 100.0));
        // The curved side is longer than its chord, and limits its corners by its length.
        assert!(corners[1].lv > 100.0 + 1.0);
        let (cut, from) = cut_corners(&path, &[10.0; 4], &[]);
        assert_eq!(from, [vec![0, 1, 1, 2, 2, 3, 3, 0]]);
        let sp = &cut.subpaths[0];
        let on_side = |p: Point| (0..=1000).map(|i| side.eval(f64::from(i) / 1000.0).distance(p)).fold(f64::MAX, f64::min);
        // Corner 1's cut ends on the curve and corner 2's starts on it, and the trimmed curve
        // between them stays on the original one.
        let (a, b) = (sp.anchors[2], sp.anchors[3]);
        assert!(on_side(a.p) < 0.1 && on_side(b.p) < 0.1);
        let trimmed = kurbo::CubicBez::new(a.p, a.h_out, b.h_in, b.p);
        for t in [0.25, 0.5, 0.75] {
            assert!(on_side(trimmed.eval(t)) < 0.1);
        }
        // Every cut leaves its sides smoothly: each anchor's handles are in line.
        for (i, x) in sp.anchors.iter().enumerate() {
            let (into, out) = if x.has_in() && x.has_out() {
                (x.p - x.h_in, x.h_out - x.p)
            } else if x.has_out() {
                (x.p - sp.anchors[(i + sp.anchors.len() - 1) % sp.anchors.len()].p, x.h_out - x.p)
            } else {
                (x.h_in - x.p, x.p - sp.anchors[(i + 1) % sp.anchors.len()].p)
            };
            assert!(into.normalize().cross(out.normalize()).abs() < 1e-6 && into.dot(out) > 0.0, "anchor {i}");
        }
    }

    /// A round cut next to a curve stays inside its corner, however large (#1512): its handles
    /// stop short of where the tangents at its ends cross, so it doesn't stick out past the sides.
    #[test]
    fn a_round_cut_next_to_a_curve_stays_inside_its_corner() {
        // A square whose top side arches up, meeting the upright sides at an angle.
        let mut sp =
            SubPath::polyline(&[Point::new(100.0, 150.0), Point::new(300.0, 150.0), Point::new(300.0, 300.0), Point::new(100.0, 300.0)], true);
        sp.anchors[0].h_out = Point::new(120.0, 20.0);
        sp.anchors[1].h_in = Point::new(280.0, 20.0);
        let path = PathData::single(sp);
        let bounds = path.bounds().unwrap().inflate(1e-6, 1e-6);
        for radius in [10.0, 60.0, 200.0, 1e6] {
            let (cut, _) = cut_corners(&path, &[radius, radius], &[]);
            let b = cut.bounds().unwrap();
            assert!(bounds.union(b) == bounds, "radius {radius}: {b:?} is outside {bounds:?}");
            // Each cut's handles end before its tangents cross: the cut turns one way only.
            let a = &cut.subpaths[0].anchors;
            for (from, to) in [(a[a.len() - 1], a[0]), (a[1], a[2])] {
                let (t0, t1, chord) = (from.h_out - from.p, to.p - to.h_in, to.p - from.p);
                assert!(t0.cross(chord) * chord.cross(t1) > 0.0 && t0.cross(t1) * t0.cross(chord) > 0.0, "radius {radius}: {from:?} {to:?}");
            }
        }
    }

    /// Inverted round and chamfer cuts end where the round one does, at any angle.
    #[test]
    fn kinds_share_their_ends() {
        let star = shapes::star(Point::new(0.0, 0.0), 50.0, 25.0, 5, 0.0);
        let ends = |k: CornerKind| -> Vec<Point> { cut_corners(&star, &[3.0; 10], &[k; 10]).0.anchors().map(|(_, _, a)| a.p).collect() };
        let round = ends(CornerKind::Round);
        assert_eq!(ends(CornerKind::InvertedRound), round);
        assert_eq!(ends(CornerKind::Chamfer), round);
        // The inverted arc stays on the circle about the corner.
        let c = path_corners(&star)[0];
        let cut = c.cut(3.0, CornerKind::InvertedRound);
        let bez = kurbo::CubicBez::new(cut.start, cut.h_out, cut.h_in, cut.end);
        let r = c.setback(3.0);
        for t in [0.25, 0.5, 0.75] {
            use kurbo::ParamCurve;
            assert!((bez.eval(t).distance(c.at) - r).abs() < r * 1e-3);
        }
    }

    /// #2332: a corner rounds until its cut reaches its neighbours' (all of a side whose other
    /// end is square); neighbours that both ask for more than half a side meet halfway.
    #[test]
    fn a_corner_rounds_up_to_its_neighbours_cuts() {
        let square = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let corners = path_corners(&square);
        let drawn = |radii: [f64; 4]| fitted_radii(&corners, &radii);
        assert_eq!(drawn([0.0, 500.0, 0.0, 0.0]), [0.0, 100.0, 0.0, 0.0]);
        assert_eq!(drawn([30.0, 500.0, 0.0, 0.0]), [30.0, 70.0, 0.0, 0.0]);
        assert_eq!(drawn([80.0, 500.0, 0.0, 0.0]), [50.0, 50.0, 0.0, 0.0]);
        assert_eq!(drawn([500.0; 4]), [50.0; 4]);
        assert_eq!(drawn([f64::NAN, 120.0, -1.0, 0.0]), [0.0, 100.0, 0.0, 0.0]);
        // The lone cut is a quarter circle from one neighbour to the other.
        let (cut, _) = cut_corners(&square, &[0.0, 500.0, 0.0, 0.0], &[]);
        let ends: Vec<Point> = cut.anchors().map(|(_, _, a)| a.p).collect();
        assert_eq!(ends, [Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(100.0, 100.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)]);
        // Dragged together with its neighbour, a corner meets it halfway; beside a square one,
        // it goes all the way.
        let tr = corners[1];
        assert!((tr.max_shared_radius(&corners, &[0.0; 4], |k| k == 0) - 50.0).abs() < 1e-9);
        assert!((tr.max_shared_radius(&corners, &[0.0; 4], |_| false) - 100.0).abs() < 1e-9);
        assert!((tr.max_shared_radius(&corners, &[0.0, 0.0, 40.0, 0.0], |_| false) - 60.0).abs() < 1e-9);
    }
}
