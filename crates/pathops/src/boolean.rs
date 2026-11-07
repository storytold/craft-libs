// Carved from VectorCraft's crates/pathops (src/boolean.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Curve-preserving boolean operations built on `linesweeper`'s robust sweep-line topology.
//!
//! Inputs are converted to closed [`BezPath`]s (open subpaths are implicitly closed, as a filled
//! open path is painted). The sweep splits curves at intersections and y-extrema;
//! [`tidy_segments`] then re-joins pieces that came from the same smooth curve by refitting, so
//! results have roughly as few anchors as the inputs.

use std::collections::BTreeSet;

use kurbo::{BezPath, CubicBez, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours, Topology, WindingNumber};

use crate::fit::{end_tangent, fit_single, sample, start_tangent};
use crate::path::{Seg, fill_bezpath, guarded, segs_to_bezpath};
use crate::planar::index::RectIndex;
use crate::work::{self, Work};
use crate::{FillRule, PathOpsError};

#[cfg(test)]
thread_local! {
    /// The contour passes made over an arrangement on this thread, each of which walks all of it.
    pub(crate) static CONTOUR_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Default curve-refit tolerance (in path units) used to merge split curve pieces back together, and
/// the width under which a sliver of the result is dropped.
pub const DEFAULT_PRECISION: f64 = 0.01;

/// Binary boolean operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BoolOp {
    /// A ∪ B
    Union,
    /// A ∩ B
    Intersect,
    /// A − B
    Difference,
    /// A ⊕ B (symmetric difference)
    Xor,
}

impl BoolOp {
    /// Evaluate the operation on "inside A" / "inside B" flags.
    pub fn apply(self, a: bool, b: bool) -> bool {
        match self {
            BoolOp::Union => a || b,
            BoolOp::Intersect => a && b,
            BoolOp::Difference => a && !b,
            BoolOp::Xor => a != b,
        }
    }
}

pub(crate) fn inside(rule: FillRule, w: i32) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// Winding numbers for an arbitrary number of input sets (tag = input index): the non-zero ones,
/// as (tag, winding) sorted by tag. A face is covered by a few of the inputs, so this stays short
/// where a vector of every input's winding would make each Pathfinder test cost one step per input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Multi(Vec<(usize, i32)>);

impl Multi {
    fn get(&self, i: usize) -> i32 {
        self.0.binary_search_by_key(&i, |e| e.0).ok().and_then(|k| self.0.get(k)).map_or(0, |e| e.1)
    }
    /// The non-zero windings, by input.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = (usize, i32)> + '_ {
        self.0.iter().copied()
    }
}

impl std::ops::Add for Multi {
    type Output = Multi;
    fn add(mut self, rhs: Self) -> Multi {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Multi {
    fn add_assign(&mut self, rhs: Self) {
        // In place, input by input (the sweep mostly adds one input's ±1 to a short list): add to
        // the input's winding, or insert it in tag order, and drop it if it reaches zero.
        for (t, k) in rhs.0 {
            match self.0.binary_search_by_key(&t, |e| e.0) {
                Ok(i) => {
                    if let Some(e) = self.0.get_mut(i) {
                        e.1 += k;
                        if e.1 == 0 {
                            self.0.remove(i);
                        }
                    }
                }
                Err(i) if k != 0 => self.0.insert(i, (t, k)),
                Err(_) => {}
            }
        }
    }
}

impl WindingNumber for Multi {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        Multi(vec![(tag, if positive { 1 } else { -1 })])
    }
    fn of_tag(&self, tag: usize) -> Self {
        let w = self.get(tag);
        Multi(if w == 0 { vec![] } else { vec![(tag, w)] })
    }
}

/// Sweep tolerance for a set of paths (same scaling rule as linesweeper's own `binary_op`).
pub(crate) fn eps_for<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<f64, PathOpsError> {
    let mut m: f64 = 0.0;
    for p in paths {
        work::add(Work::Visited, p.elements().len());
        for el in p.elements() {
            for pt in el_points(el) {
                if !pt.x.is_finite() || !pt.y.is_finite() {
                    return Err(PathOpsError::NonFinite);
                }
                m = m.max(pt.x.abs()).max(pt.y.abs());
            }
        }
    }
    let eps = (m * f64::EPSILON * 64.0).max(1e-6);
    // Past this the sweep tolerance is wider than a unit: features thinner than it collapse and the call
    // would answer `Ok` with part of the art (or all of it) gone. Refuse instead, so callers keep the original.
    if eps > MAX_SWEEP_EPS {
        return Err(PathOpsError::Degenerate);
    }
    Ok(eps)
}

/// The widest sweep tolerance [`eps_for`] accepts: one unit, reached by coordinates of about 7e13. (Applications refuse
/// coordinates far below that, so this only guards the library's own callers; it does not follow the `precision` a caller
/// asks for.)
const MAX_SWEEP_EPS: f64 = 1.0;

fn el_points(el: &kurbo::PathEl) -> impl Iterator<Item = Point> {
    use kurbo::PathEl::*;
    let v: [Option<Point>; 3] = match *el {
        MoveTo(p) | LineTo(p) => [Some(p), None, None],
        QuadTo(a, b) => [Some(a), Some(b), None],
        CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
        ClosePath => [None, None, None],
    };
    v.into_iter().flatten()
}

/// Make edges that rise by less than the sweep tolerance `eps` exactly horizontal.
///
/// The sweep orders a curve against another segment over a y-span shorter than `eps` by bounding
/// boxes alone, so an almost-horizontal edge (typically a boolean result's edge whose end, an
/// intersection point, is a rounding error off the input's horizontal) can be mis-ordered against
/// a curve it passes and the output loses a corner; exact horizontals are handled exactly. So
/// on-curve y coordinates within `eps` of each other snap to one value (the lowest of each
/// cluster, keeping distinct values at least `eps` apart), handles move with their anchors, and a
/// handle within `eps` of its anchor's height is levelled with it (no y-extremum is left in a
/// sub-`eps` sliver of a curve). Anchors move by less than `eps` and handles by less than `2·eps`,
/// the order of the sweep's own tolerance.
pub(crate) fn snap_horizontals(paths: &mut [&mut BezPath], eps: f64) {
    use kurbo::PathEl::*;
    let mut ys: Vec<f64> = paths.iter().flat_map(|p| p.elements().iter().filter_map(kurbo::PathEl::end_point)).map(|p| p.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut start = f64::NEG_INFINITY;
    let levels: Vec<f64> = ys
        .iter()
        .map(|&y| {
            if y - start >= eps {
                start = y;
            }
            start
        })
        .collect();
    let snap = |p: &mut Point| {
        let y = levels[ys.partition_point(|&v| v < p.y)];
        let moved = y - p.y;
        p.y = y;
        moved
    };
    // Shift a handle with its anchor, then level it with the anchor when within `eps`.
    let follow = |h: &mut Point, anchor: Point, moved: f64| {
        h.y += moved;
        if (h.y - anchor.y).abs() < eps {
            h.y = anchor.y;
        }
    };
    for p in paths.iter_mut() {
        // The current point (snapped) and how far snapping moved it; the subpath's start.
        let (mut cur, mut moved, mut first) = (Point::ZERO, 0.0, Point::ZERO);
        for el in p.elements_mut() {
            match el {
                MoveTo(q) => {
                    moved = snap(q);
                    (cur, first) = (*q, *q);
                }
                LineTo(q) => {
                    moved = snap(q);
                    cur = *q;
                }
                QuadTo(c, q) => {
                    let m = snap(q);
                    follow(c, cur, moved);
                    follow(c, *q, 0.0);
                    (cur, moved) = (*q, m);
                }
                CurveTo(c1, c2, q) => {
                    let m = snap(q);
                    follow(c1, cur, moved);
                    follow(c2, *q, m);
                    (cur, moved) = (*q, m);
                }
                ClosePath => {
                    // A later segment without a MoveTo starts from the subpath's (snapped) start.
                    (cur, moved) = (first, 0.0);
                }
            }
        }
    }
}

/// An arrangement (planar subdivision) of N filled paths.
pub(crate) struct Arrangement {
    pub(crate) top: Topology<Multi>,
    pub(crate) rules: Vec<FillRule>,
    /// Output cleanup that keeps the inputs' anchors.
    pub(crate) tidy: Tidy,
}

/// Output cleanup settings: refit `precision`, and anchor positions that must survive (the
/// inputs' own anchors — only joints the sweep introduced are merged away).
pub(crate) struct Tidy {
    pub(crate) precision: f64,
    /// The inputs' own on-curve points, each by the tiny box it is in, to find those near a point.
    keep: Option<RectIndex<Point>>,
    /// Input curves (with inflated bounds) used to rebuild split pieces exactly.
    curves: Vec<(kurbo::Rect, CubicBez)>,
    /// The indices in `curves` by their bounds, to find the curves whose bounds hold a point
    /// without reading them all (a column of shapes of one width would otherwise be read whole at
    /// every point).
    curve_index: Option<RectIndex<usize>>,
}

impl Tidy {
    /// Merge any smooth joint.
    pub(crate) fn free(precision: f64) -> Self {
        Self { precision, keep: None, curves: Vec::new(), curve_index: None }
    }
    /// Keep every on-curve point of `paths`.
    pub(crate) fn keeping<'a>(precision: f64, paths: impl IntoIterator<Item = &'a BezPath>) -> Self {
        let mut keep = Vec::new();
        let mut curves = Vec::new();
        for p in paths {
            work::add(Work::Visited, p.elements().len());
            for s in p.segments() {
                let seg = Seg::straight(s);
                // Points that are not numbers are no anchor to keep and no curve to find (an input
                // that cannot be swept is left out of a union, but its segments come here all the same).
                if !seg.c.p0.is_finite() {
                    continue;
                }
                keep.push((kurbo::Rect::from_points(seg.c.p0, seg.c.p0), seg.c.p0));
                if !seg.line && [seg.c.p1, seg.c.p2, seg.c.p3].iter().all(|p| p.is_finite()) {
                    let r = seg.c.bounding_box().inflate(1e-6, 1e-6);
                    curves.push((r, seg.c));
                }
            }
        }
        let keep = (!keep.is_empty()).then(|| RectIndex::new(keep));
        let curve_index = (!curves.is_empty()).then(|| RectIndex::new(curves.iter().enumerate().map(|(i, (r, _))| (*r, i)).collect()));
        Self { precision, keep, curves, curve_index }
    }
    /// Is `p` one of the inputs' own points (within a millionth of its size)?
    fn is_kept(&self, p: Point) -> bool {
        let tol = 1e-6 * (1.0 + p.x.abs().max(p.y.abs()));
        // The index reads a box a hair wider, so that its half-open boxes lose none the exact test keeps.
        self.keep.as_ref().is_some_and(|k| k.at(p, tol * (1.0 + 1e-9)).any(|q| (q.x - p.x).abs() <= tol && (q.y - p.y).abs() <= tol))
    }
}

impl Arrangement {
    pub(crate) fn new(paths: Vec<(BezPath, FillRule)>) -> Result<Self, PathOpsError> {
        Self::new_budgeted(paths, crate::budget::limits().segment_pairs)
    }

