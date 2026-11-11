//! `Curve`: the value a [`CurveEditor`](super::CurveEditor) edits, and the point rules it edits by (shared by Curves,
//! the camera point curve and the contour editor).

/// A tone curve in levels (`0..=levels`, 255 for 8-bit tones): input across, output up.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Curve {
    /// Control points `[input, output]`, sorted by input, whole levels, the first and last the
    /// curve's end points. `corners[i]` marks point `i` as a corner point (the contour editor): a
    /// corner breaks the curve, and the points from one corner to the next (both included) take
    /// the host's interpolation (its sampler) as a run of their own. A short `corners` reads as smooth
    /// points; [`Curve::smooth`] and [`Curve::from_parts`] make the value.
    #[non_exhaustive]
    Points {
        /// Control points `[input, output]`.
        points: Vec<[f32; 2]>,
        /// Per point, whether it is a corner rather than smooth.
        corners: Vec<bool>,
    },
    /// A drawn curve (pencil): one output level per input level, drawn as straight segments.
    Map(Vec<f32>),
}

/// Curves are equal when every level is, a NaN equal to another: the host's points, and the
/// levels a drawn curve took from the host's sampler, may hold one, and the edit machinery
/// compares values to tell a change from none ([`Gesture`](craft_behavior::Gesture)).
impl PartialEq for Curve {
    fn eq(&self, other: &Self) -> bool {
        let same = |a: &f32, b: &f32| a == b || (a.is_nan() && b.is_nan());
        match (self, other) {
            (Curve::Points { points: a, .. }, Curve::Points { points: b, .. }) => {
                a.len() == b.len()
                    && a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| same(a, b))
                    && self.corner_flags() == other.corner_flags()
            }
            (Curve::Map(a), Curve::Map(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
            _ => false,
        }
    }
}

impl Curve {
    /// The unchanged curve: from (0, 0) to (`levels`, `levels`).
    pub fn identity(levels: f32) -> Curve {
        Curve::smooth(vec![[0.0, 0.0], [levels, levels]])
    }

    /// `points`, none of them a corner point.
    pub fn smooth(points: Vec<[f32; 2]>) -> Curve {
        Curve::from_parts(points, Vec::new())
    }

    /// `points` with `corners[i]` marking point `i` as a corner point. A short `corners` is
    /// completed with smooth points.
    pub fn from_parts(points: Vec<[f32; 2]>, mut corners: Vec<bool>) -> Curve {
        corners.resize(points.len(), false);
        Curve::Points { points, corners }
    }

    /// The control points, or `None` for a drawn curve.
    pub fn points(&self) -> Option<&[[f32; 2]]> {
        match self {
            Curve::Points { points, .. } => Some(points),
            Curve::Map(_) => None,
        }
    }

    /// Whether point `i` is a corner point.
    pub fn is_corner(&self, i: usize) -> bool {
        matches!(self, Curve::Points { corners, .. } if corners.get(i).copied().unwrap_or(false))
    }

    /// One flag per control point: whether it is a corner point.
    pub(crate) fn corner_flags(&self) -> Vec<bool> {
        match self {
            Curve::Points { points, corners } => (0..points.len()).map(|i| corners.get(i).copied().unwrap_or(false)).collect(),
            Curve::Map(_) => Vec::new(),
        }
    }

    /// Whether the curve leaves every level as it is, within half a level.
    pub fn is_identity(&self, levels: f32) -> bool {
        match self {
            Curve::Points { points, .. } => points.iter().all(|p| p[0] == p[1]),
            Curve::Map(map) => {
                let last = map.len().saturating_sub(1).max(1) as f32;
                map.iter().enumerate().all(|(k, o)| (o - k as f32 / last * levels).abs() <= 0.5)
            }
        }
    }

    /// A drawn curve of `levels + 1` outputs from `curve`: a [`Curve::Map`] as it is (resized),
    /// points through `samples` (given the corner flags).
    pub(crate) fn drawn(curve: &Curve, samples: impl FnOnce(&[[f32; 2]], &[bool]) -> Vec<f32>, levels: f32) -> Vec<f32> {
        let count = levels.max(1.0).round() as usize + 1;
        let mut map = match curve {
            Curve::Map(map) => map.clone(),
            Curve::Points { points, corners } => samples(points, corners),
        };
        let from = map.len();
        map.resize(count, 0.0);
        for (k, o) in map.iter_mut().enumerate().skip(from) {
            *o = k as f32;
        }
        map
    }

