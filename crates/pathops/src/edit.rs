//! Object → Path commands: Simplify, Smooth, Remove redundant points, Add Anchor Points, Average,
//! Join, Split Into Grid.

use std::collections::{BTreeMap, BTreeSet};

use craft_geom::path::measured_arclen;
use craft_geom::{Anchor, AnchorKind, Anchors, PathData, SubPath};
use kurbo::{CubicBez, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, Rect, Vec2};

use crate::boolean::{Seg, segs_to_subpath};
use crate::fit::{end_tangent, fit_cubics, fit_single, fit_single_from, is_straight, largest_second_difference, sample, start_tangent, unit};

/// Options for [`simplify_with`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimplifyOptions {
    /// Maximum distance (document points) between the original and simplified path.
    pub tolerance: f64,
    /// Anchors where the path turns by more than this many degrees stay corners.
    pub corner_angle_deg: f64,
    /// Produce only straight segments ("Convert to Straight Lines").
    pub straight_lines: bool,
    /// How far (points, the spread of a Gaussian along the path) the path is smoothed before it is
    /// refitted, so that bumps and jags come out as one curve ([`smooth_level`]); 0 for none.
    pub smooth: f64,
    /// With [`smooth`](Self::smooth), refit each stretch fitted as one piece (between corners) within
    /// twice the farthest the smoothing moved any of its points when that is less than `tolerance`, and
    /// never within less than a tenth of `tolerance`. A stretch that smoothing barely moves anywhere
    /// keeps its shape. The limit is the same along the whole stretch, so where smoothing moves some of
    /// its points far, all of it is refitted within up to `tolerance`, as without this option.
    pub smooth_moved_only: bool,
    /// With [`smooth`](Self::smooth), the curve at an open subpath's first anchor leaves it along the
    /// direction of the smoothed path over the 3 spreads next to it rather than along its first
    /// segment. The anchor stays where it is. A closed subpath has no such end, and [`simplify_part`]
    /// ignores this.
    pub free_start: bool,
    /// [`free_start`](Self::free_start) for an open subpath's last anchor.
    pub free_end: bool,
}

impl Default for SimplifyOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.5,
            corner_angle_deg: 30.0,
            straight_lines: false,
            smooth: 0.0,
            smooth_moved_only: false,
            free_start: false,
            free_end: false,
        }
    }
}

/// The least share of the tolerance that [`SimplifyOptions::smooth_moved_only`] refits within.
const SMOOTH_MOVED_FLOOR: f64 = 0.1;

/// Simplify with the default corner threshold (30°).
pub fn simplify(path: &PathData, tolerance: f64) -> PathData {
    simplify_with(path, &SimplifyOptions { tolerance, ..Default::default() })
}

/// The Simplify slider's level for a tolerance of 1 pt, the middle of its scale.
pub const SIMPLIFY_LEVEL: f64 = 50.0;

/// The tolerance (points) of Simplify slider level `level`, from 0 (the fewest anchor points) to
/// 100 (the closest to the original): 1 pt at [`SIMPLIFY_LEVEL`], doubled for every 10 levels below
/// it and halved for every 10 above, so level 0 is 32 pt and level 100 is 1/32 pt. A level outside
/// 0..=100 counts as the nearer end, and one that is not a number counts as [`SIMPLIFY_LEVEL`].
pub fn simplify_level_tolerance(level: f64) -> f64 {
    let level = if level.is_finite() { level.clamp(0.0, 100.0) } else { SIMPLIFY_LEVEL };
    ((SIMPLIFY_LEVEL - level) / 10.0).exp2()
}

/// Auto-Simplify's level for paths whose bounds are `size` points on their longer side: the lowest
/// whole level whose tolerance ([`simplify_level_tolerance`]) is at most 1/400 of `size`. It returns
/// level 100 for a size of 0 or one that is not a number, and for a size under 12.5 pt, where no
/// level is that close.
pub fn auto_simplify_level(size: f64) -> f64 {
    let target = size / 400.0;
    if !(target.is_finite() && target > 0.0) {
        return 100.0;
    }
    (SIMPLIFY_LEVEL - 10.0 * target.log2()).ceil().clamp(0.0, 100.0)
}

/// Simplify never adds anchors: keep whichever subpath is smaller.
fn no_worse(original: &SubPath, fitted: SubPath) -> SubPath {
    if fitted.anchors.len() > original.anchors.len() { original.clone() } else { fitted }
}

fn subpath_segs(sp: &SubPath) -> Vec<Seg> {
    (0..sp.segment_count()).map(|i| Seg { c: sp.segment(i), line: sp.segment_is_line(i) }).collect()
}

fn turn_deg(a: &Seg, b: &Seg) -> f64 {
    let d = end_tangent(&a.c).dot(start_tangent(&b.c)).clamp(-1.0, 1.0);
    d.acos().to_degrees()
}

/// Least-squares Bézier refit of every subpath, keeping corners sharper than the threshold.
pub fn simplify_with(path: &PathData, opts: &SimplifyOptions) -> PathData {
    let mut out = Vec::new();
    for sp in &path.subpaths {
        if let Some(s) = simplify_subpath(sp, opts) {
            out.push(s);
        }
    }
    PathData::new(out)
}

/// [`simplify_with`] for one subpath; `None` when the refit leaves nothing of it. A closed subpath
/// whose refit is one segment, or lines that enclose no area, is refitted in two parts, cut at the
/// point farthest from its start, or in three when two still give that. When three parts still give
/// no area, straight lines follow the subpath within the tolerance, and without straight lines it
/// stays as it is, as does one of a single segment. When no lines within the limits on pieces enclose
/// an area, it stays as it is, or with straight lines follows the lines between samples of each
/// curved segment when the limits leave room for lines within the tolerance; one that is all in line
/// keeps the refit.
fn simplify_subpath(sp: &SubPath, opts: &SimplifyOptions) -> Option<SubPath> {
    let mut segs = subpath_segs(sp);
    let n = segs.len();
    if n == 0 || sp.closed && n < 2 {
        return Some(sp.clone());
    }
    let closed_opts = SimplifyOptions { free_start: false, free_end: false, ..*opts };
    let opts = if sp.closed { &closed_opts } else { opts };
    if sp.closed {
        // Start the loop at its first corner, so no fit runs across one.
        let k = (0..n).find(|&i| turn_deg(&segs[(i + n - 1) % n], &segs[i]) > opts.corner_angle_deg).unwrap_or(0);
        segs.rotate_left(k);
    }
    // A smoothed loop with no corner is fitted along one direction across its start, when no segment
    // stays as it is and the fit has one part. A nearly straight first or last curve becomes a line
    // along its chord, and `no_worse` can return the original subpath, so the start can keep a kink.
    let across = if sp.closed && opts.smooth > 0.0 { unit(end_tangent(&segs[n - 1].c) + start_tangent(&segs[0].c)) } else { None };
    let across = across.filter(|_| (0..n).all(|i| turn_deg(&segs[(i + n - 1) % n], &segs[i]) <= opts.corner_angle_deg));
    let mut result = simplify_run_within(&segs, opts, MAX_RUN_SAMPLES, across);
    let in_line = |pts: &[Point]| {
        let Some(&a) = pts.first() else { return true };
        let far = pts.iter().map(|&p| p - a).max_by(|u, v| u.hypot2().total_cmp(&v.hypot2())).unwrap_or_default();
        pts.iter().all(|&p| far.cross(p - a).abs() <= 1e-6 * far.hypot())
    };
    let flat = |r: &[Seg]| r.len() < 2 || r.iter().all(|s| s.line) && in_line(&r.iter().map(|s| s.c.p0).collect::<Vec<_>>());
    if sp.closed && flat(&result) {
        // Each segment in two, and the loop cut where they meet: first at the point farthest from its
        // start, then also at the point farthest from the line between those two.
        let halves: Vec<Seg> = segs
            .iter()
            .flat_map(|s| {
                let (a, b) = s.c.subdivide();
                [Seg { c: a, line: s.line }, Seg { c: b, line: s.line }]
            })
            .collect();
        let m = halves.len();
        let at = |i: usize| halves[i].c.p0;
        let farthest = |from: &dyn Fn(Point) -> f64, skip: usize| (1..m).filter(|&i| i != skip).max_by(|&a, &b| from(at(a)).total_cmp(&from(at(b))));
        let p0 = at(0);
        let j = farthest(&|p| p.distance(p0), 0).unwrap_or(m / 2);
        let chord = at(j) - p0;
        let off_chord = |p: Point| if chord.hypot2() > 0.0 { chord.cross(p - p0).abs() } else { p.distance(p0) };
        let k = farthest(&off_chord, j).unwrap_or(m / 2);
        let refit = |cuts: &[usize]| -> Vec<Seg> {
            cuts.windows(2).flat_map(|w| simplify_run_within(&halves[w[0]..w[1]], opts, MAX_RUN_SAMPLES / 3, None)).collect()
        };
        result = refit(&[0, j, m]);
        if flat(&result) && k != j {
            result = refit(&[0, j.min(k), j.max(k), m]);
        }
        let ctrl: Vec<Point> = segs.iter().flat_map(|s| [s.c.p0, s.c.p1, s.c.p2]).collect();
        if flat(&result) && !in_line(&ctrl) {
            // Each curved segment split in 2, 4, 8 and so on, up to 1,024 pieces and 65,536 pieces in all
            // with the line segments, until every piece is within the tolerance of the line between its
            // ends and those lines enclose an area: with straight lines they are the result, and without
            // them the loop stays as it is. When no split within those limits encloses an area, the loop
            // stays as it is, or with straight lines each curved segment becomes the lines between its
            // samples, up to `MAX_RUN_RESULT` lines in all with the line segments; when those do not keep
            // some curved segment within the tolerance, the loop stays as it is. A loop whose points are
            // all in line keeps the lines of the refit.
            let near = |c: &CubicBez| [c.p1, c.p2].iter().all(|&p| Line::new(c.p0, c.p3).nearest(p, 1e-9).distance_sq.sqrt() <= opts.tolerance);
            let curved = segs.iter().filter(|s| !s.line).count();
            for k in 1..=10u32 {
                let parts = 1u32 << k;
                if curved.saturating_mul(parts as usize).saturating_add(n - curved) > 1 << 16 {
                    break;
                }
                let at = |i: u32| f64::from(i) / f64::from(parts);
                let pieces: Vec<CubicBez> = segs
                    .iter()
                    .flat_map(|s| {
                        let count = if s.line { 1 } else { parts };
                        (0..count).map(move |i| if s.line { s.c } else { s.c.subsegment(at(i)..at(i + 1)) })
                    })
                    .collect();
                let lines: Vec<Seg> = pieces.iter().map(|c| Seg::line(c.p0, c.p3)).collect();
                if pieces.iter().all(near) && !flat(&lines) {
                    if !opts.straight_lines {
                        return Some(sp.clone());
                    }
                    return segs_to_subpath(&lines, true);
                }
            }
            if !opts.straight_lines {
                return Some(sp.clone());
            }
            let Some(room) = sample_room(curved, n - curved) else {
                return Some(sp.clone());
            };
            let mut lines: Vec<Seg> = Vec::new();
            for s in &segs {
                if s.line {
                    lines.push(Seg::line(s.c.p0, s.c.p3));
                    continue;
                }
                let Some(steps) = steps_within(&s.c, opts.tolerance.max(1e-6), room) else {
                    return Some(sp.clone());
                };
                let mut pts = Vec::new();
                sample(&s.c, steps, &mut pts, true);
                lines.extend(pts.windows(2).map(|p| Seg::line(p[0], p[1])));
            }
            return segs_to_subpath(&lines, true);
        }
    }
    let s = segs_to_subpath(&result, sp.closed)?;
    Some(if opts.straight_lines { s } else { no_worse(sp, s) })
}