    pub(crate) fn new_budgeted(mut paths: Vec<(BezPath, FillRule)>, cap: usize) -> Result<Self, PathOpsError> {
        if !crate::budget::fits_elements(paths.iter().map(|p| &p.0)) {
            return Err(PathOpsError::TooComplex);
        }
        let eps = eps_for(paths.iter().map(|p| &p.0))?;
        snap_horizontals(&mut paths.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
        // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
        if !crate::budget::sweep_fits_within(paths.iter().map(|p| &p.0), cap) {
            return Err(PathOpsError::TooComplex);
        }
        // The sweep can panic on rare near-degenerate input; that must never take the app down,
        // so it becomes an error (callers fall back to leaving the art unchanged).
        work::add(Work::Sweeps, 1);
        let sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Topology::<Multi>::from_paths(paths.iter().enumerate().map(|(i, (p, _))| (p, i)), eps)
        }));
        let top = sweep.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)?;
        let tidy = Tidy::keeping(DEFAULT_PRECISION, paths.iter().map(|p| &p.0));
        Ok(Self { top, rules: paths.iter().map(|p| p.1).collect(), tidy })
    }

    pub(crate) fn mask(&self, w: &Multi) -> Vec<bool> {
        // A zero winding is outside under both rules, so only the non-zero ones can set a flag.
        let mut m = vec![false; self.rules.len()];
        for (i, k) in w.iter() {
            if let (Some(&r), Some(b)) = (self.rules.get(i), m.get_mut(i)) {
                *b = inside(r, k);
            }
        }
        m
    }

    /// The inputs that cover a face of winding `w` (`mask(w)`'s true indices), back to front.
    pub(crate) fn covered<'a>(&'a self, w: &'a Multi) -> impl DoubleEndedIterator<Item = usize> + 'a {
        w.iter().filter(|&(i, k)| self.rules.get(i).is_some_and(|&r| inside(r, k))).map(|(i, _)| i)
    }

    /// [`Arrangement::contours`] with the test on the windings themselves (see [`Self::covered`]).
    pub(crate) fn contours_where(&self, pred: impl Fn(&Multi) -> bool) -> Contours {
        #[cfg(test)]
        CONTOUR_PASSES.with(|n| n.set(n.get() + 1));
        self.top.contours(pred)
    }

    pub(crate) fn contours(&self, pred: impl Fn(&[bool]) -> bool) -> Contours {
        self.contours_where(|w| pred(&self.mask(w)))
    }

    /// [`Self::contours`] with a panic in the sweep's contour pass turned into an error, as `new` does for the
    /// sweep itself (native builds only: on the web a panic aborts, whatever is caught here). Used by the calls that can
    /// return an error; the others catch a panic as a whole ([`guarded`]).
    pub(crate) fn try_contours(&self, pred: impl Fn(&[bool]) -> bool) -> Result<Contours, PathOpsError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.contours(pred))).map_err(|_| PathOpsError::Degenerate)
    }

    /// All distinct non-empty coverage masks that occur next to some edge, sorted.
    pub(crate) fn distinct_masks(&self) -> Vec<Vec<bool>> {
        let mut set = BTreeSet::new();
        for s in self.top.segment_indices() {
            let h = s.first_half();
            for w in [self.top.winding_clockwise(h), self.top.winding_counter_clockwise(h)] {
                let m = self.mask(w);
                if m.iter().any(|&b| b) {
                    set.insert(m);
                }
            }
        }
        set.into_iter().collect()
    }

    /// Vertices where three or more edges meet (true intersections / touch points).
    pub(crate) fn junctions(&self) -> Vec<Point> {
        let mut count: std::collections::HashMap<(u64, u64), (usize, Point)> = std::collections::HashMap::new();
        for s in self.top.segment_indices() {
            for h in [s.first_half(), s.second_half()] {
                let p = self.top.point(h).to_kurbo();
                count.entry((p.x.to_bits(), p.y.to_bits())).or_insert((0, p)).0 += 1;
            }
        }
        let mut v: Vec<Point> = count.into_values().filter(|(c, _)| *c >= 3).map(|(_, p)| p).collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        v
    }
}

