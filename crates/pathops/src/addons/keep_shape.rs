//! Shape-keeping anchor removal: an anchor goes, and the handles of its two neighbours that face it
//! are refitted (their directions kept, their lengths fitted) so one segment follows the two that
//! met there. [`remove_anchor_keep_shape`] removes one anchor and says how far the outline moved;
//! [`simplify_keep_shape`] removes as many as it can while every new segment stays within a
//! tolerance of the original outline.

use craft_geom::{Anchor, PathData, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveNearest};

use crate::edit::remove_anchor;
use crate::fit::{end_tangent, start_tangent};

/// The turn (degrees) at an anchor above which `protect_corners` keeps it.
pub const SHARP_CORNER_DEG: f64 = 30.0;

/// The drift (pt) each step of the tolerance the add-ons offer allows.
pub const PT_PER_TOLERANCE: f64 = 0.02;

/// Points sampled along each curve to measure how far two outlines are apart.
const SAMPLES: usize = 8;

/// Nearest-point accuracy (pt) of those measurements.
const ACCURACY: f64 = 1e-6;

/// The segment that takes the place of the two meeting at `cur` once it is removed: from `prev`
/// to `next`, their facing handles refitted ([`remove_anchor`]).
fn merged(prev: &Anchor, cur: &Anchor, next: &Anchor) -> CubicBez {
    let mut run = SubPath::new(vec![*prev, *cur, *next], false);
    remove_anchor(&mut run, 1);
    match &*run.anchors.contiguous() {
        [a, b] => CubicBez::new(prev.p, a.h_out, b.h_in, next.p),
        _ => CubicBez::new(prev.p, prev.h_out, next.h_in, next.p),
    }
}

/// How far `fitted` strays from the run of segments `original` it replaces, measured both ways
/// (pt): the farthest a sampled point of either lies from the other.
fn deviation(original: &[&CubicBez], fitted: &CubicBez) -> f64 {
    let t = |k: usize| k as f64 / SAMPLES as f64;
    let mut worst: f64 = 0.0;
    for c in original {
        for k in 0..=SAMPLES {
            worst = worst.max(fitted.nearest(c.eval(t(k)), ACCURACY).distance_sq);
        }
    }
    for k in 1..SAMPLES {
        let p = fitted.eval(t(k));
        worst = worst.max(original.iter().map(|c| c.nearest(p, ACCURACY).distance_sq).fold(f64::INFINITY, f64::min));
    }
    worst.sqrt()
}

/// The fewest anchors a subpath keeps: two when open, three when closed.
fn min_anchors(sp: &SubPath) -> usize {
    if sp.closed { 3 } else { 2 }
}

/// Indices of anchor `i`'s neighbours in a subpath of `n` anchors (`None` for an open end).
fn neighbours(n: usize, closed: bool, i: usize) -> Option<(usize, usize)> {
    match closed {
        true => (n > 0).then(|| ((i + n - 1) % n, (i + 1) % n)),
        false => (i > 0 && i + 1 < n).then(|| (i - 1, i + 1)),
    }
}

fn finite(sp: &SubPath) -> bool {
    sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
}

/// Does the path turn by more than [`SHARP_CORNER_DEG`] at anchor `i` (an inner anchor)?
fn is_sharp(sp: &SubPath, i: usize) -> bool {
    let Some((prev, _)) = neighbours(sp.anchors.len(), sp.closed, i) else { return false };
    let (into, out) = (end_tangent(&sp.segment(prev)), start_tangent(&sp.segment(i)));
    into.dot(out).clamp(-1.0, 1.0).acos().to_degrees() > SHARP_CORNER_DEG
}

/// Give anchors `prev` and `next` the facing handles of segment `c`.
fn apply(sp: &mut SubPath, prev: usize, next: usize, c: &CubicBez) {
    if let Some(a) = sp.anchors.get_mut(prev) {
        a.h_out = c.p1;
    }
    if let Some(a) = sp.anchors.get_mut(next) {
        a.h_in = c.p2;
    }
}