/// Most lines between samples of each of `curved` segments, so that they and the `kept` line segments
/// make at most [`MAX_RUN_RESULT`] lines, and each has at most [`MAX_SEGMENT_SAMPLES`]. `None` when
/// that leaves no line for some curved segment.
fn sample_room(curved: usize, kept: usize) -> Option<usize> {
    let left = MAX_RUN_RESULT.checked_sub(kept)?;
    if curved > left {
        return None;
    }
    Some((left / curved.max(1)).min(MAX_SEGMENT_SAMPLES))
}

/// How many evenly spaced lines along `c`, at most `room`, follow it: [`sample_steps`] when that
/// fits, or fewer, but never fewer than keep `c` within `tol` of them. `None` when those do not fit
/// in `room`.
fn steps_within(c: &CubicBez, tol: f64, room: usize) -> Option<usize> {
    let need = line_steps(c, tol).max(1);
    (need <= room).then(|| sample_steps(c, tol).min(room).max(need))
}

/// How far a segment may be from the lines between its samples, as a share of the tolerance.
const SAMPLE_GAP: f64 = 0.005;

/// Most samples of one segment. A segment that needs more stays as it is.
const MAX_SEGMENT_SAMPLES: usize = 1 << 16;

/// Most samples of one fit, which limits the memory a fit holds. A run with more is fitted in parts,
/// with an anchor between them.
pub(crate) const MAX_FIT_SAMPLES: usize = 1 << 20;

/// Most samples of one run, which limits its time. The segments after the run reaches this stay as
/// they are.
const MAX_RUN_SAMPLES: usize = 1 << 24;

/// Most segments of the result of one run before its other segments stay as they are, which limits
/// the memory it holds. A fit under way can add up to [`MAX_FIT_SAMPLES`] more.
pub(crate) const MAX_RUN_RESULT: usize = 1 << 21;

/// How many evenly spaced steps along `c` keep it within [`SAMPLE_GAP`] of `tol` from the lines
/// between its samples. Over a step of `1 / m`, a curve stays within `0.75 * s / (m * m)` of the line
/// between its ends, where `s` is its largest second difference. The samples of a straight segment
/// are on its line, so their spacing follows its length and `tol`, up to 256.
pub(crate) fn sample_steps(c: &CubicBez, tol: f64) -> usize {
    // Guarded: kurbo subdivides without end on huge finite coordinates.
    let len = measured_arclen(&PathSeg::Cubic(*c), 1e-6);
    let by_length = ((len / (tol * 0.5)).ceil() as usize).clamp(4, 256);
    if is_straight(c, tol * SAMPLE_GAP * 0.5) {
        return by_length;
    }
    by_length.max((0.75 * largest_second_difference(c) / (tol * SAMPLE_GAP)).sqrt().ceil() as usize)
}

/// How many evenly spaced straight lines keep `c` within `tol` of them.
fn line_steps(c: &CubicBez, tol: f64) -> usize {
    (0.75 * largest_second_difference(c) / tol).sqrt().ceil() as usize
}

/// The refit of the consecutive segments `run`, split at its corners sharper than the threshold:
/// it starts and ends where `run` does, leaving the same way.
fn simplify_run(run: &[Seg], opts: &SimplifyOptions) -> Vec<Seg> {
    simplify_run_within(run, opts, MAX_RUN_SAMPLES, None)
}

/// [`simplify_run`] with at most `budget` samples. A segment that needs more samples than
/// [`MAX_SEGMENT_SAMPLES`] or than the budget has left, or that comes after [`MAX_RUN_RESULT`] segments
/// of result, stays as it is, and the segments on each side of it are refitted without it. With
/// `straight_lines` such a segment becomes [`line_steps`] lines, or as many as those limits allow when
/// that is fewer, and then it can pass the tolerance. `ends`, when the whole run is refitted as one
/// part, is the direction the refit leaves and arrives along.
pub(crate) fn simplify_run_within(run: &[Seg], opts: &SimplifyOptions, mut budget: usize, ends: Option<Vec2>) -> Vec<Seg> {
    let tol = opts.tolerance.max(1e-6);
    let mut cuts: Vec<usize> = (1..run.len()).filter(|&i| turn_deg(&run[i - 1], &run[i]) > opts.corner_angle_deg).collect();
    cuts.insert(0, 0);
    cuts.push(run.len());
    let mut result: Vec<Seg> = Vec::new();
    // Whether `part` starts where `run` does and whether it ends where `run` does, for the free ends.
    let free = |part: &[(&Seg, usize)]| {
        let starts = part.first().zip(run.first()).is_some_and(|((s, _), r)| std::ptr::eq(*s, r));
        let ends = part.last().zip(run.last()).is_some_and(|((s, _), r)| std::ptr::eq(*s, r));
        [opts.free_start && starts, opts.free_end && ends]
    };
    for w in cuts.windows(2) {
        let piece = &run[w[0]..w[1]];
        let mut part: Vec<(&Seg, usize)> = Vec::new();
        let mut part_samples = 0;
        for s in piece {
            let steps = sample_steps(&s.c, tol);
            if steps <= budget.min(MAX_SEGMENT_SAMPLES) && result.len() < MAX_RUN_RESULT {
                if part_samples + steps > MAX_FIT_SAMPLES {
                    refit_part(&part, opts, tol, None, free(&part), &mut result);
                    part.clear();
                    part_samples = 0;
                }
                budget -= steps;
                part_samples += steps;
                part.push((s, steps));
            } else {
                part_samples = 0;
                refit_part(&part, opts, tol, None, free(&part), &mut result);
                part.clear();
                if opts.straight_lines {
                    let room = MAX_RUN_RESULT.saturating_sub(result.len());
                    let lines = line_steps(&s.c, tol).min(budget.min(MAX_SEGMENT_SAMPLES)).min(room).max(1);
                    budget = budget.saturating_sub(lines);
                    let mut pts = Vec::new();
                    sample(&s.c, lines, &mut pts, true);
                    result.extend(pts.windows(2).map(|p| Seg::line(p[0], p[1])));
                } else {
                    result.push(*s);
                }
            }
        }
        refit_part(&part, opts, tol, ends.filter(|_| part.len() == run.len()), free(&part), &mut result);
    }
    result
}