/// Convert selected contours into a tidy compound path.
pub(crate) fn contours_to_path(c: &Contours, idx: impl IntoIterator<Item = ContourIdx>, tidy: &Tidy) -> BezPath {
    let mut out = BezPath::new();
    for i in idx {
        let path = tidy.repair(&c[i].path);
        if is_sliver(&path, tidy.precision) {
            continue;
        }
        if let Some(sp) = tidy_bezpath(&path, tidy) {
            out.extend(sp.iter());
        }
    }
    out
}

pub(crate) fn all_contours_to_path(c: &Contours, tidy: &Tidy) -> BezPath {
    contours_to_path(c, (0..c.contours().count()).map(ContourIdx), tidy)
}

/// Contours whose mean width (2·area / perimeter) is below `precision` are numerical debris
/// (e.g. the lens left between two coincident curves) and are dropped.
pub(crate) fn is_sliver(bp: &BezPath, precision: f64) -> bool {
    let a = bp.area().abs();
    let per = bp.perimeter(1e-6);
    per <= 0.0 || 2.0 * a / per < precision
}

fn tidy_bezpath(bp: &BezPath, tidy: &Tidy) -> Option<BezPath> {
    let segs: Vec<Seg> = bp.segments().map(Seg::straight).collect();
    let segs = tidy_segments(segs, true, tidy);
    segs_to_bezpath(&segs, true)
}

/// Most pieces merged into one line. A line is checked against every piece of its run each time
/// one is added (a pass over the run costs its length), so the work is at most this many checks
/// for each piece, and a straight run of more pieces comes out as lines of this many.
const MAX_LINE_RUN: usize = 1024;

/// The same for pieces of curves, whose merge is a fit through every point of the run (twelve
/// to a piece), so far shorter: a cubic that follows more than this many pieces within the
/// precision is a rare thing, and a run that long comes out as curves of this many.
const MAX_CURVE_RUN: usize = 32;