/// Remove anchor `index` of `sp`, refitting the handles of its neighbours that face it so one
/// segment follows the two that met there → (the new subpath, how far its outline strays from the
/// old one, in points). `None` when the anchor can't go: out of range, an end of an open subpath,
/// a non-finite coordinate, or the subpath would keep fewer than two (open) or three (closed)
/// anchors.
pub fn remove_anchor_keep_shape(sp: &SubPath, index: usize) -> Option<(SubPath, f64)> {
    let n = sp.anchors.len();
    if n <= min_anchors(sp) || index >= n || !finite(sp) {
        return None;
    }
    let (prev, next) = neighbours(n, sp.closed, index)?;
    let c = merged(sp.anchors.get(prev)?, sp.anchors.get(index)?, sp.anchors.get(next)?);
    let error = deviation(&[&sp.segment(prev), &sp.segment(index)], &c);
    let mut out = sp.clone();
    apply(&mut out, prev, next, &c);
    out.anchors.remove(index);
    Some((out, error))
}

/// Remove as many anchors of `path` as can go while its outline stays within `tolerance` points of
/// where it was, refitting the handles beside each ([`remove_anchor_keep_shape`]). Every new segment
/// is measured against the original outline it replaces, so removals don't add up past the
/// tolerance. With `protect_corners`, anchors where the path turns by more than
/// [`SHARP_CORNER_DEG`] stay. Open ends always stay. A subpath with a non-finite coordinate, and a
/// tolerance that isn't a positive number, are left as they are.
pub fn simplify_keep_shape(path: &PathData, tolerance: f64, protect_corners: bool) -> PathData {
    if !(tolerance > 0.0 && tolerance.is_finite()) {
        return path.clone();
    }
    PathData::new(path.subpaths.iter().map(|sp| Reducer::new(sp, protect_corners).run(tolerance)).collect())
}

/// [`simplify_keep_shape`] on subpath `sp` where only the anchors `may_go` marks (one flag per
/// anchor, by index) may go: what the Anchor Eraser Brush passes over. Open ends always stay. An
/// infinite tolerance removes every marked anchor that can go.
pub fn simplify_anchors_keep_shape(sp: &SubPath, tolerance: f64, may_go: &[bool]) -> SubPath {
    if tolerance.is_nan() || tolerance <= 0.0 {
        return sp.clone();
    }
    let mut r = Reducer::new(sp, false);
    r.kept = (0..sp.anchors.len()).map(|i| !may_go.get(i).copied().unwrap_or(false)).collect();
    r.run(tolerance)
}

/// One subpath being reduced: what is left of it, and where its anchors were in the original.
struct Reducer<'a> {
    original: &'a SubPath,
    segments: Vec<CubicBez>,
    /// Anchors of the original that stay whatever the tolerance (sharp corners).
    kept: Vec<bool>,
    cur: SubPath,
    /// The original index of each anchor left.
    origin: Vec<usize>,
}

impl<'a> Reducer<'a> {
    fn new(sp: &'a SubPath, protect_corners: bool) -> Self {
        let n = sp.anchors.len();
        Reducer {
            original: sp,
            segments: (0..sp.segment_count()).map(|i| sp.segment(i)).collect(),
            kept: (0..n).map(|i| protect_corners && is_sharp(sp, i)).collect(),
            cur: sp.clone(),
            origin: (0..n).collect(),
        }
    }

    /// Each pass removes the anchors whose removal strays least, no two side by side (so each
    /// segment measured is the one that results), until none can go.
    fn run(mut self, tolerance: f64) -> SubPath {
        if self.cur.anchors.len() <= min_anchors(self.original) || !finite(self.original) {
            return self.cur;
        }
        while self.pass(tolerance) {}
        self.cur
    }

    /// The original segments between original anchors `from` and `to`, round the end of a closed
    /// path.
    fn span(&self, from: usize, to: usize) -> Option<Vec<&CubicBez>> {
        Some(match from < to {
            true => self.segments.get(from..to)?.iter().collect(),
            false => self.segments.get(from..)?.iter().chain(self.segments.get(..to)?).collect(),
        })
    }