    /// Draws a straight line from `from` to `to` (levels) into `map`, one output per input level,
    /// whole levels . Returns whether `map` changed.
    pub(crate) fn pencil_line(map: &mut [f32], from: [f32; 2], to: [f32; 2], levels: f32) -> bool {
        if !from.iter().chain(to.iter()).all(|v| v.is_finite()) || map.is_empty() {
            return false;
        }
        let clamp = |q: [f32; 2]| [q[0].round().clamp(0.0, levels), q[1].round().clamp(0.0, levels)];
        let (a, b) = (clamp(from), clamp(to));
        let (lo, hi) = if a[0] <= b[0] { (a, b) } else { (b, a) };
        let mut changed = false;
        for x in lo[0] as usize..=hi[0] as usize {
            let t = if hi[0] > lo[0] { (x as f32 - lo[0]) / (hi[0] - lo[0]) } else { 1.0 };
            let y = (lo[1] + t * (hi[1] - lo[1])).round();
            if let Some(slot) = map.get_mut(x)
                && *slot != y
            {
                *slot = y;
                changed = true;
            }
        }
        changed
    }

    /// `points` and their corner flags as the contour curve reads them: sorted by
    /// input (stable), then each point within 1e-6 of the one kept before it dropped, so the first
    /// of a run keeps its flag. The host's `Curve::Points` is not required to be ascending or
    /// distinct, and a duplicated corner would otherwise split the curve into a vertical step and
    /// give the sampler a zero-width segment.
    ///
    /// This is done where the curve is evaluated, not where the editor takes the host's value: the
    /// editor edits the host's list by index (selection, drags, field edits, corner flags), so
    /// reordering it on the way in would put every operation on the wrong point. Every drawn curve
    /// (the plot, the pencil's starting map, the previews) goes through here, so they agree.
    fn normalised(points: &[[f32; 2]], corners: &[bool]) -> (Vec<[f32; 2]>, Vec<bool>) {
        let mut kept: Vec<([f32; 2], bool)> = points.iter().enumerate().map(|(i, p)| (*p, corners.get(i).copied().unwrap_or(false))).collect();
        kept.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        kept.dedup_by(|later, earlier| (later.0[0] - earlier.0[0]).abs() < 1e-6);
        kept.into_iter().unzip()
    }

    /// The output of every input level `0..=levels` through `points` (normalised first, see
    /// `Curve::normalised`), of which
    /// `corners` marks corner points : the points are split at
    /// the corners into runs `[start..=corner]`, each run is `smooth` (the host's interpolation of
    /// those points alone, constant outside its end points), and each run owns the inputs from its
    /// first point on. A corner is where one run ends and the next begins, so the curve bends
    /// there and all-corner points make a polyline.
    pub fn through(points: &[[f32; 2]], corners: &[bool], smooth: impl Fn(&[[f32; 2]]) -> Vec<f32>) -> Vec<f32> {
        let (points, corners) = Curve::normalised(points, corners);
        let (points, corners) = (points.as_slice(), corners.as_slice());
        if !corners.iter().any(|&corner| corner) {
            return smooth(points);
        }
        let is_corner = |i: usize| corners.get(i).copied().unwrap_or(false);
        let last = points.len().saturating_sub(1);
        let mut out: Vec<f32> = Vec::new();
        let mut start = 0;
        for i in 1..=last {
            if !(is_corner(i) || i == last) {
                continue;
            }
            let run = smooth(&points[start..=i]);
            if out.is_empty() {
                out = run;
            } else {
                let from = points[start][0];
                for (k, (slot, o)) in out.iter_mut().zip(run).enumerate() {
                    if k as f32 >= from {
                        *slot = o;
                    }
                }
            }
            start = i;
        }
        if out.is_empty() { smooth(points) } else { out }
    }

