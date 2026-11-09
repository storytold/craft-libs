// Carved from VectorCraft's crates/pathops (src/edit.rs: simplify, remove redundant points, join) at
// storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Path editing: Simplify, Remove redundant points and Join.

use kurbo::{BezPath, CubicBez, ParamCurve, PathSeg, Point, Vec2};

use crate::budget::fits_elements;
use crate::fit::{
    chord_distance, end_tangent, fit_cubics, fit_single_bounded, is_straight, measured_arclen, polygon_length, sample, second_difference,
    start_tangent, unit,
};
use crate::path::{Seg, Sub, segs_to_bezpath, subs};
use crate::work::{self, Work};

/// Options for [`simplify_with`]. Start from [`SimplifyOptions::default`] and set what differs
/// (`SimplifyOptions::default().tolerance(1.0).straight_lines(true)`, or assign the fields): the
/// type is `#[non_exhaustive]`, so an option added later does not break a caller.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SimplifyOptions {
    /// Maximum distance (path units) between the original and simplified path (with `smooth`, between the
    /// smoothed and simplified path).
    pub tolerance: f64,
    /// Anchors where the path turns by more than this many degrees stay corners.
    pub corner_angle_deg: f64,
    /// Produce only straight segments.
    pub straight_lines: bool,
    /// How far (path units, the spread of a Gaussian along the path) the path is smoothed before it
    /// is refitted, so that bumps and jags come out as one curve; 0 for none. Smoothing moves the
    /// path, and `tolerance` is measured from the smoothed path, not from the original: a curve
    /// that bends sharply shrinks towards its inside (a spread of 30 takes a circle of radius 50
    /// in by about 8), whatever the tolerance.
    pub smooth: f64,
}

impl Default for SimplifyOptions {
    fn default() -> Self {
        Self { tolerance: 0.5, corner_angle_deg: 30.0, straight_lines: false, smooth: 0.0 }
    }
}

impl SimplifyOptions {
    /// These options with `tolerance` (see the field).
    #[must_use]
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// These options with `corner_angle_deg` (see the field).
    #[must_use]
    pub fn corner_angle_deg(mut self, corner_angle_deg: f64) -> Self {
        self.corner_angle_deg = corner_angle_deg;
        self
    }

    /// These options with `straight_lines` (see the field).
    #[must_use]
    pub fn straight_lines(mut self, straight_lines: bool) -> Self {
        self.straight_lines = straight_lines;
        self
    }

    /// These options with `smooth` (see the field).
    #[must_use]
    pub fn smooth(mut self, smooth: f64) -> Self {
        self.smooth = smooth;
        self
    }
}

/// Simplify with the default corner threshold (30°).
pub fn simplify(path: &BezPath, tolerance: f64) -> BezPath {
    simplify_with(path, &SimplifyOptions { tolerance, ..Default::default() })
}

/// Simplify never adds anchors: keep whichever subpath is smaller.
fn no_worse(original: &Sub, fitted: BezPath, fitted_anchors: usize) -> BezPath {
    if fitted_anchors > original.anchors() {
        let mut out = BezPath::new();
        if original.closed {
            original.push_closed(&mut out)
        } else {
            original.push_open(&mut out)
        }
        out
    } else {
        fitted
    }
}

fn turn_deg(a: &Seg, b: &Seg) -> f64 {
    let d = end_tangent(&a.c).dot(start_tangent(&b.c)).clamp(-1.0, 1.0);
    d.acos().to_degrees()
}

/// Least-squares Bézier refit of every subpath, keeping corners sharper than the threshold.
///
/// The work is bounded: a path of more than [`crate::MAX_ELEMENTS`] elements is returned as it is, a
/// run of segments that would take more than [`MAX_FIT_POINTS`] samples to fit stays as it is, and
/// so does every run left when the samples taken, smoothed and fitted add up to [`FIT_WORK`].
pub fn simplify_with(path: &BezPath, opts: &SimplifyOptions) -> BezPath {
    let limits = crate::budget::limits();
    simplify_within(path, opts, &mut Fit::new(limits.fit_work, limits.fit_points))
}

/// [`simplify_with`] within the limits `fit`.
fn simplify_within(path: &BezPath, opts: &SimplifyOptions, fit: &mut Fit) -> BezPath {
    if !fits_elements([path]) {
        return path.clone();
    }
    let mut out = BezPath::new();
    for sub in subs(path) {
        if let Some(s) = simplify_subpath(&sub, opts, fit) {
            out.extend(s.iter());
        }
    }
    out
}

/// What a simplify may still spend, the most points it samples from one part of a run of segments, and what it has made so far.
struct Fit {
    work: usize,
    max_points: usize,
    /// The segments made by the fits and the lines of the whole call so far (the pieces that stay as they were are not counted),
    /// charged as they are or may be made, and held to [`crate::Limits::elements`] in all, over every run of every subpath ([`Fit::room`]).
    made: usize,
}

impl Fit {
    /// A fit with `work` to spend, parts of at most `max_points` samples, and nothing made yet.
    fn new(work: usize, max_points: usize) -> Self {
        Self { work, max_points, made: 0 }
    }

    /// How many more segments the call may make: [`crate::Limits::elements`] less what it has made.
    fn room(&self) -> usize {
        crate::budget::limits().elements.saturating_sub(self.made)
    }

    /// The most samples a part may hold: [`Fit::max_points`], and not more than there is room to make segments for (a refit has
    /// fewer segments than it has samples).
    fn part_size(&self) -> usize {
        self.max_points.min(self.room())
    }

    /// Spends `n`, or all that is left (and says so) when that is not enough.
    fn charge(&mut self, n: usize) -> bool {
        match self.work.checked_sub(n) {
            Some(left) => {
                self.work = left;
                work::add(Work::Fit, n);
                true
            }
            None => {
                self.work = 0;
                false
            }
        }
    }
}

/// [`simplify_with`] for one subpath; `None` when the refit leaves nothing of it.
fn simplify_subpath(sub: &Sub, opts: &SimplifyOptions, fit: &mut Fit) -> Option<BezPath> {
    let mut segs = sub.chain();
    let n = segs.len();
    if n == 0 {
        return None;
    }
    if sub.closed {
        // Start the loop at its first corner, so no fit runs across one.
        let k = (0..n).find(|&i| turn_deg(&segs[(i + n - 1) % n], &segs[i]) > opts.corner_angle_deg).unwrap_or(0);
        segs.rotate_left(k);
    }
    // A smoothed loop with no corner is fitted along one direction across its start, when it is fitted as one piece: with its
    // first and last curves' own tangents, whose sum it is, the start would keep the kink the smoothing took out of it.
    let across = if sub.closed && opts.smooth > 0.0 { unit(end_tangent(&segs[n - 1].c) + start_tangent(&segs[0].c)) } else { None };
    let across = across.filter(|_| (0..n).all(|i| turn_deg(&segs[(i + n - 1) % n], &segs[i]) <= opts.corner_angle_deg));
    let result = simplify_run(&segs, opts, fit, across);
    let s = segs_to_bezpath(&result, sub.closed)?;
    if opts.straight_lines {
        return Some(s);
    }
    let anchors = result.len() + usize::from(!sub.closed);
    Some(no_worse(sub, s, anchors))
}

/// Most points sampled from one run of segments to refit: 1,048,576 (the default of [`crate::Limits::fit_points`]).
pub const MAX_FIT_POINTS: usize = 1 << 20;

/// Most work one [`simplify_with`] does in all: 268,435,456 (the default of [`crate::Limits::fit_work`]), counting a point for each one sampled,
/// each one a smoothing reads, and each one a fit reads (seven times over, for its rounds of
/// reparameterisation). A release build takes about 2 s for it.
pub const FIT_WORK: usize = 1 << 28;

/// Most intervals one segment is sampled in, however long it is (the default; see [`chord_intervals`]).
const MAX_INTERVALS: usize = 256;

/// Most intervals one segment is sampled in when its chords must stay within a distance of it.
const MAX_CHORD_INTERVALS: usize = 1 << 16;

/// The intervals of equal parameter that keep every chord of `c` within `within` of it, if that is
/// at most [`MAX_CHORD_INTERVALS`]. A chord of an interval of length `h` is no further from the curve than
/// `h² / 8` times the largest second derivative, which for a cubic is at most 6 times the larger of
/// its two second differences of control points: `h` of `sqrt(0.75 d / within)` suffices.
fn chord_intervals(c: &CubicBez, within: f64) -> Option<usize> {
    let need = (0.75 * second_difference(c) / within).sqrt().ceil();
    (need.is_finite() && need <= MAX_CHORD_INTERVALS as f64).then_some(need as usize)
}

/// How far the segment `s`, sampled in `m` intervals of equal parameter, can be from the polyline of
/// its samples between two of them ([`chord_intervals`]'s bound, `0.75 d / m²`). A line is on its
/// chords. A cubic straight to within `straight` of its chord lies, with its samples, in the strip of
/// that half-width along the chord (its control points do, and so their hull); a point of it is then
/// within `3 straight` of the chord of two samples on either side of where it is along the chord, a
/// bound that does not grow with the length (handles on the ends make the second differences as
/// long as the segment, and the chords are on it).
fn sample_error(s: &Seg, m: usize, straight: f64) -> f64 {
    if s.line {
        return 0.0;
    }
    let m = m.max(1) as f64;
    let bound = 0.75 * second_difference(&s.c) / (m * m);
    if is_straight(&s.c, straight) { bound.min(3.0 * straight) } else { bound }
}

/// The refit of the consecutive segments `run`, split at its corners sharper than the threshold:
/// it starts and ends where `run` does, leaving the same way. `ends`, when the whole run is one piece (no corner in it), is the
/// direction the refit leaves and arrives along, in place of those of its first and last segments.
fn simplify_run(run: &[Seg], opts: &SimplifyOptions, fit: &mut Fit, ends: Option<Vec2>) -> Vec<Seg> {
    let tol = opts.tolerance.max(1e-6);
    let mut cuts: Vec<usize> = (1..run.len()).filter(|&i| turn_deg(&run[i - 1], &run[i]) > opts.corner_angle_deg).collect();
    cuts.insert(0, 0);
    cuts.push(run.len());
    let mut result: Vec<Seg> = Vec::new();
    for w in cuts.windows(2) {
        let piece = &run[w[0]..w[1]];
        if piece.is_empty() {
            continue;
        }
        // Too long to fit, past the work, or past what the call may make: the piece stays as it is, what was made of it dropped
        // and given back to what the call may make.
        let (kept, charged) = (result.len(), fit.made);
        let ends = ends.filter(|_| piece.len() == run.len());
        if !simplify_piece(piece, (tol, ends), opts, fit, &mut result) {
            result.truncate(kept);
            fit.made = charged;
            result.extend(piece.iter().copied());
        }
    }
    result
}

/// The samples of the part of a piece that is fitted together, and how far the piece can be from the polyline of them between two.
#[derive(Default)]
struct Part {
    segs: Vec<Seg>,
    pts: Vec<Point>,
    between: f64,
}

/// How a segment is sampled ([`intervals`]).
enum Sampling {
    /// In this many intervals of equal parameter.
    In(usize),
    /// The chords it needs to stay within the tolerance are more than a segment is sampled in ([`MAX_CHORD_INTERVALS`]).
    TooFine,
    /// Its coordinates are past what its length is measured to: it cannot be sampled, as a point either.
    Unmeasurable,
}

/// The intervals to sample `s` in: enough for a sample every half tolerance, from 4 to [`MAX_INTERVALS`]; and for Convert to Straight
/// Lines, as many as keep the chords of a curve within a tenth of the tolerance of it ([`chord_intervals`]), where a segment holds
/// that many.
fn intervals(s: &Seg, tol: f64, opts: &SimplifyOptions) -> Sampling {
    // Guarded: kurbo subdivides without end on huge finite coordinates.
    let Some(len) = measured_arclen(&PathSeg::Cubic(s.c), 1e-6) else { return Sampling::Unmeasurable };
    let m = ((len / (tol * 0.5)).ceil() as usize).clamp(4, MAX_INTERVALS);
    // Lines are drawn between the samples, and the curve is not: the chords can be further from the curve than the tolerance
    // allows where too few samples are taken (a quarter circle of radius 500,000 at a tolerance of 1 is 2.4 from its 256 chords). So
    // the samples are as many as keep the chords within a tenth of it, or the segment is too fine to sample. (A segment straight to
    // within a twentieth of the tolerance is within three twentieths of the polyline of its samples, however few.)
    if opts.straight_lines && !is_straight(&s.c, tol * 0.05) && sample_error(s, m, tol * 0.05) > tol * 0.1 {
        return chord_intervals(&s.c, tol * 0.1).map_or(Sampling::TooFine, |need| Sampling::In(m.max(need)));
    }
    Sampling::In(m)
}

/// The refit of one `piece` (between two corners): it is sampled and fitted in parts of at most [`Fit::max_points`] samples each,
/// with an anchor where two parts meet (a run longer than one fit holds is refitted in parts, not left as it is), and a curve too
/// fine to sample for Convert to Straight Lines becomes as many equal lines as keep it within the tolerance, if a segment holds
/// that many ([`MAX_CHORD_INTERVALS`]). False (and what was made may be dropped by the caller) for a segment that cannot be measured
/// or sampled, and when the work runs out.
fn simplify_piece(piece: &[Seg], (tol, ends): (f64, Option<Vec2>), opts: &SimplifyOptions, fit: &mut Fit, result: &mut Vec<Seg>) -> bool {
    let mut part = Part::default();
    for s in piece {
        let m = match intervals(s, tol, opts) {
            Sampling::In(m) => m,
            // Only Convert to Straight Lines asks for chords, so only it finds a segment too fine to sample ([`intervals`]).
            Sampling::TooFine => {
                // Evenly spaced lines keep it within the tolerance, if a segment holds so many: the neighbours are fitted without it.
                let lines = line_steps(&s.c, tol).max(1);
                // A segment holds up to [`MAX_CHORD_INTERVALS`] lines, and the call [`Fit::room`] segments in all, charged (after what
                // the part before made) before the lines are.
                if lines > MAX_CHORD_INTERVALS || !flush(&mut part, (tol, ends), opts, fit, result) || lines > fit.room() || !fit.charge(lines + 1) {
                    return false;
                }
                fit.made += lines;
                work::add(Work::FitSamples, lines + 1);
                let mut pts = Vec::with_capacity(lines + 1);
                sample(&s.c, lines, &mut pts, true);
                result.extend(pts.windows(2).map(|p| Seg::line(p[0], p[1])));
                continue;
            }
            Sampling::Unmeasurable => return false,
        };
        // The first segment of a part has a sample at its start besides one at the end of each interval.
        let first = part.segs.is_empty();
        if !first && part.pts.len().saturating_add(m) > fit.part_size() && !flush(&mut part, (tol, ends), opts, fit, result) {
            return false;
        }
        let taken = m + usize::from(part.segs.is_empty());
        if taken > fit.part_size() || !fit.charge(taken) {
            return false;
        }
        work::add(Work::FitSamples, taken);
        part.between = part.between.max(sample_error(s, m, tol * 0.05));
        sample(&s.c, m, &mut part.pts, part.segs.is_empty());
        part.segs.push(*s);
    }
    flush(&mut part, (tol, ends), opts, fit, result)
}