/// Merge segments that were split from one curve (or collinear lines) back together, dropping
/// degenerate pieces. `closed` segments form a loop. The work is proportional to the segments:
/// a run longer than [`MAX_LINE_RUN`] lines or [`MAX_CURVE_RUN`] curves is merged in parts.
pub(crate) fn tidy_segments(segs: Vec<Seg>, closed: bool, tidy: &Tidy) -> Vec<Seg> {
    let scale = segs.iter().map(|s| s.c.p0.distance(s.c.p3)).fold(0.0, f64::max);
    let tiny = (scale * 1e-9).max(1e-9);
    let mut segs: Vec<Seg> = segs
        .into_iter()
        .filter(|s| {
            let r = kurbo::Rect::from_points(s.c.p0, s.c.p3).union_pt(s.c.p1).union_pt(s.c.p2);
            r.width() + r.height() > tiny
        })
        .collect();
    let n = segs.len();
    if n < 2 {
        return segs;
    }
    // Joint i sits at the start of segment i.
    let mergeable = |a: &Seg, b: &Seg| -> bool {
        if a.line != b.line || tidy.is_kept(b.c.p0) {
            return false;
        }
        let ta = end_tangent(&a.c);
        let tb = start_tangent(&b.c);
        ta.dot(tb) > 0.9998 && ta.cross(tb).abs() < 0.02
    };
    if closed {
        // Rotate so we start at a real corner (if there is one).
        if let Some(k) = (0..n).find(|&i| !mergeable(&segs[(i + n - 1) % n], &segs[i])) {
            segs.rotate_left(k);
        }
    }
    let mut out: Vec<Seg> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut cur = segs[i];
        let mut j = i + 1;
        // Each piece added tries the whole run again, so a run of m costs m squared; a run is cut
        // at a length, and the next starts where it stopped.
        let longest = if segs[i].line { MAX_LINE_RUN } else { MAX_CURVE_RUN };
        while j < n && j - i < longest && mergeable(&segs[j - 1], &segs[j]) {
            match try_merge(&segs[i..=j], tidy) {
                Some(m) => {
                    cur = m;
                    j += 1;
                }
                None => break,
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

fn try_merge(run: &[Seg], tidy: &Tidy) -> Option<Seg> {
    work::add(Work::TidyMerge, run.len());
    let precision = tidy.precision;
    let first = run[0].c;
    let last = run[run.len() - 1].c;
    if run[0].line {
        let chord = last.p3 - first.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let ok = run.iter().all(|s| ((s.c.p3 - first.p0).cross(chord) / len).abs() <= precision * 0.5);
        return ok.then(|| Seg::line(first.p0, last.p3));
    }
    if let Some(c) = exact_merge(run, tidy) {
        return Some(Seg { c, line: false });
    }
    let mut pts = Vec::with_capacity(run.len() * 12 + 1);
    for (k, s) in run.iter().enumerate() {
        sample(&s.c, 12, &mut pts, k == 0);
    }
    let (c, err, _) = fit_single(&pts, start_tangent(&first), end_tangent(&last));
    if err > precision {
        return None;
    }
    Some(Seg { c, line: false })
}

/// If every piece of `run` lies on one input cubic, return the exact sub-curve spanning the run.
fn exact_merge(run: &[Seg], tidy: &Tidy) -> Option<CubicBez> {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let a = run[0].c.p0;
    let b = run[run.len() - 1].c.p3;
    let scale = 1.0 + a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs());
    let tol = 1e-6 * scale;
    for i in tidy.curve_indices_around(a, b) {
        let Some((_, c)) = tidy.curves.get(i) else { continue };
        let na = c.nearest(a, 1e-12);
        let nb = c.nearest(b, 1e-12);
        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol {
            continue;
        }
        let on_curve = run.iter().all(|s| c.nearest(s.c.eval(0.5), 1e-12).distance_sq.sqrt() <= tol * 10.0);
        if !on_curve || (na.t - nb.t).abs() < 1e-12 {
            continue;
        }
        let sub = if na.t < nb.t {
            c.subsegment(na.t..nb.t)
        } else {
            let r = c.subsegment(nb.t..na.t);
            CubicBez::new(r.p3, r.p2, r.p1, r.p0)
        };
        return Some(CubicBez::new(a, sub.p1, sub.p2, b));
    }
    None
}

impl Tidy {
    /// The indices, ascending, of the input curves whose bounds hold both `a` and `b`. The index
    /// gives the curves whose bounds hold `a`, in the order of `curves`.
    fn curve_indices_around(&self, a: Point, b: Point) -> Vec<usize> {
        let Some(index) = &self.curve_index else { return Vec::new() };
        index.at(a, 0.0).copied().filter(|&i| self.curves.get(i).is_some_and(|(r, _)| r.contains(b))).collect()
    }

    /// The input curves whose bounds hold both `a` and `b`.
    fn curves_around(&self, a: Point, b: Point) -> impl Iterator<Item = &CubicBez> {
        self.curve_indices_around(a, b).into_iter().filter_map(|i| self.curves.get(i)).map(|(_, c)| c)
    }

    /// `bp` with each curve piece that strays from the one input curve through both its ends
    /// rebuilt as that curve's exact sub-curve. linesweeper 0.5 can emit a piece that doesn't
    /// follow its curve; out and back between two points of one curve, such pieces are a lens
    /// instead of nothing (VectorCraft issue 1028). A piece on a curve, or whose ends two different input
    /// curves pass through (a real lens between them), is left as it is.
    pub(crate) fn repair(&self, bp: &BezPath) -> BezPath {
        use kurbo::{ParamCurve, ParamCurveNearest, PathEl};
        if self.curves.is_empty() {
            return bp.clone();
        }
        let mut out = BezPath::new();
        let mut cur = Point::ZERO;
        let mut start = Point::ZERO;
        for el in bp.iter() {
            let piece = match el {
                PathEl::QuadTo(c, p) => Some(kurbo::QuadBez::new(cur, c, p).raise()),
                PathEl::CurveTo(c1, c2, p) => Some(CubicBez::new(cur, c1, c2, p)),
                _ => None,
            };
            match (el, piece) {
                (_, Some(raw)) => {
                    let (a, b) = (raw.p0, raw.p3);
                    let scale = 1.0 + a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs());
                    let tol = 1e-6 * scale;
                    let mut subs: Vec<CubicBez> = vec![];
                    let mut follows = false;
                    for c in self.curves_around(a, b) {
                        let (na, nb) = (c.nearest(a, 1e-12), c.nearest(b, 1e-12));
                        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol || (na.t - nb.t).abs() < 1e-12 {
                            continue;
                        }
                        if [0.25, 0.5, 0.75].iter().all(|&t| c.nearest(raw.eval(t), 1e-12).distance_sq.sqrt() <= tol * 10.0) {
                            follows = true;
                            break;
                        }
                        let sub = if na.t < nb.t {
                            c.subsegment(na.t..nb.t)
                        } else {
                            let r = c.subsegment(nb.t..na.t);
                            CubicBez::new(r.p3, r.p2, r.p1, r.p0)
                        };
                        subs.push(CubicBez::new(a, sub.p1, sub.p2, b));
                    }
                    let same = |p: &CubicBez, q: &CubicBez| p.p1.distance(q.p1) <= tol * 10.0 && p.p2.distance(q.p2) <= tol * 10.0;
                    match subs.split_first() {
                        Some((sub, rest)) if !follows && rest.iter().all(|q| same(sub, q)) => out.curve_to(sub.p1, sub.p2, b),
                        _ => out.push(el),
                    }
                    cur = b;
                }
                (PathEl::MoveTo(p), None) => {
                    out.push(el);
                    (cur, start) = (p, p);
                }
                (PathEl::LineTo(p), None) => {
                    out.push(el);
                    cur = p;
                }
                (PathEl::ClosePath, None) => {
                    out.push(el);
                    cur = start;
                }
                (_, None) => out.push(el),
            }
        }
        out
    }
}

/// `precision` when it is a length a refit can use (finite and above zero), else
/// [`DEFAULT_PRECISION`], as `pathfinder_with` takes it: a NaN would let every refit pass.
pub(crate) fn sane_precision(precision: f64) -> f64 {
    if precision.is_finite() && precision > 0.0 { precision } else { DEFAULT_PRECISION }
}

/// `Ok` when `paths` together are within [`crate::MAX_ELEMENTS`], else [`PathOpsError::TooComplex`].
pub(crate) fn ensure_size<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<(), PathOpsError> {
    if crate::budget::fits_elements(paths) { Ok(()) } else { Err(PathOpsError::TooComplex) }
}

/// Resolve self-intersections / overlaps of a single filled path into simple, consistently
/// oriented contours (outer contours and holes).
pub fn normalize(path: &BezPath, rule: FillRule) -> BezPath {
    try_normalize(path, rule).unwrap_or_default()
}

/// Fallible [`normalize`].
pub fn try_normalize(path: &BezPath, rule: FillRule) -> Result<BezPath, PathOpsError> {
    ensure_size([path])?;
    let bp = fill_bezpath(path);
    let c = normalize_bez(&bp, rule)?;
    Ok(all_contours_to_path(&c, &Tidy::keeping(DEFAULT_PRECISION, [&bp])))
}

pub(crate) fn normalize_bez(bp: &BezPath, rule: FillRule) -> Result<Contours, PathOpsError> {
    if !crate::budget::fits_elements([bp]) {
        return Err(PathOpsError::TooComplex);
    }
    let eps = eps_for([bp])?;
    let mut bp = bp.clone();
    snap_horizontals(&mut [&mut bp], eps);
    // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
    if !crate::budget::sweep_fits([&bp]) {
        return Err(PathOpsError::TooComplex);
    }
    // As in `Sweep::new`, a panic in the sweep on near-degenerate input becomes an error.
    work::add(Work::Sweeps, 1);
    let swept =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<i32>::from_path(&bp, eps).map(|top| top.contours(|w| inside(rule, *w)))));
    swept.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)
}

/// Boolean operation between two filled paths. Returns an empty path on invalid (non-finite) input.
pub fn boolean(a: &BezPath, a_rule: FillRule, b: &BezPath, b_rule: FillRule, op: BoolOp) -> BezPath {
    try_boolean(a, a_rule, b, b_rule, op, DEFAULT_PRECISION).unwrap_or_default()
}

/// Boolean operation with an explicit refit `precision`; errors on non-finite input.
///
/// The sweep can give some faces of the arrangement a wrong winding when an operand crosses or
/// touches itself or has overlapping subpaths, and the result then silently gains or loses a
/// region (VectorCraft issue 5478). So the windings are checked against each operand's own area
/// (`windings_add_up`); when they don't add up, each operand is first resolved on its own into
/// simple contours (`resolved`) and the operation runs again on those. When that doesn't add up
/// either, the first result is kept (as before the check).
pub fn try_boolean(a: &BezPath, a_rule: FillRule, b: &BezPath, b_rule: FillRule, op: BoolOp, precision: f64) -> Result<BezPath, PathOpsError> {
    ensure_size([a, b])?;
    let precision = sane_precision(precision);
    let (pa, pb) = (fill_bezpath(a), fill_bezpath(b));
    let arr = Arrangement::new(vec![(pa.clone(), a_rule), (pb.clone(), b_rule)])?;
    let c = arr.try_contours(|m| op.apply(m[0], m[1]))?;
    if !arrangement_adds_up(&arr, [&pa, &pb])
        && let Some(r) = resolved_boolean([(&pa, a_rule), (&pb, b_rule)], op, precision)
    {
        return Ok(r);
    }
    let tidy = Tidy { precision, ..arr.tidy };
    Ok(all_contours_to_path(&c, &tidy))
}

