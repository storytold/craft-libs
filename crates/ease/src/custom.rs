//! [`CustomEase`]: an editable curve of several Bezier segments from (0, 0) to (1, 1).

use crate::{EaseError, Vec2, bez, solve_monotonic};

/// Most points (anchors) a custom ease keeps; longer curves are thinned evenly.
pub const MAX_EASE_POINTS: usize = 64;
/// Values a custom ease (and its handles) may reach: overshoot well past 0..1, never unbounded.
pub const MAX_EASE_Y: f64 = 10.0;
/// Most points read from a file or a caller before thinning (bounds the sort).
const MAX_EASE_INPUT: usize = 65_536;

/// A point of a custom ease: where it is (x = time, y = progress), its handles as offsets from
/// it (in toward the previous point, out toward the next), and whether it is smooth (handles in
/// line, moved together) or a corner. In files the in handle is `"in"`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EasePoint {
    pub at: Vec2,
    #[cfg_attr(feature = "serde", serde(rename = "in", default))]
    pub h_in: Vec2,
    #[cfg_attr(feature = "serde", serde(default))]
    pub out: Vec2,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub smooth: bool,
}

#[cfg(feature = "serde")]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Which handle of a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    In,
    Out,
}

/// A custom ease: Bezier segments through its points, from (0, 0) to (1, 1).
///
/// Always valid, however it was made: 2 to [`MAX_EASE_POINTS`] finite points sorted by x, the
/// ends fixed at (0, 0) and (1, 1), every handle reaching toward its neighbor in x and every
/// segment's x running forwards (so time never runs backwards), and every value within
/// ±[`MAX_EASE_Y`]. A handle whose x lies between its point and that neighbor always keeps x
/// running forwards; a handle may reach past the neighbor (as a piece of a split segment can)
/// only while its segment's x still runs forwards. Reading a file repairs a curve the way
/// [`CustomEase::new`] does.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(from = "CustomEaseFile"))]
pub struct CustomEase {
    points: Vec<EasePoint>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct CustomEaseFile {
    #[serde(default, deserialize_with = "read_points")]
    points: Vec<EasePoint>,
}

/// The first [`MAX_EASE_INPUT`] points of a file's list, the way [`CustomEase::new`] keeps them.
#[cfg(feature = "serde")]
fn read_points<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<EasePoint>, D::Error> {
    crate::read_first(d, MAX_EASE_INPUT)
}

#[cfg(feature = "serde")]
impl From<CustomEaseFile> for CustomEase {
    fn from(f: CustomEaseFile) -> CustomEase {
        CustomEase::new(f.points)
    }
}

impl Default for CustomEase {
    fn default() -> CustomEase {
        CustomEase::linear()
    }
}

impl CustomEase {
    /// The straight line, its handles a third of the way along.
    pub fn linear() -> CustomEase {
        CustomEase::linear_points()
    }