/// Adds the refit of `part`, consecutive segments with no corner between them and the steps to
/// sample each one with, to `result`. `ends`, given only when `part` is a whole closed loop, is the
/// direction it leaves and arrives along; when the loop is smoothed, the smoothing runs across its
/// start and the direction comes from the smoothed samples there. `free` says whether the start and
/// the end of `part` are free ends of an open subpath: when it is smoothed, the refit leaves such a
/// start and arrives at such an end along [`free_end_direction`], or along its first or last segment
/// when that gives none or turns from the segment by 90° or more.
fn refit_part(part: &[(&Seg, usize)], opts: &SimplifyOptions, tol: f64, mut ends: Option<Vec2>, free: [bool; 2], result: &mut Vec<Seg>) {
    let (Some((first, _)), Some((last, _))) = (part.first(), part.last()) else { return };
    let mut pts = Vec::new();
    for (k, (s, steps)) in part.iter().enumerate() {
        sample(&s.c, *steps, &mut pts, k == 0);
    }
    let mut tol = tol;
    let (mut t0, mut t1) = (start_tangent(&first.c), end_tangent(&last.c));
    if opts.smooth > 0.0 {
        let smoothed = if ends.is_some() { smooth_loop_points(&pts, opts.smooth) } else { smooth_points(&pts, opts.smooth) };
        if ends.is_some()
            && let (Some(&next), Some(&prev)) = (smoothed.get(1), smoothed.len().checked_sub(2).and_then(|i| smoothed.get(i)))
        {
            ends = unit(next - prev).or(ends);
        }
        if ends.is_none() {
            if free[0]
                && let Some(d) = free_end_direction(&smoothed, opts.smooth, false).filter(|d| d.dot(t0) > 0.0)
            {
                t0 = d;
            }
            if free[1]
                && let Some(d) = free_end_direction(&smoothed, opts.smooth, true).filter(|d| d.dot(t1) > 0.0)
            {
                t1 = d;
            }
        }
        if opts.smooth_moved_only {
            tol = tol.min((2.0 * smoothing_moved(&pts, &smoothed)).max(tol * SMOOTH_MOVED_FLOOR));
        }
        pts = smoothed;
    }
    if opts.straight_lines {
        let keep = rdp(&pts, tol * (1.0 - SAMPLE_GAP));
        result.extend(keep.windows(2).map(|p| Seg::line(p[0], p[1])));
        return;
    }
    // The fit may be `tol * 0.98` from the lines between the samples, those lines may be
    // `SAMPLE_GAP` of `tol` from the path, and the curve the test below turns into a line moves by up
    // to 1.5% of `tol`: 3/4 of how far its handles may be from the line. The fitted curves join
    // smoothly, so a curve becomes a line only when the line keeps both of its joins smooth. The last
    // curve of a loop fitted across its start leaves along the loop's first segment.
    let (t0, t1) = ends.map_or((t0, t1), |d| (d, d));
    let fitted = fit_cubics(&pts, t0, t1, tol * 0.98);
    let from = result.len();
    let mut arrive = t0;
    for (k, &c) in fitted.iter().enumerate() {
        let leave = match fitted.get(k + 1) {
            Some(next) => start_tangent(next),
            None if ends.is_some() => result.get(from).map_or(t1, |s| start_tangent(&s.c)),
            None => t1,
        };
        let line = is_straight(&c, tol * 0.02) && line_keeps_joins(&c, arrive, leave);
        arrive = if line { unit(c.p3 - c.p0).unwrap_or(arrive) } else { end_tangent(&c) };
        result.push(if line { Seg::line(c.p0, c.p3) } else { Seg { c, line } });
    }
}

/// Most degrees between the line from a refitted curve's start to its end and the directions the
/// path arrives and leaves along, for the curve to become that line: so every join next to such a
/// line turns by at most this much. VectorCraft's own value.
const LINE_JOIN_DEG: f64 = 0.5;

/// Whether the line from `c.p0` to `c.p3` is within [`LINE_JOIN_DEG`] of `arrive`, the direction the
/// path arrives at `c.p0` along, and of `leave`, the direction it leaves `c.p3` along. A curve whose
/// ends are at one point, or a direction of no length, gives `false`.
fn line_keeps_joins(c: &CubicBez, arrive: Vec2, leave: Vec2) -> bool {
    let (Some(dir), Some(arrive), Some(leave)) = (unit(c.p3 - c.p0), unit(arrive), unit(leave)) else { return false };
    let min = LINE_JOIN_DEG.to_radians().cos();
    dir.dot(arrive) >= min && dir.dot(leave) >= min
}

/// `pts` (a run's samples, in order) smoothed along the path: each point becomes the Gaussian-weighted
/// mean of the points around it, `sigma` points of arc length wide, the window narrowing towards the
/// ends so that the first and last points stay where they are. Consecutive samples within
/// 1/[`SMOOTH_GROUP`] of a spread of each other along the path are weighed together, as their count at
/// their mean place, so the work per point is bounded by the groups within three spreads of it however
/// densely the path is sampled.
fn smooth_points(pts: &[Point], sigma: f64) -> Vec<Point> {
    smooth_points_counted(pts, sigma, &mut 0)
}

/// [`smooth_points`] for the samples `pts` of a whole closed loop, whose last sample is its first:
/// the samples at its start are smoothed like the others, and the first and last ones move together.
/// The loop is smoothed twice as an open run of the same samples, once from its start and once from
/// the sample nearest halfway round by length, where [`smooth_points`] keeps each run's ends in place.
/// Each sample takes a blend of the two that runs smoothly round the loop, with weights
/// `(1 ± cos θ) / 2`: `θ` goes from 0 at the start to π at the halfway sample and on to 2π at the end,
/// in proportion to length, so the start takes only the second run and the halfway sample only the
/// first, and no sample takes any of a run's fixed ends.
fn smooth_loop_points(pts: &[Point], sigma: f64) -> Vec<Point> {
    let Some(m) = pts.len().checked_sub(1).filter(|&m| m >= 3) else { return smooth_points(pts, sigma) };
    let mut along = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).and_then(|j| pts.get(j)) {
            total += prev.distance(*p);
        }
        along.push(total);
    }
    if !(total.is_finite() && total > 0.0) {
        return smooth_points(pts, sigma);
    }
    let half = (1..m).min_by(|&a, &b| (along[a] - total / 2.0).abs().total_cmp(&(along[b] - total / 2.0).abs())).unwrap_or(m / 2);
    let mid = along[half];
    if !(mid > 0.0 && mid < total) {
        return smooth_points(pts, sigma);
    }
    let ring = &pts[..m];
    let turned: Vec<Point> = ring[half..].iter().chain(&ring[..=half]).copied().collect();
    let from_half = smooth_points(&turned, sigma);
    let mut out = smooth_points(pts, sigma);
    for (i, (p, &s)) in out.iter_mut().zip(&along).enumerate() {
        // Sample `i` of the loop is sample `i - half` of `turned`, round the loop.
        let Some(&q) = from_half.get((i + m - half) % m) else { continue };
        let theta = if s <= mid { s / mid } else { 1.0 + (s - mid) / (total - mid) } * std::f64::consts::PI;
        let w = 0.5 * (1.0 + theta.cos());
        *p = p.lerp(q, w);
    }
    if let Some(&start) = out.first()
        && let Some(end) = out.last_mut()
    {
        *end = start;
    }
    out
}

/// How far [`smooth_points`] moved the samples `pts` to `smoothed` (the same number of points, in
/// order) across the path: the largest distance from a smoothed point to the lines between the
/// samples up to two on either side of the one it came from.
fn smoothing_moved(pts: &[Point], smoothed: &[Point]) -> f64 {
    smoothed
        .iter()
        .enumerate()
        .map(|(i, &q)| {
            let own = pts.get(i).map_or(f64::INFINITY, |p| p.distance(q));
            let (lo, hi) = (i.saturating_sub(2), (i + 2).min(pts.len().saturating_sub(1)));
            (lo..hi).filter_map(|j| Some(Line::new(*pts.get(j)?, *pts.get(j + 1)?).nearest(q, 1e-9).distance_sq.sqrt())).fold(own, f64::min)
        })
        .fold(0.0, f64::max)
}

/// How far along the path, in spreads of the smoothing, [`free_end_direction`] looks from the end:
/// as far as the Gaussian reaches. VectorCraft's own value.
const FREE_END_SPREADS: f64 = 3.0;

/// [`free_end_direction`] gives no direction when the path near the end spreads out from it more
/// evenly than this: the ratio of the least to the most squared distance from a line through the end,
/// over all such lines. VectorCraft's own value.
const FREE_END_MAX_SPREAD: f64 = 1.0 / 3.0;