    /// The output level of every input level `0..=levels` through `points` on a monotone cubic
    /// (Fritsch–Carlson), which never overshoots between points; constant outside the end
    /// points. The editor's default sampler.
    pub fn monotone(points: &[[f32; 2]], levels: f32) -> Vec<f32> {
        let count = levels.max(1.0).round() as usize + 1;
        let points: Vec<[f64; 2]> = points.iter().filter(|p| p.iter().all(|v| v.is_finite())).map(|p| [f64::from(p[0]), f64::from(p[1])]).collect();
        let at = |x: f64| -> f64 {
            match points.as_slice() {
                [] => x,
                [only] => only[1],
                _ => monotone_at(&points, x),
            }
        };
        (0..count).map(|k| at(k as f64).clamp(0.0, f64::from(levels)) as f32).collect()
    }
}

/// The corner flags after `before` became `after` by one insertion, one removal or moves: each flag
/// follows its point, a new point is smooth.
pub(crate) fn follow_corners(before: &[[f32; 2]], after: &[[f32; 2]], corners: &mut Vec<bool>) {
    corners.resize(before.len(), false);
    let first_diff = |a: &[[f32; 2]], b: &[[f32; 2]]| (0..a.len()).find(|&i| a.get(i) != b.get(i)).unwrap_or(a.len());
    if after.len() == before.len() + 1 {
        corners.insert(first_diff(after, before).min(corners.len()), false);
    } else if after.len() + 1 == before.len() {
        let i = first_diff(before, after);
        if i < corners.len() {
            corners.remove(i);
        }
    }
    corners.resize(after.len(), false);
}

/// The monotone cubic through `points` (two or more, inputs ascending) at `x`.
fn monotone_at(points: &[[f64; 2]], x: f64) -> f64 {
    let n = points.len();
    let (first, last) = (points[0], points[n - 1]);
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    let secant = |k: usize| {
        let dx = points[k + 1][0] - points[k][0];
        if dx > 0.0 { (points[k + 1][1] - points[k][1]) / dx } else { 0.0 }
    };
    let d: Vec<f64> = (0..n - 1).map(secant).collect();
    let mut m: Vec<f64> = (0..n)
        .map(|k| match k {
            0 => d[0],
            k if k == n - 1 => d[n - 2],
            k if d[k - 1] * d[k] <= 0.0 => 0.0,
            k => (d[k - 1] + d[k]) / 2.0,
        })
        .collect();
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[k] / d[k], m[k + 1] / d[k]);
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[k] = t * a * d[k];
            m[k + 1] = t * b * d[k];
        }
    }
    let k = points.windows(2).position(|w| x < w[1][0]).unwrap_or(n - 2);
    let ([x0, y0], [x1, y1]) = (points[k], points[k + 1]);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * m[k] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * m[k + 1]
}

/// Within this many input levels of an existing point, a new point is that point.
pub(crate) const SAME_POINT: f32 = 2.0;

/// The point rules of a curve with `levels` levels and at most `max_points` points: what
/// [`CurveEditor`](super::CurveEditor) applies to its points, for hosts that add points
/// themselves (a Curves eyedropper).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CurveRules {
    /// Greatest level of the curve; inputs and outputs run `0..=levels`.
    pub levels: f32,
    /// Most control points the curve may hold.
    pub max_points: usize,
}

impl CurveRules {
    /// The rules of a curve with `levels` levels and at most `max_points` points.
    pub const fn new(levels: f32, max_points: usize) -> CurveRules {
        CurveRules { levels, max_points }
    }

    /// Adds a point at `v`, kept sorted, and returns its index. A point within
    /// `SAME_POINT` input levels of `v` is returned instead; a full curve takes none.
    pub fn insert(&self, points: &mut Vec<[f32; 2]>, v: [f32; 2]) -> Option<usize> {
        if !v.iter().all(|x| x.is_finite()) {
            return None;
        }
        let v = [v[0].round().clamp(0.0, self.levels), v[1].round().clamp(0.0, self.levels)];
        if let Some(i) = points.iter().position(|q| (q[0] - v[0]).abs() < SAME_POINT) {
            return Some(i);
        }
        if points.len() >= self.max_points {
            return None;
        }
        let i = points.iter().position(|q| q[0] > v[0]).unwrap_or(points.len());
        points.insert(i, v);
        Some(i)
    }

    /// Whether point `i` may go: the end points stay, and a curve keeps two points.
    pub fn can_delete(points: &[[f32; 2]], i: usize) -> bool {
        points.len() > 2 && i > 0 && i < points.len() - 1
    }

    /// Removes point `i`; false when it may not go.
    pub fn delete(points: &mut Vec<[f32; 2]>, i: usize) -> bool {
        if !CurveRules::can_delete(points, i) {
            return false;
        }
        points.remove(i);
        true
    }