    /// One segment with handles `(x1, y1)` and `(x2, y2)` in the unit box (x is clamped to
    /// `0..=1`).
    pub fn from_bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> CustomEase {
        let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
        CustomEase::new(vec![
            EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::new(x1, y1), smooth: false },
            EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(x2 - 1.0, y2 - 1.0), out: Vec2::ZERO, smooth: false },
        ])
    }

    /// The curve of absolute points `[p0, c1, c2, p1, c3, c4, p2, …]` (3n + 1 of them): `None`
    /// when the count is wrong.
    pub fn from_flat(pts: &[Vec2]) -> Option<CustomEase> {
        if pts.len() < 4 || pts.len() % 3 != 1 || pts.len() > MAX_EASE_INPUT {
            return None;
        }
        let mut out: Vec<EasePoint> = Vec::with_capacity(pts.len() / 3 + 1);
        for (i, at) in pts.iter().enumerate().step_by(3) {
            let h_in = if i == 0 { Vec2::ZERO } else { pts.get(i - 1).map_or(Vec2::ZERO, |c| *c - *at) };
            let h_out = pts.get(i + 1).map_or(Vec2::ZERO, |c| *c - *at);
            // Handles in line (opposite ways) make a smooth point, as an editor would draw it.
            let smooth = h_in.length() > 1e-6 && h_out.length() > 1e-6 && (h_in.normalized().dot(h_out.normalized()) + 1.0).abs() < 1e-3;
            out.push(EasePoint { at: *at, h_in, out: h_out, smooth });
        }
        Some(CustomEase::new(out))
    }

    /// A curve from points a caller supplies, checked: an error for a point or handle that is
    /// not finite, fewer than 2 points or more than [`MAX_EASE_POINTS`]. The points are then
    /// sorted, the ends pinned and the handles brought within reach as [`CustomEase::new`] does.
    pub fn try_new(pts: Vec<EasePoint>) -> Result<CustomEase, EaseError> {
        if pts.iter().any(|p| !(p.at.is_finite() && p.h_in.is_finite() && p.out.is_finite())) {
            return Err(EaseError::NotFinite("ease point"));
        }
        if pts.len() < 2 {
            return Err(EaseError::TooFew { what: "a custom ease", need: 2 });
        }
        if pts.len() > MAX_EASE_POINTS {
            return Err(EaseError::Full(MAX_EASE_POINTS));
        }
        Ok(CustomEase::new(pts))
    }

    /// A valid curve from any points, as a file holds them (see the type's docs): non-finite
    /// points are dropped and non-finite handles zeroed, the rest sorted by x, a curve longer
    /// than [`MAX_EASE_POINTS`] thinned evenly, the ends pinned and every handle brought within
    /// reach. Fewer than two points make the straight line.
    pub fn new(mut pts: Vec<EasePoint>) -> CustomEase {
        pts.truncate(MAX_EASE_INPUT);
        pts.retain(|p| p.at.is_finite());
        for p in pts.iter_mut() {
            p.at = Vec2::new(p.at.x.clamp(0.0, 1.0), p.at.y.clamp(-MAX_EASE_Y, MAX_EASE_Y));
            if !p.h_in.is_finite() {
                p.h_in = Vec2::ZERO;
            }
            if !p.out.is_finite() {
                p.out = Vec2::ZERO;
            }
        }
        pts.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
        if pts.len() > MAX_EASE_POINTS {
            let n = pts.len();
            let keep: Vec<usize> = (0..MAX_EASE_POINTS).map(|k| k * (n - 1) / (MAX_EASE_POINTS - 1)).collect();
            pts = keep.into_iter().filter_map(|i| pts.get(i).copied()).collect();
        }
        if pts.len() < 2 {
            return CustomEase::linear_points();
        }
        if let Some(p) = pts.first_mut() {
            p.at = Vec2::ZERO;
        }
        if let Some(p) = pts.last_mut() {
            p.at = Vec2::new(1.0, 1.0);
        }
        let mut c = CustomEase { points: pts };
        // A point marked smooth whose handles are not in line is a corner: it loses the mark
        // before the handles are brought within reach, so they are clipped as a corner's, and
        // again if bringing them within reach put them out of line.
        c.clear_broken_smooth();
        c.clamp_handles();
        c.clear_broken_smooth();
        c
    }

    /// Clears the smooth mark of every end and of every point whose handles are not in line.
    fn clear_broken_smooth(&mut self) {
        let n = self.points.len();
        for (i, p) in self.points.iter_mut().enumerate() {
            if p.smooth && (i == 0 || i + 1 == n || !handles_in_line(p.h_in, p.out)) {
                p.smooth = false;
            }
        }
    }

    /// [`CustomEase::linear`] built directly (so [`CustomEase::new`] does not recurse).
    fn linear_points() -> CustomEase {
        let third = 1.0 / 3.0;
        CustomEase {
            points: vec![
                EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::new(third, third), smooth: false },
                EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-third, -third), out: Vec2::ZERO, smooth: false },
            ],
        }
    }

    /// Brings every handle within reach (see the type's docs): each handle toward its neighbor
    /// in x and within ±[`MAX_EASE_Y`], then, in a segment whose x would run backwards, each
    /// handle that reaches past the neighbor back to it. A smooth point's handles are shortened
    /// along their own line, so they stay in line; a corner's are clipped.
    fn clamp_handles(&mut self) {
        let n = self.points.len();
        for (i, p) in self.points.iter_mut().enumerate() {
            let (lo, hi) = (-MAX_EASE_Y - p.at.y, MAX_EASE_Y - p.at.y);
            p.h_in = if i == 0 { Vec2::ZERO } else { fit_y(Vec2::new(p.h_in.x.min(0.0), p.h_in.y), lo, hi, p.smooth) };
            p.out = if i + 1 == n { Vec2::ZERO } else { fit_y(Vec2::new(p.out.x.max(0.0), p.out.y), lo, hi, p.smooth) };
        }
        for i in 1..n {
            let (Some(a), Some(b)) = (self.points.get(i - 1).copied(), self.points.get(i).copied()) else { continue };
            let span = b.at.x - a.at.x;
            if runs_forwards(span, a.out.x, -b.h_in.x) {
                continue;
            }
            if let Some(p) = self.points.get_mut(i - 1) {
                p.out = fit_x(p.out, span, p.smooth);
            }
            if let Some(p) = self.points.get_mut(i) {
                p.h_in = fit_x(p.h_in, span, p.smooth);
            }
        }
    }

    pub fn points(&self) -> &[EasePoint] {
        &self.points
    }

    /// The control points of segment `i` (from point `i` to point `i + 1`), absolute; x runs
    /// forwards along it (see the type's docs).
    pub fn segment(&self, i: usize) -> Option<[Vec2; 4]> {
        let (a, b) = (self.points.get(i)?, self.points.get(i.checked_add(1)?)?);
        Some([a.at, a.at + a.out, b.at + b.h_in, b.at])
    }

    /// The segment that time `t` falls in.
    fn segment_at(&self, t: f64) -> usize {
        self.points.partition_point(|p| p.at.x <= t).saturating_sub(1).min(self.points.len().saturating_sub(2))
    }

    /// The eased value at time `t` (0 at the start, 1 at the end, anything between, overshoot
    /// included). `t` outside `0..=1` is clamped, and NaN reads as 0.
    pub fn apply(&self, t: f64) -> f64 {
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let Some([p0, p1, p2, p3]) = self.segment(self.segment_at(t)) else { return t };
        if p3.x <= p0.x {
            return p3.y;
        }
        let s = solve_monotonic(t, [p0.x, p1.x, p2.x, p3.x]);
        bez(s, [p0.y, p1.y, p2.y, p3.y])
    }

    /// Adds a point on the curve at time `t`, leaving its shape as it was (the segment is split
    /// in two): the new curve and the point's index. `None` at the ends, on an existing point,
    /// where the split cannot be placed at `t` (on a curve so flat there that its time cannot be
    /// solved closely enough), or when the curve is full.
    pub fn insert_at(&self, t: f64) -> Option<(CustomEase, usize)> {
        if !(t > 0.0 && t < 1.0) || self.points.len() >= MAX_EASE_POINTS || self.points.iter().any(|p| p.at.x == t) {
            return None;
        }
        let i = self.segment_at(t);
        let [p0, p1, p2, p3] = self.segment(i)?;
        let s = solve_monotonic(t, [p0.x, p1.x, p2.x, p3.x]);
        // de Casteljau at s.
        let (a, b, c) = (p0.lerp(p1, s), p1.lerp(p2, s), p2.lerp(p3, s));
        let (d, e) = (a.lerp(b, s), b.lerp(c, s));
        let m = d.lerp(e, s);
        // Refused when the split cannot be placed at `t`: it lands on an anchor, or farther from
        // `t` than a millionth of the way to the nearer anchor.
        if !(m.x > p0.x && m.x < p3.x && (m.x - t).abs() <= 1e-6 * (t - p0.x).min(p3.x - t)) {
            return None;
        }
        let mut pts = self.points.clone();
        if let Some(q) = pts.get_mut(i) {
            q.out = a - p0;
        }
        if let Some(q) = pts.get_mut(i + 1) {
            q.h_in = c - p3;
        }
        pts.insert(i + 1, EasePoint { at: m, h_in: d - m, out: e - m, smooth: true });
        Some((CustomEase::new(pts), i + 1))
    }

    /// Moves point `i` (not an end) to `to`, its handles with it; its x stays between its
    /// neighbors' so time keeps running forwards. `None` for an end, a missing point or a
    /// non-finite `to`.
    pub fn move_point(&self, i: usize, to: Vec2) -> Option<CustomEase> {
        if i == 0 || i >= self.points.len().saturating_sub(1) || !to.is_finite() {
            return None;
        }
        let (lo, hi) = (self.points.get(i - 1)?.at.x, self.points.get(i + 1)?.at.x);
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        p.at = Vec2::new(to.x.clamp(lo, hi), to.y.clamp(-MAX_EASE_Y, MAX_EASE_Y));
        c.clamp_handles();
        Some(c)
    }

    /// Moves handle `which` of point `i` to `to` (absolute); a smooth point's other handle turns
    /// to stay in line, keeping its length. `None` for a handle the point does not have (the
    /// first point's in, the last's out) or a non-finite `to`.
    pub fn move_handle(&self, i: usize, which: Handle, to: Vec2) -> Option<CustomEase> {
        let n = self.points.len();
        if !to.is_finite() || i >= n || (which == Handle::In && i == 0) || (which == Handle::Out && i + 1 == n) {
            return None;
        }
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        let off = to - p.at;
        match which {
            Handle::In => p.h_in = off,
            Handle::Out => p.out = off,
        }
        c.clamp_handles();
        if i > 0 && i + 1 < n {
            let (prev, next) = (c.points.get(i - 1)?.at.x, c.points.get(i + 1)?.at.x);
            if let Some(p) = c.points.get_mut(i).filter(|p| p.smooth) {
                let at = p.at;
                let (moved, other, x_reach) = match which {
                    Handle::In => (p.h_in, &mut p.out, next - at.x),
                    Handle::Out => (p.out, &mut p.h_in, prev - at.x),
                };
                let dir = -moved.normalized();
                if dir != Vec2::ZERO {
                    // Shortened to stay within reach along `dir`, so the clamp below does not
                    // turn it out of line.
                    // A handle with no length is given one, so the point stays smooth.
                    let mut len = other.length();
                    if len <= 0.0 {
                        len = 1e-3;
                    }
                    if dir.x != 0.0 {
                        len = len.min((x_reach / dir.x).max(0.0));
                    }
                    if dir.y != 0.0 {
                        let y_reach = if dir.y > 0.0 { MAX_EASE_Y - at.y } else { -MAX_EASE_Y - at.y };
                        len = len.min((y_reach / dir.y).max(0.0));
                    }
                    *other = dir * len;
                }
            }
        }
        c.clamp_handles();
        Some(c)
    }

    /// Removes point `i` (not an end). `None` for an end or a missing point.
    pub fn remove(&self, i: usize) -> Option<CustomEase> {
        if i == 0 || i >= self.points.len().saturating_sub(1) {
            return None;
        }
        let mut pts = self.points.clone();
        pts.remove(i);
        Some(CustomEase::new(pts))
    }

    /// Makes point `i` smooth (its handles in line with its neighbors' chord, a third of the way
    /// to each) or a corner (its handles on the point, so the curve turns sharply there).
    pub fn set_smooth(&self, i: usize, smooth: bool) -> Option<CustomEase> {
        let n = self.points.len();
        let prev = if i == 0 { None } else { self.points.get(i - 1).map(|p| p.at) };
        let next = i.checked_add(1).and_then(|j| self.points.get(j)).map(|p| p.at);
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        p.smooth = smooth && i > 0 && i + 1 < n;
        if smooth {
            let (a, b) = (prev.unwrap_or(p.at), next.unwrap_or(p.at));
            let chord = b - a;
            let dir = if chord.x > 1e-6 { chord * (1.0 / chord.x) } else { Vec2::new(1.0, 0.0) };
            if prev.is_some() {
                p.h_in = -dir * ((p.at.x - a.x) / 3.0);
            }
            if next.is_some() {
                p.out = dir * ((b.x - p.at.x) / 3.0);
            }
        } else {
            p.h_in = Vec2::ZERO;
            p.out = Vec2::ZERO;
        }
        c.clamp_handles();
        Some(c)
    }

    /// The flat list of absolute points `[p0, c1, c2, p1, …]` (see [`CustomEase::from_flat`]).
    pub fn to_flat(&self) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(self.points.len() * 3);
        for (i, p) in self.points.iter().enumerate() {
            if i > 0 {
                out.push(p.at + p.h_in);
            }
            out.push(p.at);
            if i + 1 < self.points.len() {
                out.push(p.at + p.out);
            }
        }
        out
    }
}

