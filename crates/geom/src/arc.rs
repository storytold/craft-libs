//! Arc-length parametrization of a path: the point and direction at a distance along it, and the
//! distance along it of the point nearest another (type on a path lays its glyphs out and the
//! selection tools drag its brackets by distance along the path).

use kurbo::{Affine, ParamCurve, ParamCurveArclen, ParamCurveExtrema, ParamCurveNearest, PathSeg, Point, Vec2};

use crate::PathData;

const ACCURACY: f64 = 1e-4;

/// A path's segments with their lengths, measured once.
#[derive(Clone, Debug, Default)]
pub struct ArcPath {
    /// Segments of some length, each with the distance to its start and its length.
    segs: Vec<(PathSeg, f64, f64)>,
    len: f64,
    closed: bool,
}

impl ArcPath {
    pub fn new(path: &PathData) -> Self {
        let mut segs = Vec::new();
        let mut cum = 0.0;
        for s in path.to_bezpath().segments() {
            let l = crate::path::measured_arclen(&s, ACCURACY);
            if l.is_finite() && l > 1e-9 {
                segs.push((s, cum, l));
                cum += l;
            }
        }
        Self { segs, len: cum, closed: path.is_closed() }
    }

    /// The path's length.
    pub fn len(&self) -> f64 {
        self.len
    }

    /// No segment has any length.
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Point and unit tangent at distance `s` along the path: round and round a closed path, held
    /// at an open one's ends.
    pub fn at(&self, s: f64) -> (Point, Vec2) {
        let s = if self.closed && self.len > 0.0 { s.rem_euclid(self.len) } else { s.clamp(0.0, self.len) };
        let i = self.segs.partition_point(|(_, c, _)| *c <= s).saturating_sub(1);
        let Some(&(seg, c, l)) = self.segs.get(i) else { return (Point::ZERO, Vec2::new(1.0, 0.0)) };
        // (The solver's precision is relative to the segment's length: a long one needs a looser one.)
        let t = seg.inv_arclen((s - c).min(l), ACCURACY.max(l * 1e-9)).clamp(0.0, 1.0);
        let p = seg.eval(t);
        let (t0, t1) = ((t - 1e-4).max(0.0), (t + 1e-4).min(1.0));
        let d = seg.eval(t1) - seg.eval(t0);
        let len = d.hypot();
        (p, if len > 1e-12 { d / len } else { Vec2::new(1.0, 0.0) })
    }