/// The direction the samples `pts` of an open run leave their first point along, or with `at_end`
/// arrive at their last point along: the direction of the line through that point that the path
/// within [`FREE_END_SPREADS`] spreads `sigma` of it along its length, and at most half its length,
/// lies closest to by the least sum of squared distances, pointing along the path. `None` when the
/// path there has no length or is not finite, when its mean point is within a tenth of its length
/// there of that point (it doubles back), or when it spreads from that point more evenly than
/// [`FREE_END_MAX_SPREAD`].
fn free_end_direction(pts: &[Point], sigma: f64, at_end: bool) -> Option<Vec2> {
    let n = pts.len();
    let total: f64 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let reach = (FREE_END_SPREADS * sigma).min(total / 2.0);
    if !(reach.is_finite() && reach > 0.0) {
        return None;
    }
    let at = |i: usize| pts.get(if at_end { n - 1 - i } else { i }).copied();
    let origin = at(0)?;
    // The integrals along the path of v vᵀ and of v, for v from the end to each point.
    let (mut sxx, mut sxy, mut syy, mut sum, mut along) = (0.0, 0.0, 0.0, Vec2::ZERO, 0.0);
    let mut a = Vec2::ZERO;
    for i in 1..n {
        let Some(p) = at(i) else { break };
        let mut b = p - origin;
        let mut len = (b - a).hypot();
        if along + len > reach {
            b = a.lerp(b, (reach - along) / len);
            len = reach - along;
        }
        sxx += len * (a.x * a.x + a.x * b.x + b.x * b.x) / 3.0;
        syy += len * (a.y * a.y + a.y * b.y + b.y * b.y) / 3.0;
        sxy += len * (2.0 * a.x * a.y + a.x * b.y + b.x * a.y + 2.0 * b.x * b.y) / 6.0;
        sum += (a + b) * (len / 2.0);
        along += len;
        a = b;
        if along >= reach {
            break;
        }
    }
    let (trace, apart) = (sxx + syy, (sxx - syy).hypot(2.0 * sxy));
    // The two principal values are (trace ± apart) / 2.
    if !(trace.is_finite() && apart.is_finite() && trace > 0.0) || trace - apart > FREE_END_MAX_SPREAD * (trace + apart) {
        return None;
    }
    let mean = sum / along;
    if !mean.hypot().is_finite() || mean.hypot() < 0.1 * along {
        return None;
    }
    let d = Vec2::from_angle(0.5 * (2.0 * sxy).atan2(sxx - syy));
    let d = if d.dot(mean) >= 0.0 { d } else { -d };
    unit(if at_end { -d } else { d })
}

/// How many groups of samples a spread holds at most in [`smooth_points`]. Weighing them in groups
/// leaves each smoothed point within a thousandth of a spread of the mean of its samples weighed one
/// by one.
const SMOOTH_GROUP: f64 = 16.0;