    /// Anchor `i` of what is left, if it may go: (how far its removal strays, the segment that
    /// replaces its two).
    fn candidate(&self, i: usize, tolerance: f64) -> Option<(f64, CubicBez)> {
        if self.origin.get(i).and_then(|&o| self.kept.get(o)) != Some(&false) {
            return None;
        }
        let (prev, next) = neighbours(self.cur.anchors.len(), self.cur.closed, i)?;
        let a = &self.cur.anchors;
        let c = merged(a.get(prev)?, a.get(i)?, a.get(next)?);
        let e = deviation(&self.span(*self.origin.get(prev)?, *self.origin.get(next)?)?, &c);
        (e <= tolerance).then_some((e, c))
    }

    /// One pass: whether any anchor went.
    fn pass(&mut self, tolerance: f64) -> bool {
        let m = self.cur.anchors.len();
        let min = min_anchors(self.original);
        if m <= min {
            return false;
        }
        let mut found: Vec<(f64, usize, CubicBez)> = (0..m).filter_map(|i| self.candidate(i, tolerance).map(|(e, c)| (e, i, c))).collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut gone = vec![false; m];
        let mut left = m - min;
        for (_, i, c) in found {
            let Some((prev, next)) = neighbours(m, self.cur.closed, i) else { continue };
            if left == 0 || gone.get(prev) != Some(&false) || gone.get(next) != Some(&false) {
                continue;
            }
            apply(&mut self.cur, prev, next, &c);
            if let Some(g) = gone.get_mut(i) {
                *g = true;
            }
            left -= 1;
        }
        let mut keep = gone.iter().map(|g| !g);
        self.cur.anchors.retain(|_| keep.next().unwrap_or(true));
        let mut keep = gone.iter().map(|g| !g);
        self.origin.retain(|_| keep.next().unwrap_or(true));
        left < m - min
    }
}

#[cfg(test)]
mod tests {
    use craft_geom::{Anchor, PathData, SubPath};
    use kurbo::{ParamCurveNearest, Point, Vec2};

    use super::{remove_anchor_keep_shape, simplify_keep_shape};

    /// A circle of radius `r` about the origin drawn with `n` smooth anchors.
    fn circle(r: f64, n: usize) -> SubPath {
        let step = std::f64::consts::TAU / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
        let anchors = (0..n)
            .map(|i| {
                let a = step * i as f64;
                let p = Point::new(r * a.cos(), r * a.sin());
                Anchor::smooth(p, p + Vec2::new(-a.sin(), a.cos()) * k)
            })
            .collect();
        SubPath::new(anchors, true)
    }