    /// Moves point `i` to `v` in whole levels, strictly between its neighbours' inputs; false
    /// when nothing changed.
    pub fn move_to(&self, points: &mut [[f32; 2]], i: usize, v: [f32; 2]) -> bool {
        let Some(old) = points.get(i).copied() else { return false };
        if !v.iter().chain(old.iter()).all(|x| x.is_finite()) {
            return false;
        }
        let lo = i.checked_sub(1).and_then(|j| points.get(j)).map_or(0.0, |p| p[0] + 1.0);
        let hi = points.get(i + 1).map_or(self.levels, |p| p[0] - 1.0);
        let new = [v[0].round().clamp(lo, hi.max(lo)), v[1].round().clamp(0.0, self.levels)];
        points[i] = new;
        new != old
    }

    /// Where a point added from the keyboard goes: halfway between point `selected` and the next
    /// (the one before for the last point), or across the widest gap when nothing is selected,
    /// on the curve `samples` draws (one output per input level).
    pub fn between(points: &[[f32; 2]], selected: Option<usize>, samples: &[f32]) -> Option<[f32; 2]> {
        let gap = match selected.filter(|&i| i < points.len()) {
            Some(i) if i + 1 < points.len() => i,
            Some(i) => i.checked_sub(1)?,
            None => (0..points.len().checked_sub(1)?).max_by(|&a, &b| {
                let width = |k: usize| points[k + 1][0] - points[k][0];
                // The first of equal gaps.
                width(a).total_cmp(&width(b)).then(b.cmp(&a))
            })?,
        };
        let x = ((points[gap][0] + points[gap + 1][0]) / 2.0).round();
        let y = samples.get(x as usize).copied().unwrap_or((points[gap][1] + points[gap + 1][1]) / 2.0);
        Some([x, y.round()])
    }
}

/// One-shot edits of a curve's points, as the graph's keys, the delete clicks and the fields make them. Each returns
/// `None` when it changes nothing or the curve is a drawn one.
impl CurveRules {
    /// `curve` with point `i` moved by `by` levels, input then output.
    pub(crate) fn nudged(&self, curve: &Curve, i: usize, by: [f32; 2]) -> Option<Curve> {
        let mut points = curve.points()?.to_vec();
        let p = *points.get(i)?;
        if !self.move_to(&mut points, i, [p[0] + by[0], p[1] + by[1]]) {
            return None;
        }
        Some(Curve::from_parts(points, curve.corner_flags()))
    }

    /// `curve` with point `i` at `to`, strictly between its neighbours' inputs.
    pub(crate) fn moved(&self, curve: &Curve, i: usize, to: [f32; 2]) -> Option<Curve> {
        let mut points = curve.points()?.to_vec();
        if !self.move_to(&mut points, i, to) {
            return None;
        }
        Some(Curve::from_parts(points, curve.corner_flags()))
    }

    /// `curve` with a point added on the curve `samples` draws, halfway from point `after` to the next (across the
    /// widest gap for `None`), and the new point's index. `None` when the gap is too narrow or the curve is full.
    pub(crate) fn inserted(&self, curve: &Curve, after: Option<usize>, samples: &[f32]) -> Option<(Curve, usize)> {
        let mut points = curve.points()?.to_vec();
        let before = points.clone();
        let mut corners = curve.corner_flags();
        let v = CurveRules::between(&points, after, samples)?;
        let count = points.len();
        let i = self.insert(&mut points, v)?;
        if points.len() == count {
            return None;
        }
        follow_corners(&before, &points, &mut corners);
        Some((Curve::from_parts(points, corners), i))
    }

    /// `curve` without point `i`.
    pub(crate) fn deleted(curve: &Curve, i: usize) -> Option<Curve> {
        let mut points = curve.points()?.to_vec();
        let before = points.clone();
        let mut corners = curve.corner_flags();
        if !CurveRules::delete(&mut points, i) {
            return None;
        }
        follow_corners(&before, &points, &mut corners);
        Some(Curve::from_parts(points, corners))
    }

    /// `curve` with point `i` a corner point (`true`) or a smooth one.
    pub(crate) fn cornered(curve: &Curve, i: usize, on: bool) -> Option<Curve> {
        let points = curve.points()?.to_vec();
        let mut corners = curve.corner_flags();
        let flag = corners.get_mut(i)?;
        if *flag == on {
            return None;
        }
        *flag = on;
        Some(Curve::from_parts(points, corners))
    }
}