/// The widest winding (either sign) [`windings_add_up`] checks: one contour pass per level, so a
/// more tangled input is taken as it is.
const MAX_CHECKED_WINDING: i32 = 8;

/// Do the windings `arr` gave each input add up to that input's own area ([`windings_add_up`])?
/// Also true when it can't tell.
fn arrangement_adds_up(arr: &Arrangement, inputs: [&BezPath; 2]) -> bool {
    inputs.iter().enumerate().all(|(tag, p)| windings_add_up(&arr.top, |w: &Multi| w.get(tag), p).unwrap_or(true))
}

/// Does the winding `winding` that the sweep `top` gave its faces add up to the signed area of the
/// path `input` it was made from? By Green's theorem the integral of a path's winding number over
/// the plane is the path's signed area, and that integral is Σₖ |{w ≥ k}| − Σₖ |{w ≤ −k}| (k ≥ 1),
/// each area read from the contours of that region. A face given a wrong winding (VectorCraft issue 5478) puts it
/// off by the face's area. None when it can't tell: windings past [`MAX_CHECKED_WINDING`], or a
/// contour pass that fails.
fn windings_add_up<W: WindingNumber>(top: &Topology<W>, winding: impl Fn(&W) -> i32, input: &BezPath) -> Option<bool> {
    let (mut lo, mut hi) = (0, 0);
    for s in top.segment_indices() {
        let h = s.first_half();
        for w in [top.winding_clockwise(h), top.winding_counter_clockwise(h)] {
            let k = winding(w);
            (lo, hi) = (lo.min(k), hi.max(k));
        }
    }
    if hi > MAX_CHECKED_WINDING || lo < -MAX_CHECKED_WINDING {
        return None;
    }
    let region = |pred: &dyn Fn(i32) -> bool| -> Option<f64> {
        let c = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| top.contours(|w| pred(winding(w))))).ok()?;
        Some(c.contours().map(|ct| ct.path.area()).sum::<f64>().abs())
    };
    let mut integral = 0.0;
    for k in 1..=hi {
        integral += region(&|w| w >= k)?;
    }
    for k in 1..=-lo {
        integral -= region(&|w| w <= -k)?;
    }
    let want = input.area();
    // The sweep moves points by up to its tolerance, which moves an area by about that much along
    // each edge; a misplaced face is far larger.
    let bounds = input.bounding_box();
    let eps = eps_for([input]).ok()?;
    let tol = 1e-6 * bounds.area().abs() + 16.0 * eps * input.perimeter(1e-3) * f64::from(hi - lo).max(1.0);
    Some((integral - want).abs() <= tol || (integral + want).abs() <= tol)
}

/// `input` resolved on its own under `rule` into simple contours (outer contours and holes, filled
/// non-zero), when the windings of that sweep add up ([`windings_add_up`]).
fn resolved(input: &BezPath, rule: FillRule) -> Option<BezPath> {
    if !crate::budget::fits_elements([input]) {
        return None;
    }
    let eps = eps_for([input]).ok()?;
    let mut bp = input.clone();
    snap_horizontals(&mut [&mut bp], eps);
    if !crate::budget::sweep_fits([&bp]) {
        return None;
    }
    work::add(Work::Sweeps, 1);
    let top = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<i32>::from_path(&bp, eps))).ok()?.ok()?;
    if !windings_add_up(&top, |w| *w, &bp)? {
        return None;
    }
    let c = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| top.contours(|w| inside(rule, *w)))).ok()?;
    let mut out = BezPath::new();
    for ct in c.contours() {
        out.extend(ct.path.iter());
    }
    Some(out)
}

/// [`try_boolean`] on the operands resolved into simple contours first ([`resolved`]): the result
/// when every sweep's windings add up, else None.
fn resolved_boolean(inputs: [(&BezPath, FillRule); 2], op: BoolOp, precision: f64) -> Option<BezPath> {
    let [(pa, a_rule), (pb, b_rule)] = inputs;
    let (ra, rb) = (resolved(pa, a_rule)?, resolved(pb, b_rule)?);
    let arr = Arrangement::new(vec![(ra.clone(), FillRule::NonZero), (rb.clone(), FillRule::NonZero)]).ok()?;
    if !arrangement_adds_up(&arr, [&ra, &rb]) {
        return None;
    }
    let c = arr.try_contours(|m| op.apply(m[0], m[1])).ok()?;
    // The refit keeps the operands' own anchors, not only those of their resolved contours.
    let tidy = Tidy::keeping(precision, [pa, pb]);
    Some(all_contours_to_path(&c, &tidy))
}

/// [`boolean_n`] that reports a refused input ([`PathOpsError::TooComplex`], non-finite
/// coordinates) instead of returning an empty path.
pub fn try_boolean_n(paths: &[(&BezPath, FillRule)], pred: impl Fn(&[bool]) -> bool) -> Result<BezPath, PathOpsError> {
    ensure_size(paths.iter().map(|(p, _)| *p))?;
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    let arr = Arrangement::new(bps)?;
    Ok(all_contours_to_path(&arr.try_contours(pred)?, &arr.tidy))
}

/// N-ary boolean: the area where `pred(inside_flags)` holds, `inside_flags[i]` telling whether a
/// point is inside `paths[i]` under its fill rule. A refused input gives an empty path; callers that
/// can report an error use [`try_boolean_n`].
pub fn boolean_n(paths: &[(&BezPath, FillRule)], pred: impl Fn(&[bool]) -> bool) -> BezPath {
    if ensure_size(paths.iter().map(|(p, _)| *p)).is_err() {
        return BezPath::new();
    }
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    match Arrangement::new(bps) {
        Ok(arr) => guarded(|| all_contours_to_path(&arr.contours(pred), &arr.tidy)).unwrap_or_default(),
        Err(_) => BezPath::new(),
    }
}

/// [`unite_all`] that reports a refused input instead of leaving it out of the union or returning
/// an empty path.
pub fn try_unite_all(paths: &[(&BezPath, FillRule)]) -> Result<BezPath, PathOpsError> {
    try_unite_all_with(paths, DEFAULT_PRECISION)
}

/// [`try_unite_all`] refit to `precision`, dropping slivers thinner than it.
pub fn try_unite_all_with(paths: &[(&BezPath, FillRule)], precision: f64) -> Result<BezPath, PathOpsError> {
    ensure_size(paths.iter().map(|(p, _)| *p))?;
    let precision = sane_precision(precision);
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        for ct in normalize_bez(&bp, *r)?.contours() {
            all.extend(ct.path.iter());
        }
    }
    if all.elements().is_empty() {
        return Ok(BezPath::new());
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    Ok(all_contours_to_path(&normalize_bez(&all, FillRule::NonZero)?, &tidy))
}

