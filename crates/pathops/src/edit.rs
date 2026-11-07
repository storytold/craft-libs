// Carved from VectorCraft's crates/pathops (src/edit.rs: simplify, remove redundant points, join) at
// storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Path editing: Simplify, Remove redundant points and Join.

use kurbo::{BezPath, CubicBez, PathSeg, Point, Vec2};

use crate::budget::fits_elements;
use crate::fit::{end_tangent, fit_cubics, fit_single, is_straight, measured_arclen, sample, start_tangent};
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
    simplify_within(path, opts, &mut Fit { work: limits.fit_work, max_points: limits.fit_points })
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

/// What a simplify may still spend, and the most points it samples from one run of segments.
struct Fit {
    work: usize,
    max_points: usize,
}

impl Fit {
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
    let result = simplify_run(&segs, opts, fit);
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

/// The refit of the consecutive segments `run`, split at its corners sharper than the threshold:
/// it starts and ends where `run` does, leaving the same way.
fn simplify_run(run: &[Seg], opts: &SimplifyOptions, fit: &mut Fit) -> Vec<Seg> {
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
        let mut pts = Vec::new();
        let mut too_many = false;
        for (k, s) in piece.iter().enumerate() {
            // Guarded: kurbo subdivides without end on huge finite coordinates. A segment that cannot
            // be measured cannot be sampled finely enough for the tolerance either: the run stays.
            let Some(len) = measured_arclen(&PathSeg::Cubic(s.c), 1e-6) else {
                too_many = true;
                break;
            };
            let m = ((len / (tol * 0.5)).ceil() as usize).clamp(4, 256);
            if pts.len().saturating_add(m) > fit.max_points || !fit.charge(m) {
                too_many = true;
                break;
            }
            work::add(Work::FitSamples, m);
            sample(&s.c, m, &mut pts, k == 0);
        }
        // Too long to fit, or past the work: the run stays as it is.
        let done = !too_many && fit_piece(piece, pts, tol, opts, fit, &mut result);
        if !done {
            result.extend(piece.iter().copied());
        }
    }
    result
}

/// The refit of one `piece` from its sampled points `pts`, appended to `result`; false (and
/// nothing appended) when `fit` runs out of work first.
fn fit_piece(piece: &[Seg], pts: Vec<Point>, tol: f64, opts: &SimplifyOptions, fit: &mut Fit, result: &mut Vec<Seg>) -> bool {
    let pts = if opts.smooth > 0.0 {
        let Some(smoothed) = smooth_points(&pts, opts.smooth, fit) else { return false };
        smoothed
    } else {
        pts
    };
    if opts.straight_lines {
        let Some(keep) = rdp(&pts, tol, fit) else { return false };
        result.extend(keep.windows(2).map(|p| Seg::line(p[0], p[1])));
        return true;
    }
    let (Some(first), Some(last)) = (piece.first(), piece.last()) else { return true };
    let (t0, t1) = (start_tangent(&first.c), end_tangent(&last.c));
    // Out of work, or no fit within the tolerance: the piece stays as it is (the work is spent only
    // in the first case).
    let Ok(cubics) = fit_cubics(&pts, t0, t1, tol, &mut fit.work) else { return false };
    for c in cubics {
        let line = is_straight(&c, tol * 0.02);
        result.push(if line { Seg::line(c.p0, c.p3) } else { Seg { c, line } });
    }
    true
}