/// [`smooth_points`], adding the number of sample groups it looks at to `work`.
fn smooth_points_counted(pts: &[Point], sigma: f64, work: &mut usize) -> Vec<Point> {
    let Some(&origin) = pts.first() else { return Vec::new() };
    if pts.len() < 3 || !(sigma.is_finite() && sigma > 0.0) {
        return pts.to_vec();
    }
    let mut s = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).and_then(|j| pts.get(j)) {
            total += prev.distance(*p);
        }
        s.push(total);
    }
    let groups = SampleGroups::new(pts, &s, origin);
    let (mut lo, mut hi) = (0usize, 0usize);
    pts.iter()
        .zip(&s)
        .map(|(&p, &si)| {
            let reach = (3.0 * sigma).min(si).min(total - si);
            if reach <= 1e-9 {
                return p;
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
            *work += acc.work;
            if acc.weight > 0.0 { origin + acc.sum / acc.weight } else { p }
        })
        .collect()
}

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
    fn new(pts: &[Point], s: &'a [f64], origin: Point) -> Self {
        let size = 4 * pts.len();
        let mut groups = Self { s, sum_s: vec![0.0; size], sum_q: vec![Vec2::ZERO; size] };
        groups.build(0, 0, pts.len(), pts, origin);
        groups
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

/// What [`SampleGroups::weigh`] has added up: the weighted sum of the samples (from the first
/// sample), the sum of the weights, and the number of nodes looked at.
#[derive(Default)]
struct Weighed {
    sum: Vec2,
    weight: f64,
    work: usize,
}

/// What [`simplify_part`] made of a path.
#[derive(Clone, Debug, PartialEq)]
pub struct SimplifiedPart {
    pub path: PathData,
    /// The segments of the simplified runs, as (subpath, segment) in `path`.
    pub segments: BTreeSet<(usize, usize)>,
    /// Where each anchor of the original path that is still there went: (subpath, anchor) in the
    /// original → in `path`. Anchors inside a simplified run are gone; every other anchor is kept.
    pub anchors: BTreeMap<(usize, usize), (usize, usize)>,
}

/// Simplify only the `segments` (subpath, segment) of `path`: each run of consecutive segments
/// among them (a closed subpath's run may cross its start point) is refitted on its own between its
/// two end anchors, which stay where they are with their handles on the side away from the run.
/// The other segments and anchors stay exactly as they are. A subpath whose every segment is in
/// `segments` is simplified whole, as [`simplify_with`] does. A run whose refit would have more
/// segments than it had is left as it is (except with Convert to Straight Lines).
pub fn simplify_part(path: &PathData, segments: &BTreeSet<(usize, usize)>, opts: &SimplifyOptions) -> SimplifiedPart {
    let opts = &SimplifyOptions { free_start: false, free_end: false, ..*opts };
    let (mut runs, mut anchors) = (BTreeSet::new(), BTreeMap::new());
    let mut subpaths = Vec::with_capacity(path.subpaths.len());
    for (si, sp) in path.subpaths.iter().enumerate() {
        let n = sp.segment_count();
        let picked: Vec<bool> = (0..n).map(|i| segments.contains(&(si, i))).collect();
        let ni = subpaths.len();
        if !picked.contains(&true) {
            anchors.extend((0..sp.anchors.len()).map(|ai| ((si, ai), (ni, ai))));
            subpaths.push(sp.clone());
        } else if !picked.contains(&false) {
            if let Some(s) = simplify_subpath(sp, opts) {
                runs.extend((0..s.segment_count()).map(|i| (ni, i)));
                subpaths.push(s);
            }
        } else {
            let (s, segs, kept) = simplify_runs(sp, &picked, opts);
            runs.extend(segs.into_iter().map(|i| (ni, i)));
            anchors.extend(kept.into_iter().map(|(old, new)| ((si, old), (ni, new))));
            subpaths.push(s);
        }
    }
    SimplifiedPart { path: PathData::new(subpaths), segments: runs, anchors }
}

/// [`simplify_part`] for a subpath with some but not all segments `picked`: → (the subpath, the
/// segments of its refitted runs, old → new index of each anchor kept).
fn simplify_runs(sp: &SubPath, picked: &[bool], opts: &SimplifyOptions) -> (SubPath, Vec<usize>, Vec<(usize, usize)>) {
    let m = sp.anchors.len();
    let n = picked.len();
    // An anchor inside a run (both its segments picked) goes; a closed subpath's walk starts at the
    // first anchor that stays, so that no run crosses the start of the walk.
    let inside = |a: usize| (sp.closed && a == 0 && picked[n - 1] && picked[0]) || (a > 0 && a < n && picked[a - 1] && picked[a]);
    let start = (0..m).find(|&a| !inside(a)).unwrap_or(0);
    let mut anchors = vec![sp.anchors[start]];
    let mut kept = vec![(start, 0)];
    let mut runs = vec![];
    let mut k = 0;
    while k < n {
        let j = (start + k) % n;
        if !picked[j] {
            let next = (j + 1) % m;
            if k + 1 < n || !sp.closed {
                kept.push((next, anchors.len()));
                anchors.push(sp.anchors[next]);
            }
            k += 1;
            continue;
        }
        let len = (k..n).take_while(|&i| picked[(start + i) % n]).count();
        let run: Vec<Seg> = (k..k + len).map(|i| (start + i) % n).map(|i| Seg { c: sp.segment(i), line: sp.segment_is_line(i) }).collect();
        let end = (start + k + len) % m;
        let fitted = simplify_run(&run, opts);
        let keep_original = fitted.is_empty() || !opts.straight_lines && fitted.len() > run.len();
        let first = anchors.len() - 1;
        if keep_original {
            anchors.extend((k + 1..k + len).map(|i| sp.anchors[(start + i) % m]));
        } else {
            let a = anchors[first];
            anchors[first] = with_handle(a, true, if fitted[0].line { a.p } else { fitted[0].c.p1 });
            for w in fitted.windows(2) {
                let p = w[1].c.p0;
                let h_in = if w[0].line { p } else { w[0].c.p2 };
                let h_out = if w[1].line { p } else { w[1].c.p1 };
                anchors.push(Anchor::with_handles(p, h_in, h_out));
            }
        }
        // The run's segments, from its first anchor on: a run back to the walk's start ends with
        // the closing segment, the last index.
        runs.extend(first..first + if keep_original { run.len() } else { fitted.len() });
        let end_seg = fitted.last().filter(|_| !keep_original);
        if sp.closed && k + len == n {
            // The run ends where the walk started.
            if let Some(s) = end_seg {
                anchors[0] = with_handle(anchors[0], false, if s.line { anchors[0].p } else { s.c.p2 });
            }
        } else {
            let a = sp.anchors[end];
            kept.push((end, anchors.len()));
            anchors.push(match end_seg {
                Some(s) => with_handle(a, false, if s.line { a.p } else { s.c.p2 }),
                None => a,
            });
        }
        k += len;
    }
    (SubPath::new(anchors, sp.closed), runs, kept)
}

/// `a` with its out (`out`) or in handle at `h` and the other as it is: still smooth when it was
/// and its handles still line up, a corner otherwise.
fn with_handle(a: Anchor, out: bool, h: Point) -> Anchor {
    let (h_in, h_out) = if out { (a.h_in, h) } else { (h, a.h_out) };
    let kind = if a.kind == AnchorKind::Smooth { Anchor::with_handles(a.p, h_in, h_out).kind } else { AnchorKind::Corner };
    Anchor { p: a.p, h_in, h_out, kind }
}

/// Ramer–Douglas–Peucker polyline reduction. Each point dropped is within `tol` of the line segment
/// that replaces it: a point beyond either end of the segment is measured to that end.
fn rdp(pts: &[Point], tol: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
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
        let (pa, pb) = (pts[a], pts[b]);
        let chord = pb - pa;
        let len2 = chord.hypot2();
        let mut best = (0.0, a);
        for (i, &p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len2 < 1e-24 { p.distance(pa) } else { p.distance(pa + chord * ((p - pa).dot(chord) / len2).clamp(0.0, 1.0)) };
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
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Object → Path → Smooth: pull handles towards Catmull-Rom-like tangents. `amount` ∈ [0, 1]
/// (0 = unchanged, 1 = fully smooth). Endpoints of open paths keep their handles.
pub fn smooth(path: &PathData, amount: f64) -> PathData {
    let t = amount.clamp(0.0, 1.0);
    let mut out = path.clone();
    for sp in &mut out.subpaths {
        let n = sp.anchors.len();
        if n < 3 {
            continue;
        }
        let orig = sp.anchors.clone();
        for i in 0..n {
            if !sp.closed && (i == 0 || i == n - 1) {
                continue;
            }
            let prev = orig[(i + n - 1) % n].p;
            let next = orig[(i + 1) % n].p;
            let p = orig[i].p;
            let Some(dir) = crate::fit::unit(next - prev) else { continue };
            let tin = p - dir * (p.distance(prev) / 3.0);
            let tout = p + dir * (p.distance(next) / 3.0);
            let a = &orig[i];
            let h_in = a.h_in.lerp(tin, t);
            let h_out = a.h_out.lerp(tout, t);
            sp.anchors[i] = Anchor::with_handles(p, h_in, h_out);
        }
    }
    out
}

/// The Smooth slider's level Auto-Smooth picks, halfway between minimum and maximum smoothing.
/// VectorCraft's own rule: the help page says only that Auto-Smooth smooths the path automatically
/// (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/refine-path-segments-with-the-smooth-slider.html).
/// [`smooth_level`] already scales with each path's size, so one level suits every path.
pub const SMOOTH_AUTO_LEVEL: f64 = 50.0;

/// The refit Object › Path › Smooth makes of `path` at slider level `level` ([`smooth_level`]): a
/// corner only where the path turns back on itself, the path smoothed along its length by up to 5 %
/// of its longer side (none at level 0) and refitted within 0.1 % (level 0) to 2.5 % (level 100) of
/// it. A level outside 0..=100 counts as the nearer end, and one that is not a number as
/// [`SMOOTH_AUTO_LEVEL`]; a path of no finite size gets no smoothing.
pub fn smooth_options(path: &PathData, level: f64) -> SimplifyOptions {
    let level = if level.is_finite() { level.clamp(0.0, 100.0) } else { SMOOTH_AUTO_LEVEL };
    let size = path.bounds().map_or(0.0, |b| b.width().max(b.height()));
    let size = if size.is_finite() { size } else { 0.0 };
    SimplifyOptions {
        tolerance: size * (0.001 + 0.024 * level / 100.0),
        corner_angle_deg: SMOOTH_CORNER_DEG,
        straight_lines: false,
        smooth: size * 0.05 * level / 100.0,
        smooth_moved_only: false,
        free_start: false,
        free_end: false,
    }
}

/// Object › Path › Smooth at slider level `level`, from 0 (minimum smoothing: the path stays within
/// 0.1 % of its longer side of where it was) to 100 (maximum). The help page names only the two
/// ends; the rest is VectorCraft's own rule ([`smooth_options`]). Each subpath is smoothed along its
/// length, so that bumps and jags give way to one curve, then refitted with curves as Simplify
/// does, keeping a corner only where the path turns back on itself. A refit never has more anchor
/// points than the subpath had: a shape whose corners it can't round, such as a rectangle, stays as
/// it is. Closed subpaths stay closed, with no kink left where they start unless rounding it would
/// pass the tolerance, and open ones keep their end points.
pub fn smooth_level(path: &PathData, level: f64) -> PathData {
    let opts = smooth_options(path, level);
    if opts.tolerance <= 0.0 {
        return path.clone();
    }
    let smoothed = |sp: &SubPath| {
        let Some(mut fit) = simplify_subpath(sp, &opts).filter(|f| f != sp) else { return sp.clone() };
        smooth_seam(&mut fit, sp, opts.tolerance);
        fit
    };
    PathData::new(path.subpaths.iter().map(smoothed).collect())
}

/// [`smooth_level`] keeps a corner only where the path turns by more than this (turns back).
const SMOOTH_CORNER_DEG: f64 = 179.0;

/// A closed subpath's refit leaves its start anchor with the tangents of the original segments on
/// either side of it, a kink when the path had one there: turn it into a smooth anchor with the same
/// handle lengths, unless the path turns back there or the rounded path would stray from `original`
/// by more than `tol` (or than the refit already does).
fn smooth_seam(sp: &mut SubPath, original: &SubPath, tol: f64) {
    if !sp.closed {
        return;
    }
    let Some(a) = sp.anchors.first() else { return };
    let (Some(i), Some(o)) = (crate::fit::unit(a.p - a.h_in), crate::fit::unit(a.h_out - a.p)) else { return };
    if i.dot(o) < SMOOTH_CORNER_DEG.to_radians().cos() {
        return;
    }
    let Some(dir) = crate::fit::unit(i + o) else { return };
    let mut rounded = sp.clone();
    if let Some(r) = rounded.anchors.first_mut() {
        *r = Anchor::with_handles(a.p, a.p - dir * a.h_in.distance(a.p), a.p + dir * a.h_out.distance(a.p));
    }
    if seam_error(original, sp, &rounded, tol) <= tol.max(seam_error(original, sp, sp, tol)) {
        *sp = rounded;
    }
}

/// How far the segments on either side of the start anchor of `fit` (a refit of the closed
/// `original` whose other segments match `before`) are from `original`, both ways: the largest
/// distance from those segments to `original`, and from the parts of `original` that `before`'s
/// segments there cover (within `tol`) to `fit`.
fn seam_error(original: &SubPath, before: &SubPath, fit: &SubPath, tol: f64) -> f64 {
    use kurbo::ParamCurveNearest;
    let n = fit.segment_count();
    if n == 0 || before.segment_count() != n || original.segment_count() == 0 {
        return f64::INFINITY;
    }
    let is_seam = |j: usize| j == 0 || j + 1 == n;
    let dist = |q: Point, sp: &SubPath, seam: bool| {
        (0..sp.segment_count())
            .filter(|&j| is_seam(j) == seam)
            .map(|j| sp.segment(j).nearest(q, 1e-9).distance_sq)
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    };
    let samples = |sp: &SubPath, seam_only: bool| {
        let mut pts = Vec::new();
        for j in (0..sp.segment_count()).filter(|&j| !seam_only || is_seam(j)) {
            let c = sp.segment(j);
            let m = ((measured_arclen(&PathSeg::Cubic(c), 1e-6) / (tol.max(1e-6) * 0.5)).ceil() as usize).clamp(4, 256);
            sample(&c, m, &mut pts, true);
        }
        pts
    };
    let all = |q: Point| dist(q, original, true).min(dist(q, original, false));
    let forward = samples(fit, true).into_iter().map(all).fold(0.0, f64::max);
    let back = samples(original, false)
        .into_iter()
        .filter(|&q| dist(q, before, true) <= tol)
        .map(|q| {
            let d = dist(q, fit, true);
            if d > tol { d.min(dist(q, fit, false)) } else { d }
        })
        .fold(0.0, f64::max);
    forward.max(back)
}

/// Remove anchors that can be deleted without changing the shape by more than `tolerance`
/// (collinear corner points, duplicate points, and curve joints that one cubic can replace).
pub fn remove_redundant_points(path: &PathData, tolerance: f64) -> PathData {
    let tol = tolerance.max(1e-9);
    let mut out = Vec::new();
    for sp in &path.subpaths {
        let mut segs = subpath_segs(sp);
        if segs.is_empty() {
            out.push(sp.clone());
            continue;
        }
        // Drop zero-length segments.
        segs.retain(|s| !(s.c.p0.distance(s.c.p3) < 1e-9 && s.c.p1.distance(s.c.p0) < 1e-9 && s.c.p2.distance(s.c.p0) < 1e-9));
        let min_segs = if sp.closed { 2 } else { 1 };
        // Joints are merged along the path in one pass (a merged segment is tried with the one
        // before it, then the next); the search below then only finds what a closed path's wrap
        // leaves. (Merging at the first joint of a dense line one at a time shifts the rest each time.)
        let total = segs.len();
        let mut merged = 0;
        let mut done: Vec<Seg> = Vec::with_capacity(total);
        for s in segs.drain(..) {
            done.push(s);
            while done.len() >= 2 && total - merged > min_segs {
                let n = done.len();
                let Some(m) = merge_pair(&done[n - 2], &done[n - 1], tol) else { break };
                done.truncate(n - 1);
                done[n - 2] = m;
                merged += 1;
            }
        }
        segs = done;
        let mut changed = true;
        while changed && segs.len() > min_segs {
            changed = false;
            let n = segs.len();
            let joints = if sp.closed { n } else { n - 1 };
            for j in 0..joints {
                let (i0, i1) = if sp.closed { ((j + n - 1) % n, j) } else { (j, j + 1) };
                if let Some(m) = merge_pair(&segs[i0], &segs[i1], tol) {
                    segs[i0] = m;
                    segs.remove(i1);
                    if sp.closed && i1 == 0 {
                        // The merged segment now starts the loop.
                        if let Some(last) = segs.pop() {
                            segs.insert(0, last);
                        }
                    }
                    changed = true;
                    break;
                }
            }
        }
        if let Some(s) = segs_to_subpath(&segs, sp.closed) {
            out.push(s);
        }
    }
    PathData::new(out)
}

fn merge_pair(a: &Seg, b: &Seg, tol: f64) -> Option<Seg> {
    if a.line && b.line {
        let chord = b.c.p3 - a.c.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let mid = a.c.p3 - a.c.p0;
        let along = mid.dot(chord) / len;
        return ((mid.cross(chord) / len).abs() <= tol && along >= 0.0 && along <= len).then(|| Seg::line(a.c.p0, b.c.p3));
    }
    // Only merge across (near-)smooth joints.
    if end_tangent(&a.c).dot(start_tangent(&b.c)) < 0.99 {
        return None;
    }
    let mut pts = Vec::new();
    sample(&a.c, 16, &mut pts, true);
    sample(&b.c, 16, &mut pts, false);
    let (c, err, _) = fit_single(&pts, start_tangent(&a.c), end_tangent(&b.c));
    (err <= tol).then_some(Seg { c, line: false })
}

/// Remove anchor `index` without opening the path (the Delete Anchor Point tool and Object → Path
/// → Remove Anchor Points). Its neighbours keep their handle directions and their facing handles
/// are refitted so one cubic follows the two segments that met there; two straight segments
/// become one. Removing an end of an open path drops its segment. Returns false, leaving `sp`
/// alone, when `index` is out of range.
pub fn remove_anchor(sp: &mut SubPath, index: usize) -> bool {
    let n = sp.anchors.len();
    if index >= n {
        return false;
    }
    if (sp.closed || (index > 0 && index + 1 < n)) && n >= 3 {
        refit_across(&mut sp.anchors, (index + n - 1) % n, index, (index + 1) % n);
    }
    #[cfg(test)]
    tests::count_moves(n - index - 1);
    sp.anchors.remove(index);
    if !sp.closed {
        // A new end has nothing beyond it.
        if let Some(a) = sp.anchors.first_mut() {
            a.h_in = a.p;
        }
        if let Some(a) = sp.anchors.last_mut() {
            a.h_out = a.p;
        }
    }
    if sp.anchors.len() < 3 {
        sp.closed = sp.closed && sp.anchors.len() == 2 && sp.anchors.iter().any(|a| a.has_in() || a.has_out());
    }
    true
}

/// Refit the facing handles of anchors `pi` and `ni` so one cubic follows the two segments that
/// meet at anchor `index` between them ([`remove_anchor`]). Two straight segments are left as
/// they are, and so are the handles when the fit is not finite.
fn refit_across(anchors: &mut Anchors, pi: usize, index: usize, ni: usize) {
    let (Some(&a), Some(&b), Some(&c)) = (anchors.get(pi), anchors.get(index), anchors.get(ni)) else { return };
    let line = |a: &Anchor, b: &Anchor| !a.has_out() && !b.has_in();
    if line(&a, &b) && line(&b, &c) {
        return;
    }
    let (left, right) = (CubicBez::new(a.p, a.h_out, b.h_in, b.p), CubicBez::new(b.p, b.h_out, c.h_in, c.p));
    const N: usize = 16;
    let mut pts = Vec::with_capacity(2 * N + 1);
    sample(&left, N, &mut pts, true);
    sample(&right, N, &mut pts, false);
    let (t0, t1) = (start_tangent(&left), end_tangent(&right));
    // The new curve passes the removed point at some parameter k: the left samples sit at k·i/N,
    // the right ones at k + (1 − k)·i/N. Search k for the closest fit.
    let fit = |k: f64| {
        let u = (0..=N).map(|i| k * i as f64 / N as f64).chain((1..=N).map(|i| k + (1.0 - k) * i as f64 / N as f64)).collect();
        fit_single_from(&pts, u, t0, t1)
    };
    let (curve, _, _) = golden_min(fit, 0.0, 1.0);
    if curve.p1.is_finite() && curve.p2.is_finite() {
        anchors[pi].h_out = curve.p1;
        anchors[ni].h_in = curve.p2;
    }
}

/// [`remove_anchor`] for each of `indexes`, the last first, with the same result, moving each
/// anchor that stays at most once: removing many anchors from a long subpath one at a time shifts
/// the rest of it each time. Indexes out of range and repeats are skipped. Returns how many
/// anchors were removed.
pub fn remove_anchors(sp: &mut SubPath, indexes: &[usize]) -> usize {
    let total = sp.anchors.len();
    let mut targets: Vec<usize> = indexes.iter().copied().filter(|&i| i < total).collect();
    targets.sort_unstable_by(|a, b| b.cmp(a));
    targets.dedup();
    if targets.is_empty() {
        return 0;
    }
    // Every anchor below the one being removed is still there, so it keeps its index, the one
    // before it is `index - 1` and the first one left is anchor 0 until anchor 0 itself goes.
    let mut gone = vec![false; total];
    let mut len = total;
    let mut last = total - 1;
    // The lowest anchor removed so far and the first one left after it.
    let (mut lowest, mut after_lowest) = (total, None);
    for &index in &targets {
        let next = if index + 1 < lowest { Some(index + 1) } else { after_lowest };
        if (sp.closed || (index > 0 && next.is_some())) && len >= 3 {
            let pi = if index > 0 { index - 1 } else { last };
            refit_across(&mut sp.anchors, pi, index, next.unwrap_or(0));
        }
        gone[index] = true;
        len -= 1;
        (lowest, after_lowest) = (index, next);
        if next.is_none() {
            last = index.saturating_sub(1);
        }
        if len == 0 {
            continue;
        }
        let first = if index > 0 { 0 } else { next.unwrap_or(0) };
        if !sp.closed {
            // A new end has nothing beyond it.
            if let Some(a) = sp.anchors.get_mut(first) {
                a.h_in = a.p;
            }
            if let Some(a) = sp.anchors.get_mut(last) {
                a.h_out = a.p;
            }
        }
        if len < 3 {
            let ends = [first, last].map(|i| sp.anchors.get(i).is_some_and(|a| a.has_in() || a.has_out()));
            sp.closed = sp.closed && len == 2 && (ends[0] || ends[1]);
        }
    }
    if len == 0 {
        sp.closed = false;
    }
    let mut kept = 0;
    for (i, &g) in gone.iter().enumerate() {
        if !g {
            if kept != i {
                sp.anchors[kept] = sp.anchors[i];
                #[cfg(test)]
                tests::count_moves(1);
            }
            kept += 1;
        }
    }
    sp.anchors.truncate(kept);
    targets.len()
}

/// The fit `f` (curve, error, worst point) with the least error for an argument in `[a, b]`, by a
/// golden-section search (a bounded number of steps).
fn golden_min(f: impl Fn(f64) -> (CubicBez, f64, usize), mut a: f64, mut b: f64) -> (CubicBez, f64, usize) {
    const R: f64 = 0.618_033_988_749_895;
    let (mut x1, mut x2) = (b - R * (b - a), a + R * (b - a));
    let (mut f1, mut f2) = (f(x1), f(x2));
    for _ in 0..40 {
        // A NaN error never wins.
        if f1.1 < f2.1 || f2.1.is_nan() {
            (b, x2, f2) = (x2, x1, f1);
            x1 = b - R * (b - a);
            f1 = f(x1);
        } else {
            (a, x1, f1) = (x1, x2, f2);
            x2 = a + R * (b - a);
            f2 = f(x2);
        }
    }
    if f1.1 <= f2.1 { f1 } else { f2 }
}

/// Object → Path → Add Anchor Points: one new anchor at the middle (t = 0.5) of every segment.
pub fn add_anchor_points(path: &PathData) -> PathData {
    let mut out = path.clone();
    for sp in &mut out.subpaths {
        let n = sp.segment_count();
        if n == 0 {
            continue;
        }
        // Built in one pass (inserting into the middle of the vector shifts the rest each time):
        // anchor k lands at 2k and the new anchor of segment k at 2k + 1.
        let mut v = Vec::with_capacity(sp.anchors.len() * 2);
        for (k, a) in sp.anchors.iter().enumerate() {
            v.push(*a);
            if k < n {
                v.push(*a);
            }
        }
        for seg in 0..n {
            let c = sp.segment(seg);
            let (l, r) = (c.subsegment(0.0..0.5), c.subsegment(0.5..1.0));
            if sp.segment_is_line(seg) {
                v[2 * seg + 1] = Anchor::corner(l.p3);
            } else {
                let at = (2 * seg + 2) % v.len();
                v[2 * seg].h_out = l.p1;
                v[at].h_in = r.p2;
                v[2 * seg + 1] = Anchor { p: l.p3, h_in: l.p2, h_out: r.p1, kind: AnchorKind::Smooth };
            }
        }
        sp.anchors = v.into();
    }
    out
}

/// Axis for [`average`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AverageAxis {
    /// Align on a common horizontal line (same y).
    Horizontal,
    /// Align on a common vertical line (same x).
    Vertical,
    /// Collapse to the common centroid.
    Both,
}

/// Object → Path → Average: move the selected anchors `(subpath, anchor)` (with their handles)
/// to their average position along `axis`.
pub fn average(path: &PathData, selection: &[(usize, usize)], axis: AverageAxis) -> PathData {
    let mut out = path.clone();
    let pts: Vec<Point> = selection.iter().filter_map(|&(s, a)| path.subpaths.get(s).and_then(|sp| sp.anchors.get(a)).map(|a| a.p)).collect();
    if pts.is_empty() {
        return out;
    }
    let c = pts.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / pts.len() as f64;
    for &(s, a) in selection {
        if let Some(an) = out.anchor_mut(s, a) {
            let d = match axis {
                AverageAxis::Horizontal => Vec2::new(0.0, c.y - an.p.y),
                AverageAxis::Vertical => Vec2::new(c.x - an.p.x, 0.0),
                AverageAxis::Both => c - an.p.to_vec2(),
            };
            an.translate(d);
        }
    }
    out
}

/// Object → Path → Join. Open subpaths of all inputs are joined nearest-endpoint-first into one
/// open path (endpoints closer than `tolerance` are merged into a single anchor, otherwise a
/// straight segment connects them). A single open subpath is closed instead. Closed subpaths
/// pass through unchanged.
pub fn join(paths: &[PathData], tolerance: f64) -> PathData {
    let mut closed: Vec<SubPath> = Vec::new();
    let mut open: Vec<SubPath> = Vec::new();
    for p in paths {
        for sp in &p.subpaths {
            if sp.closed || sp.anchors.len() < 2 {
                if !sp.anchors.is_empty() {
                    closed.push(sp.clone());
                }
            } else {
                open.push(sp.clone());
            }
        }
    }
    if open.len() == 1
        && let Some(mut sp) = open.pop()
    {
        let ends_meet = sp.anchors.len() > 2 && end_point(&sp, false).zip(end_point(&sp, true)).is_some_and(|(a, b)| a.distance(b) <= tolerance);
        if ends_meet
            && let Some(last) = sp.anchors.pop()
            && let Some(f) = sp.anchors.first_mut()
        {
            *f = Anchor::with_handles(f.p, last.h_in, f.h_out);
        } else {
            if let Some(f) = sp.anchors.first_mut() {
                f.h_in = f.p;
            }
            if let Some(l) = sp.anchors.last_mut() {
                l.h_out = l.p;
            }
        }
        sp.closed = true;
        closed.push(sp);
        return PathData::new(closed);
    }
    while open.len() > 1 {
        // Find the closest pair of endpoints on different subpaths. Past this many subpaths every
        // pair at every merge is too much (cubic): the first one is joined to its nearest.
        const SEARCH_ALL: usize = 200;
        let first_only = open.len() > SEARCH_ALL;
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
        let mut b = open.remove(j);
        let a = &mut open[i];
        if !ei {
            a.reverse();
        }
        if ej {
            b.reverse();
        }
        if d <= tolerance && !b.anchors.is_empty() {
            let first = b.anchors.remove(0);
            if let Some(l) = a.anchors.last_mut() {
                *l = Anchor::with_handles(l.p, l.h_in, first.h_out);
            }
        } else {
            if let Some(l) = a.anchors.last_mut() {
                l.h_out = l.p;
            }
            if let Some(f) = b.anchors.first_mut() {
                f.h_in = f.p;
            }
        }
        a.anchors.extend(b.anchors);
    }
    closed.extend(open);
    PathData::new(closed)
}

/// Object → Path → Split Into Grid: `rows × cols` rectangles filling `rect` with a gutter between
/// the rows and one between the columns (row-major, top-left first). No cells when they would have
/// no size.
pub fn split_into_grid(rect: Rect, rows: usize, cols: usize, (row_gutter, col_gutter): (f64, f64)) -> Vec<PathData> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }
    let w = (rect.width() - col_gutter * (cols as f64 - 1.0)) / cols as f64;
    let h = (rect.height() - row_gutter * (rows as f64 - 1.0)) / rows as f64;
    // Written so that a size that is not a number gives no cells either.
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let x = rect.x0 + c as f64 * (w + col_gutter);
            let y = rect.y0 + r as f64 * (h + row_gutter);
            out.push(PathData::single(SubPath::polyline(
                &[Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
                true,
            )));
        }
    }
    out
}