/// Union of many filled paths. Each input is first normalised on its own, then all are swept
/// together with a single integer winding number, which keeps this fast for thousands of shapes.
pub fn unite_all(paths: &[(&BezPath, FillRule)]) -> BezPath {
    unite_all_with(paths, DEFAULT_PRECISION)
}

/// [`unite_all`] refit to `precision`, dropping slivers thinner than it.
pub fn unite_all_with(paths: &[(&BezPath, FillRule)], precision: f64) -> BezPath {
    if ensure_size(paths.iter().map(|(p, _)| *p)).is_err() {
        return BezPath::new();
    }
    let precision = sane_precision(precision);
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        let Ok(c) = normalize_bez(&bp, *r) else { continue };
        for ct in c.contours() {
            all.extend(ct.path.iter());
        }
    }
    if all.elements().is_empty() {
        return BezPath::new();
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    match normalize_bez(&all, FillRule::NonZero) {
        Ok(c) => all_contours_to_path(&c, &tidy),
        Err(_) => BezPath::new(),
    }
}

/// [`area`] that reports a refused input instead of an area of zero.
pub fn try_area(path: &BezPath, rule: FillRule) -> Result<f64, PathOpsError> {
    ensure_size([path])?;
    let c = normalize_bez(&fill_bezpath(path), rule)?;
    Ok(c.contours().map(|ct| ct.path.area()).sum::<f64>().abs())
}