/// Whether a segment `span` wide in x whose out handle reaches `a` forwards and whose in handle
/// reaches `c` backwards (both at least 0) runs forwards in x. Its x′ is the quadratic with
/// Bernstein coefficients `a`, `span − a − c` and `c`, which is never negative when the middle
/// one is at least −√(ac); that always holds while neither handle reaches past the other end,
/// and never holds when one reaches past 4/3 of the span. The test is made on reaches divided by
/// the span, so squaring them does not underflow on a very narrow segment.
fn runs_forwards(span: f64, a: f64, c: f64) -> bool {
    if a <= span && c <= span {
        return true;
    }
    if a > 2.0 * span || c > 2.0 * span {
        return false;
    }
    // Here span > 0: a handle reaches past it, and no more than twice it.
    let (a, c) = (a / span, c / span);
    let b = 1.0 - a - c;
    b >= 0.0 || b * b <= a * c * (1.0 + 1e-12)
}

/// Whether handles `a` and `b` point opposite ways along one line (as a smooth point's do), or
/// one of them has no length.
fn handles_in_line(a: Vec2, b: Vec2) -> bool {
    a.length() <= 1e-9 || b.length() <= 1e-9 || (a.normalized().dot(b.normalized()) + 1.0).abs() < 1e-3
}

/// Handle `h` (an offset) with its y within `lo..=hi`: a smooth point's shortened along its line,
/// a corner's clipped.
fn fit_y(h: Vec2, lo: f64, hi: f64, smooth: bool) -> Vec2 {
    let k = if h.y > hi {
        hi / h.y
    } else if h.y < lo {
        lo / h.y
    } else {
        return h;
    };
    if smooth { Vec2::new(h.x * k, (h.y * k).clamp(lo, hi)) } else { Vec2::new(h.x, h.y.clamp(lo, hi)) }
}

/// Handle `h` (an offset) reaching at most `span` in x: a smooth point's shortened along its line,
/// a corner's clipped.
fn fit_x(h: Vec2, span: f64, smooth: bool) -> Vec2 {
    let reach = h.x.abs();
    if reach <= span {
        return h;
    }
    let span = span.max(0.0);
    if smooth {
        let k = span / reach;
        Vec2::new((h.x * k).clamp(-span, span), h.y * k)
    } else {
        Vec2::new(span.copysign(h.x), h.y)
    }
}