/// The first (`last == false`) or last anchor point of a subpath.
fn end_point(sp: &SubPath, last: bool) -> Option<Point> {
    if last { sp.anchors.last() } else { sp.anchors.first() }.map(|a| a.p)
}

#[cfg(test)]
mod tests {
    use super::*;

    std::thread_local! {
        /// Anchors moved within a subpath's anchor list by [`remove_anchor`] and [`remove_anchors`].
        static MOVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    pub(super) fn count_moves(n: usize) {
        MOVES.set(MOVES.get() + n);
    }

    /// A subpath of `n` anchors from `seed`: corners, smooth points and points with one handle.
    fn random_subpath(seed: &mut u64, n: usize, closed: bool) -> SubPath {
        let mut rnd = || {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed
        };
        let anchors = (0..n)
            .map(|i| {
                let p = Point::new(i as f64 * 20.0 + (rnd() % 100) as f64 / 10.0, (rnd() % 400) as f64 / 4.0);
                let off = Vec2::new((rnd() % 300) as f64 / 10.0 - 15.0, (rnd() % 300) as f64 / 10.0 - 15.0);
                match rnd() % 4 {
                    0 => Anchor::corner(p),
                    1 => Anchor::smooth(p, p + off),
                    2 => Anchor::with_handles(p, p, p + off),
                    _ => Anchor::with_handles(p, p - off, p),
                }
            })
            .collect();
        SubPath::new(anchors, closed)
    }

    /// `sp` with [`remove_anchor`] run for each of `indexes`, the last first.
    fn one_at_a_time(sp: &SubPath, indexes: &[usize]) -> SubPath {
        let mut order = indexes.to_vec();
        order.sort_unstable_by(|a, b| b.cmp(a));
        order.dedup();
        let mut out = sp.clone();
        for i in order {
            remove_anchor(&mut out, i);
        }
        out
    }

    fn assert_same_bits(got: &SubPath, want: &SubPath, what: &str) {
        let bits = |sp: &SubPath| -> Vec<[u64; 6]> {
            sp.anchors.iter().map(|a| [a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y].map(f64::to_bits)).collect()
        };
        assert_eq!(got.closed, want.closed, "closed: {what}");
        assert_eq!(bits(got), bits(want), "{what}");
        assert_eq!(got.anchors.iter().map(|a| a.kind).collect::<Vec<_>>(), want.anchors.iter().map(|a| a.kind).collect::<Vec<_>>(), "{what}");
    }

    #[test]
    fn removing_anchors_together_gives_what_removing_them_one_at_a_time_gives() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for case in 0..600 {
            let n = case % 40 + 1;
            let closed = case % 2 == 0;
            let sp = random_subpath(&mut seed, n, closed);
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Up to n + 2 picks over 0..n + 2: runs of neighbors, both ends, everything, repeats
            // and indexes past the end.
            let picks = (seed % (n as u64 + 3)) as usize;
            let mut indexes: Vec<usize> = (0..picks).map(|k| ((seed >> (k % 48)) as usize).wrapping_add(k * 7) % (n + 2)).collect();
            if case % 5 == 0 {
                indexes = (0..n).collect();
            }
            let mut got = sp.clone();
            let removed = remove_anchors(&mut got, &indexes);
            let want = one_at_a_time(&sp, &indexes);
            let what = format!("case {case}: {n} anchors, closed {closed}, removing {indexes:?}");
            assert_same_bits(&got, &want, &what);
            assert_eq!(removed, sp.anchors.len() - want.anchors.len(), "{what}");
        }
    }