    /// The distance along the path of its point nearest `p`.
    pub fn nearest(&self, p: Point) -> Option<f64> {
        self.segs
            .iter()
            .map(|(seg, c, l)| {
                let n = seg.nearest(p, ACCURACY);
                (n.distance_sq, c + seg.subsegment(0.0..n.t).arclen(ACCURACY).min(*l))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s)
    }

    /// [`Self::nearest`] as a fraction (0..1) of the path's length.
    pub fn fraction_at(&self, p: Point) -> Option<f64> {
        (self.len > 0.0).then(|| self.nearest(p)).flatten().map(|s| (s / self.len).clamp(0.0, 1.0))
    }

    /// The distance along the path of the middle of its topmost points (the least y) where `xf`
    /// draws it: the topmost point when there is one, the middle of a level stretch (a
    /// rectangle's top edge), and when several points or stretches are topmost, the middle of the
    /// shortest part of the path that holds them all. None for a path with no length, or a closed
    /// path whose every point is topmost.
    pub fn topmost(&self, xf: Affine) -> Option<f64> {
        let drawn: Vec<(PathSeg, PathSeg, f64, f64)> = self.segs.iter().map(|&(s, c, l)| (s, xf * s, c, l)).collect();
        // The candidates for the least y in each segment: its ends and where it turns.
        let turns = |d: &PathSeg| [0.0, 1.0].into_iter().chain(d.extrema()).collect::<Vec<f64>>();
        let mut bounds: Option<kurbo::Rect> = None;
        let mut top = f64::INFINITY;
        for (_, d, _, _) in &drawn {
            let b = d.bounding_box();
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
            top = turns(d).into_iter().map(|t| d.eval(t).y).fold(top, f64::min);
        }
        let b = bounds?;
        if !(top.is_finite() && b.width().is_finite() && b.height().is_finite()) {
            return None;
        }
        // Points this close to the top count as topmost: a part of the drawn size, and no less than
        // the rounding error of coordinates this far from the origin.
        let tol = (1e-9 * (b.width() + b.height())).max(64.0 * f64::EPSILON * b.y0.abs().max(b.y1.abs()));
        // Stretches this close along the path meet.
        let near = 1e-9 * self.len;
        // The topmost points as stretches of distance along the path, in order.
        let mut runs: Vec<(f64, f64)> = vec![];
        for (s, d, c, l) in &drawn {
            let db = d.bounding_box();
            if db.y1 - db.y0 <= tol && db.y0 <= top + tol {
                runs.push((*c, c + l));
                continue;
            }
            for t in turns(d) {
                if d.eval(t).y <= top + tol {
                    let at = c + s.subsegment(0.0..t).arclen(ACCURACY).min(*l);
                    runs.push((at, at));
                }
            }
        }
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Stretches that meet or overlap are one.
        let mut merged: Vec<(f64, f64)> = vec![];
        for (a, z) in runs {
            match merged.last_mut() {
                Some(last) if a <= last.1 + near => last.1 = last.1.max(z),
                _ => merged.push((a, z)),
            }
        }
        let (first, last) = (*merged.first()?, *merged.last()?);
        if !self.closed {
            return Some((first.0 + last.1) * 0.5);
        }
        let len = self.len;
        // Round a closed path, the shortest part holding every stretch leaves out the longest
        // gap between two of them (the gap from the last round to the first included).
        let wrap = first.0 + len - last.1;
        let (mut gap, mut from, mut to) = (wrap, first.0, last.1);
        for w in merged.windows(2) {
            if let [a, b] = w
                && b.0 - a.1 > gap
            {
                (gap, from, to) = (b.0 - a.1, b.0, a.1);
            }
        }
        if gap <= near {
            return None;
        }
        let run = (to - from).rem_euclid(len);
        Some((from + run * 0.5).rem_euclid(len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;
    use kurbo::{Rect, Shape};

    #[test]
    fn points_and_nearest_distances_along_a_path() {
        let line = PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (100.0, 0.0)).to_path(0.1));
        let a = ArcPath::new(&line);
        assert!((a.len() - 100.0).abs() < 1e-6 && !a.is_closed());
        assert!(a.at(25.0).0.distance(Point::new(25.0, 0.0)) < 1e-6);
        // Held at an open path's ends.
        assert!(a.at(150.0).0.distance(Point::new(100.0, 0.0)) < 1e-6);
        assert!((a.nearest(Point::new(40.0, 30.0)).unwrap() - 40.0).abs() < 1e-6);
        // Round a closed one.
        let sq = ArcPath::new(&shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(sq.is_closed() && (sq.len() - 40.0).abs() < 1e-6);
        assert!(sq.at(45.0).0.distance(sq.at(5.0).0) < 1e-6);
        assert!(ArcPath::new(&PathData::default()).is_empty());
    }

    fn polyline(points: &[Point], closed: bool) -> PathData {
        PathData::single(crate::SubPath::polyline(points, closed))
    }

    /// Where `topmost` puts the middle, drawn by `xf`.
    fn top_point(a: &ArcPath, xf: Affine) -> Point {
        xf * a.at(a.topmost(xf).unwrap()).0
    }

    #[test]
    fn topmost_points_as_drawn() {
        // A circle starts at 9 o'clock and runs clockwise (y down): its top is a quarter of the way round, its
        // bottom three quarters.
        let circle = ArcPath::new(&shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert!((circle.topmost(Affine::IDENTITY).unwrap() - circle.len() * 0.25).abs() < 1e-6);
        assert!((circle.topmost(Affine::FLIP_Y).unwrap() - circle.len() * 0.75).abs() < 1e-6);
        // As the transform draws it: turned a quarter turn the other way, 3 o'clock is at the top.
        let turned = Affine::rotate_about(-std::f64::consts::FRAC_PI_2, Point::new(50.0, 50.0));
        assert!((circle.topmost(turned).unwrap() - circle.len() * 0.5).abs() < 1e-6);
        // Turned part way, the top lies inside a segment.
        let tilted = Affine::rotate_about(0.4, Point::new(50.0, 50.0));
        assert!(top_point(&circle, tilted).distance(Point::new(50.0, 0.0)) < 0.05, "{:?}", top_point(&circle, tilted));
        // An ellipse squashed and skewed by its transform.
        let skewed = Affine::skew(0.5, 0.0) * Affine::scale_non_uniform(2.0, 0.5);
        let p = top_point(&circle, skewed);
        let lowest = (0..3600).map(|i| (skewed * circle.at(circle.len() * i as f64 / 3600.0).0).y).fold(f64::INFINITY, f64::min);
        assert!((p.y - lowest).abs() < 1e-3, "{p:?} {lowest}");
        // A level top: the middle of the top edge, for a rectangle and a rounded rectangle.
        for path in [shapes::rectangle(Rect::new(10.0, 20.0, 110.0, 70.0)), shapes::rounded_rectangle(Rect::new(10.0, 20.0, 110.0, 70.0), 8.0)] {
            let a = ArcPath::new(&path);
            assert!(top_point(&a, Affine::IDENTITY).distance(Point::new(60.0, 20.0)) < 1e-6, "{:?}", top_point(&a, Affine::IDENTITY));
            assert!(top_point(&a, Affine::FLIP_Y).distance(Point::new(60.0, -70.0)) < 1e-6);
        }
        // Far from the origin the top is still the top edge's middle.
        let square = ArcPath::new(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)));
        let far = Affine::translate((1e12, 1e12));
        assert!(top_point(&square, far).distance(Point::new(1e12 + 50.0, 1e12)) < 1e-3, "{:?}", top_point(&square, far));
        // A level top across the path's start: the start in the middle of the top edge.
        let across = ArcPath::new(&polyline(
            &[Point::new(50.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 50.0), Point::new(0.0, 50.0), Point::new(0.0, 0.0)],
            true,
        ));
        assert!(top_point(&across, Affine::IDENTITY).distance(Point::new(50.0, 0.0)) < 1e-6);
        // Two peaks: the middle of the shortest part that holds both, the dip between them.
        let peaks = ArcPath::new(&polyline(
            &[Point::new(0.0, 10.0), Point::new(2.0, 0.0), Point::new(4.0, 5.0), Point::new(6.0, 0.0), Point::new(8.0, 10.0)],
            true,
        ));
        assert!(top_point(&peaks, Affine::IDENTITY).distance(Point::new(4.0, 5.0)) < 1e-6, "{:?}", top_point(&peaks, Affine::IDENTITY));
        // An open path: the middle of its topmost points too.
        let open = ArcPath::new(&polyline(&[Point::new(0.0, 10.0), Point::new(5.0, 0.0), Point::new(10.0, 10.0)], false));
        assert!(top_point(&open, Affine::IDENTITY).distance(Point::new(5.0, 0.0)) < 1e-6);
        // No top: a closed path that is level all round, and a path with no length.
        let flat = ArcPath::new(&shapes::ellipse(Rect::new(0.0, 10.0, 100.0, 10.0)));
        assert!(!flat.is_empty() && flat.topmost(Affine::IDENTITY).is_none());
        assert!(ArcPath::new(&PathData::default()).topmost(Affine::IDENTITY).is_none());
        let point = ArcPath::new(&polyline(&[Point::new(5.0, 5.0), Point::new(5.0, 5.0)], true));
        assert!(point.topmost(Affine::IDENTITY).is_none());
    }
}