/// The refit of the part `part` of a piece, appended to `result`, and the part made empty; false when `fit` runs out of work.
///
/// The refit has fewer segments than the part has samples (a part is held to [`Fit::part_size`], which is no more than the room the
/// call has left), and the call is charged for as many before the refit is made, and given back what it did not make.
fn flush(part: &mut Part, (tol, ends): (f64, Option<Vec2>), opts: &SimplifyOptions, fit: &mut Fit, result: &mut Vec<Seg>) -> bool {
    let part = std::mem::take(part);
    if part.segs.is_empty() {
        return true;
    }
    let (charged, kept) = (part.pts.len(), result.len());
    if charged > fit.room() {
        return false;
    }
    fit.made += charged;
    let done = fit_piece(&part.segs, part.pts, (tol, part.between), ends, opts, fit, result);
    fit.made -= charged - result.len().saturating_sub(kept).min(charged);
    done
}

/// How many evenly spaced straight lines keep `c` within `tol` of them: a chord of a step of `1 / m` is no further from the curve than
/// `0.75 d / m²`, where `d` is the larger second difference of its control points ([`chord_intervals`] for a tolerance of `tol`).
fn line_steps(c: &CubicBez, tol: f64) -> usize {
    let need = (0.75 * second_difference(c) / tol).sqrt().ceil();
    if need.is_finite() && need <= usize::MAX as f64 { need as usize } else { usize::MAX }
}

/// The refit of one `piece` from its sampled points `pts`, appended to `result`; false (and
/// nothing appended) when `fit` runs out of work first. `tol` is the tolerance and how far the piece
/// can be from the polyline of `pts` between two of them ([`sample_error`]).
fn fit_piece(
    piece: &[Seg],
    pts: Vec<Point>,
    (tol, between): (f64, f64),
    ends: Option<Vec2>,
    opts: &SimplifyOptions,
    fit: &mut Fit,
    result: &mut Vec<Seg>,
) -> bool {
    let pts = if opts.smooth > 0.0 {
        let Some(smoothed) = smooth_points(&pts, opts.smooth, fit) else { return false };
        smoothed
    } else {
        pts
    };
    if opts.straight_lines {
        // A chord that keeps the samples within the tolerance less what the curve can be from them
        // between two keeps the curve within the tolerance (the distance to a chord is greatest at an
        // end of a piece of the polyline). With smoothing the tolerance is from the smoothed points.
        let within = if opts.smooth > 0.0 { tol } else { tol - between };
        let Some(keep) = rdp(&pts, within, fit) else { return false };
        result.extend(keep.windows(2).map(|p| Seg::line(p[0], p[1])));
        return true;
    }
    let (Some(first), Some(last)) = (piece.first(), piece.last()) else { return true };
    let (t0, t1) = ends.map_or((start_tangent(&first.c), end_tangent(&last.c)), |d| (d, d));
    // Out of work, or no fit within the tolerance: the piece stays as it is (the work is spent only
    // in the first case). The fit is held to the tolerance between the samples too, which the piece
    // can be `between` from (with smoothing, the tolerance is from the smoothed points' polyline).
    let between = if opts.smooth > 0.0 { 0.0 } else { between };
    let Ok(cubics) = fit_cubics(&pts, t0, t1, (tol, between), &mut fit.work) else { return false };
    for (c, off) in cubics {
        // A cubic that is nearly straight is its chord, where the chord is within the tolerance of the
        // piece too: no further from the cubic than the cubic's slack under the tolerance.
        let line = is_straight(&c, tol * 0.02) && off + chord_distance(&c) <= tol;
        result.push(if line { Seg::line(c.p0, c.p3) } else { Seg { c, line } });
    }
    true
}

/// `pts` (a run's samples, in order) smoothed along the path: each point becomes the Gaussian-weighted
/// mean of the points around it, `sigma` points of arc length wide, the window narrowing towards the
/// ends so that the first and last points stay where they are. Consecutive samples within
/// 1/[`SMOOTH_GROUP`] of a spread of each other along the path are weighed together, as their count at
/// their mean place, found through a tree of sums over the samples ([`SampleGroups`]), so that the work for a point is bounded by
/// the groups within three spreads of it however densely the path is sampled (VectorCraft 4a7d732; weighing every sample took
/// the square of the samples).
///
/// The tree is charged to `fit` by its samples before it is made, and each point by the nodes it looks at (counted as they are looked at,
/// at most a few hundred for a point, bounded by its groups and the depth of the tree, whatever the windows hold); `None` when that runs
/// out, or when the tree cannot be allocated.
fn smooth_points(pts: &[Point], sigma: f64, fit: &mut Fit) -> Option<Vec<Point>> {
    let Some(&origin) = pts.first() else { return Some(Vec::new()) };
    if pts.len() < 3 || !(sigma.is_finite() && sigma > 0.0) {
        return Some(pts.to_vec());
    }
    let mut s = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).and_then(|j| pts.get(j)) {
            total += prev.distance(*p);
        }
        s.push(total);
    }
    if !fit.charge(pts.len()) {
        return None;
    }
    let groups = SampleGroups::new(pts, &s, origin)?;
    let (mut lo, mut hi) = (0usize, 0usize);
    let mut smoothed = Vec::with_capacity(pts.len());
    for (&p, &si) in pts.iter().zip(&s) {
        let reach = (3.0 * sigma).min(si).min(total - si);
        if reach <= 1e-9 {
            smoothed.push(p);
            continue;
        }
        let spread = reach / 3.0;
        while s.get(lo).is_some_and(|&v| v < si - reach) {
            lo += 1;
        }
        while s.get(hi + 1).is_some_and(|&v| v <= si + reach) {
            hi += 1;
        }
        let window = Window { lo, end: hi + 1, at: si, spread, group: spread / SMOOTH_GROUP };
        let mut acc = Weighed::default();
        groups.weigh(0, (0, pts.len()), &window, &mut acc);
        if !fit.charge(acc.work) {
            return None;
        }
        smoothed.push(if acc.weight > 0.0 { origin + acc.sum / acc.weight } else { p });
    }
    Some(smoothed)
}

/// How many groups of samples a spread holds at most in [`smooth_points`]. Weighing them in groups
/// leaves each smoothed point within a thousandth of a spread of the mean of its samples weighed one
/// by one.
const SMOOTH_GROUP: f64 = 16.0;

/// The samples [`smooth_points`] weighs for one point: indices `lo..end`, weighed by a Gaussian of
/// `spread` around arc length `at`, in groups at most `group` long along the path.
struct Window {
    lo: usize,
    end: usize,
    at: f64,
    spread: f64,
    group: f64,
}

/// The sums of halves, quarters and so on of a run's samples, by index: node 0 holds all of them and
/// node `k`'s halves are nodes `2k + 1` and `2k + 2`.
struct SampleGroups<'a> {
    /// Arc length of each sample.
    s: &'a [f64],
    /// Sum of the arc lengths of each node's samples.
    sum_s: Vec<f64>,
    /// Sum of each node's samples, measured from the first sample.
    sum_q: Vec<Vec2>,
}

impl<'a> SampleGroups<'a> {
    /// The tree of `pts`, whose arc lengths are `s`; `None` where memory for it (four nodes for each sample) is not to be had.
    fn new(pts: &[Point], s: &'a [f64], origin: Point) -> Option<Self> {
        let size = pts.len().checked_mul(4)?;
        let mut sum_s: Vec<f64> = Vec::new();
        let mut sum_q: Vec<Vec2> = Vec::new();
        sum_s.try_reserve_exact(size).ok()?;
        sum_q.try_reserve_exact(size).ok()?;
        sum_s.resize(size, 0.0);
        sum_q.resize(size, Vec2::ZERO);
        let mut groups = Self { s, sum_s, sum_q };
        groups.build(0, 0, pts.len(), pts, origin);
        Some(groups)
    }

    fn build(&mut self, node: usize, l: usize, r: usize, pts: &[Point], origin: Point) {
        if r <= l {
            return;
        }
        let (sum_s, sum_q) = if r - l == 1 {
            (self.s.get(l).copied().unwrap_or(0.0), pts.get(l).map_or(Vec2::ZERO, |&p| p - origin))
        } else {
            let m = l + (r - l) / 2;
            self.build(2 * node + 1, l, m, pts, origin);
            self.build(2 * node + 2, m, r, pts, origin);
            let at = |v: &Vec<f64>, k: usize| v.get(k).copied().unwrap_or(0.0);
            let q = |k: usize| self.sum_q.get(k).copied().unwrap_or(Vec2::ZERO);
            (at(&self.sum_s, 2 * node + 1) + at(&self.sum_s, 2 * node + 2), q(2 * node + 1) + q(2 * node + 2))
        };
        if let Some(a) = self.sum_s.get_mut(node) {
            *a = sum_s;
        }
        if let Some(a) = self.sum_q.get_mut(node) {
            *a = sum_q;
        }
    }

    /// Adds the weighted samples of node `node` (indices `l..r`) that are inside `w` to `acc`: a node
    /// inside `w` no longer than its groups as its count at its mean arc length, any other node by
    /// its halves.
    fn weigh(&self, node: usize, (l, r): (usize, usize), w: &Window, acc: &mut Weighed) {
        if r <= w.lo || l >= w.end || r <= l {
            return;
        }
        acc.work += 1;
        let length = self.s.get(r - 1).copied().unwrap_or(0.0) - self.s.get(l).copied().unwrap_or(0.0);
        // One sample is always a group, even where arc lengths that overflowed leave `length` NaN.
        if r - l == 1 || (w.lo <= l && r <= w.end && length <= w.group) {
            let count = (r - l) as f64;
            let t = self.sum_s.get(node).copied().unwrap_or(0.0) / count - w.at;
            let g = (-t * t / (2.0 * w.spread * w.spread)).exp();
            acc.sum += self.sum_q.get(node).copied().unwrap_or(Vec2::ZERO) * g;
            acc.weight += count * g;
            return;
        }
        let m = l + (r - l) / 2;
        self.weigh(2 * node + 1, (l, m), w, acc);
        self.weigh(2 * node + 2, (m, r), w, acc);
    }
}

/// What [`SampleGroups::weigh`] has added up: the weighted sum of the samples (from the first sample),
/// the sum of the weights, and the number of nodes looked at.
#[derive(Default)]
struct Weighed {
    sum: Vec2,
    weight: f64,
    work: usize,
}