    #[test]
    fn removing_anchors_together_keeps_each_step_of_one_at_a_time() {
        let curve = |p: Point, d: f64| Anchor::smooth(p, p + Vec2::new(d, 25.0));
        let pts = [Point::new(0.0, 0.0), Point::new(50.0, 40.0), Point::new(100.0, 0.0), Point::new(150.0, 40.0), Point::new(200.0, 0.0)];
        let cases: Vec<(SubPath, Vec<usize>)> = vec![
            // A refit at one removal makes the next segment curved, which the next removal refits.
            (
                SubPath::new(
                    vec![Anchor::corner(pts[0]), Anchor::corner(pts[1]), curve(pts[2], 20.0), Anchor::corner(pts[3]), Anchor::corner(pts[4])],
                    false,
                ),
                vec![1, 2, 3],
            ),
            // The last anchor of an open path goes, then the one before it becomes the end.
            (SubPath::new(pts.iter().map(|&p| curve(p, 15.0)).collect(), false), vec![2, 3, 4]),
            // The first anchor of a closed path refits across the wrap.
            (SubPath::new(pts.iter().map(|&p| curve(p, -10.0)).collect(), true), vec![0, 4]),
            // A closed path down to two anchors stays closed with a handle, and opens without one.
            (SubPath::new(pts.iter().map(|&p| curve(p, 10.0)).collect(), true), vec![1, 2, 3]),
            (SubPath::polyline(&pts, true), vec![1, 2, 3]),
            (SubPath::polyline(&pts, true), vec![0, 1, 2, 3]),
            (SubPath::polyline(&pts[..1], false), vec![0]),
            (SubPath::new(vec![], true), vec![0]),
        ];
        for (sp, indexes) in cases {
            let mut got = sp.clone();
            remove_anchors(&mut got, &indexes);
            assert_same_bits(&got, &one_at_a_time(&sp, &indexes), &format!("{sp:?} removing {indexes:?}"));
        }
    }