/// Unsigned filled area of a path under a fill rule (normalises first, so it is exact for
/// self-overlapping input).
pub fn area(path: &BezPath, rule: FillRule) -> f64 {
    if ensure_size([path]).is_err() {
        return 0.0;
    }
    let bp = fill_bezpath(path);
    match normalize_bez(&bp, rule) {
        Ok(c) => c.contours().map(|ct| ct.path.area()).sum::<f64>().abs(),
        Err(_) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use kurbo::ParamCurve;

    use super::*;

    /// A quarter-circle-like arc, the input curve the pieces below are cut from.
    fn arc() -> CubicBez {
        CubicBez::new((0.0, 0.0), (0.0, 55.0), (45.0, 100.0), (100.0, 100.0))
    }

    #[test]
    fn a_stray_piece_out_and_back_along_one_curve_is_no_outline() {
        // VectorCraft issue 1028: linesweeper 0.5 handed back a contour running out along an input curve and
        // back by a piece that doesn't follow it, which refit into a visible lens.
        let c = arc();
        let tidy =
            Tidy::keeping(DEFAULT_PRECISION, [&BezPath::from_vec(vec![kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)])]);
        let out = c.subsegment(0.2..0.6);
        let mut contour = BezPath::new();
        contour.move_to(out.p0);
        contour.curve_to(out.p1, out.p2, out.p3);
        // back to the start, bulging off the curve and overshooting the end like the reported piece
        contour.curve_to(out.p0 + (out.p0 - out.p3) * 0.15 + kurbo::Vec2::new(4.0, -3.0), out.p1 + kurbo::Vec2::new(3.0, -2.0), out.p0);
        contour.close_path();
        assert!(!is_sliver(&contour, tidy.precision), "the stray piece makes a lens");
        let fixed = tidy.repair(&contour);
        assert!(is_sliver(&fixed, tidy.precision), "{}", fixed.to_svg());
        // a piece on its curve is left exactly as it is
        let mut on = BezPath::new();
        on.move_to(out.p0);
        on.curve_to(out.p1, out.p2, out.p3);
        on.line_to(out.p0);
        on.close_path();
        assert_eq!(tidy.repair(&on), on);
    }

    #[test]
    fn a_lens_between_two_curves_through_the_same_points_stays() {
        // Two input curves meeting at both ends bound a real face: neither piece is "repaired"
        // onto the other.
        let lower = CubicBez::new((0.0, 0.0), (30.0, 20.0), (70.0, 20.0), (100.0, 0.0));
        let upper = CubicBez::new((0.0, 0.0), (30.0, -20.0), (70.0, -20.0), (100.0, 0.0));
        let input = |c: CubicBez| BezPath::from_vec(vec![kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)]);
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&input(lower), &input(upper)]);
        let mut lens = BezPath::new();
        lens.move_to(lower.p0);
        lens.curve_to(lower.p1, lower.p2, lower.p3);
        lens.curve_to(upper.p2, upper.p1, upper.p0);
        lens.close_path();
        assert_eq!(tidy.repair(&lens), lens);
    }

    #[test]
    fn arrangement_new_budgeted_fails_on_complexity_and_open_paths() {
        let mut bp = BezPath::new();
        for i in 0..100 {
            let r = kurbo::Rect::new(0.0, 0.0, 10.0 + i as f64, 10.0 + i as f64);
            bp.extend(r.to_path(0.1).iter());
        }
        let too_complex = Arrangement::new_budgeted(vec![(bp, FillRule::NonZero)], 5);
        assert_eq!(too_complex.err(), Some(PathOpsError::TooComplex));

        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 10.0));
        let failed_arr = Arrangement::new_budgeted(vec![(open, FillRule::NonZero)], 1000);
        assert_eq!(failed_arr.err(), Some(PathOpsError::OpenPath));
    }

    fn square(x: f64, y: f64, size: f64) -> BezPath {
        let mut bp = BezPath::new();
        bp.move_to((x, y));
        bp.line_to((x + size, y));
        bp.line_to((x + size, y + size));
        bp.line_to((x, y + size));
        bp.close_path();
        bp
    }

    #[test]
    fn coordinates_wider_than_the_precision_are_refused_not_emptied() {
        // At 1e15 the sweep tolerance is far above a 10 unit square: both squares used to vanish and the call said `Ok`.
        let near = square(0.0, 0.0, 10.0);
        let far = square(1e15, 1e15, 10.0);
        for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
            assert_eq!(
                try_boolean(&near, FillRule::NonZero, &far, FillRule::NonZero, op, DEFAULT_PRECISION),
                Err(PathOpsError::Degenerate),
                "{op:?}"
            );
        }
        assert_eq!(try_area(&far, FillRule::NonZero), Err(PathOpsError::Degenerate));
        assert_eq!(try_normalize(&far, FillRule::NonZero), Err(PathOpsError::Degenerate));
        // The infallible entry points keep answering with an empty path.
        assert!(boolean(&near, FillRule::NonZero, &far, FillRule::NonZero, BoolOp::Union).elements().is_empty());
    }

    #[test]
    fn coordinates_near_the_float_limit_do_not_reach_the_sweep() {
        // A 1e308 rectangle made linesweeper panic (a caught panic here, an abort on the web).
        let huge = square(0.0, 0.0, 1e308);
        let small = square(0.0, 0.0, 10.0);
        assert_eq!(try_area(&huge, FillRule::NonZero), Err(PathOpsError::Degenerate));
        assert_eq!(try_boolean(&huge, FillRule::NonZero, &small, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION), Err(PathOpsError::Degenerate));
        assert_eq!(try_unite_all(&[(&huge, FillRule::NonZero), (&small, FillRule::NonZero)]), Err(PathOpsError::Degenerate));
    }

    #[test]
    fn a_precision_that_is_no_length_is_the_default() {
        let (a, b) = (square(0.0, 0.0, 10.0), square(5.0, 5.0, 10.0));
        let want = try_boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION);
        assert!(want.is_ok());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(try_boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union, bad), want, "{bad}");
            assert_eq!(
                unite_all_with(&[(&a, FillRule::NonZero), (&b, FillRule::NonZero)], bad),
                unite_all(&[(&a, FillRule::NonZero), (&b, FillRule::NonZero)])
            );
        }
    }

    /// Curves in a column, all of one width, are found by the band they lie in and not by a scan
    /// of them all: a lookup reads a leaf or two, however many curves there are.
    #[test]
    fn curves_in_a_column_are_found_by_their_band_not_by_a_scan() {
        let n = 2_000;
        let column = crate::testutil::column_of_lenses(n);
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&column]);
        work::reset();
        for i in 0..n {
            // Inside the upper cubic of lens `i`, whose bounds are 3 wide and 3 high.
            let y = 20.0 + i as f64 * 10.0;
            let found = tidy.curve_indices_around(Point::new(1.0, y + 1.0), Point::new(2.0, y + 2.0));
            assert_eq!(found, vec![2 * i], "lens {i}");
        }
        let read = work::take(Work::IndexCandidates);
        // A leaf of the tree is 16 curves, read whole.
        assert!(read <= 24 * n, "{read} candidates read for {n} lookups; a scan would read {}", 2 * n * n);
    }

    /// The same column united: the work grows with the column, not with its square.
    #[test]
    fn uniting_a_column_reads_candidates_in_proportion_to_its_size() {
        let reads = |n: usize| {
            let column = crate::testutil::column_of_lenses(n);
            work::reset();
            let united = try_unite_all(&[(&column, FillRule::NonZero)]).unwrap();
            let lens = area(&crate::testutil::lens(0.0, 0.0), FillRule::NonZero);
            assert!((area(&united, FillRule::NonZero) - n as f64 * lens).abs() < 1e-6 * n as f64 * lens);
            work::take(Work::IndexCandidates)
        };
        let (small, large) = (reads(250), reads(1_000));
        assert!(small > 0 && large < 6 * small, "{small} for 250 lenses, {large} for 1,000: four times as many should read about four times as much");
    }

    /// A shape far from all the others costs the rest nothing. An index laid out evenly over the
    /// union of the bounds (a uniform grid) put every shape of the grid below in one cell as soon
    /// as one lens lay ten million away, and every lookup read them all.
    #[test]
    fn a_shape_far_from_a_grid_of_shapes_does_not_slow_the_rest() {
        let reads = |n: usize, far: bool| {
            let mut grid = crate::testutil::grid_of_lenses(n);
            if far {
                grid.extend(crate::testutil::lens(1e7, 1e7).iter());
            }
            work::reset();
            let united = try_unite_all(&[(&grid, FillRule::NonZero)]).unwrap();
            let lens = area(&crate::testutil::lens(0.0, 0.0), FillRule::NonZero);
            let want = (n + usize::from(far)) as f64 * lens;
            assert!((area(&united, FillRule::NonZero) - want).abs() < 1e-6 * want, "n={n} far={far}");
            work::take(Work::IndexCandidates) + work::take(Work::IndexNodes)
        };
        let (near, near_far) = (reads(1_024, false), reads(1_024, true));
        assert!(near > 0 && near_far <= 2 * near + 1_000, "{near} without the far lens, {near_far} with it");
        let large = reads(4_096, true);
        assert!(
            large <= 6 * near_far,
            "{near_far} for 1,024 lenses and one far, {large} for 4,096: four times as many should read about four times as much"
        );
    }

    /// Kept points that lie on one vertical line but for float noise (each a hair off the last, less
    /// than the tolerance) are found by their place, not by a scan of the points within the
    /// tolerance in x: a sorted list searched on x alone would read them all for every lookup.
    #[test]
    fn kept_points_on_a_nearly_vertical_line_are_found_by_their_place() {
        let n = 5_000;
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        for i in 1..n {
            line.line_to((f64::from(i) * 1e-9, f64::from(i)));
        }
        line.line_to((10.0, 2_500.0));
        line.close_path();
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&line]);
        work::reset();
        let mut lookups = 0;
        for i in (0..n).step_by(7) {
            assert!(tidy.is_kept(Point::new(0.0, f64::from(i))), "point {i}");
            lookups += 1;
        }
        assert!(!tidy.is_kept(Point::new(0.0, 100.5)), "between two of them");
        let read = work::take(Work::IndexCandidates) + work::take(Work::IndexNodes);
        assert!(read <= 100 * (lookups + 1), "{read} read for {lookups} lookups; a scan would read {}", 5_000 * lookups);
    }

    /// A path of points that are not numbers costs the paths beside it nothing. The unions that skip
    /// an input they cannot sweep (`unite_all`, `pathfinder(Unite)`, `merge_regions`) still keep
    /// the anchors of every input; those of the bad one must not be filed, nor scanned at every
    /// lookup (a list of them scanned for each point made the union quadratic).
    #[test]
    fn a_path_of_non_finite_points_costs_the_others_nothing() {
        use crate::{PathfinderOp, Region, Shape, merge_regions, pathfinder};
        let bad = |n: usize| {
            let mut b = BezPath::new();
            for _ in 0..n {
                b.extend(crate::testutil::lens(f64::NAN, f64::NAN).iter());
            }
            b
        };
        let reads = |n: usize, with_bad: bool, how: usize| {
            let grid = crate::testutil::grid_of_lenses(n);
            let bad = if with_bad { bad(n) } else { BezPath::new() };
            work::reset();
            let area = |p: &BezPath| try_area(p, FillRule::NonZero).unwrap();
            let want = n as f64 * area(&crate::testutil::lens(0.0, 0.0));
            work::reset();
            let united = match how {
                0 => unite_all(&[(&grid, FillRule::NonZero), (&bad, FillRule::NonZero)]),
                1 => {
                    let shapes = [Shape::new(grid.clone(), FillRule::NonZero, 0), Shape::new(bad.clone(), FillRule::NonZero, 1)];
                    pathfinder(PathfinderOp::Unite, &shapes).into_iter().next().map(|s| s.path).unwrap_or_default()
                }
                _ => {
                    let (a, b) = (Region { path: grid.clone(), sources: vec![0] }, Region { path: bad.clone(), sources: vec![1] });
                    merge_regions(&[&a, &b])
                }
            };
            let read = work::take(Work::IndexCandidates) + work::take(Work::IndexNodes);
            assert!((area(&united) - want).abs() < 1e-6 * want, "n={n} bad={with_bad} how={how}: {} against {want}", area(&united));
            read
        };
        for (how, name) in [(0, "unite_all"), (1, "pathfinder(Unite)"), (2, "merge_regions")] {
            let (clean, dirty) = (reads(256, false, how), reads(256, true, how));
            assert!(clean > 0 && dirty <= 2 * clean + 1_000, "{name}: {clean} read without the bad path, {dirty} with it");
            let large = reads(1_024, true, how);
            assert!(large <= 6 * dirty, "{name}: {dirty} read for 256 lenses and a bad path, {large} for 1,024");
        }
    }

    /// Pieces of one long run that can merge are tried at each length, so the work to merge a run
    /// of m pieces grows as m squared unless the run is cut: the tidy cuts it, and the work is
    /// proportional to the pieces, the jittered vertical line of a stroke outline included.
    #[test]
    fn a_long_run_of_mergeable_pieces_is_tidied_in_proportion_to_its_length() {
        // A thin closed outline: up a line whose points are 1e-9 off a vertical (each joint is a
        // sweep's, far below the tolerance), and back down a straight one.
        let outline = |n: usize| {
            let mut b = BezPath::new();
            b.move_to((0.0, 0.0));
            for i in 1..=n {
                b.line_to((if i % 2 == 0 { 1e-9 } else { 0.0 }, i as f64));
            }
            b.line_to((10.0, n as f64));
            b.line_to((10.0, 0.0));
            b.close_path();
            b
        };
        let reads = |n: usize| {
            work::reset();
            let region = crate::try_stroke_region(&[outline(n)]).unwrap();
            let want = 10.0 * n as f64;
            assert!((area(&region, FillRule::NonZero) - want).abs() < 1e-3 * want, "n={n}");
            work::take(Work::TidyMerge)
        };
        let (small, large) = (reads(2_048), reads(8_192));
        assert!(
            small > 0 && large <= 5 * small,
            "{small} for 2,048 points, {large} for 8,192: four times as many should take about four times as much"
        );
        assert!(large <= 600 * 8_192, "{large} for 8,192 points");
        // Pieces of one curve, each tangent to the next: a run of cubics the fit tries at each length.
        let pieces = |n: usize| -> Vec<Seg> {
            let whole = CubicBez::new((0.0, 0.0), (400.0, 100.0), (800.0, 100.0), (1200.0, 0.0));
            (0..n).map(|k| Seg { c: whole.subsegment(k as f64 / n as f64..(k + 1) as f64 / n as f64), line: false }).collect()
        };
        let curve_reads = |n: usize| {
            work::reset();
            let merged = tidy_segments(pieces(n), false, &Tidy::free(DEFAULT_PRECISION));
            assert!(!merged.is_empty() && merged.len() < n, "{} pieces of {n}", merged.len());
            work::take(Work::TidyMerge)
        };
        let (small, large) = (curve_reads(512), curve_reads(1_024));
        assert!(small > 0 && large * 10 <= small * 25, "{small} for 512 pieces, {large} for 1,024: twice as many should take about twice as much");
    }

    /// The inputs' own points are found exactly, also a comb of them on one vertical line (a scan
    /// of the points at one x gave up past 256 of them, and kept every joint after).
    #[test]
    fn kept_points_on_one_vertical_line_are_found_exactly() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        for i in 1..5_000 {
            line.line_to((0.0, f64::from(i)));
        }
        line.line_to((10.0, 2_500.0));
        line.close_path();
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&line]);
        work::reset();
        let lookups = 5_000 / 7 + 5;
        for i in (0..5_000).step_by(7) {
            assert!(tidy.is_kept(Point::new(0.0, f64::from(i))), "point {i}");
        }
        assert!(!tidy.is_kept(Point::new(0.0, 100.5)), "between two of them");
        assert!(!tidy.is_kept(Point::new(1e-3, 100.0)), "beside one of them");
        assert!(tidy.is_kept(Point::new(0.0, 100.0 + 1e-7)), "within a millionth of one");
        assert!(tidy.is_kept(Point::new(10.0, 2_500.0)));
        assert!(!Tidy::free(0.01).is_kept(Point::new(0.0, 0.0)));
        let read = work::take(Work::IndexCandidates);
        assert!(read <= 32 * lookups, "{read} candidates read for {lookups} lookups; a scan would read {}", 5_000 * lookups);
    }

    #[test]
    fn large_but_sensible_coordinates_still_work() {
        // Well inside the limit: far-out but sensible offsets.
        for at in [1e5, 1e7] {
            let u = try_boolean(
                &square(at, at, 10.0),
                FillRule::NonZero,
                &square(at + 5.0, at, 10.0),
                FillRule::NonZero,
                BoolOp::Union,
                DEFAULT_PRECISION,
            )
            .unwrap();
            let a = try_area(&u, FillRule::NonZero).unwrap();
            assert!((a - 150.0).abs() < 1.0, "at {at}: area {a}");
        }
    }

    #[test]
    fn the_coordinate_limit_sits_where_the_tolerance_passes_a_unit() {
        // 100 unit squares: tolerance 0.7 at 5e13 (kept), 1.4 at 1e14 (refused).
        assert!(try_area(&square(5e13, 5e13, 100.0), FillRule::NonZero).is_ok());
        assert_eq!(try_area(&square(1e14, 1e14, 100.0), FillRule::NonZero), Err(PathOpsError::Degenerate));
    }

    /// VectorCraft issue 5478: the sweep of a triangle against an operand that crosses itself gives faces windings
    /// that don't add up to the operands' areas; resolving the operands first gives ones that do.
    #[test]
    fn wrong_windings_are_caught_and_resolving_the_operands_fixes_them() {
        let poly = |pts: &[(f64, f64)]| {
            let mut bp = BezPath::new();
            for (i, p) in pts.iter().enumerate() {
                if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
            }
            bp.close_path();
            bp
        };
        let a = poly(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]);
        let b = poly(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]);
        let arr = Arrangement::new(vec![(a.clone(), FillRule::NonZero), (b.clone(), FillRule::NonZero)]).unwrap();
        assert!(!arrangement_adds_up(&arr, [&a, &b]), "the sweep's windings are off");
        let (ra, rb) = (resolved(&a, FillRule::NonZero).unwrap(), resolved(&b, FillRule::NonZero).unwrap());
        let arr = Arrangement::new(vec![(ra.clone(), FillRule::NonZero), (rb.clone(), FillRule::NonZero)]).unwrap();
        assert!(arrangement_adds_up(&arr, [&ra, &rb]));
        // A plain pair adds up at once.
        let (s, t) = (poly(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]), poly(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]));
        let arr = Arrangement::new(vec![(s.clone(), FillRule::NonZero), (t.clone(), FillRule::EvenOdd)]).unwrap();
        assert!(arrangement_adds_up(&arr, [&s, &t]));
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn a_panic_in_the_contour_pass_becomes_an_error() {
        let arr = Arrangement::new(vec![(fill_bezpath(&square(0.0, 0.0, 10.0)), FillRule::NonZero)]).unwrap();
        assert!(arr.try_contours(|m| m[0]).is_ok());
        assert_eq!(arr.try_contours(|_| panic!("simulated panic in the sweep")).err(), Some(PathOpsError::Degenerate));
    }
}