    /// The farthest a point of `sp`'s outline lies from the circle of radius `r`.
    fn off_circle(sp: &SubPath, r: f64) -> f64 {
        (0..sp.segment_count())
            .flat_map(|i| (0..=20).map(move |k| (i, k as f64 / 20.0)))
            .map(|(i, t)| (kurbo::ParamCurve::eval(&sp.segment(i), t).to_vec2().hypot() - r).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_circle_keeps_its_shape_with_fewer_anchors() {
        let sp = circle(50.0, 16);
        let out = simplify_keep_shape(&PathData::single(sp), 0.2, false);
        let s = &out.subpaths[0];
        assert!(s.anchors.len() <= 8 && s.anchors.len() >= 3, "{} anchors left", s.anchors.len());
        assert!(s.closed);
        assert!(off_circle(s, 50.0) <= 0.2 + 1e-6, "strays {}", off_circle(s, 50.0));
    }

    #[test]
    fn one_removal_reports_how_far_the_outline_moved() {
        let sp = circle(50.0, 16);
        let (out, err) = remove_anchor_keep_shape(&sp, 3).unwrap();
        assert_eq!(out.anchors.len(), 15);
        assert!(err < 0.05, "{err}");
        assert!((off_circle(&out, 50.0) - err).abs() < 0.05);
        // A corner of a square moves its outline a long way.
        let sq = SubPath::polyline(&[(0.0, 0.0).into(), (10.0, 0.0).into(), (10.0, 10.0).into(), (0.0, 10.0).into()], true);
        let (_, err) = remove_anchor_keep_shape(&sq, 1).unwrap();
        assert!((err - 50f64.sqrt()).abs() < 1e-3, "{err}");
    }

    #[test]
    fn sharp_corners_stay_when_protected() {
        // A 90° corner at (1, 0) that a 1 pt tolerance would otherwise take away.
        let sp = SubPath::polyline(&[(0.0, 0.0).into(), (1.0, 0.0).into(), (1.0, 1.0).into()], false);
        let path = PathData::single(sp);
        assert_eq!(simplify_keep_shape(&path, 1.0, false).anchor_count(), 2);
        assert_eq!(simplify_keep_shape(&path, 1.0, true).anchor_count(), 3);
        // Collinear anchors on a straight edge go either way; the square's corners stay.
        let sq = SubPath::polyline(
            &[(0.0, 0.0).into(), (5.0, 0.0).into(), (10.0, 0.0).into(), (10.0, 10.0).into(), (0.0, 10.0).into(), (0.0, 5.0).into()],
            true,
        );
        let out = simplify_keep_shape(&PathData::single(sq), 0.01, true);
        let pts: Vec<Point> = out.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts, [(0.0, 0.0).into(), (10.0, 0.0).into(), (10.0, 10.0).into(), (0.0, 10.0).into()] as [Point; 4]);
    }

    #[test]
    fn removals_do_not_add_up_past_the_tolerance() {
        // A gentle arc of many anchors: each removal alone is small, together they would flatten it.
        let pts: Vec<Point> = (0..=40).map(|i| Point::new(i as f64, (i as f64 / 40.0 * std::f64::consts::PI).sin() * 3.0)).collect();
        let sp = SubPath::polyline(&pts, false);
        let out = simplify_keep_shape(&PathData::single(sp.clone()), 0.1, false);
        let s = &out.subpaths[0];
        assert!(s.anchors.len() < 41);
        for p in &pts {
            let d = (0..s.segment_count()).map(|i| s.segment(i).nearest(*p, 1e-6).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
            assert!(d <= 0.1 + 1e-6, "{p:?} is {d} away");
        }
    }

    #[test]
    fn degenerate_input_is_left_alone() {
        let line = SubPath::polyline(&[(0.0, 0.0).into(), (1.0, 0.0).into()], false);
        assert!(remove_anchor_keep_shape(&line, 0).is_none(), "two anchors stay");
        let three = SubPath::polyline(&[(0.0, 0.0).into(), (1.0, 0.0).into(), (2.0, 0.0).into()], false);
        assert!(remove_anchor_keep_shape(&three, 0).is_none(), "an open end stays");
        assert!(remove_anchor_keep_shape(&three, 9).is_none(), "out of range");
        assert_eq!(remove_anchor_keep_shape(&three, 1).unwrap().0.anchors.len(), 2);
        let tri = SubPath::polyline(&[(0.0, 0.0).into(), (1.0, 0.0).into(), (0.0, 1.0).into()], true);
        assert!(remove_anchor_keep_shape(&tri, 1).is_none(), "a closed path keeps three");
        let nan = SubPath::polyline(&[(0.0, 0.0).into(), (f64::NAN, 0.0).into(), (2.0, 0.0).into()], false);
        assert!(remove_anchor_keep_shape(&nan, 1).is_none());
        for path in [PathData::default(), PathData::single(SubPath::default()), PathData::single(nan.clone()), PathData::single(three.clone())] {
            // Compared as text: NaN never equals itself.
            for tol in [f64::NAN, -1.0, 0.0, f64::INFINITY] {
                assert_eq!(format!("{:?}", simplify_keep_shape(&path, tol, false)), format!("{path:?}"));
            }
            let _ = simplify_keep_shape(&path, 1.0, true);
        }
        assert_eq!(format!("{:?}", simplify_keep_shape(&PathData::single(nan.clone()), 1.0, false)), format!("{:?}", PathData::single(nan)));
    }
}