/// `pts` (a run's samples, in order) smoothed along the path: each point becomes the Gaussian-weighted
/// mean of the points around it, `sigma` points of arc length wide, the window narrowing towards the
/// ends so that the first and last points stay where they are. Most points looked at per point is
/// bounded by the samples within three spreads of it, and charged to `fit`: `None` when that runs out.
fn smooth_points(pts: &[Point], sigma: f64, fit: &mut Fit) -> Option<Vec<Point>> {
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
        if !fit.charge(hi + 1 - lo.min(hi + 1)) {
            return None;
        }
        let (mut sum, mut weight) = (Vec2::ZERO, 0.0);
        for (q, &sj) in pts.iter().zip(&s).take(hi + 1).skip(lo) {
            let w = (-(sj - si) * (sj - si) / (2.0 * spread * spread)).exp();
            sum += q.to_vec2() * w;
            weight += w;
        }
        smoothed.push(if weight > 0.0 { (sum / weight).to_point() } else { p });
    }
    Some(smoothed)
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
/// one pass along each subpath leaves (at a closed subpath's start) is limited to [`REDUNDANT_WORK`]
/// joints read in all.
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
        let origins: Vec<Point> = segs.iter().map(|s| s.c.p0).collect();
        let mut pieces: Vec<Piece> = segs.iter().enumerate().map(|(first, &seg)| Piece { seg, dev: 0.0, first, count: 1 }).collect();
        // Joints are merged along the path in one pass (a merged segment is tried with the one
        // before it, then the next); the search below then only finds what a closed path's wrap
        // leaves. (Merging at the first joint of a dense line one at a time shifts the rest each time.)
        let total = pieces.len();
        let mut merged = 0;
        let mut done: Vec<Piece> = Vec::with_capacity(total);
        for p in pieces.drain(..) {
            done.push(p);
            while done.len() >= 2 && total - merged > min_segs {
                let n = done.len();
                let Some(m) = merge_pieces(&done[n - 2], &done[n - 1], tol, &origins) else { break };
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
                if let Some(m) = merge_pieces(&pieces[i0], &pieces[i1], tol, &origins) {
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

/// Most joints [`remove_redundant_points`] reads in all in its search for what its first pass
/// along a subpath left: 67,108,864 (the default of [`crate::Limits::redundant_work`]).
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

/// `a` and `b`, next to each other, as one piece, if the one segment [`merge_pair`] makes stays within
/// `tol` of every segment of the run both stand for: not only of the joint between them, which
/// is all `merge_pair` sees (a run built a joint at a time can drift from a curve it was sampled
/// from, each step within `tol` of the last: a half circle of ten thousand chords came out as its
/// diameter). A bound is kept: the segments of `a` and `b` are within `a.dev` and `b.dev` of them, and
/// them from the merged one by that and the step of the merge at most. For lines, when that
/// bound passes `tol`, the joints of the run are checked against the line (up to
/// [`MAX_CHECKED_RUN`]), which gives the real deviation. `origins` are the starts of the original
/// segments.
fn merge_pieces(a: &Piece, b: &Piece, tol: f64, origins: &[Point]) -> Option<Piece> {
    let (seg, step) = merge_pair(&a.seg, &b.seg, tol)?;
    let (first, count) = (a.first, a.count + b.count);
    let bound = a.dev.max(b.dev) + step;
    if bound <= tol {
        return Some(Piece { seg, dev: bound, first, count });
    }
    if !seg.line || count > MAX_CHECKED_RUN {
        return None;
    }
    // The joints inside the run are the starts of its segments but the first.
    work::add(Work::RunChecks, count);
    let chord = seg.c.p3 - seg.c.p0;
    let len2 = chord.hypot2();
    let mut worst: f64 = 0.0;
    for k in 1..count {
        let p = *origins.get((first + k) % origins.len().max(1))?;
        let along = if len2 > 0.0 { ((p - seg.c.p0).dot(chord) / len2).clamp(0.0, 1.0) } else { 0.0 };
        worst = worst.max(p.distance(seg.c.p0 + chord * along));
    }
    (worst <= tol).then_some(Piece { seg, dev: worst, first, count })
}

/// The one segment replacing `a` and `b` (next to each other), if it passes their joint within
/// `tol`, and how far from it the joint is.
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
    if end_tangent(&a.c).dot(start_tangent(&b.c)) < 0.99 {
        return None;
    }
    let mut pts = Vec::new();
    sample(&a.c, 16, &mut pts, true);
    sample(&b.c, 16, &mut pts, false);
    let (c, err, _) = fit_single(&pts, start_tangent(&a.c), end_tangent(&b.c));
    (err <= tol).then_some((Seg { c, line: false }, err))
}

/// Join. Open subpaths of all inputs are joined nearest-endpoint-first into one open path
/// (endpoints closer than `tolerance` are merged into a single anchor, otherwise a straight segment
/// connects them). A single open subpath is closed instead. Closed subpaths pass through, in the form
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
        let ends_meet = segs.len() >= 2 && ends(&segs).is_some_and(|(a, b)| a.distance(b) <= tolerance);
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

    /// The largest distance from a sample of `a` to the outline of `b`.
    fn deviation(a: &BezPath, b: &BezPath) -> f64 {
        use kurbo::{ParamCurve, ParamCurveNearest};
        let mut worst: f64 = 0.0;
        for seg in a.segments() {
            for k in 0..=64 {
                let p = seg.eval(f64::from(k) / 64.0);
                let d = b.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
                worst = worst.max(d);
            }
        }
        worst
    }

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
        assert!(deviation(&quarter, &simpler) <= 1.1, "{} from the curve", deviation(&quarter, &simpler));
        // One that can be measured is simplified, as ever.
        let r = 5e5;
        let k = 0.552_284_749_830_793_6 * r;
        let mut small = BezPath::new();
        small.move_to((r, 0.0));
        small.curve_to((r, k), (k, r), (0.0, r));
        assert!(deviation(&small, &simplify_with(&small, &opts)) <= 1.1);
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
    fn a_run_too_long_to_sample_stays_as_it_is() {
        let circle = circle_polygon(400, 50.0);
        work::reset();
        let mut fit = Fit { work: usize::MAX, max_points: 10_000 };
        let same = simplify_within(&circle, &SimplifyOptions { tolerance: 0.01, ..Default::default() }, &mut fit);
        assert_eq!(anchor_count(&same), 400);
        let sampled = work::take(Work::FitSamples);
        assert!(sampled > 0 && sampled <= 10_000, "{sampled} samples");
        // With room, the same circle is a handful of curves.
        let mut fit = Fit { work: usize::MAX, max_points: MAX_FIT_POINTS };
        let fitted = simplify_within(&circle, &SimplifyOptions { tolerance: 0.01, ..Default::default() }, &mut fit);
        assert!(anchor_count(&fitted) <= 12, "{}", anchor_count(&fitted));
        assert!(work::take(Work::FitSamples) > 10_000);
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
        let full = simplify_within(&path, &opts, &mut Fit { work: usize::MAX, max_points: MAX_FIT_POINTS });
        let total = work::take(Work::Fit);
        assert!(total > 10_000, "{total}");
        let half = total / 2;
        let partly = simplify_within(&path, &opts, &mut Fit { work: half, max_points: MAX_FIT_POINTS });
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
            let mut fit = Fit { work, max_points: MAX_FIT_POINTS };
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
        let all = simplify_within(&circle, &opts, &mut Fit { work: usize::MAX / 2, max_points: MAX_FIT_POINTS });
        let used = work::take(Work::Fit);
        assert!(anchor_count(&all) < 300 && used > 0);
        let starved = simplify_within(&circle, &opts, &mut Fit { work: used / 2, max_points: MAX_FIT_POINTS });
        assert_eq!(anchor_count(&starved), 300);
        assert!(work::take(Work::Fit) <= used / 2);
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
}