    #[test]
    fn removing_anchors_together_moves_each_anchor_left_at_most_once() {
        let n = 2001;
        let pts: Vec<Point> = (0..n).map(|i| Point::new(i as f64, (i % 2) as f64)).collect();
        let mut sp = SubPath::polyline(&pts, false);
        let every_other: Vec<usize> = (1..n).step_by(2).collect();
        MOVES.set(0);
        assert_eq!(remove_anchors(&mut sp, &every_other), n / 2);
        assert_eq!(sp.anchors.len(), n - n / 2);
        let moves = MOVES.get();
        assert!(moves < n, "{moves} anchors moved to remove {} of {n}", n / 2);
    }

    /// The samples Object › Path › Smooth at `level` smooths for `sp`, a subpath with no corner,
    /// and the spread it smooths them with.
    fn smooth_samples(sp: &SubPath, level: f64) -> (Vec<Point>, f64) {
        let opts = smooth_options(&PathData::single(sp.clone()), level);
        let mut pts = Vec::new();
        for (k, s) in subpath_segs(sp).iter().enumerate() {
            sample(&s.c, sample_steps(&s.c, opts.tolerance), &mut pts, k == 0);
        }
        (pts, opts.smooth)
    }

    /// A traced outline: `n` straight segments around a circle of radius 200, each anchor moved in
    /// or out by up to 0.5 pt.
    fn traced_circle(n: usize) -> SubPath {
        let pts: Vec<Point> = (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                let r = 200.0 + 0.5 * ((i * 7919) % 13) as f64 / 13.0;
                Point::new(300.0 + r * a.cos(), 300.0 + r * a.sin())
            })
            .collect();
        SubPath::polyline(&pts, true)
    }

    /// [`smooth_points`] weighing every sample on its own.
    fn smooth_points_one_by_one(pts: &[Point], sigma: f64) -> Vec<Point> {
        let mut s = vec![0.0];
        for w in pts.windows(2) {
            s.push(s.last().copied().unwrap_or(0.0) + w[0].distance(w[1]));
        }
        let total = s.last().copied().unwrap_or(0.0);
        pts.iter()
            .zip(&s)
            .map(|(&p, &si)| {
                let reach = (3.0 * sigma).min(si).min(total - si);
                if reach <= 1e-9 {
                    return p;
                }
                let spread = reach / 3.0;
                let (mut sum, mut weight) = (Vec2::ZERO, 0.0);
                for (q, &sj) in pts.iter().zip(&s).filter(|(_, sj)| (*sj - si).abs() <= reach) {
                    let w = (-(sj - si) * (sj - si) / (2.0 * spread * spread)).exp();
                    sum += (*q - p) * w;
                    weight += w;
                }
                p + sum / weight
            })
            .collect()
    }

    /// Next value in 0..1 of a small linear congruential generator.
    fn next(seed: &mut u64) -> f64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn smoothing_work_per_sample_stays_bounded_on_dense_paths() {
        // Weighing every sample within three spreads took 360 steps per sample for 1,000 anchors and
        // 2,949 for 16,000: the work grew with the square of the anchor count.
        for n in [1_000usize, 16_000] {
            let (pts, sigma) = smooth_samples(&traced_circle(n), 100.0);
            let mut work = 0;
            smooth_points_counted(&pts, sigma, &mut work);
            assert!(work <= 400 * pts.len(), "{n} anchors: {work} steps for {} samples", pts.len());
        }
        // A curve sampled at a tiny tolerance and smoothed far beyond its length.
        let c = CubicBez::new((0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
        let mut pts = Vec::new();
        sample(&c, sample_steps(&c, 0.00001), &mut pts, true);
        let mut work = 0;
        smooth_points_counted(&pts, 1000.0, &mut work);
        assert!(work <= 400 * pts.len(), "{work} steps for {} samples", pts.len());
    }

    /// A spike on a closed loop whose samples are ten times denser on one half than on the other is
    /// smoothed out wherever it is, including at the sample halfway through the list, which is only
    /// about a quarter of the way round by length.
    #[test]
    fn loop_smoothing_smooths_a_spike_on_unevenly_sampled_loops() {
        let c = Point::new(300.0, 300.0);
        let on = |a: f64, r: f64| c + r * Vec2::from_angle(a);
        let mut pts: Vec<Point> = (0..300).map(|i| on(f64::from(i) / 300.0 * std::f64::consts::PI, 100.0)).collect();
        pts.extend((0..30).map(|i| on(std::f64::consts::PI * (1.0 + f64::from(i) / 30.0), 100.0)));
        let m = pts.len();
        pts.push(pts[0]);
        for spike in [m / 2, 0, 100] {
            let mut p = pts.clone();
            p[spike] = on(std::f64::consts::PI * spike as f64 / 300.0, 105.0);
            if spike == 0 {
                p[m] = p[0];
            }
            let out = smooth_loop_points(&p, 10.0);
            assert_eq!(out.len(), p.len());
            assert_eq!(out[0], out[m], "spike at {spike}: the loop stays closed");
            let left = out[spike].distance(c) - 100.0;
            assert!(left < 1.0, "spike at {spike}: {left} pt of 5 pt left");
        }
    }

    #[test]
    fn the_samples_of_a_flat_loop_and_its_line_segments_stay_within_the_result_limit() {
        for (curved, kept) in [(1, 0), (32, 1), (40, 1_000_000), (1_000, MAX_RUN_RESULT - 1_000), (3, MAX_RUN_RESULT - 7)] {
            let room = sample_room(curved, kept).unwrap_or_else(|| panic!("{curved} curved, {kept} kept: no room"));
            assert!((1..=MAX_SEGMENT_SAMPLES).contains(&room), "{curved} curved, {kept} kept: room {room}");
            assert!(curved * room + kept <= MAX_RUN_RESULT, "{curved} curved, {kept} kept: room {room}");
        }
        assert_eq!(sample_room(32, 0), Some(MAX_SEGMENT_SAMPLES));
        assert_eq!(sample_room(2, MAX_RUN_RESULT - 1), None);
        assert_eq!(sample_room(1, MAX_RUN_RESULT + 5), None);
    }

    #[test]
    fn lines_with_too_little_room_for_the_tolerance_are_not_used() {
        let c = CubicBez::new((0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
        let tol = 0.01;
        let need = line_steps(&c, tol);
        assert!(need > 4, "{need}");
        assert_eq!(steps_within(&c, tol, need - 1), None);
        for room in [need, need + 3, MAX_SEGMENT_SAMPLES] {
            let steps = steps_within(&c, tol, room).unwrap_or_else(|| panic!("room {room}: no steps"));
            assert!((need..=room).contains(&steps), "room {room}: {steps} steps");
            let mut pts = Vec::new();
            sample(&c, steps, &mut pts, true);
            let lines: Vec<Line> = pts.windows(2).map(|p| Line::new(p[0], p[1])).collect();
            for i in 0..=1000 {
                let p = c.eval(f64::from(i) / 1000.0);
                let gap = lines.iter().map(|l| l.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
                assert!(gap <= tol + 1e-9, "room {room}, {steps} steps: {gap} at {i}");
            }
        }
    }

    #[test]
    fn smoothing_points_whose_arc_length_overflows_returns_every_point() {
        let pts: Vec<Point> = (0..40).map(|i| Point::new(if i % 2 == 0 { -1e308 } else { 1e308 }, f64::from(i))).collect();
        assert_eq!(smooth_points(&pts, 10.0).len(), pts.len());
    }

    #[test]
    fn grouped_smoothing_stays_within_a_thousandth_of_a_spread_of_weighing_one_by_one() {
        let mut seed = 7u64;
        for case in 0..300 {
            let n = 3 + (next(&mut seed) * 2000.0) as usize;
            let mut p = Point::ZERO;
            let mut dir: f64 = 0.0;
            let dense = next(&mut seed) < 0.5;
            let pts: Vec<Point> = (0..n)
                .map(|_| {
                    dir += (next(&mut seed) - 0.5) * 1.5;
                    let step = if dense { next(&mut seed) * 0.05 } else { next(&mut seed) * 5.0 };
                    p += Vec2::new(dir.cos(), dir.sin()) * step;
                    p
                })
                .collect();
            let sigma = 0.01 + next(&mut seed) * 50.0;
            let exact = smooth_points_one_by_one(&pts, sigma);
            let grouped = smooth_points(&pts, sigma);
            assert_eq!(grouped.len(), exact.len());
            for (i, (a, b)) in exact.iter().zip(&grouped).enumerate() {
                assert!(a.distance(*b) <= 1e-3 * sigma, "case {case}, point {i}: {a:?} and {b:?}, spread {sigma}");
            }
            assert_eq!((grouped.first(), grouped.last()), (pts.first(), pts.last()), "case {case}");
        }
    }
}