/// Ramer–Douglas–Peucker polyline reduction. Each scan for the point farthest from a chord is
/// charged to `fit` before it is made; `None` when that runs out, with the reads that were made
/// charged and no more.
fn rdp(pts: &[Point], tol: f64, fit: &mut Fit) -> Option<Vec<Point>> {
    if pts.len() < 3 {
        return fit.charge(pts.len()).then(|| pts.to_vec());
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    // Distances measured at most: lopsided splits (a shallow zigzag) make it quadratic. Past it,
    // the points of the parts left are kept as they are.
    let mut budget = pts.len().saturating_mul(64).saturating_add(1 << 20);
    while let Some((a, b)) = stack.pop() {
        budget = match budget.checked_sub(b - a) {
            Some(left) => left,
            None => {
                keep[a..=b].fill(true);
                continue;
            }
        };
        if !fit.charge(b - a) {
            return None;
        }
        work::add(Work::RdpReads, b - a);
        let (pa, pb) = (pts[a], pts[b]);
        let chord = pb - pa;
        let len = chord.hypot();
        let mut best = (0.0, a);
        for (i, &p) in pts.iter().enumerate().take(b).skip(a + 1) {
            // The distance to the segment, not to the line through it: a path that runs out past
            // an end and back along the line is that far from the segment.
            let d = if len < 1e-12 {
                p.distance(pa)
            } else {
                let along = ((p - pa).dot(chord) / (len * len)).clamp(0.0, 1.0);
                p.distance(pa + chord * along)
            };
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    Some(pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect())
}

/// Remove anchors that can be deleted without changing the shape by more than `tolerance`
/// (collinear corner points, duplicate points, and curve joints that one cubic can replace).
///
/// A path of more than [`crate::MAX_ELEMENTS`] elements is returned as it is. The search for what
/// one pass along each subpath leaves (at a closed subpath's start), the checks of a run against
/// the line that replaces it, and the points a run of curves is refitted from, are limited to
/// [`REDUNDANT_WORK`] in all.
pub fn remove_redundant_points(path: &BezPath, tolerance: f64) -> BezPath {
    let mut work = crate::budget::limits().redundant_work;
    remove_redundant_within(path, tolerance, &mut work)
}

/// [`remove_redundant_points`] reading at most `work` joints in its search after the first pass.
fn remove_redundant_within(path: &BezPath, tolerance: f64, work: &mut usize) -> BezPath {
    if !fits_elements([path]) {
        return path.clone();
    }
    let tol = tolerance.max(1e-9);
    let mut out = BezPath::new();
    for sp in subs(path) {
        let mut segs = sp.chain();
        if segs.is_empty() {
            continue;
        }
        // Drop zero-length segments.
        segs.retain(|s| !(s.c.p0.distance(s.c.p3) < 1e-9 && s.c.p1.distance(s.c.p0) < 1e-9 && s.c.p2.distance(s.c.p0) < 1e-9));
        let min_segs = if sp.closed { 2 } else { 1 };
        // Each segment stands for the run of the original ones it replaced, and remembers how far
        // they can be from it (see [`merge_pieces`]).
        let originals = Originals::new(&segs);
        let mut pieces: Vec<Piece> = segs.iter().enumerate().map(|(first, &seg)| Piece { seg, dev: 0.0, first, count: 1 }).collect();
        // Joints are merged along the path in one pass (a merged segment is tried with the one
        // before it, then the next); the search below then only finds what a closed path's wrap
        // leaves. (Merging at the first joint of a dense line one at a time shifts the rest each time.)
        let total = pieces.len();
        let mut merged = 0;
        let mut done: Vec<Piece> = Vec::with_capacity(total);
        let mut next = 0;
        while next < total {
            // A long piece is extended by the originals after it, as many at a time as fit.
            if let Some(&top) = done.last() {
                let ahead = (total - next).min((total - merged).saturating_sub(min_segs));
                let smooth = pieces.get(next).is_some_and(|n| smooth_joint(&top.seg.c, &n.seg.c));
                if top.count >= GALLOP_FROM && !top.seg.line && smooth && ahead > 0 {
                    // Not past a joint the pair merge refuses, which a refit would round off (within the
                    // tolerance). The table of the corners is made here, when a stride is first tried.
                    let ahead = ahead.min(originals.corner_from(next + 1).saturating_sub(next));
                    if let Some((longer, took)) = extend_run(&top, ahead, tol, &originals, work) {
                        if let Some(slot) = done.last_mut() {
                            *slot = longer;
                        }
                        next += took;
                        merged += took;
                        continue;
                    }
                }
            }
            let Some(&p) = pieces.get(next) else { break };
            next += 1;
            done.push(p);
            while done.len() >= 2 && total - merged > min_segs {
                let n = done.len();
                let Some(m) = merge_pieces(&done[n - 2], &done[n - 1], tol, &originals, work) else { break };
                done.truncate(n - 1);
                done[n - 2] = m;
                merged += 1;
            }
        }
        pieces = done;
        let mut changed = true;
        // Each pass reads every joint; past the work the rest of the wrap is left alone.
        while changed && pieces.len() > min_segs {
            changed = false;
            let n = pieces.len();
            let Some(left) = work.checked_sub(n) else { break };
            *work = left;
            work::add(Work::Redundant, n);
            let joints = if sp.closed { n } else { n - 1 };
            for j in 0..joints {
                let (i0, i1) = if sp.closed { ((j + n - 1) % n, j) } else { (j, j + 1) };
                if let Some(m) = merge_pieces(&pieces[i0], &pieces[i1], tol, &originals, work) {
                    pieces[i0] = m;
                    pieces.remove(i1);
                    if sp.closed && i1 == 0 {
                        // The merged segment now starts the loop.
                        if let Some(last) = pieces.pop() {
                            pieces.insert(0, last);
                        }
                    }
                    changed = true;
                    break;
                }
            }
        }
        let segs: Vec<Seg> = pieces.iter().map(|p| p.seg).collect();
        if let Some(s) = segs_to_bezpath(&segs, sp.closed) {
            out.extend(s.iter());
        }
    }
    out
}

/// Most work [`remove_redundant_points`] does in all, besides the pass along each subpath that merges
/// what needs no check: the joints it reads in its search for what that pass left, the points of
/// a run it checks one by one against the line that replaces it, and the points a run of curves is
/// refitted from (each as much as 24 of those, for it takes as long as that many points checked against a
/// line); a merge that needs the check and finds none left is not made: 67,108,864 (the default of
/// [`crate::Limits::redundant_work`]).
pub const REDUNDANT_WORK: usize = 1 << 26;

/// A segment standing for a run of the segments of the subpath it was made from.
#[derive(Clone, Copy)]
struct Piece {
    seg: Seg,
    /// A bound on how far the segments of the run lie from `seg` (0 for one that stands for itself).
    dev: f64,
    /// The first of the run, as an index into the subpath's own segments (counting round a closed
    /// subpath's start), and how many it has.
    first: usize,
    count: usize,
}

/// Most original joints checked one by one against a line when the bound on the run has passed
/// the tolerance; a longer run is not merged then.
const MAX_CHECKED_RUN: usize = 512;

/// The original segments of a subpath, and the lengths of their control polygons that a run of them
/// is held to, made when first asked for.
struct Originals<'a> {
    segs: &'a [Seg],
    polygons: std::cell::OnceCell<Vec<f64>>,
    corners: std::cell::OnceCell<Vec<usize>>,
}

impl<'a> Originals<'a> {
    fn new(segs: &'a [Seg]) -> Self {
        Self { segs, polygons: std::cell::OnceCell::new(), corners: std::cell::OnceCell::new() }
    }

    /// The first corner at or after the joint `from`, as the index of the joint: the joint `j` is between
    /// segment `j - 1` and segment `j`, and is a corner when [`smooth_joint`] refuses it. The number of
    /// segments when there is none.
    fn corner_from(&self, from: usize) -> usize {
        let n = self.segs.len();
        let corners = self.corners.get_or_init(|| {
            work::add(Work::CornerTables, 1);
            let mut firsts = vec![n; n + 1];
            let mut first = n;
            for j in (1..n).rev() {
                if let (Some(a), Some(b)) = (self.segs.get(j - 1), self.segs.get(j))
                    && !smooth_joint(&a.c, &b.c)
                {
                    first = j;
                }
                if let Some(slot) = firsts.get_mut(j) {
                    *slot = first;
                }
            }
            if let Some(slot) = firsts.get_mut(0) {
                *slot = first;
            }
            firsts
        });
        corners.get(from).copied().unwrap_or(n)
    }

    /// The segment `i` places on from the first, counting round a closed subpath's start.
    fn seg(&self, i: usize) -> Option<&Seg> {
        self.segs.get(i.checked_rem(self.segs.len())?)
    }

    /// At least the length of the `count` segments from `first` (counting round the start), but for rounding
    /// ([`LENGTH_ROUNDING`]): the lengths of their control polygons, which a curve is no longer than. They are
    /// added up from the lengths themselves, not taken from running totals, so that the sum is as exact as they
    /// are (it is the length of a short stretch of a long subpath that a run is held to), and with the rounding
    /// of the additions compensated, so that it is off by the same share however many there are.
    fn polygon_length(&self, first: usize, count: usize) -> Option<f64> {
        let lens = self.polygons.get_or_init(|| self.segs.iter().map(|s| polygon_length(&s.c)).collect());
        let n = lens.len();
        let start = first.checked_rem(n)?;
        let end = start.checked_add(count)?;
        if end <= n {
            Some(compensated_sum(lens.get(start..end)?.iter().copied()))
        } else {
            Some(compensated_sum(lens.get(start..)?.iter().chain(lens.get(..end - n)?.iter()).copied()))
        }
    }
}

/// The joint between `a` and the cubic `b` after it is smooth enough to merge across: the tangents agree
/// to within about 8 degrees. A joint that is not is a corner, and stays one.
fn smooth_joint(a: &CubicBez, b: &CubicBez) -> bool {
    end_tangent(a).dot(start_tangent(b)) >= 0.99
}

/// Fewest and most intervals of a run that it is refitted from ([`refit_run`]): the points taken are one
/// more than the intervals, and where the run has fewer segments than the intervals every segment is cut
/// in equal steps, which can bring the points to 2,047 (1,023 segments, each cut in two).
const MIN_WITNESSES: usize = 8;
const MAX_WITNESSES: usize = 1 << 10;

/// The share of the tolerance that the distance between those points is meant to leave, for each of the
/// two ways a run and its cubic can be from the chords between them (the cubic's own, and the run's).
const WITNESS_SLACK: f64 = 0.02;

/// What a point of a refit is charged to the work: a fit takes as long as this many points read
/// against a line.
const WITNESS_COST: usize = 24;

/// A piece of this many original segments or more is extended in strides ([`extend_run`]).
const GALLOP_FROM: usize = 8;

/// Half the width of the lens a path no longer than `along` lies in, from one end of a chord of length
/// `chord` to the other (an ellipse with the ends as its foci): no point of the path is further than this
/// from the chord. `along` is the sum of the lengths of control polygons ([`Originals::polygon_length`]) and
/// `chord` one rounded length, and the path may be longer, and the chord shorter, by what rounding took
/// ([`LENGTH_ROUNDING`] of each), so that where the two come out equal (a shallow curve with long control
/// edges, whose edges are longer than its chord by less than a unit in the last place of each, can come out as
/// long as it) the width is the least that rounding allows and not 0. Infinite where a length is not a number,
/// or where the width is too big to be one.
fn lens_width(along: f64, chord: f64) -> f64 {
    if !along.is_finite() || !chord.is_finite() {
        return f64::INFINITY;
    }
    let (along, chord) = (along * (1.0 + LENGTH_ROUNDING), chord * (1.0 - LENGTH_ROUNDING));
    // The difference of the squares as a product, which does not overflow where the squares do (past 1e154).
    let width = 0.5 * ((along - chord).max(0.0) * (along + chord)).sqrt();
    if width.is_nan() { f64::INFINITY } else { width }
}

/// What rounding may take of the length of a polygon, and of a chord, as a share of it: four units of rounding
/// ([`f64::EPSILON`]). The length of one segment's polygon is three distances, each a difference of coordinates
/// (rounded) and a hypotenuse (rounded), under two units; the three added up in order are off by one more (half a unit
/// for each addition after the first), so three at most. A run's polygon is those added up with their rounding
/// compensated ([`compensated_sum`]), which adds half a unit of the whole however many there are, where a sum
/// in order adds half a unit for each (a run of 200 thousand segments, whose lens was a hundred times its tolerance
/// wide at 8 points, was never merged whole). The chord is one distance: two.
const LENGTH_ROUNDING: f64 = 4.0 * f64::EPSILON;

/// The sum of `values` with the rounding of its additions compensated (Neumaier's): off by half a unit of rounding of the
/// sum, and a term in the square of that unit times the number of values, however many they are.
fn compensated_sum(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut lost) = (0.0f64, 0.0f64);
    for x in values {
        let t = sum + x;
        lost += if sum.abs() >= x.abs() { (sum - t) + x } else { (x - t) + sum };
        sum = t;
    }
    sum + lost
}

/// How many intervals to take of a run that a cubic with the second difference `sd` is to replace: those
/// for which the distance of that cubic from its chords over them is about [`WITNESS_SLACK`] of `tol`.
fn witnesses_for(sd: f64, tol: f64) -> usize {
    let wanted = 1.25 * (0.75 * sd / (WITNESS_SLACK * tol)).sqrt();
    (wanted.ceil() as usize).clamp(MIN_WITNESSES, MAX_WITNESSES)
}

/// The share of the tolerance that the lens of one interval between two witness points may be (see [`lens_floor`]).
const LENS_SHARE: f64 = 0.25;

/// The fewest intervals of a run whose control polygons add up to `along`, taken as far apart as rounding lets a run be
/// shown to be within `tol` of its chord: a stretch as long as its chord to the last place lies in a lens whose half width is
/// still `sqrt(LENGTH_ROUNDING)` of its length ([`lens_width`]), so the stretches are no longer than a share of the
/// tolerance over that. A run that is straight needs as many witness points as that and no more than its curvature asks
/// for, which is the eight of [`MIN_WITNESSES`]: 100,000 straight cubics 1e4 long had a lens of 0.014 in each of eight
/// stretches of 4.7e5, more than the tolerance of 0.01, and were never merged whole.
fn lens_floor(along: f64, tol: f64) -> usize {
    let longest = LENS_SHARE * tol / LENGTH_ROUNDING.sqrt();
    if !(along.is_finite() && longest.is_finite() && longest > 0.0) {
        return 0;
    }
    (along / longest).ceil() as usize
}

/// How many points [`witness_points`] takes for `steps` intervals of a run of `count` segments.
fn points_for(count: usize, steps: usize) -> usize {
    if count >= steps { steps } else { count.saturating_mul(steps.div_ceil(count.max(1))) }
}

/// The points taken of a run for a refit: `steps` intervals of its segments, at their joints, or where
/// there are fewer segments than that, each segment in as many equal steps of its parameter as bring the
/// intervals to `steps` at least. The points are in order along the run, the first at its start and the last
/// at its end, and `gap` is how far the run can be from the chords between them: the run between two
/// of them is no longer than the control polygons of its segments (the parts of them, for the ends of
/// the stretch inside a segment), so none of its points is further than half the width of the lens that
/// length allows ([`lens_width`]) from their chord, and every point of the chord has a point of the
/// run within that. Where the run is smooth and its segments are short that width is small, whatever
/// the length of the run.
fn witness_points(originals: &Originals, first: usize, count: usize, steps: usize) -> Option<(Vec<Point>, f64)> {
    if count == 0 || steps == 0 {
        return None;
    }
    let mut pts = vec![originals.seg(first)?.c.p0];
    let mut gap = 0.0f64;
    if count >= steps {
        let mut from = 0;
        for k in 1..=steps {
            let to = k.saturating_mul(count) / steps;
            let p = if to < count { originals.seg(first.checked_add(to)?)?.c.p0 } else { originals.seg(first.checked_add(count - 1)?)?.c.p3 };
            let along = originals.polygon_length(first.checked_add(from)?, to - from)?;
            gap = gap.max(lens_width(along, pts.last()?.distance(p)));
            pts.push(p);
            from = to;
        }
    } else {
        let per = steps.div_ceil(count);
        for r in 0..count {
            let c = originals.seg(first.checked_add(r)?)?.c;
            for j in 1..=per {
                let (from, to) = ((j - 1) as f64 / per as f64, j as f64 / per as f64);
                let p = if j == per { c.p3 } else { c.eval(to) };
                gap = gap.max(lens_width(polygon_length(&c.subsegment(from..to)), pts.last()?.distance(p)));
                pts.push(p);
            }
        }
    }
    Some((pts, gap))
}

/// The one cubic replacing the `count` original segments from `first`, with how far it and they can be
/// from each other, or `None` when that is not known: the work ran out, or a length was not a number.
/// The cubic is fitted to points of the run ([`witness_points`]) with the tangents the run has at its
/// ends, and the bound is [`fit_single_bounded`]'s: its error at them, its distance from the chords
/// between its values there, and the run's from the chords between the points. Nothing of how the run
/// was made comes into it, so it does not grow with the number of segments or of merges. `seed` is a
/// guess at the second difference of the cubic, for the number of points; they are taken again when the
/// cubic needs more. Each point is charged to `work` ([`WITNESS_COST`]) before the fit.
fn refit_run(originals: &Originals, first: usize, count: usize, tol: f64, seed: f64, work: &mut usize) -> Option<(CubicBez, f64)> {
    if count == 0 {
        return None;
    }
    let (head, tail) = (originals.seg(first)?, originals.seg(first.checked_add(count - 1)?)?);
    let (t0, t1) = (start_tangent(&head.c), end_tangent(&tail.c));
    // The least intervals that keep the lens of each within a share of the tolerance, however straight the run is.
    let floor = lens_floor(originals.polygon_length(first, count)?, tol);
    let mut sd = seed;
    for round in 0..2 {
        let steps = witnesses_for(sd, tol).max(floor.min(MAX_WITNESSES));
        let points = points_for(count, steps);
        let cost = points.saturating_add(1).saturating_mul(WITNESS_COST).saturating_add(count / 4);
        let Some(left) = work.checked_sub(cost) else {
            *work = 0;
            return None;
        };
        *work = left;
        work::add(Work::Witnesses, points + 1);
        let (pts, gap) = witness_points(originals, first, count, steps)?;
        let (c, err, rest) = fit_single_bounded(&pts, t0, t1, gap);
        let bound = err + rest;
        let wanted = witnesses_for(second_difference(&c), tol);
        if wanted <= steps + steps / 4 || round == 1 {
            return bound.is_finite().then_some((c, bound));
        }
        sd = second_difference(&c);
    }
    None
}

/// The piece `top` made longer by the original segments that follow its run, at most `ahead` of them, as
/// many as it can take, and how many it took: the number is doubled while the run still fits one
/// cubic ([`refit_run`]), and then halved between the last that fitted and the first that did not (the
/// run is taken to fit one cubic for a length up to a point, and not past it). A piece made by merging one
/// original at a time costs a merge for each; this costs a few for the whole stretch. `ahead` stops short
/// of the first corner after the piece (a joint [`merge_pair`] refuses): a refit sees only that the cubic
/// is within the tolerance of the run, and would round off a corner that is.
fn extend_run(top: &Piece, ahead: usize, tol: f64, originals: &Originals, work: &mut usize) -> Option<(Piece, usize)> {
    let sd = second_difference(&top.seg.c);
    let mut try_extra = |extra: usize| -> Option<Piece> {
        let count = top.count.checked_add(extra)?;
        let ratio = count as f64 / top.count as f64;
        let (c, dev) = refit_run(originals, top.first, count, tol, sd * ratio * ratio, work)?;
        (dev <= tol).then_some(Piece { seg: Seg { c, line: false }, dev, first: top.first, count })
    };
    let (mut best, mut fits, mut stride) = (None, 0usize, 1usize);
    let mut fails = None;
    while stride <= ahead {
        match try_extra(stride) {
            Some(piece) => {
                best = Some((piece, stride));
                fits = stride;
                stride = stride.saturating_mul(2);
            }
            None => {
                fails = Some(stride);
                break;
            }
        }
    }
    if fails.is_none() && fits < ahead {
        match try_extra(ahead) {
            Some(piece) => return Some((piece, ahead)),
            None => fails = Some(ahead),
        }
    }
    if let Some(mut fail) = fails {
        while fail - fits > 1 {
            let mid = fits + (fail - fits) / 2;
            match try_extra(mid) {
                Some(piece) => {
                    best = Some((piece, mid));
                    fits = mid;
                }
                None => fail = mid,
            }
        }
    }
    best
}

/// `a` and `b`, next to each other, as one piece, if the one segment that replaces them stays within
/// `tol` of every segment of the run both stand for: not only of the joint between them, which
/// is all [`merge_pair`] sees (a run built a joint at a time can drift from a curve it was sampled
/// from, each step within `tol` of the last: a half circle of ten thousand chords came out as its
/// diameter). A bound is kept: the segments of `a` and `b` are within `a.dev` and `b.dev` of them, and
/// them from the merged one by that and the step of the merge at most.
///
/// That bound is the sum of the worst case of every merge the run was made by, and a long run, made
/// of many short segments, passes it after a few dozen (each step is a share of the tolerance, however
/// short the segments are that it merges) though it is far from using the tolerance up. So when
/// it passes `tol` the run itself is held to the segment that replaces it, and the distance found
/// replaces the bound. For lines, the joints of the run are checked against the line (up to
/// [`MAX_CHECKED_RUN`]); for curves the run is refitted from points of its original segments
/// ([`refit_run`]), however long it is. `originals` are the segments of the subpath.
fn merge_pieces(a: &Piece, b: &Piece, tol: f64, originals: &Originals, work: &mut usize) -> Option<Piece> {
    let (seg, step) = merge_pair(&a.seg, &b.seg, tol)?;
    let (first, count) = (a.first, a.count + b.count);
    let bound = a.dev.max(b.dev) + step;
    if bound <= tol {
        return Some(Piece { seg, dev: bound, first, count });
    }
    if !seg.line {
        // A cubic of a run is as long, so as curved, as the longer of its parts at their own scale.
        let grown = |p: &Piece| {
            let ratio = count as f64 / p.count as f64;
            second_difference(&p.seg.c) * ratio * ratio
        };
        let (c, dev) = refit_run(originals, first, count, tol, grown(a).max(grown(b)), work)?;
        return (dev <= tol).then_some(Piece { seg: Seg { c, line: false }, dev, first, count });
    }
    if count > MAX_CHECKED_RUN {
        return None;
    }
    // The joints inside the run are the starts of its segments but the first. They are read from the
    // same work as the search for joints to merge; with none left the run is not merged.
    let Some(left) = work.checked_sub(count) else {
        *work = 0;
        return None;
    };
    *work = left;
    work::add(Work::RunChecks, count);
    let chord = seg.c.p3 - seg.c.p0;
    let len2 = chord.hypot2();
    let mut worst: f64 = 0.0;
    for k in 1..count {
        let p = originals.seg(first.checked_add(k)?)?.c.p0;
        let along = if len2 > 0.0 { ((p - seg.c.p0).dot(chord) / len2).clamp(0.0, 1.0) } else { 0.0 };
        worst = worst.max(p.distance(seg.c.p0 + chord * along));
    }
    (worst <= tol).then_some(Piece { seg, dev: worst, first, count })
}

/// Intervals each of two segments is sampled at to merge them into one cubic ([`merge_pair`]).
const PAIR_INTERVALS: usize = 16;

/// The one segment replacing `a` and `b` (next to each other) and how far from them it can be: for lines,
/// the joint's distance from the chord (a line that is further is none); for curves, the bound of
/// [`fit_single_bounded`] over samples of both, which can pass `tol` (a cubic whose own error at the
/// samples passes it is none).
fn merge_pair(a: &Seg, b: &Seg, tol: f64) -> Option<(Seg, f64)> {
    if a.line && b.line {
        let chord = b.c.p3 - a.c.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let mid = a.c.p3 - a.c.p0;
        let along = mid.dot(chord) / len;
        let off = (mid.cross(chord) / len).abs();
        return (off <= tol && along >= 0.0 && along <= len).then(|| (Seg::line(a.c.p0, b.c.p3), off));
    }
    // Only merge across (near-)smooth joints.
    if !smooth_joint(&a.c, &b.c) {
        return None;
    }
    // Held to the tolerance between the samples too: the cubic's error at them, how far it can be from
    // its chords between them, and how far `a` and `b` can be from theirs, added up. Each is sampled at
    // 16 intervals, as VectorCraft samples them; where those leave more than the tolerance has room for,
    // the run is refitted ([`merge_pieces`]) rather than sampled more finely here.
    let mut pts = Vec::with_capacity(2 * PAIR_INTERVALS + 1);
    sample(&a.c, PAIR_INTERVALS, &mut pts, true);
    sample(&b.c, PAIR_INTERVALS, &mut pts, false);
    let between = sample_error(a, PAIR_INTERVALS, tol * 0.05).max(sample_error(b, PAIR_INTERVALS, tol * 0.05));
    let (c, err, rest) = fit_single_bounded(&pts, start_tangent(&a.c), end_tangent(&b.c), between);
    (err <= tol).then_some((Seg { c, line: false }, err + rest))
}

/// Join. Open subpaths of all inputs are joined nearest-endpoint-first into one open path
/// (endpoints closer than `tolerance` are merged into a single anchor, otherwise a straight segment
/// connects them). A single open subpath is closed instead: its ends become one anchor when they are
/// within `tolerance` and it has two segments or more, or is one cubic that loops round an area, and
/// otherwise a straight segment closes it. Closed subpaths pass through, in the form
/// of a closed subpath that ends with a segment back to its start.
///
/// More than [`crate::MAX_ELEMENTS`] elements are not joined: the subpaths come back as they are.
/// The work of the rest is counted: an endpoint distance measured, and a segment of a chain
/// reversed, moved or appended each cost one, and past [`JOIN_WORK`] the subpaths left are not
/// joined either.
pub fn join(paths: &[BezPath], tolerance: f64) -> BezPath {
    join_within(paths, tolerance, crate::budget::limits().join_work)
}

/// [`join`] within `budget` work.
fn join_within(paths: &[BezPath], tolerance: f64, budget: usize) -> BezPath {
    if !fits_elements(paths) {
        let mut all = BezPath::new();
        for p in paths {
            all.extend(p.iter());
        }
        return all;
    }
    let mut closed = BezPath::new();
    let mut open: Vec<Vec<Seg>> = Vec::new();
    for p in paths {
        for sub in subs(p) {
            if sub.closed {
                sub.push_closed(&mut closed);
            } else {
                open.push(sub.chain());
            }
        }
    }
    if open.len() == 1
        && let Some(mut segs) = open.pop()
    {
        // A path of one segment closes onto its start only when it is a cubic whose handles take it
        // round an area: a line, or a cubic along one, would close onto nothing (a point, or a line
        // out and back), so it gets a line back instead.
        let one_loop = |s: &Seg| !s.line && crate::path::handles_loop(s.c.p0, s.c.p1, s.c.p2);
        let loops = segs.len() >= 2 || segs.first().is_some_and(one_loop);
        let ends_meet = loops && ends(&segs).is_some_and(|(a, b)| a.distance(b) <= tolerance);
        if ends_meet {
            // The last segment arrives at the first point.
            let start = segs.first().map(|s| s.c.p0);
            if let (Some(last), Some(start)) = (segs.last_mut(), start) {
                last.set_end(start);
            }
        } else if let Some((end, start)) = ends(&segs).map(|(start, end)| (end, start)) {
            segs.push(Seg::line(end, start));
        }
        closed.extend(segs_to_bezpath(&segs, true).iter().flat_map(BezPath::iter));
        return closed;
    }
    let mut work = 0usize;
    while open.len() > 1 {
        // Find the closest pair of endpoints on different subpaths. Past this many subpaths every
        // pair at every merge is too much (cubic): the first one is joined to its nearest.
        const SEARCH_ALL: usize = 200;
        let first_only = open.len() > SEARCH_ALL;
        let scan = if first_only { 4 * open.len() } else { 2 * open.len() * open.len() };
        if work.saturating_add(scan) > budget {
            break;
        }
        work += scan;
        work::add(Work::Join, scan);
        let mut best = (f64::INFINITY, 0, 0, false, false);
        for i in 0..if first_only { 1 } else { open.len() } {
            for j in (i + 1)..open.len() {
                for ei in [false, true] {
                    for ej in [false, true] {
                        let (Some(pi), Some(pj)) = (end_point(&open[i], ei), end_point(&open[j], ej)) else { continue };
                        let d = pi.distance(pj);
                        if d < best.0 {
                            best = (d, i, j, ei, ej);
                        }
                    }
                }
            }
        }
        let (d, i, j, ei, ej) = best;
        // What the merge moves: the chains reversed and appended, and the subpaths shifted down.
        let moved = open.get(i).map_or(0, Vec::len).saturating_add(open.get(j).map_or(0, Vec::len));
        if work.saturating_add(moved) > budget {
            break;
        }
        work += moved;
        work::add(Work::Join, moved);
        let mut b = open.remove(j);
        let a = &mut open[i];
        if !ei {
            reverse_chain(a);
        }
        if ej {
            reverse_chain(&mut b);
        }
        if let (Some(end), Some(first)) = (end_point(a, true), b.first_mut()) {
            if d <= tolerance {
                // The two ends are one anchor, at the end of `a`.
                first.set_start(end);
            } else {
                // A straight segment connects them.
                a.push(Seg::line(end, first.c.p0));
            }
        }
        a.extend(b);
    }
    for segs in open {
        closed.extend(segs_to_bezpath(&segs, false).iter().flat_map(BezPath::iter));
    }
    closed
}

/// Most work one [`join`] does in all: 67,108,864 (the default of [`crate::Limits::join_work`]). A release build takes under half a second for it.
pub const JOIN_WORK: usize = 1 << 26;

/// The first (`last == false`) or last point of a chain.
fn end_point(segs: &[Seg], last: bool) -> Option<Point> {
    if last { segs.last().map(|s| s.c.p3) } else { segs.first().map(|s| s.c.p0) }
}

/// The first and last points of a chain.
fn ends(segs: &[Seg]) -> Option<(Point, Point)> {
    end_point(segs, false).zip(end_point(segs, true))
}

/// The chain run the other way.
fn reverse_chain(segs: &mut [Seg]) {
    segs.reverse();
    for s in segs.iter_mut() {
        s.c = CubicBez::new(s.c.p3, s.c.p2, s.c.p1, s.c.p0);
    }
}

#[cfg(test)]
mod vc_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{anchor_count, polyline, subpath_count};

    /// A closed polygon of `n` sides round a circle.
    fn circle_polygon(n: usize, r: f64) -> BezPath {
        let pts: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                (r * a.cos(), r * a.sin())
            })
            .collect();
        polyline(&pts, true)
    }

    /// The largest distance from a probe of `a` to the outline of `b`. The probes are the start of each
    /// segment and 98 more, at parameters that are no multiple of a power of two's reciprocal (a sample
    /// of the simplification at 1/256 of a segment is a probe of a grid of 64 or 256, and is on the outline).
    fn deviation(a: &BezPath, b: &BezPath) -> f64 {
        use kurbo::{ParamCurve, ParamCurveNearest};
        let mut worst: f64 = 0.0;
        for seg in a.segments() {
            for t in std::iter::once(0.0).chain((0..=97).map(|k| (f64::from(k) + 0.37).min(97.0) / 97.0)) {
                let p = seg.eval(t);
                let d = b.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
                worst = worst.max(d);
            }
        }
        worst
    }

    /// More work than a test spends, as a number a 32-bit `usize` holds (the work is charged to a `usize`).
    const BIG_WORK: usize = 1 << 30;

    /// How far past the tolerance a measured deviation may be: the distances are computed at
    /// coordinates of 500,000, whose rounding is 1.2e-10 (a few of those, and the nearest point found
    /// on a curve to 1e-9 of its parameter, which moves a distance to it by the square of that).
    const MEASURED: f64 = 1e-9;

    /// A curve too big to measure the length of is not sampled as if it were a point: its run stays
    /// as it is, rather than becoming a few chords far from it.
    #[test]
    fn a_curve_too_big_to_measure_stays_as_it_is() {
        let r = 5e9;
        let k = 0.552_284_749_830_793_6 * r;
        let mut quarter = BezPath::new();
        quarter.move_to((r, 0.0));
        quarter.curve_to((r, k), (k, r), (0.0, r));
        let opts = SimplifyOptions::default().tolerance(1.0).straight_lines(true);
        let simpler = simplify_with(&quarter, &opts);
        assert_eq!(simpler, quarter, "the run stays as it is");
        // One that can be measured is simplified, as ever: to lines, within the tolerance both ways.
        let r = 5e5;
        let k = 0.552_284_749_830_793_6 * r;
        let mut small = BezPath::new();
        small.move_to((r, 0.0));
        small.curve_to((r, k), (k, r), (0.0, r));
        let simpler = simplify_with(&small, &opts);
        assert!(simpler.segments().count() > 1, "{simpler:?}");
        assert!(simpler.segments().all(|s| matches!(s, PathSeg::Line(_))), "only lines: {simpler:?}");
        let tol = opts.tolerance;
        assert!(deviation(&small, &simpler) <= tol + MEASURED, "the curve is {} from the result", deviation(&small, &simpler));
        // The chords, between the samples they are drawn from, are as near the curve.
        assert!(deviation(&simpler, &small) <= tol + MEASURED, "the result is {} from the curve", deviation(&simpler, &small));
    }

    /// The most chords of a curve are as far from it as `chord_intervals` says they can be, at the
    /// number of intervals it asks for; a line, or a curve that is nearly one, needs none of the extra.
    #[test]
    fn the_intervals_that_keep_the_chords_of_a_curve_near_it_are_enough() {
        use kurbo::{ParamCurve, ParamCurveNearest};
        let quarter = |r: f64| {
            let k = 0.552_284_749_830_793_6 * r;
            CubicBez::new((r, 0.0), (r, k), (k, r), (0.0, r))
        };
        let s_curve = CubicBez::new((0.0, 0.0), (300.0, 0.0), (-200.0, 100.0), (100.0, 100.0));
        for (c, within) in [(quarter(5e5), 0.1), (quarter(1e4), 0.01), (s_curve, 0.001)] {
            let m = chord_intervals(&c, within).expect("within the most intervals");
            let mut worst: f64 = 0.0;
            for i in 0..m {
                let (a, b) = (c.eval(i as f64 / m as f64), c.eval((i + 1) as f64 / m as f64));
                // The point of the chord at its middle and a quarter along.
                for f in [0.25, 0.5, 0.75] {
                    let p = a.lerp(b, f);
                    worst = worst.max(c.nearest(p, 1e-9).distance_sq.sqrt());
                }
            }
            assert!(worst <= within, "{m} intervals leave a chord {worst} from the curve, over {within}");
        }
        // The largest a segment can be measured at (a coordinate of 1e9) is within the most, at a tolerance of 1.
        assert!(chord_intervals(&quarter(1e9), 0.1).is_some_and(|m| m > MAX_INTERVALS));
        // A curve that needs more than the most is none: the run stays.
        assert_eq!(chord_intervals(&quarter(2e6), 1e-4), None);
        // A line written as a cubic has second differences that are not 0 and would need more than the
        // most, but its chords are on it: the run does not ask (a long straight run is still one line).
        let line = Seg::line(Point::new(0.0, 0.0), Point::new(1e9, 0.0)).c;
        assert!(is_straight(&line, 1e-3) && chord_intervals(&line, 1e-4).is_none());
        let long = polyline(&[(0.0, 0.0), (5e8, 0.0), (1e9, 0.0)], false);
        let opts = SimplifyOptions::default().tolerance(0.5).straight_lines(true);
        assert_eq!(anchor_count(&simplify_with(&long, &opts)), 2);
    }

    /// A short curl, sampled at the intervals its length asks for (four), can be further from the chords
    /// of its samples than a tenth of the tolerance: it is sampled more, as a long curve is, so that what
    /// the reduction takes off the tolerance for it is small and the result is within the tolerance.
    #[test]
    fn a_short_curl_is_sampled_as_finely_as_its_chords_need() {
        use kurbo::{ParamCurve, ParamCurveNearest};
        let c = CubicBez::new((0.0, 0.0), (3.0, 2.0), (-2.0, 2.0), (1.0, 0.0));
        let tol = 3.0;
        let curl = Seg { c, line: false };
        assert!(sample_error(&curl, 4, tol * 0.05) > tol * 0.1, "{}", sample_error(&curl, 4, tol * 0.05));
        let mut path = BezPath::new();
        path.move_to(c.p0);
        path.curve_to(c.p1, c.p2, c.p3);
        work::reset();
        let simpler = simplify_with(&path, &SimplifyOptions::default().tolerance(tol).straight_lines(true));
        let samples = work::take(Work::FitSamples);
        assert!(samples > 5, "{samples} samples: more than the five of four intervals");
        let worst = (0..=1000)
            .map(|k| {
                let p = c.eval(f64::from(k) / 1000.0);
                simpler.segments().map(|s| s.nearest(p, 1e-12).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
            })
            .fold(0.0, f64::max);
        assert!(worst <= tol, "{worst} from the result");
    }

    /// The polyline reduction is charged before it scans: with little of the fit work left after the
    /// sampling it reads no more than that, and the run stays as it is.
    #[test]
    fn the_reduction_of_a_polyline_reads_no_more_than_the_work_that_is_left() {
        let circle = circle_polygon(3_000, 100.0);
        let opts = SimplifyOptions::default().tolerance(0.05).straight_lines(true);
        work::reset();
        let full = simplify_with(&circle, &opts);
        let samples = work::take(Work::FitSamples);
        let free_reads = work::take(Work::RdpReads);
        assert!(anchor_count(&full) < 3_000 && free_reads > 10_000, "{} anchors, {free_reads} reads", anchor_count(&full));
        // Room for the samples and 200 more reads.
        let mut limits = crate::Limits { fit_work: samples + 200, ..crate::Limits::default() };
        work::reset();
        let starved = crate::with_limits(limits, || simplify_with(&circle, &opts));
        let reads = work::take(Work::RdpReads);
        assert!(reads <= 200, "{reads} points read with 200 of the work left");
        assert!(work::take(Work::Fit) <= limits.fit_work, "the work charged passes the limit");
        assert_eq!(starved, circle, "the run is left as it is");
        // And exactly enough is enough.
        limits.fit_work = samples + free_reads;
        assert_eq!(crate::with_limits(limits, || simplify_with(&circle, &opts)), full);
    }

    /// Points that lie about a straight line, each within the tolerance of it, are one line (the run is
    /// checked against its line when the bound on its deviation has passed the tolerance), and a long
    /// run of them costs a bounded number of checks for each point, not one for each of the run.
    #[test]
    fn a_noisy_straight_line_within_the_tolerance_is_one_line_and_costs_little() {
        let line = |n: usize, noise: f64| {
            let pts: Vec<(f64, f64)> = (0..=n).map(|i| (i as f64, if i % 2 == 0 { noise } else { -noise })).collect();
            polyline(&pts, false)
        };
        let one = remove_redundant_points(&line(200, 0.0004), 0.001);
        assert_eq!(anchor_count(&one), 2, "{} anchors", anchor_count(&one));
        // Noise past the tolerance is kept.
        assert!(anchor_count(&remove_redundant_points(&line(200, 0.004), 0.001)) > 100);
        // A line of many points: the checks stay in proportion.
        let checks = |n: usize| {
            work::reset();
            let l = remove_redundant_points(&line(n, 0.0004), 0.001);
            assert!(anchor_count(&l) <= n / 400 + 3, "{} anchors for {n} points", anchor_count(&l));
            work::take(Work::RunChecks)
        };
        let (small, large) = (checks(4_000), checks(16_000));
        assert!(small > 0 && large <= 5 * small, "{small} checks for 4,000 points, {large} for 16,000");
        assert!(large <= 16_000 * 512, "{large} checks");
    }

    /// The points checked one by one against the line that replaces a run are charged to the same work
    /// as the joints read: with 1,000 of it, the two together are at most 1,000, in the first pass and
    /// in the search that follows (a closed noisy ring merges across its start there), and nothing is
    /// merged that needed a check past it.
    #[test]
    fn the_run_checks_of_the_merges_are_charged_to_the_work() {
        let noisy = |n: usize, closed: bool| {
            let mut pts: Vec<(f64, f64)> = (0..=n).map(|i| (i as f64, if i % 2 == 0 { 0.0004 } else { -0.0004 })).collect();
            if closed {
                pts.extend([(n as f64, 100.0), (0.0, 100.0)]);
            }
            polyline(&pts, closed)
        };
        for closed in [false, true] {
            let path = noisy(16_000, closed);
            work::reset();
            let free = remove_redundant_points(&path, 0.001);
            let (free_checks, free_joints) = (work::take(Work::RunChecks), work::take(Work::Redundant));
            assert!(free_checks > 100_000, "{free_checks} checks without a limit (closed: {closed})");
            for limit in [0usize, 1_000, 50_000] {
                let limits = crate::Limits { redundant_work: limit, ..crate::Limits::default() };
                work::reset();
                let cut = crate::with_limits(limits, || remove_redundant_points(&path, 0.001));
                let (checks, joints) = (work::take(Work::RunChecks), work::take(Work::Redundant));
                assert!(checks + joints <= limit, "{checks} checks and {joints} joints read with {limit} of work (closed: {closed})");
                // What needed no check is merged all the same, and what did is left, not lost.
                assert!(anchor_count(&cut) >= anchor_count(&free));
            }
            // Room for all of it gives the same result.
            let limits = crate::Limits { redundant_work: free_checks + free_joints, ..crate::Limits::default() };
            assert_eq!(crate::with_limits(limits, || remove_redundant_points(&path, 0.001)), free);
        }
    }

    /// The same for points that lie on a straight line exactly: the bound passes, and nothing is checked
    /// one by one.
    #[test]
    fn collinear_points_are_one_line_without_checking_them_one_by_one() {
        let pts: Vec<(f64, f64)> = (0..=20_000).map(|i| (f64::from(i), f64::from(i) * 0.5)).collect();
        work::reset();
        let l = remove_redundant_points(&polyline(&pts, false), 0.001);
        assert_eq!(anchor_count(&l), 2);
        assert_eq!(work::take(Work::RunChecks), 0);
    }

    /// A line of `chain` tiny steps from (0, 0) to (1, 0), and `others` short lines at either end of
    /// it, each farther out than the last: every merge takes the whole chain along.
    fn chain_and_others(chain: usize, others: usize) -> Vec<BezPath> {
        let mut long = BezPath::new();
        long.move_to((0.0, 0.0));
        for i in 1..=chain {
            long.line_to((i as f64 / chain as f64, 0.0));
        }
        let mut paths = vec![long];
        let (mut left, mut right) = (0.0f64, 1.0f64);
        for k in 0..others {
            let gap = 10.0 + k as f64;
            let x = if k % 2 == 0 {
                left -= gap;
                left
            } else {
                right += gap;
                right
            };
            paths.push(polyline(&[(x, 0.0), (x, 0.5)], false));
        }
        paths
    }

    #[test]
    fn join_with_room_joins_everything_into_one_path() {
        let all = join_within(&chain_and_others(3_000, 40), 0.0, usize::MAX);
        assert_eq!(subpath_count(&all), 1);
    }

    /// Reversing a long chain at every merge is work: a chain of 3,000 segments and 40 short lines
    /// at either end, each merge taking the whole chain along (it was uncounted, and 4,096 such
    /// lines took seconds).
    #[test]
    fn join_counts_the_chains_it_moves_and_stops_at_its_budget() {
        let paths = chain_and_others(3_000, 40);
        let budget = 40_000;
        work::reset();
        let out = join_within(&paths, 0.0, budget);
        let counted = work::take(Work::Join);
        let left = subpath_count(&out);
        let merges = 41 - left;
        assert!(counted <= budget, "{counted} of {budget}");
        assert!((1..40).contains(&merges), "{merges} merges");
        // Each merge moved the chain, 3,000 segments or more.
        assert!(counted >= 3_000 * merges, "{counted} counted for {merges} merges of a 3,000 segment chain");
        // Nothing was lost on the way.
        let length = |p: &BezPath| crate::testutil::length(p);
        let total: f64 = paths.iter().map(length).sum();
        assert!(length(&out) >= total, "{} of {total}", length(&out));
    }

    /// Endpoint distances are charged: past 200 open subpaths the nearest is searched from the first only.
    #[test]
    fn join_counts_its_searches() {
        let lines: Vec<BezPath> = (0..150).map(|i| polyline(&[(f64::from(i) * 2.0, 0.0), (f64::from(i) * 2.0 + 1.0, 0.0)], false)).collect();
        work::reset();
        let _ = join_within(&lines, 0.0, 100_000);
        assert!(work::take(Work::Join) <= 100_000);
        work::reset();
        let _ = join_within(&lines, 0.0, usize::MAX);
        assert!(work::take(Work::Join) > 100_000, "joining all of them takes more");
    }

    #[test]
    fn a_run_too_long_for_one_fit_is_fitted_in_parts() {
        let circle = circle_polygon(400, 50.0);
        let opts = SimplifyOptions { tolerance: 0.01, ..Default::default() };
        work::reset();
        let mut fit = Fit::new(usize::MAX, 10_000);
        let parts = simplify_within(&circle, &opts, &mut fit);
        // The samples of the whole circle are many more than a fit holds, and are fitted in parts of at most that many
        // (an anchor where two meet): more anchors than the circle fitted whole, far fewer than it had.
        let sampled = work::take(Work::FitSamples);
        assert!(sampled > 50_000, "{sampled} samples");
        let anchors = anchor_count(&parts);
        assert!((14..=60).contains(&anchors), "{anchors} anchors");
        assert!(deviation(&circle, &parts) <= 0.01 + MEASURED && deviation(&parts, &circle) <= 0.01 + MEASURED, "within the tolerance both ways");
        // With room, the same circle is a handful of curves (12 where the split was always at the worst point, 13 where it is moved inward).
        let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
        let fitted = simplify_within(&circle, &opts, &mut fit);
        assert!(anchor_count(&fitted) <= 14, "{}", anchor_count(&fitted));
        assert!(anchor_count(&fitted) < anchors, "{} anchors whole, {anchors} in parts", anchor_count(&fitted));
    }

    /// Arcs with sharp corners between them: each is a run of its own, so the work is spent run by run.
    fn scallops(arcs: usize) -> BezPath {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        for i in 0..arcs {
            let x = i as f64 * 40.0;
            // An arc of 20 polyline sides, then a corner back down.
            for k in 1..=20 {
                let a = f64::from(k) / 20.0 * std::f64::consts::PI;
                b.line_to((x + 20.0 - 20.0 * a.cos(), 20.0 * a.sin()));
            }
            b.line_to((x + 40.0, -30.0));
            b.line_to((x + 40.0, 0.0));
        }
        b
    }

    #[test]
    fn simplify_spends_one_budget_over_all_its_runs() {
        let path = scallops(60);
        let opts = SimplifyOptions { tolerance: 0.2, ..Default::default() };
        work::reset();
        let full = simplify_within(&path, &opts, &mut Fit::new(usize::MAX, MAX_FIT_POINTS));
        let total = work::take(Work::Fit);
        assert!(total > 10_000, "{total}");
        let half = total / 2;
        let partly = simplify_within(&path, &opts, &mut Fit::new(half, MAX_FIT_POINTS));
        let spent = work::take(Work::Fit);
        assert!(spent <= half, "{spent} of {half}");
        // The runs the budget reached are refitted and the others are as they were.
        let (before, after_all, after_half) = (anchor_count(&path), anchor_count(&full), anchor_count(&partly));
        assert!(after_all < after_half && after_half < before, "{before} anchors, {after_half} with half the work, {after_all} with all of it");
    }

    #[test]
    fn smoothing_is_charged_to_the_same_budget() {
        let circle = circle_polygon(300, 50.0);
        let opts = SimplifyOptions { tolerance: 0.5, smooth: 1e9, ..Default::default() };
        work::reset();
        let spent = |work: usize| {
            let mut fit = Fit::new(work, MAX_FIT_POINTS);
            let out = simplify_within(&circle, &opts, &mut fit);
            (anchor_count(&out), work::take(Work::Fit))
        };
        let (anchors, all) = spent(usize::MAX / 2);
        assert!(anchors < 300 && all > 300 * 300, "{anchors} anchors, {all} work: every point reads every other");
        // Too little for the window of every point: the run stays as it is.
        let (anchors, used) = spent(5_000);
        assert_eq!(anchors, 300);
        assert!(used <= 5_000, "{used}");
    }

    #[test]
    fn the_straight_lines_option_is_charged_for_the_points_it_reads() {
        let circle = circle_polygon(300, 50.0);
        let opts = SimplifyOptions { tolerance: 0.5, straight_lines: true, ..Default::default() };
        work::reset();
        let all = simplify_within(&circle, &opts, &mut Fit::new(usize::MAX / 2, MAX_FIT_POINTS));
        let used = work::take(Work::Fit);
        assert!(anchor_count(&all) < 300 && used > 0);
        let starved = simplify_within(&circle, &opts, &mut Fit::new(used / 2, MAX_FIT_POINTS));
        assert_eq!(anchor_count(&starved), 300);
        assert!(work::take(Work::Fit) <= used / 2);
    }

    /// A piece of `n` segments of four intervals each has `4n + 1` samples, the one at its start included:
    /// all are counted against the work and against the most points of a run.
    #[test]
    fn the_sample_at_the_start_of_a_piece_is_counted_with_the_rest() {
        let opts = SimplifyOptions { tolerance: 5.0, ..Default::default() };
        for n in [1usize, 2, 5] {
            let pts: Vec<(f64, f64)> = (0..=n).map(|i| (i as f64, 0.0)).collect();
            let line = polyline(&pts, false);
            work::reset();
            let free = simplify_within(&line, &opts, &mut Fit::new(usize::MAX / 2, MAX_FIT_POINTS));
            assert_eq!(work::take(Work::FitSamples), 4 * n + 1, "{n} segments");
            assert_eq!(anchor_count(&free), 2, "a straight line of {n} segments is one");
            // One point fewer than the samples a run needs: it is fitted in two parts, the last segment alone, with an anchor
            // between them (a part of one segment takes 4 + 1 samples, so for one segment there is no part: it stays).
            let capped = simplify_within(&line, &opts, &mut Fit::new(usize::MAX / 2, 4 * n));
            if n == 1 {
                assert_eq!(capped, line, "{n} segments, room for {} points", 4 * n);
            } else {
                assert_eq!(anchor_count(&capped), 3, "{n} segments, room for {} points: two parts", 4 * n);
            }
            let roomy = simplify_within(&line, &opts, &mut Fit::new(usize::MAX / 2, 4 * n + 1));
            assert_eq!(anchor_count(&roomy), 2);
            // The same for the work: the samples alone are 4n + 1 of it.
            work::reset();
            let starved = simplify_within(&line, &opts, &mut Fit::new(4 * n, MAX_FIT_POINTS));
            assert_eq!(starved, line, "{n} segments, work for {}", 4 * n);
            assert!(work::take(Work::FitSamples) <= 4 * n, "no sample is taken that the work does not cover");
        }
    }

    /// A closed path that starts in the middle of a straight side leaves its last joint to the
    /// search after the first pass; with no work for it, the joint stays.
    #[test]
    fn the_search_after_the_first_pass_is_charged_and_stops_at_its_budget() {
        let square = polyline(&[(5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)], true);
        work::reset();
        assert_eq!(anchor_count(&remove_redundant_within(&square, 1e-6, &mut 100)), 4);
        let used = work::take(Work::Redundant);
        assert!((5..=100).contains(&used), "{used}");
        assert_eq!(anchor_count(&remove_redundant_within(&square, 1e-6, &mut 0)), 5);
        assert_eq!(work::take(Work::Redundant), 0);
        // Four reads are not enough for the first pass over five segments: none are made.
        assert_eq!(anchor_count(&remove_redundant_within(&square, 1e-6, &mut 4)), 5);
        assert_eq!(work::take(Work::Redundant), 0);
        // And the public function has room for it.
        assert_eq!(anchor_count(&remove_redundant_points(&square, 1e-6)), 4);
    }

    /// A circle of radius `r` in `n` cubics, each the standard arc of its angle, from `(r, 0)` round.
    fn cut_circle(n: usize, r: f64) -> BezPath {
        let mut circle = BezPath::new();
        for i in 0..n {
            let (a0, a1) = (std::f64::consts::TAU * i as f64 / n as f64, std::f64::consts::TAU * (i + 1) as f64 / n as f64);
            let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * r;
            let (p0, p3) = (Point::new(r * a0.cos(), r * a0.sin()), Point::new(r * a1.cos(), r * a1.sin()));
            if i == 0 {
                circle.move_to(p0);
            }
            circle.curve_to(p0 + Vec2::new(-a0.sin(), a0.cos()) * k, p3 - Vec2::new(-a1.sin(), a1.cos()) * k, p3);
        }
        circle.close_path();
        circle
    }

    /// An open chain of `n` smooth cubics, each `step` along x, through the points of a sine wave.
    fn wavy_chain(n: usize, step: f64, amp: f64, wave: f64) -> BezPath {
        let at = |i: usize| {
            let x = i as f64 * step;
            (Point::new(x, amp * (wave * x).sin()), Vec2::new(1.0, amp * wave * (wave * x).cos()) * (step / 3.0))
        };
        let mut chain = BezPath::new();
        chain.move_to(at(0).0);
        for i in 0..n {
            let ((p0, d0), (p3, d3)) = (at(i), at(i + 1));
            chain.curve_to(p0 + d0, p3 - d3, p3);
        }
        chain
    }

    /// The segments of the first subpath of `path`, as the merges read them.
    fn chain_of(path: &BezPath) -> Vec<Seg> {
        subs(path).first().map(Sub::chain).unwrap_or_default()
    }

    /// The `count` segments of `segs` from `first` (counting round the end), as a path.
    fn run_of(segs: &[Seg], first: usize, count: usize) -> BezPath {
        let run: Vec<Seg> = (0..count).filter_map(|k| segs.get((first + k) % segs.len()).copied()).collect();
        segs_to_bezpath(&run, false).unwrap_or_default()
    }

    /// The path of one cubic.
    fn cubic_path(c: CubicBez) -> BezPath {
        let mut path = BezPath::new();
        path.move_to(c.p0);
        path.curve_to(c.p1, c.p2, c.p3);
        path
    }

    /// The run between two points taken of it is within the width of the lens of their chord, and every
    /// point of the chord has a point of the run within that width (and the width is not far over what it
    /// bounds): for runs of short segments, which the points leave a stretch of at a time, for long ones,
    /// which they take at several places, and for runs that go round the start of a closed subpath.
    #[test]
    fn the_run_between_two_points_taken_of_it_is_within_the_lens_of_their_chord() {
        use kurbo::{Line, ParamCurveNearest};
        let near = |p: Point, lines: &[Line]| lines.iter().map(|l| l.nearest(p, 1e-12).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
        for (n, first, count, steps) in
            [(200usize, 5usize, 100usize, 8usize), (200, 190, 30, 8), (200, 0, 150, 24), (40, 3, 3, 8), (40, 30, 20, 16), (12, 0, 12, 8)]
        {
            let segs = chain_of(&cut_circle(n, 50.0));
            let originals = Originals::new(&segs);
            let (pts, gap) = witness_points(&originals, first, count, steps).unwrap();
            let mut dense = Vec::new();
            for r in 0..count {
                let c = segs[(first + r) % n].c;
                dense.extend((0..400).map(|k| c.eval(f64::from(k) / 400.0)));
            }
            dense.push(segs[(first + count - 1) % n].c.p3);
            let run: Vec<Line> = dense.windows(2).map(|w| Line::new(w[0], w[1])).collect();
            let chords: Vec<Line> = pts.windows(2).map(|w| Line::new(w[0], w[1])).collect();
            assert_eq!(pts.first(), dense.first(), "the first point is the start of the run");
            assert_eq!(pts.last(), dense.last(), "the last point is the end of the run");
            let run_from_chords = dense.iter().map(|&p| near(p, &chords)).fold(0.0, f64::max);
            let chords_from_run =
                chords.iter().flat_map(|l| (0..=20).map(move |k| l.eval(f64::from(k) / 20.0))).map(|p| near(p, &run)).fold(0.0, f64::max);
            let worst = run_from_chords.max(chords_from_run);
            assert!(worst <= gap, "n {n} first {first} count {count} steps {steps}: {worst} against the gap {gap}");
            assert!(gap <= 2.0 * worst, "n {n} first {first} count {count} steps {steps}: the gap {gap} for {worst}");
        }
    }

    /// The length a run is held to is the sum of the control polygons of its segments, round a closed
    /// subpath's start as anywhere else.
    #[test]
    fn the_length_of_a_run_round_a_closed_start_is_the_sum_of_its_segments() {
        let segs = chain_of(&cut_circle(200, 50.0));
        let originals = Originals::new(&segs);
        let direct = |first: usize, count: usize| (0..count).map(|k| polygon_length(&segs[(first + k) % 200].c)).sum::<f64>();
        for (first, count) in [(0usize, 5usize), (190, 30), (199, 2), (150, 200), (200, 3)] {
            let got = originals.polygon_length(first, count).unwrap();
            assert!(
                (got - direct(first, count)).abs() <= 1e-12 * direct(first, count),
                "first {first} count {count}: {got} against {}",
                direct(first, count)
            );
        }
        assert_eq!(Originals::new(&[]).polygon_length(0, 1), None, "no segments, no length");
    }

    /// A refit's bound is no less than how far the cubic and the run are from each other, both ways (as
    /// measured), however the run is cut and wherever it starts; and where the run is a smooth curve of
    /// short segments, which one cubic can follow, it is no more than a share of the tolerance (what the
    /// sampling takes off it is not most of it).
    #[test]
    fn a_refit_is_within_its_bound_of_its_run_both_ways() {
        let circle = cut_circle(40, 50.0);
        let wavy = wavy_chain(60, 1.0, 3.0, 0.5);
        // (path, first, count, tolerance, the most the bound is of it, for a run one cubic follows)
        for (path, first, count, tol, most) in [
            (&circle, 0, 3, 0.01, Some(0.5)),
            (&circle, 35, 12, 0.05, Some(1.0)),
            (&circle, 5, 9, 0.5, Some(0.5)),
            (&circle, 0, 20, 0.2, None),
            (&wavy, 4, 20, 0.05, None),
            (&wavy, 0, 12, 0.5, None),
            (&wavy, 30, 6, 0.001, None),
        ] {
            let segs = chain_of(path);
            let originals = Originals::new(&segs);
            let mut work = usize::MAX;
            let (c, bound) = refit_run(&originals, first, count, tol, 0.0, &mut work).unwrap();
            let (run, fit) = (run_of(&segs, first, count), cubic_path(c));
            let (from, to) = (deviation(&run, &fit), deviation(&fit, &run));
            assert!(
                from.max(to) <= bound + MEASURED,
                "first {first} count {count} tol {tol}: the run is {from} from the cubic, the cubic {to} from the run, bound {bound}"
            );
            if let Some(most) = most {
                assert!(bound <= most * tol, "first {first} count {count} tol {tol}: bound {bound}");
                // What the points taken leave over what is there is a few hundredths of the tolerance.
                assert!(bound <= from.max(to) + 0.1 * tol, "first {first} count {count} tol {tol}: bound {bound} for {from} and {to}");
            }
        }
    }

    /// A refit holds the run to what the points taken of it leave: the gap between them is in the bound.
    /// Here the cubic is on every point of the run (one cubic cut in four by its own parameter), so the
    /// bound is the gap and the spread of the cubic from its chords, and no less than the gap.
    #[test]
    fn a_refit_of_a_run_the_cubic_is_on_exactly_has_the_gap_in_its_bound() {
        let c = CubicBez::new((0.0, 0.0), (10.0, 20.0), (30.0, 20.0), (40.0, 0.0));
        let parts: Vec<Seg> = (0..4).map(|k| Seg { c: c.subsegment(f64::from(k) / 4.0..f64::from(k + 1) / 4.0), line: false }).collect();
        let originals = Originals::new(&parts);
        let tol = 1.0;
        // The points of the first try, as many as the cubic asks for.
        let steps = witnesses_for(second_difference(&c), tol);
        let (_, gap) = witness_points(&originals, 0, 4, steps).unwrap();
        let mut work = usize::MAX;
        let (_, bound) = refit_run(&originals, 0, 4, tol, second_difference(&c), &mut work).unwrap();
        assert!(gap > 0.0 && bound >= gap, "bound {bound} gap {gap}");
    }

    /// A circle of twenty thousand short segments is taken in a few hundred refits (a hundred or so a piece),
    /// not one for each merge, which is hundreds of thousands of the points.
    #[test]
    fn a_long_smooth_run_is_taken_in_a_few_refits() {
        let circle = cut_circle(20_000, 1000.0);
        work::reset();
        let simpler = remove_redundant_points(&circle, 0.01);
        let points = work::take(Work::Witnesses);
        assert!(anchor_count(&simpler) <= 36, "{} anchors", anchor_count(&simpler));
        assert!(points > 0 && points < 300_000, "{points} points refitted");
    }

    /// The points of a refit are charged to the work as the joints are: with little of it, no more is
    /// taken than there is, and what the refits do not reach is left as it was, not lost.
    #[test]
    fn the_points_of_a_refit_are_charged_to_the_work() {
        let circle = cut_circle(5_000, 1000.0);
        work::reset();
        let free = remove_redundant_points(&circle, 0.01);
        let (free_points, free_joints) = (work::take(Work::Witnesses), work::take(Work::Redundant));
        assert!(free_points > 100, "{free_points} points refitted without a limit");
        for limit in [0usize, 500, 20_000, 200_000] {
            let limits = crate::Limits { redundant_work: limit, ..crate::Limits::default() };
            work::reset();
            let cut = crate::with_limits(limits, || remove_redundant_points(&circle, 0.01));
            let (points, joints) = (work::take(Work::Witnesses), work::take(Work::Redundant));
            assert!(points * WITNESS_COST + joints <= limit, "{points} points and {joints} joints read with {limit} of work");
            assert!(anchor_count(&cut) >= anchor_count(&free) && anchor_count(&cut) <= 5_000);
        }
        // Room for all of it gives the same result, and a little less does not: the work is the whole of what
        // was spent.
        let mut left = BIG_WORK;
        assert_eq!(remove_redundant_within(&circle, 0.01, &mut left), free);
        let spent = BIG_WORK - left;
        assert!(spent >= free_points * WITNESS_COST + free_joints, "{spent} spent for {free_points} points and {free_joints} joints");
        assert_eq!(remove_redundant_within(&circle, 0.01, &mut spent.clone()), free);
        assert_ne!(remove_redundant_within(&circle, 0.01, &mut (spent / 2)), free);
    }

    /// The segments of `path` as the arrays of their points.
    fn points_of(path: &BezPath) -> Vec<Vec<Point>> {
        path.segments()
            .map(|s| match s {
                PathSeg::Line(l) => vec![l.p0, l.p1],
                PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
                PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
            })
            .collect()
    }

    /// The same point, a point that is not a number being the same as another (which `==` is not).
    fn same_point(a: Point, b: Point) -> bool {
        let same = |a: f64, b: f64| a == b || (a.is_nan() && b.is_nan());
        same(a.x, b.x) && same(a.y, b.y)
    }

    /// A length that is not a number, or too big to add up, leaves the run as it is: nothing is merged on
    /// a bound that is not known, and nothing panics. The cubic it is the length of comes out as it
    /// went in, point for point (a point that is not a number being the same as another), where the
    /// ones round it, which are straight and level, are the ones merged.
    #[test]
    fn a_run_with_a_length_that_is_not_a_number_is_not_refitted() {
        for bad in [f64::NAN, f64::INFINITY, 1e300] {
            let mut chain = BezPath::new();
            chain.move_to((0.0, 0.0));
            for i in 0..40 {
                let x = f64::from(i) * 10.0;
                let h = if i == 20 { bad } else { x + 3.0 };
                chain.curve_to((h, 0.0), (x + 7.0, 0.0), (x + 10.0, 0.0));
            }
            let affected = points_of(&chain).into_iter().nth(20).unwrap();
            let simpler = remove_redundant_points(&chain, 0.01);
            let kept = points_of(&simpler);
            assert!(
                kept.iter().any(|seg| seg.len() == affected.len() && seg.iter().zip(&affected).all(|(a, b)| same_point(*a, *b))),
                "{bad}: the cubic is not as it was: {kept:?}"
            );
        }
    }

    /// The width of the lens is half the minor axis of the ellipse a path of the given length can reach
    /// from one end of a chord to the other, with what rounding may have taken of the lengths added (to a
    /// path, and taken off the chord), and is still that where the squares of the lengths overflow (past
    /// 1e154 they are infinite, and their difference was no number, and so no width at all).
    #[test]
    fn the_lens_width_is_the_width_where_the_squares_overflow_too() {
        let near = |a: f64, b: f64| (a / b - 1.0).abs() < 1e-6;
        assert!(near(lens_width(5.0, 3.0), 2.0), "{}", lens_width(5.0, 3.0));
        assert_eq!(lens_width(3.0, 5.0), 0.0, "a path shorter than its chord is none: no width");
        let (along, chord): (f64, f64) = (2e154, 1.9e154);
        assert!((along * along).is_infinite() && (along * along - chord * chord).is_nan(), "the squares overflow");
        let width = lens_width(along, chord);
        let exact = 0.5 * ((along - chord) * (along + chord)).sqrt();
        assert!(width > 1e152 && near(width, exact), "{width} against {exact}");
        // A length that is not a number, or too big for a width, is an infinite one.
        for bad in [f64::NAN, f64::INFINITY] {
            assert_eq!(lens_width(bad, 1.0), f64::INFINITY);
            assert_eq!(lens_width(1.0, bad), f64::INFINITY);
        }
        assert_eq!(lens_width(f64::MAX, 1.0), f64::INFINITY);
        assert_eq!(lens_width(f64::MAX, f64::MAX), f64::INFINITY);
    }

    /// Lengths that rounding has made equal are not a width of 0: a shallow cubic with long control edges
    /// (three of 1e8, rising 0.75) has edges that add up to its chord, to the last place, and is 0.56 off
    /// it. The width is that of the least the path could be longer by, as much as its length gives.
    #[test]
    fn lengths_that_rounding_made_equal_are_not_a_width_of_nothing() {
        let (edge, rise): (f64, f64) = (1e8, 0.75);
        let polygon = polygon_length(&CubicBez::new((0.0, 0.0), (edge, rise), (2.0 * edge, rise), (3.0 * edge, 0.0)));
        let chord = Point::new(0.0, 0.0).distance(Point::new(3.0 * edge, 0.0));
        assert_eq!(polygon, chord, "the edges add up to the chord to the last place: the width was 0");
        let width = lens_width(polygon, chord);
        assert!(width > 5.0, "{width}: the bump is 0.5625 high");
        // The width of equal lengths is their size times the root of the rounding: four units of it, each way.
        let expected = 3e8 * (4.0 * f64::EPSILON).sqrt();
        assert!((width / expected - 1.0).abs() < 0.2, "{width} against {expected}");
        assert!(lens_width(1.0, 1.0) > 0.0 && lens_width(1.0, 1.0) < 1e-6);
    }

    /// The sum of the lengths of a run is off by the rounding of its whole, not of each addition: a million
    /// and a half copies of a tenth (whose sum in order is far from exact) and numbers that a sum in order
    /// loses (a unit beside 1e16) add up to what they are.
    #[test]
    fn a_run_of_lengths_is_added_with_its_rounding_compensated() {
        let n = 1usize << 20;
        assert_eq!(compensated_sum(std::iter::repeat_n(0.1, n)), 0.1 * n as f64, "a power of two copies of a tenth");
        assert!(std::iter::repeat_n(0.1, n).sum::<f64>() != 0.1 * n as f64, "added in order they are not");
        assert_eq!(compensated_sum([1e16, 1.0, -1e16].into_iter()), 1.0);
        assert_eq!(compensated_sum([].into_iter()), 0.0);
        assert!(compensated_sum([1.0, f64::NAN].into_iter()).is_nan());
        assert!(!compensated_sum([f64::INFINITY, 1.0].into_iter()).is_finite());
    }

    /// A run with bumps that rounded lengths hide is not taken whole: twelve straight cubics 3e8 long, a bump
    /// up (handles 1e8 across rising 0.75, 0.5625 at its middle) and a bump down after it, and two straight
    /// cubics. The strides (1, 2, 4, 8 more than the eight first merged) fit up to the fourth, and the eighth
    /// takes the points of its run at the joints 12 and 14, with the two bumps between them, whose edges
    /// add up to their chord to the last place: it had no width between them, and the bumps were lost at a
    /// tolerance of 0.001.
    #[test]
    fn bumps_that_rounded_lengths_hide_are_not_merged_away() {
        let len = 3e8;
        let mut chain = BezPath::new();
        chain.move_to((0.0, 0.0));
        // The cubic from `x` whose two handles are `rise` high.
        let cubic = |chain: &mut BezPath, i: usize, rise: f64| {
            let x = i as f64 * len;
            chain.curve_to((x + len / 3.0, rise), (x + 2.0 * len / 3.0, rise), (x + len, 0.0));
        };
        for i in 0..16 {
            cubic(
                &mut chain,
                i,
                match i {
                    12 => 0.75,
                    13 => -0.75,
                    _ => 0.0,
                },
            );
        }
        let simpler = remove_redundant_points(&chain, 0.001);
        let bounds = kurbo::Shape::bounding_box(&simpler);
        assert!(
            bounds.y1 > 0.5 && bounds.y0 < -0.5,
            "the bumps are {} high and {} low, 0.5625 before: {} segments",
            bounds.y1,
            -bounds.y0,
            simpler.segments().count()
        );
        // What is straight is merged, up to the first bump and after the second.
        assert!(simpler.segments().count() >= 4 && simpler.segments().count() < 16, "{} segments", simpler.segments().count());
    }

    /// Two arcs a quarter circle each, the second turning `turn` degrees away from where the first arrives,
    /// each of `n` standard arcs of a cubic: the corner is at the joint `n`.
    fn two_arcs(n: usize, r: f64, turn: f64) -> BezPath {
        let quarter = std::f64::consts::FRAC_PI_2;
        let mut path = BezPath::new();
        let arc = |path: &mut BezPath, centre: Point, from: f64| {
            for i in 0..n {
                let (a0, a1) = (from + quarter * i as f64 / n as f64, from + quarter * (i + 1) as f64 / n as f64);
                let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * r;
                let at = |a: f64| centre + Vec2::new(r * a.cos(), r * a.sin());
                let (p0, p3) = (at(a0), at(a1));
                if path.elements().is_empty() {
                    path.move_to(p0);
                }
                path.curve_to(p0 + Vec2::new(-a0.sin(), a0.cos()) * k, p3 - Vec2::new(-a1.sin(), a1.cos()) * k, p3);
            }
        };
        arc(&mut path, Point::ZERO, 0.0);
        let corner = Point::new(0.0, r);
        let leaves = std::f64::consts::PI + turn.to_radians();
        let centre = corner + Vec2::new(r * (leaves + quarter).cos(), r * (leaves + quarter).sin());
        arc(&mut path, centre, (corner.y - centre.y).atan2(corner.x - centre.x));
        path
    }

    /// The first corner at or after a joint is found from a table made once: for two arcs with a corner of
    /// 45 degrees between them, it is the joint between them from any joint before, that joint itself,
    /// and none (the number of segments) from any after; a smooth chain has none, and joint 0 is no joint.
    #[test]
    fn the_first_corner_at_or_after_a_joint_is_found_from_a_table() {
        let segs = chain_of(&two_arcs(30, 50.0, 45.0));
        let originals = Originals::new(&segs);
        assert_eq!(segs.len(), 60);
        for from in [0, 1, 17, 29, 30] {
            assert_eq!(originals.corner_from(from), 30, "from {from}");
        }
        for from in [31, 45, 59, 60, 61, 1_000] {
            assert_eq!(originals.corner_from(from), 60, "from {from}");
        }
        // 5 degrees is smooth enough to merge across: no corner.
        let smooth = chain_of(&two_arcs(30, 50.0, 5.0));
        let originals = Originals::new(&smooth);
        assert_eq!(originals.corner_from(0), 60);
        assert_eq!(Originals::new(&[]).corner_from(0), 0);
    }

    /// The segments `0..count` of `originals` as the piece a run of them is: refitted.
    fn piece_of(originals: &Originals, count: usize, tol: f64) -> Piece {
        let mut work = usize::MAX;
        let (c, dev) = refit_run(originals, 0, count, tol, 0.0, &mut work).unwrap();
        Piece { seg: Seg { c, line: false }, dev, first: 0, count }
    }

    /// A piece is extended by as many segments as fit, whatever the number is: not only a number a stride
    /// of doubling reaches (the strides are halved between the last that fitted and the first that did not),
    /// and the whole of what is ahead when it all fits though that is not one either (the last stride past
    /// it is cut to it).
    #[test]
    fn a_piece_is_extended_by_as_many_segments_as_fit() {
        let tol = 1.0;
        let segs = chain_of(&cut_circle(2000, 1000.0));
        let originals = Originals::new(&segs[..1500]);
        let top = piece_of(&originals, 8, tol);
        // How many more than the 8 make a run one cubic can follow within the tolerance (the run only
        // gets worse as it grows).
        let sd = second_difference(&top.seg.c);
        let fits = |extra: usize| {
            let mut work = usize::MAX;
            let ratio = (8 + extra) as f64 / 8.0;
            refit_run(&originals, 0, 8 + extra, tol, sd * ratio * ratio, &mut work).is_some_and(|(_, dev)| dev <= tol)
        };
        let most = (1..=1_000).take_while(|&extra| fits(extra)).count();
        assert!(!fits(most + 1) && !fits(most + 50), "{most} is the most");
        assert!(most > 100 && !most.is_power_of_two(), "{most} is no number to double to");
        let mut work = usize::MAX;
        let (longer, took) = extend_run(&top, 1_000, tol, &originals, &mut work).unwrap();
        assert_eq!((took, longer.count, longer.first), (most, 8 + most, 0));
        assert!(longer.dev <= tol);
        // Fewer ahead than fit, and no power of two: all of them.
        let ahead = 37;
        assert!(ahead < most);
        let (longer, took) = extend_run(&top, ahead, tol, &originals, &mut work).unwrap();
        assert_eq!((took, longer.count), (ahead, 8 + ahead));
        // Not one when not one fits.
        let bad = chain_of(&two_arcs(30, 50.0, 90.0));
        let originals = Originals::new(&bad);
        let top = piece_of(&originals, 8, 1e-6);
        assert!(extend_run(&top, 50, 1e-6, &originals, &mut work).is_none());
    }

    /// A refit is charged for each round of it: its points, and the segments of the run to find them in.
    /// With a guess of the cubic that is far off it is two rounds, with one that is near it is one; and a
    /// refit with too little work left leaves none (as the checks of a line do), so that a cheaper one after
    /// it is not made on what a dearer one could not.
    #[test]
    fn a_refit_is_charged_for_each_round_and_leaves_no_work_it_could_not_pay() {
        let segs = chain_of(&cut_circle(8000, 1000.0));
        let originals = Originals::new(&segs);
        let count = 3_000;
        let big = BIG_WORK;
        let spent = |seed: f64| {
            work::reset();
            let mut left = big;
            let (c, _) = refit_run(&originals, 0, count, 1.0, seed, &mut left).unwrap();
            (big - left, work::take(Work::Witnesses), c)
        };
        let (two, points, c) = spent(0.0);
        assert_eq!(two, points * WITNESS_COST + 2 * (count / 4), "two rounds, {points} points");
        let (one, points, _) = spent(second_difference(&c));
        assert_eq!(one, points * WITNESS_COST + count / 4, "one round, {points} points");
        assert!(two > one);
        // Starved: no cheaper refit is made on what a dearer one could not.
        for left in [0usize, 10, 1_000] {
            let mut work = left;
            assert!(refit_run(&originals, 0, count, 1.0, 0.0, &mut work).is_none());
            assert_eq!(work, 0, "{left} of work");
        }
    }

    /// The guess a merge gives its refit is the cubic's own, grown: one round, not a round at the least
    /// and another at the points the cubic asks for.
    #[test]
    fn a_merge_that_is_refitted_is_refitted_in_one_round() {
        let segs = chain_of(&cut_circle(2000, 1000.0));
        let originals = Originals::new(&segs);
        let tol = 0.01;
        let (a, b) = (piece_of(&originals, 30, tol), {
            let mut work = usize::MAX;
            let (c, dev) = refit_run(&originals, 30, 30, tol, 0.0, &mut work).unwrap();
            Piece { seg: Seg { c, line: false }, dev, first: 30, count: 30 }
        });
        work::reset();
        let mut left = BIG_WORK;
        let _ = merge_pieces(&a, &b, tol, &originals, &mut left);
        let (spent, points) = ((BIG_WORK - left), work::take(Work::Witnesses));
        assert!(points > 0, "the sum of the bounds passes the tolerance: the run is refitted");
        assert_eq!(spent, points * WITNESS_COST + 60 / 4, "one round, {points} points");
    }

    /// Two segments are merged on 16 samples of each, as VectorCraft does: the cubic is the fit to
    /// them, and the step is its error and the rest of its bound.
    #[test]
    fn a_pair_is_merged_on_sixteen_intervals_of_each() {
        let segs = chain_of(&cut_circle(400, 100.0));
        let (a, b) = (segs[10], segs[11]);
        let tol = 0.5;
        let (seg, step) = merge_pair(&a, &b, tol).unwrap();
        let mut pts = Vec::new();
        sample(&a.c, 16, &mut pts, true);
        sample(&b.c, 16, &mut pts, false);
        let between = sample_error(&a, 16, tol * 0.05).max(sample_error(&b, 16, tol * 0.05));
        let (c, err, rest) = fit_single_bounded(&pts, start_tangent(&a.c), end_tangent(&b.c), between);
        assert_eq!(seg.c, c);
        assert_eq!(step, err + rest);
    }

    /// A piece that nothing more fits to is found out by one refit: the first stride is of one segment, not
    /// two (which is two refits, one for two and one for the one that is left to try), and no table of the
    /// corners is made when no stride is tried.
    #[test]
    fn an_extension_that_fails_at_once_costs_one_refit() {
        let tol = 1.0;
        let segs = chain_of(&cut_circle(2000, 1000.0));
        let originals = Originals::new(&segs[..1500]);
        // Past the most that fit (a run of 646 does at this tolerance), so that one more does not.
        let top = piece_of(&originals, 700, tol);
        let sd = second_difference(&top.seg.c);
        let ratio = 701.0 / 700.0;
        work::reset();
        let mut left = BIG_WORK;
        assert!(refit_run(&originals, 0, 701, tol, sd * ratio * ratio, &mut left).is_some_and(|(_, dev)| dev > tol), "one more does not fit");
        let (one_refit, points) = (BIG_WORK - left, work::take(Work::Witnesses));
        let mut left = BIG_WORK;
        assert!(extend_run(&top, 50, tol, &originals, &mut left).is_none());
        assert_eq!((BIG_WORK - left, work::take(Work::Witnesses)), (one_refit, points), "one refit, as many points as that one");
    }

    /// The table of the corners of a subpath is made when a stride is first tried, and once: not for a path
    /// no stride is tried on (lines, or curves in pieces of fewer than eight), and not again for each pass
    /// of the first merge pass.
    #[test]
    fn the_table_of_corners_is_made_when_a_stride_first_asks_for_it_and_once() {
        work::reset();
        let polygon = remove_redundant_points(&circle_polygon(2_000, 100.0), 0.01);
        assert!(anchor_count(&polygon) < 2_000);
        // Curves, in pieces too short for a stride: two cubics, and then seven on their own.
        let short = remove_redundant_points(&cut_circle(7, 100.0), 0.01);
        assert!(short.segments().count() <= 7);
        assert_eq!(work::take(Work::CornerTables), 0, "no stride, no table");
        let circle = remove_redundant_points(&cut_circle(2_000, 100.0), 0.01);
        assert!(circle.segments().count() < 20);
        assert_eq!(work::take(Work::CornerTables), 1, "one table for the one subpath");
        // Two subpaths of long pieces: a table for each.
        let mut two = cut_circle(2_000, 100.0);
        two.extend(cut_circle(2_000, 50.0).iter());
        let _ = remove_redundant_points(&two, 0.01);
        assert_eq!(work::take(Work::CornerTables), 2);
    }

    /// The gap of a witness point is the width of the lens of its stretch, which does not grow with the number of
    /// segments in it: the lengths of a run are added with their rounding compensated, and the allowance is
    /// the same share of the length for two segments or two hundred thousand. On straight cubics the path is as
    /// long as its chord and the gap is that allowance alone.
    #[test]
    fn the_gap_of_a_witness_point_is_the_lens_of_its_stretch_whatever_the_number_of_segments() {
        // Straight cubics end to end (each ends where the next starts, to the bit).
        let straight = |n: usize| -> Vec<Seg> {
            (0..n)
                .map(|i| {
                    let (a, b) = (i as f64 * 0.7, (i + 1) as f64 * 0.7);
                    Seg { c: CubicBez::new((a, 0.0), (a + (b - a) / 3.0, 0.0), (a + 2.0 * (b - a) / 3.0, 0.0), (b, 0.0)), line: false }
                })
                .collect()
        };
        let segs = straight(64);
        let originals = Originals::new(&segs);
        // Eight intervals of eight segments.
        let (pts, gap) = witness_points(&originals, 0, 64, 8).unwrap();
        let by_hand =
            pts.windows(2).enumerate().map(|(g, w)| lens_width(originals.polygon_length(8 * g, 8).unwrap(), w[0].distance(w[1]))).fold(0.0, f64::max);
        assert!(by_hand > 0.0 && gap == by_hand, "{gap} against {by_hand}");
        // Four segments cut in two each: a stretch inside one segment.
        let (pts, gap) = witness_points(&originals, 0, 4, 8).unwrap();
        let by_hand = pts
            .windows(2)
            .enumerate()
            .map(|(i, w)| {
                let half = segs[i / 2].c.subsegment((i % 2) as f64 / 2.0..(i % 2 + 1) as f64 / 2.0);
                lens_width(polygon_length(&half), w[0].distance(w[1]))
            })
            .fold(0.0, f64::max);
        assert!(by_hand > 0.0 && gap == by_hand, "{gap} against {by_hand}");
        // Two hundred thousand segments in eight intervals of 25,000 (140 across): the width is the length times
        // the root of four units of rounding, 5e-4, as it is for the 8 of them. Added in order and allowed a
        // unit for each segment's three lengths, as it was, it was 0.07, seven times the tolerance of 0.01.
        let long = straight(200_000);
        let originals = Originals::new(&long);
        let (_, gap) = witness_points(&originals, 0, 200_000, 8).unwrap();
        let interval = 25_000.0 * 0.7;
        assert!(gap > 0.5 * interval * (4.0 * f64::EPSILON).sqrt() && gap < 2.0 * interval * (4.0 * f64::EPSILON).sqrt(), "{gap}");
        assert!(gap < 0.01, "{gap}: a run of straight cubics is within the tolerance of its chord");
    }

    /// The length of a long run is the sum of the lengths of its segments to the last place, as the lens takes it to
    /// be, however many there are (added in order, 65,536 copies of three tenths come out a few units in the last
    /// place off), where the run starts and where it goes round the end of a closed subpath.
    #[test]
    fn the_length_of_a_long_run_is_the_sum_of_its_segments_to_the_last_place() {
        let n = 1usize << 16;
        let seg = Seg { c: CubicBez::new((0.0, 0.0), (0.1, 0.0), (0.2, 0.0), (0.3, 0.0)), line: false };
        let segs = vec![seg; n];
        let one = polygon_length(&seg.c);
        let exact = one * n as f64;
        assert!(std::iter::repeat_n(one, n).sum::<f64>() != exact, "added in order, the sum is off: the test would tell nothing");
        let originals = Originals::new(&segs);
        assert_eq!(originals.polygon_length(0, n), Some(exact));
        assert_eq!(originals.polygon_length(n / 2, n), Some(exact), "round the end of the subpath");
        assert_eq!(originals.polygon_length(5, 100), Some(one * 100.0));
    }

    /// A run is refitted from as many sample points as its curvature asks for, which is the eight of `MIN_WITNESSES` for
    /// a run that is straight, and the lens of one of those eight stretches, whose width is 3e-8 of its length however
    /// straight it is, passed the tolerance for a run of 2.7 million (a quarter of a hundred thousand cubics 1e4 long):
    /// 100,000 straight cubics came out as 55 elements at a tolerance of 0.01 (1c1f05c gave 2), and a parabola of radius
    /// 1e16 (a line within 1e-9 for each) as 266, more as the tolerance was less but not less as it was more (89 at 1).
    /// The samples are no fewer than keep that lens within a share of the tolerance ([`lens_floor`]), up to 1,024.
    #[test]
    fn a_long_straight_run_is_taken_whole_where_its_lens_asks_for_more_samples_than_its_curvature() {
        let straight = |n: usize| {
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            for i in 0..n {
                let x = i as f64 * 1e4;
                bp.curve_to((x + 1e4 / 3.0, 0.0), (x + 2e4 / 3.0, 0.0), (x + 1e4, 0.0));
            }
            bp
        };
        let elements = |bp: &BezPath, tol: f64| remove_redundant_points(bp, tol).elements().len();
        assert!(elements(&straight(100_000), 0.01) <= 6, "{}", elements(&straight(100_000), 0.01));
        assert_eq!(elements(&straight(20_000), 0.01), 2);
        let parabola = |r: f64| {
            let (h, f, df) = (1e4, |x: f64| x * x / (2.0 * r), |x: f64| x / r);
            let mut bp = BezPath::new();
            for i in 0..100_000usize {
                let (x0, x1) = (i as f64 * h, (i + 1) as f64 * h);
                if i == 0 {
                    bp.move_to((x0, f(x0)));
                }
                bp.curve_to((x0 + h / 3.0, f(x0) + df(x0) * h / 3.0), (x1 - h / 3.0, f(x1) - df(x1) * h / 3.0), (x1, f(x1)));
            }
            bp
        };
        // The count does not rise with the tolerance, and is a handful at 1e16 and at the radii where it was hundreds.
        let chain = parabola(1e16);
        let counts: Vec<usize> = [0.005, 0.01, 0.02, 0.05, 0.1, 1.0].iter().map(|&tol| elements(&chain, tol)).collect();
        assert!(counts.windows(2).all(|w| w[1] <= w[0]) && counts[1] <= 8 && counts[5] == 2, "{counts:?}");
        for r in [1e14, 1e15] {
            assert!(elements(&parabola(r), 0.01) <= 8, "radius {r:e}");
        }
        // The floor grows with the run and falls with the tolerance, and is nothing for a run that is not a number.
        assert!(lens_floor(1e9, 0.01) > lens_floor(1e8, 0.01) && lens_floor(1e8, 0.01) > lens_floor(1e8, 0.1));
        assert_eq!(lens_floor(f64::NAN, 0.01), 0);
        assert!(lens_floor(1e4, 0.01) <= 1, "a segment of ordinary length asks for nothing more than the eight");
    }
}
