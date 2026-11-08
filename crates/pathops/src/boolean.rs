// Carved from VectorCraft's crates/pathops (src/boolean.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Curve-preserving boolean operations built on `linesweeper`'s robust sweep-line topology.
//!
//! Inputs are converted to closed [`BezPath`]s (open subpaths are implicitly closed, as a filled
//! open path is painted). The sweep splits curves at intersections and y-extrema;
//! [`tidy_segments`] then re-joins pieces that came from the same smooth curve by refitting, so
//! results have roughly as few anchors as the inputs.

use kurbo::{BezPath, CubicBez, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours, Topology, WindingNumber};

use crate::fit::{end_tangent, fit_single, sample, start_tangent};
use crate::path::{Seg, fill_bezpath, segs_to_bezpath};
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

/// Is a point of winding number `w` inside under `rule`?
pub(crate) fn inside(rule: FillRule, w: i32) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// Winding numbers for an arbitrary number of input sets (tag = input index): the non-zero ones,
/// as (tag, winding) sorted by tag. A face is covered by a few of the inputs, so this stays short
/// where a vector of every input's winding would make each Pathfinder test cost one step per input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Multi(Vec<(usize, i32)>);

impl Multi {
    fn get(&self, i: usize) -> i32 {
        self.0.binary_search_by_key(&i, |e| e.0).ok().and_then(|k| self.0.get(k)).map_or(0, |e| e.1)
    }
    /// How many inputs have a non-zero winding.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
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
    /// Adds `rhs`'s winding of each input to this one's, keeping the entries in tag order and none of zero.
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
    /// The winding of one crossing of the input `tag`'s boundary: 1 or -1 by its direction.
    fn single(tag: usize, positive: bool) -> Self {
        Multi(vec![(tag, if positive { 1 } else { -1 })])
    }
    /// The winding of the input `tag` alone, the others left out.
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
    let eps = sweep_tolerance(m);
    // Past this the sweep tolerance is wider than a unit: features thinner than it collapse and the call
    // would answer `Ok` with part of the art (or all of it) gone. Refuse instead, so callers keep the original.
    if eps > MAX_SWEEP_EPS {
        return Err(PathOpsError::Degenerate);
    }
    Ok(eps)
}

/// The sweep's tolerance for paths whose points reach `extent` in x or y (control points too): the
/// size of the coordinates times 64 ulps, and not under a millionth.
pub(crate) fn sweep_tolerance(extent: f64) -> f64 {
    (extent * f64::EPSILON * 64.0).max(1e-6)
}

/// The widest sweep tolerance [`eps_for`] accepts: one unit, reached by coordinates of about 7e13. (Applications refuse
/// coordinates far below that, so this only guards the library's own callers; it does not follow the `precision` a caller
/// asks for.)
const MAX_SWEEP_EPS: f64 = 1.0;

/// The points of the element `el`, control points too, in order (none for a `ClosePath`).
pub(crate) fn el_points(el: &kurbo::PathEl) -> impl Iterator<Item = Point> {
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

/// Most flags a boolean operation sets to tell its predicate what covers the faces: 268,435,456
/// (the default of [`crate::Limits::mask_work`]). The predicate is given one for every input each time
/// it is asked, about four times for each segment of the sweep, so many inputs (most of them
/// empty, say) make the work the product of their number and the sweep's, which the other limits
/// do not count. About a quarter of a second in a release build. Listing the inputs that cover each
/// side of each edge, for the faces [`crate::pathfinder`] Divide and [`crate::regions`] give, is held
/// to as many entries (one for each covering input and one for each side).
pub const MASK_WORK: usize = 1 << 28;

/// The order of two lists of covering inputs (ascending) that the flags of every input would give:
/// at the first input where one has a flag and the other has none, the one with none is the smaller;
/// a list that is the start of another is the smaller.
pub(crate) fn cmp_masks(a: &[usize], b: &[usize]) -> std::cmp::Ordering {
    a.iter().zip(b).find(|(x, y)| x != y).map_or_else(|| a.len().cmp(&b.len()), |(x, y)| y.cmp(x))
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

/// The answers of a predicate for the windings a sweep stores, found by where they are: the two sides
/// of each segment are stored in one run of memory, so a winding's address says which it is and a
/// lookup hashes nothing (a winding of nested shapes is a long list). A reference to a winding
/// that is not one of those stored (the sweep gives none) is answered by the predicate.
struct SideAnswers {
    /// The lowest address of a stored winding, and the size of one.
    base: usize,
    /// For the winding at `base + k * size_of::<Multi>()`: 0 not stored, 1 false, 2 true.
    known: Vec<u8>,
}

impl SideAnswers {
    /// Asks `pred` of each winding `top` stores on a side of a segment, once, and keeps the answers by
    /// where each is stored when they lie in one run (no answers kept otherwise).
    fn new(top: &Topology<Multi>, pred: &impl Fn(&Multi) -> bool) -> Self {
        let step = std::mem::size_of::<Multi>();
        let mut stored: Vec<(usize, bool)> = Vec::new();
        for s in top.segment_indices() {
            let h = s.first_half();
            for w in [top.winding_clockwise(h), top.winding_counter_clockwise(h)] {
                stored.push((std::ptr::from_ref(w).addr(), pred(w)));
            }
        }
        let base = stored.iter().map(|e| e.0).min().unwrap_or(0);
        let span = stored.iter().map(|e| (e.0 - base) / step + 1).max().unwrap_or(0);
        // Windings in one run, as the sweep keeps them; anything else is answered by the predicate.
        let mut known = Vec::new();
        if span <= stored.len().saturating_mul(4).saturating_add(16) && stored.iter().all(|e| (e.0 - base) % step == 0) {
            known = vec![0u8; span];
            for (addr, answer) in stored {
                if let Some(slot) = known.get_mut((addr - base) / step) {
                    *slot = 1 + u8::from(answer);
                }
            }
        }
        Self { base, known }
    }

    /// The answer kept for the winding `w` by where it is stored, or `pred`'s when none is kept for it.
    fn of(&self, w: &Multi, pred: &impl Fn(&Multi) -> bool) -> bool {
        let offset = std::ptr::from_ref(w).addr().wrapping_sub(self.base);
        let step = std::mem::size_of::<Multi>();
        match offset.is_multiple_of(step).then(|| self.known.get(offset / step).copied()).flatten() {
            Some(1) => false,
            Some(2) => true,
            _ => {
                work::add(Work::AnswerMisses, 1);
                pred(w)
            }
        }
    }
}

impl Arrangement {
    /// The sweep of `paths` together, each under its rule, held to the default limit on pairs of segments.
    pub(crate) fn new(paths: Vec<(BezPath, FillRule)>) -> Result<Self, PathOpsError> {
        Self::new_budgeted(paths, crate::budget::limits().segment_pairs)
    }

    /// The sweep of `paths` together, snapped to its tolerance first: `TooComplex` past the element
    /// limit or past `cap` pairs of segments, `Degenerate` when the sweep panics, `OpenPath` when it
    /// refuses the paths.
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

    /// For each input, whether the winding `w` is inside it under its rule.
    pub(crate) fn mask(&self, w: &Multi) -> Vec<bool> {
        // A zero winding is outside under both rules, so only the non-zero ones can set a flag.
        work::add(Work::MaskFlags, self.rules.len());
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
    ///
    /// `pred` is asked once of each side of each segment before the pass, and the pass is given the
    /// answers: it asks about the same sides again and again (it scans west through the segments that
    /// are no boundary for every contour it starts, twice of each), so asked every time `pred` costs
    /// the scans times what it costs to look at a winding, and a winding of nested shapes is long.
    pub(crate) fn contours_where(&self, pred: impl Fn(&Multi) -> bool) -> Contours {
        #[cfg(test)]
        CONTOUR_PASSES.with(|n| n.set(n.get() + 1));
        let answers = SideAnswers::new(&self.top, &pred);
        self.top.contours(|w| answers.of(w, &pred))
    }

    /// The contour pass with the answer of `pred`, which is given a flag for every input, remembered
    /// for each distinct winding: `pred` is called, and given its flags, at most twice for each
    /// segment of the sweep in all (and not again for a winding it has seen).
    pub(crate) fn contours(&self, pred: impl Fn(&[bool]) -> bool) -> Contours {
        let answers: std::cell::RefCell<std::collections::HashMap<Multi, bool>> = std::cell::RefCell::new(std::collections::HashMap::new());
        self.contours_where(|w| {
            work::add(Work::WindingKeys, w.len().max(1));
            if let Some(&known) = answers.borrow().get(w) {
                return known;
            }
            let answer = pred(&self.mask(w));
            answers.borrow_mut().insert(w.clone(), answer);
            answer
        })
    }

    /// [`Self::contours`] with a panic in the sweep's contour pass turned into an error, as `new` does for the
    /// sweep itself (native builds only: on the web a panic aborts, whatever is caught here). Used by the calls that can
    /// return an error; the others catch a panic as a whole ([`guarded`]).
    pub(crate) fn try_contours(&self, pred: impl Fn(&[bool]) -> bool) -> Result<Contours, PathOpsError> {
        // The predicate is given a flag for every input each time it is asked about a winding, which
        // is once of each distinct one ([`Self::contours`]): two for every segment of the sweep at
        // most, counted here as four.
        let segments = self.top.segment_indices().count();
        if segments.saturating_mul(4).saturating_mul(self.rules.len()) > crate::budget::limits().mask_work {
            return Err(PathOpsError::TooComplex);
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.contours(pred))).map_err(|_| PathOpsError::Degenerate)
    }

    /// All distinct non-empty sets of inputs that cover a face next to some edge, each as the list of
    /// the inputs (ascending, [`Self::covered`]), in the order the flags of every input would sort in
    /// (a set comes before one that has an input where it has none, at the first input they differ
    /// at). Walks the covering inputs of each side of each edge, not a flag for every input: the
    /// work is what the faces are covered by, near the number of segments when shapes lie apart.
    /// `None` when that is more than [`crate::Limits::mask_work`] entries.
    pub(crate) fn distinct_masks(&self) -> Option<Vec<Vec<usize>>> {
        let mut budget = crate::budget::limits().mask_work;
        let mut set: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
        let mut buf: Vec<usize> = Vec::new();
        for s in self.top.segment_indices() {
            let h = s.first_half();
            for w in [self.top.winding_clockwise(h), self.top.winding_counter_clockwise(h)] {
                // Charged before the entries are walked.
                let entries = w.len().saturating_add(1);
                budget = budget.checked_sub(entries)?;
                work::add(Work::MaskEntries, entries);
                buf.clear();
                buf.extend(self.covered(w));
                if !buf.is_empty() && !set.contains(buf.as_slice()) {
                    set.insert(buf.clone());
                }
            }
        }
        let mut masks: Vec<Vec<usize>> = set.into_iter().collect();
        masks.sort_by(|a, b| cmp_masks(a, b));
        Some(masks)
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

/// Appends the contours `c` to `all`, if `all` then holds no more than `budget` elements: the
/// union of many inputs is made of the contours of each, kept until the last is swept, and
/// inputs that cross themselves a lot give many more elements than they have. An error as soon as
/// they pass `budget`, with nothing added, rather than after all of them are made.
pub(crate) fn append_contours(all: &mut BezPath, c: &Contours, budget: usize) -> Result<(), PathOpsError> {
    let added: usize = c.contours().map(|ct| ct.path.elements().len()).sum();
    if all.elements().len().saturating_add(added) > budget {
        return Err(PathOpsError::TooComplex);
    }
    for ct in c.contours() {
        all.extend(ct.path.iter());
    }
    Ok(())
}

pub(crate) fn all_contours_to_path(c: &Contours, tidy: &Tidy) -> BezPath {
    contours_to_path(c, (0..c.contours().count()).map(ContourIdx), tidy)
}

/// Contours whose mean width (2·area / perimeter) is below `precision` are numerical debris
/// (e.g. the lens left between two coincident curves) and are dropped.
pub(crate) fn is_sliver(bp: &BezPath, precision: f64) -> bool {
    let a = bp.area().abs();
    // To a thousandth of the length: the width is compared with the precision, which is not that close,
    // and a bounded measure ([`crate::fit::measured_perimeter`]) cannot be made to subdivide for long.
    // A segment that cannot be measured (coordinates past a billion) is taken at the length of its control polygon.
    let per = crate::fit::measured_perimeter(bp, 1e-3).unwrap_or_else(|| {
        bp.segments()
            .map(|s| match s {
                kurbo::PathSeg::Line(l) => (l.p1 - l.p0).hypot(),
                kurbo::PathSeg::Quad(q) => (q.p1 - q.p0).hypot() + (q.p2 - q.p1).hypot(),
                kurbo::PathSeg::Cubic(c) => (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot(),
            })
            .sum()
    });
    per <= 0.0 || (per.is_finite() && 2.0 * a / per < precision)
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
    // A piece is noise when it is a billionth of the longest chord, but never one the sweep's own
    // tolerance (set by the size of the coordinates) could not have made: a notch of a unit beside
    // an edge of a billion is a notch, not a speck.
    let extent = segs.iter().flat_map(|s| [s.c.p0, s.c.p1, s.c.p2, s.c.p3]).fold(0.0_f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let tiny = (scale * 1e-9).max(1e-9).min((extent * f64::EPSILON * 64.0).max(1e-6));
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

/// How near the ends and the middle of a piece must be to an input curve for the piece to be taken
/// for a part of it, at coordinates as large as `scale - 1`: a millionth of `scale` (which is
/// noise in a drawing's own units) up to coordinates of a thousand, then no more than the noise of
/// the sweep itself, 16 times its tolerance. A tolerance that went on growing with the coordinates
/// was a whole unit at a million, and took the arch beside a thin one for the thin one's own.
fn match_tolerance(scale: f64) -> f64 {
    if scale <= 1001.0 {
        return 1e-6 * scale;
    }
    let sweep = ((scale - 1.0) * f64::EPSILON * 64.0).max(1e-6);
    1e-3_f64.max(16.0 * sweep)
}

/// One segment for the `run` of pieces, within the tidy's precision of each: a line along their chord
/// when they are lines, the part of the input cubic they all lie on, or a cubic fitted to samples of them.
/// `None` when none of these is close enough.
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
    let tol = match_tolerance(scale);
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
                    let tol = match_tolerance(scale);
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
    // The coordinates as given, before the fill leaves out a contour that draws nothing ([`eps_for`]).
    eps_for([path])?;
    let bp = fill_bezpath(path);
    let c = normalize_bez(&bp, rule)?;
    Ok(all_contours_to_path(&c, &Tidy::keeping(DEFAULT_PRECISION, [&bp])))
}

pub(crate) fn normalize_bez(bp: &BezPath, rule: FillRule) -> Result<Contours, PathOpsError> {
    normalize_bez_within(bp, rule, usize::MAX)?.ok_or(PathOpsError::TooComplex)
}

/// [`normalize_bez`] whose contours may have `budget` elements: `Ok(None)`, before any contour is made,
/// when the boundary of the sweep has more segments than that. The contours
/// are made of the boundary segments (the sweep cannot be stopped part way through making them), so a
/// crossing-heavy input is counted before its contours are, against what is left of the element budget
/// of the union it is one input of, and not after.
///
/// A boundary segment is one element or less (pieces of one input curve that follow each other are one
/// curve again), and a contour adds a move and a close: so what is made is at most three times
/// `budget`, and the caller's check of the elements made is exact.
pub(crate) fn normalize_bez_within(bp: &BezPath, rule: FillRule, budget: usize) -> Result<Option<Contours>, PathOpsError> {
    Ok(normalize_bez_counted(bp, rule, budget)?.map(|(contours, _)| contours))
}

/// [`normalize_bez_within`], and whether every segment of the sweep is on the boundary (its two
/// sides differ under the rule). One that is not is a piece of `bp` inside the area it fills, or
/// outside it (two passes that cancel): an edge that bounds nothing.
pub(crate) fn normalize_bez_counted(bp: &BezPath, rule: FillRule, budget: usize) -> Result<Option<(Contours, bool)>, PathOpsError> {
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
    let swept = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Topology::<i32>::from_path(&bp, eps).map(|top| {
            let (mut boundary, mut segments) = (0usize, 0usize);
            for s in top.segment_indices() {
                segments += 1;
                let h = s.first_half();
                boundary += usize::from(inside(rule, *top.winding_clockwise(h)) != inside(rule, *top.winding_counter_clockwise(h)));
            }
            work::add(Work::BoundarySegments, boundary);
            (boundary <= budget).then(|| (top.contours(|w| inside(rule, *w)), boundary == segments))
        })
    }));
    let Some((contours, all_boundary)) = swept.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)? else { return Ok(None) };
    work::add(Work::ContourElements, contours.contours().map(|c| c.path.elements().len()).sum());
    Ok(Some((contours, all_boundary)))
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
/// simple contours (`resolved`) and the operation runs again on those. When that is over a limit
/// the call is [`PathOpsError::TooComplex`], not the first result, which is known to be wrong, and
/// it is an error too when a sweep or a contour pass of the recovery fails (the sweep of an operand
/// resolved on its own, or of the resolved operands together, panics, or a contour pass of the windings
/// check of either does, or the predicate does: [`PathOpsError::Degenerate`]); when it is done but its
/// windings do not add up either (the check has a tolerance, and is off by a little for some curves),
/// the first result is kept.
pub fn try_boolean(a: &BezPath, a_rule: FillRule, b: &BezPath, b_rule: FillRule, op: BoolOp, precision: f64) -> Result<BezPath, PathOpsError> {
    ensure_size([a, b])?;
    // The coordinates as given, before the fill leaves out a contour that draws nothing ([`eps_for`]).
    eps_for([a, b])?;
    checked_boolean(&[(fill_bezpath(a), a_rule), (fill_bezpath(b), b_rule)], &|m| op.apply(m[0], m[1]), sane_precision(precision))
}

/// The operation `pred` over `inputs` (filled paths), its result checked ([`try_boolean`]).
fn checked_boolean(inputs: &[(BezPath, FillRule)], pred: &dyn Fn(&[bool]) -> bool, precision: f64) -> Result<BezPath, PathOpsError> {
    let arr = Arrangement::new(inputs.to_vec())?;
    let paths: Vec<&BezPath> = inputs.iter().map(|(p, _)| p).collect();
    // A check that cannot be made (a contour pass of it panics) does not find this result wrong.
    if !arrangement_adds_up(&arr, &paths, Panic::Unknown).unwrap_or(true)
        && let Some(r) = resolved_boolean(inputs, pred, precision)?
    {
        return Ok(r);
    }
    let c = arr.try_contours(pred)?;
    let tidy = Tidy { precision, ..arr.tidy };
    Ok(all_contours_to_path(&c, &tidy))
}

/// The widest winding (either sign) [`windings_add_up`] checks: one contour pass per level, so a
/// more tangled input is taken as it is.
const MAX_CHECKED_WINDING: i32 = 8;

/// What a panic in a contour pass of [`arrangement_adds_up`] is, beside an input found off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panic {
    /// It cannot tell, as windings too wide to check cannot: an input found off decides, and the passes
    /// made are the same whether the panic is an answer or not (the check of the first arrangement,
    /// which recovers when it is off).
    Unknown,
    /// It is the call's error, whatever another input gives (the check of the recovery, which has no
    /// result to give when a pass of it panics).
    Error,
}

/// Do the windings `arr` gave each input add up to that input's own area ([`windings_add_up`])?
/// Also true when it can't tell for some (and no other is off). A contour pass of the check that
/// panics is [`PathOpsError::Degenerate`]: when no input checked is off ([`Panic::Unknown`]), or
/// whatever the others give ([`Panic::Error`]).
fn arrangement_adds_up(arr: &Arrangement, inputs: &[&BezPath], panic: Panic) -> Result<bool, PathOpsError> {
    // An input that draws nothing has no winding to check and costs no pass, so it is not one of the
    // inputs counted: seven empty paths beside two that are wrong must not switch the check off. Nor
    // does one that is only points (segments of no length), which the sweep is given no segment of.
    // Each keeps its own index, which is the input's tag in the windings.
    let drawn: Vec<(usize, &BezPath)> = inputs.iter().copied().enumerate().filter(|(_, p)| has_length(p)).collect();
    // One check is several contour passes: of the many inputs of a large operation none is checked.
    if drawn.len() > MAX_CHECKED_INPUTS {
        return Ok(true);
    }
    let (mut off, mut failed) = (false, None);
    for &(tag, p) in &drawn {
        match input_adds_up(arr, tag, p) {
            // The first input found off ends the check, unless a panic is the error whatever it finds.
            Ok(Some(false)) if panic == Panic::Unknown => return Ok(false),
            Ok(Some(false)) => off = true,
            Ok(_) => {}
            Err(e) if panic == Panic::Error => return Err(e),
            Err(e) => failed = Some(e),
        }
    }
    match failed {
        Some(e) if !off => Err(e),
        _ => Ok(!off),
    }
}

/// The windings check of the input tagged `tag`, `p`, in `arr` ([`windings_add_up`]); in a test, what
/// the test has the recovery's check answer for it ([`RECHECK_ANSWERS`]).
fn input_adds_up(arr: &Arrangement, tag: usize, p: &BezPath) -> Result<Option<bool>, PathOpsError> {
    #[cfg(test)]
    if CHECKING.with(std::cell::Cell::get) == Some(Step::Recheck)
        && let Some(answer) = RECHECK_ANSWERS.with(|a| a.borrow().iter().find(|(t, _)| *t == tag).map(|(_, r)| *r))
    {
        return answer;
    }
    windings_add_up(&arr.top, |w: &Multi| w.get(tag), p)
}

/// Most inputs that draw something whose windings an operation checks ([`arrangement_adds_up`]).
const MAX_CHECKED_INPUTS: usize = 8;

/// Has `path` a segment of some length (a point, or a cubic all of whose points are one, has none)?
fn has_length(path: &BezPath) -> bool {
    path.segments().any(|s| match s {
        kurbo::PathSeg::Line(l) => l.p0 != l.p1,
        kurbo::PathSeg::Quad(q) => q.p0 != q.p1 || q.p0 != q.p2,
        kurbo::PathSeg::Cubic(c) => c.p0 != c.p1 || c.p0 != c.p2 || c.p0 != c.p3,
    })
}

/// Does the winding `winding` that the sweep `top` gave its faces add up to the signed area of the
/// path `input` it was made from? By Green's theorem the integral of a path's winding number over
/// the plane is the path's signed area, and that integral is Σₖ |{w ≥ k}| − Σₖ |{w ≤ −k}| (k ≥ 1),
/// each area read from the contours of that region. A face given a wrong winding (VectorCraft issue 5478) puts it
/// off by the face's area. `Ok(None)` when it can't tell (windings past [`MAX_CHECKED_WINDING`], or
/// coordinates the sweep does not take), and [`PathOpsError::Degenerate`] when a contour pass of the
/// check panics.
fn windings_add_up<W: WindingNumber>(top: &Topology<W>, winding: impl Fn(&W) -> i32, input: &BezPath) -> Result<Option<bool>, PathOpsError> {
    let (mut lo, mut hi) = (0, 0);
    for s in top.segment_indices() {
        let h = s.first_half();
        for w in [top.winding_clockwise(h), top.winding_counter_clockwise(h)] {
            let k = winding(w);
            (lo, hi) = (lo.min(k), hi.max(k));
        }
    }
    if hi > MAX_CHECKED_WINDING || lo < -MAX_CHECKED_WINDING {
        return Ok(None);
    }
    let region = |pred: &dyn Fn(i32) -> bool| -> Result<f64, PathOpsError> {
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            if let Some(step) = CHECKING.with(std::cell::Cell::get) {
                forced(step);
            }
            top.contours(|w| pred(winding(w)))
        }));
        let c = pass.map_err(|_| PathOpsError::Degenerate)?;
        Ok(c.contours().map(|ct| ct.path.area()).sum::<f64>().abs())
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
    let Ok(eps) = eps_for([input]) else { return Ok(None) };
    let Some(perimeter) = crate::fit::measured_perimeter(input, 1e-3) else { return Ok(None) };
    let tol = 1e-6 * bounds.area().abs() + 16.0 * eps * perimeter * f64::from(hi - lo).max(1.0);
    Ok(Some((integral - want).abs() <= tol || (integral + want).abs() <= tol))
}

/// A step of the recovery ([`resolved_boolean`]) that a test can make panic ([`forced`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The sweep of an operand resolved on its own.
    Sweep,
    /// A contour pass of the windings check of that sweep.
    Check,
    /// Its contour pass.
    Pass,
    /// A contour pass of the windings check of the resolved operands together.
    Recheck,
}

/// What a test has the recovery's windings check answer for the input of a tag ([`RECHECK_ANSWERS`]).
#[cfg(test)]
type Answer = (usize, Result<Option<bool>, PathOpsError>);

#[cfg(test)]
thread_local! {
    /// The step of the recovery made to panic, which no input is known to make any of them do, so
    /// that a test can see what a panic in the recovery gives.
    static FORCE_RESOLVE_PANIC: std::cell::Cell<Option<Step>> = const { std::cell::Cell::new(None) };
    /// The step of the recovery whose windings check is running, while one is.
    static CHECKING: std::cell::Cell<Option<Step>> = const { std::cell::Cell::new(None) };
    /// What the windings check of the resolved operands together answers for each input tag, in place
    /// of the check, so that a test can give one input off and another a panic.
    static RECHECK_ANSWERS: std::cell::RefCell<Vec<Answer>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Panics when a test has made `step` of the recovery panic ([`FORCE_RESOLVE_PANIC`]).
#[cfg(test)]
fn forced(step: Step) {
    if FORCE_RESOLVE_PANIC.with(std::cell::Cell::get) == Some(step) {
        std::panic::resume_unwind(Box::new("forced in a test"));
    }
}

/// `input` resolved on its own under `rule` into simple contours (outer contours and holes, filled
/// non-zero), when the windings of that sweep add up ([`windings_add_up`]): `Ok(None)` when they
/// do not or it cannot tell, and an error when it is over a limit, or when its sweep, a contour pass
/// of its check, or its own contour pass panics ([`PathOpsError::Degenerate`]: the first result was
/// found wrong, and this one is not there to put in its place).
fn resolved(input: &BezPath, rule: FillRule) -> Result<Option<BezPath>, PathOpsError> {
    if !crate::budget::fits_elements([input]) {
        return Err(PathOpsError::TooComplex);
    }
    let Ok(eps) = eps_for([input]) else { return Ok(None) };
    let mut bp = input.clone();
    snap_horizontals(&mut [&mut bp], eps);
    if !crate::budget::sweep_fits([&bp]) {
        return Err(PathOpsError::TooComplex);
    }
    work::add(Work::Sweeps, 1);
    let sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(test)]
        forced(Step::Sweep);
        Topology::<i32>::from_path(&bp, eps)
    }));
    let top = match sweep {
        Ok(Ok(top)) => top,
        Ok(Err(_)) => return Ok(None),
        Err(_) => return Err(PathOpsError::Degenerate),
    };
    #[cfg(test)]
    CHECKING.with(|c| c.set(Some(Step::Check)));
    let adds_up = windings_add_up(&top, |w| *w, &bp);
    #[cfg(test)]
    CHECKING.with(|c| c.set(None));
    if adds_up? != Some(true) {
        return Ok(None);
    }
    let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(test)]
        forced(Step::Pass);
        top.contours(|w| inside(rule, *w))
    }));
    let Ok(c) = pass else { return Err(PathOpsError::Degenerate) };
    let mut out = BezPath::new();
    for ct in c.contours() {
        out.extend(ct.path.iter());
    }
    Ok(Some(out))
}

/// The operation `pred` on `inputs` resolved into simple contours first ([`resolved`]): the result
/// when every sweep's windings add up, `Ok(None)` when they do not, and an error when it is over a
/// limit or when a sweep or contour pass of it panics (of a windings check too).
fn resolved_boolean(inputs: &[(BezPath, FillRule)], pred: &dyn Fn(&[bool]) -> bool, precision: f64) -> Result<Option<BezPath>, PathOpsError> {
    let mut flat: Vec<(BezPath, FillRule)> = Vec::with_capacity(inputs.len());
    for (p, rule) in inputs {
        let Some(r) = resolved(p, *rule)? else { return Ok(None) };
        flat.push((r, FillRule::NonZero));
    }
    // The resolved operands lie within the operands' bounds, which the first sweep took: `Degenerate`
    // here is a panic of the sweep.
    let arr = match Arrangement::new(flat.clone()) {
        Ok(arr) => arr,
        Err(e @ (PathOpsError::TooComplex | PathOpsError::Degenerate)) => return Err(e),
        Err(_) => return Ok(None),
    };
    #[cfg(test)]
    CHECKING.with(|c| c.set(Some(Step::Recheck)));
    let adds_up = arrangement_adds_up(&arr, &flat.iter().map(|(p, _)| p).collect::<Vec<_>>(), Panic::Error);
    #[cfg(test)]
    CHECKING.with(|c| c.set(None));
    if !adds_up? {
        return Ok(None);
    }
    // A contour pass that fails (the sweep panics, or `pred` does) is the call's error: the first
    // arrangement is known to be wrong, and asking `pred` again of that one could answer a predicate
    // that panicked a moment ago with a result that is not the operation's.
    let c = arr.try_contours(pred)?;
    // The refit keeps the operands' own anchors, not only those of their resolved contours.
    let tidy = Tidy::keeping(precision, inputs.iter().map(|(p, _)| p));
    Ok(Some(all_contours_to_path(&c, &tidy)))
}

/// [`boolean_n`] that reports a refused input ([`PathOpsError::TooComplex`], non-finite
/// coordinates) instead of returning an empty path. Like [`try_boolean`] it checks the windings of
/// the sweep against the area of each input (of up to eight that draw something: an empty path, or one
/// of points only, is not counted), and resolves the inputs first when they do not add up, as there. `pred` is asked
/// as [`boolean_n`] says.
pub fn try_boolean_n(paths: &[(&BezPath, FillRule)], pred: impl Fn(&[bool]) -> bool) -> Result<BezPath, PathOpsError> {
    ensure_size(paths.iter().map(|(p, _)| *p))?;
    // The coordinates as given, before the fill leaves out a contour that draws nothing ([`eps_for`]).
    eps_for(paths.iter().map(|(p, _)| *p))?;
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    checked_boolean(&bps, &pred, DEFAULT_PRECISION)
}

/// N-ary boolean: the area where `pred(inside_flags)` holds, `inside_flags[i]` telling whether a
/// point is inside `paths[i]` under its fill rule. A refused input gives an empty path; callers that
/// can report an error use [`try_boolean_n`].
///
/// `pred` must answer by its flags alone. It is asked once for each distinct winding of the sweep's
/// faces, not once for each distinct set of flags: two windings of one input that differ only in
/// how many times it covers a face (shapes nested and wound the same way are windings 1 and 2 under
/// the non-zero rule) give the same flags and are asked about separately, so a `pred` that counts
/// its calls can see more than one for a set of flags.
pub fn boolean_n(paths: &[(&BezPath, FillRule)], pred: impl Fn(&[bool]) -> bool) -> BezPath {
    try_boolean_n(paths, pred).unwrap_or_default()
}

/// [`unite_all`] that reports a refused input instead of leaving it out of the union or returning
/// an empty path.
pub fn try_unite_all(paths: &[(&BezPath, FillRule)]) -> Result<BezPath, PathOpsError> {
    try_unite_all_with(paths, DEFAULT_PRECISION)
}

/// [`try_unite_all`] refit to `precision`, dropping slivers thinner than it.
pub fn try_unite_all_with(paths: &[(&BezPath, FillRule)], precision: f64) -> Result<BezPath, PathOpsError> {
    ensure_size(paths.iter().map(|(p, _)| *p))?;
    // The inputs as given, before the fill leaves out a contour that draws nothing (a handle that is no
    // number makes a loop of one cubic draw nothing, and that is an error, not an empty input).
    eps_for(paths.iter().map(|(p, _)| *p))?;
    let precision = sane_precision(precision);
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        let left = crate::budget::limits().elements.saturating_sub(all.elements().len());
        let Some(c) = normalize_bez_within(&bp, *r, left)? else { return Err(PathOpsError::TooComplex) };
        append_contours(&mut all, &c, crate::budget::limits().elements)?;
    }
    if all.elements().is_empty() {
        return Ok(BezPath::new());
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    held_union(&all, &tidy)
}

/// The union of `all` (the contours of the inputs, each already resolved), as one clean path held to
/// [`crate::Limits::elements`] as each input's sweep is: the boundary of the sweep is counted before its
/// contours are made (a sweep whose boundary has more segments than the limit makes none), and the
/// elements made are checked after; past it the union is [`PathOpsError::TooComplex`]. Inputs that
/// cross each other make more boundary than they hold elements (two sets of 20 parallel strokes,
/// 100 elements each, cross 400 times).
pub(crate) fn held_union(all: &BezPath, tidy: &Tidy) -> Result<BezPath, PathOpsError> {
    let limit = crate::budget::limits().elements;
    let Some(c) = normalize_bez_within(all, FillRule::NonZero, limit)? else { return Err(PathOpsError::TooComplex) };
    if c.contours().map(|ct| ct.path.elements().len()).sum::<usize>() > limit {
        return Err(PathOpsError::TooComplex);
    }
    Ok(all_contours_to_path(&c, tidy))
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
        let left = crate::budget::limits().elements.saturating_sub(all.elements().len());
        let c = match normalize_bez_within(&bp, *r, left) {
            Ok(Some(c)) => c,
            // Past the limit the union would be refused as too complex when swept: it is not made.
            Ok(None) => return BezPath::new(),
            Err(_) => continue,
        };
        // Past the limit the union would be refused as too complex when swept: it is not made.
        if append_contours(&mut all, &c, crate::budget::limits().elements).is_err() {
            return BezPath::new();
        }
    }
    if all.elements().is_empty() {
        return BezPath::new();
    }
    let tidy = Tidy::keeping(precision, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    // Past the limit, or an error of the sweep: no union.
    held_union(&all, &tidy).unwrap_or_default()
}

/// [`area`] that reports a refused input instead of an area of zero.
pub fn try_area(path: &BezPath, rule: FillRule) -> Result<f64, PathOpsError> {
    ensure_size([path])?;
    // The coordinates as given: a loop of one cubic whose handle is no number, or so large that the
    // cross product of the handles overflows, draws nothing and the fill leaves it out, and that is
    // an error ([`PathOpsError::NonFinite`], [`PathOpsError::Degenerate`]), not an area of 0.
    eps_for([path])?;
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

    /// The flags a predicate is given are counted before the pass that sets them, and a pass past
    /// the limit is not made: many inputs with nothing in them (an empty path is an input) made the
    /// work the product of their number and the sweep's, which no other limit counted.
    #[test]
    fn the_flags_of_a_predicate_are_bounded_before_the_pass() {
        let empty = BezPath::new();
        let squares: Vec<BezPath> = (0..20).map(|i| square(f64::from(i) * 20.0, 0.0, 10.0)).collect();
        let inputs_with = |empties: usize| -> Vec<(&BezPath, FillRule)> {
            let mut inputs: Vec<(&BezPath, FillRule)> = squares.iter().map(|s| (s, FillRule::NonZero)).collect();
            inputs.extend(std::iter::repeat_n((&empty, FillRule::NonZero), empties));
            inputs
        };
        let flags = |empties: usize| {
            work::reset();
            let got = try_boolean_n(&inputs_with(empties), |m| m[0]).unwrap();
            assert!((area(&got, FillRule::NonZero) - 100.0).abs() < 1e-9);
            work::take(Work::MaskFlags)
        };
        // The flags grow with the product of the inputs and the segments, as the claim says.
        let (few, many) = (flags(0), flags(1000));
        assert!(many > 40 * few, "{few} flags for 20 inputs, {many} for 1,020");
        // The limit is on that product (four for each segment of the sweep and input): at it the
        // operation is made, one under it is refused before any flag is set.
        let inputs = inputs_with(1000);
        let arr = Arrangement::new(inputs.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect()).unwrap();
        let wanted = 4 * arr.top.segment_indices().count() * inputs.len();
        let mut limits = crate::Limits { mask_work: wanted, ..crate::Limits::default() };
        assert!(crate::with_limits(limits, || try_boolean_n(&inputs, |m| m[0])).is_ok());
        limits.mask_work = wanted - 1;
        work::reset();
        assert_eq!(crate::with_limits(limits, || try_boolean_n(&inputs, |m| m[0])), Err(PathOpsError::TooComplex));
        assert!(crate::with_limits(limits, || boolean_n(&inputs, |m| m[0])).elements().is_empty());
        assert_eq!(work::take(Work::MaskFlags), 0, "no flag was set");
        // Two inputs are far under it.
        assert!(try_boolean(&squares[0], FillRule::NonZero, &squares[1], FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION).is_ok());
    }

    /// One crossing-heavy input of a union is counted before its contours are made, against what is
    /// left of the element budget: a sweep whose boundary has more segments than that makes none (the
    /// sweep cannot be stopped part way through making them, and they were checked only after).
    #[test]
    fn a_sweep_whose_boundary_passes_what_is_left_makes_no_contours() {
        let hatch = crate::testutil::diagonals(20);
        let nz = FillRule::NonZero;
        work::reset();
        let all = normalize_bez(&hatch, nz).unwrap();
        let (boundary, made) = (work::take(Work::BoundarySegments), work::take(Work::ContourElements));
        let made_by_count = all.contours().map(|c| c.path.elements().len()).sum::<usize>();
        assert_eq!(made, made_by_count);
        assert!(boundary > 1_500 && made > 1_500 && hatch.elements().len() < 300, "{boundary} boundary segments, {made} elements");
        assert!(made <= 3 * boundary, "{made} elements for {boundary} boundary segments: a contour is its pieces, a move and a close");
        // At the number of boundary segments the contours are made; one under it, none are.
        work::reset();
        assert!(normalize_bez_within(&hatch, nz, boundary).unwrap().is_some());
        assert_eq!((work::take(Work::BoundarySegments), work::take(Work::ContourElements)), (boundary, made));
        assert!(normalize_bez_within(&hatch, nz, boundary - 1).unwrap().is_none());
        assert_eq!((work::take(Work::BoundarySegments), work::take(Work::ContourElements)), (boundary, 0), "counted, and none made");
        // Through the unions: room for the input and not for the sweep of it, and none are made.
        let few = crate::Limits { elements: boundary - 1, ..crate::Limits::default() };
        assert!(few.elements > hatch.elements().len());
        let small = crate::testutil::rect(0.0, 0.0, 10.0, 10.0);
        work::reset();
        normalize_bez(&small, nz).unwrap();
        let small_made = work::take(Work::ContourElements);
        assert_eq!(crate::with_limits(few, || try_unite_all(&[(&hatch, nz)])), Err(PathOpsError::TooComplex));
        assert!(crate::with_limits(few, || unite_all(&[(&hatch, nz)])).elements().is_empty());
        // An input that fits does not make the union of it with one that does not: that is refused, not left out.
        assert!(crate::with_limits(few, || unite_all(&[(&small, nz), (&hatch, nz)])).elements().is_empty());
        assert_eq!(crate::with_limits(few, || try_unite_all(&[(&small, nz), (&hatch, nz)])), Err(PathOpsError::TooComplex));
        assert_eq!(crate::with_limits(few, || crate::try_stroke_region(&[hatch.clone(), hatch.clone()])), Err(PathOpsError::TooComplex));
        assert!(crate::with_limits(few, || crate::stroke_region(&[hatch.clone(), hatch.clone()])).elements().is_empty());
        // The square that fits is swept, twice (the two unions); the hatch never has a contour made.
        assert_eq!(work::take(Work::ContourElements), 2 * small_made, "no contour of the hatch was made by any of them");
        // With room for the contours, the union is made.
        let roomy = crate::Limits { elements: 3 * boundary, ..crate::Limits::default() };
        assert!(crate::with_limits(roomy, || try_unite_all(&[(&hatch, nz)])).is_ok());
    }

    /// The predicate is asked once for each distinct winding of a side of a segment, and not once for
    /// each distinct set of flags: two nested squares wound the same way are windings 1 and 2 under
    /// the non-zero rule, the same flag for the input, and are asked about separately. So a caller
    /// that counts its calls sees one for each winding, and more than one for a set of flags.
    #[test]
    fn the_predicate_is_asked_once_for_each_winding_and_not_for_each_set_of_flags() {
        let mut nested = crate::testutil::rect(0.0, 0.0, 10.0, 10.0);
        nested.extend(crate::testutil::rect(2.0, 2.0, 8.0, 8.0).iter());
        let arr = Arrangement::new(vec![(nested, FillRule::NonZero)]).unwrap();
        let asked = std::cell::RefCell::new(Vec::<Vec<bool>>::new());
        let c = arr.contours(|m| {
            asked.borrow_mut().push(m.to_vec());
            m[0]
        });
        assert_eq!(c.contours().count(), 1, "the boundary of the area filled is the outer square");
        let asked = asked.into_inner();
        // Outside (winding 0), the ring between the squares (1) and the inner square (2).
        assert_eq!(asked.len(), 3, "{asked:?}");
        let flags: std::collections::HashSet<&Vec<bool>> = asked.iter().collect();
        assert_eq!(flags.len(), 2, "two sets of flags, [false] and [true], the second asked twice: {asked:?}");
        assert_eq!(asked.iter().filter(|m| m[0]).count(), 2);
    }

    /// The reviewer's input for the flags: `west` tall thin rectangles in one input, to the west of
    /// `squares` squares in a column, each its own input. The contour pass starts a contour in the
    /// column and scans west through every rectangle's segments, asking the predicate about both
    /// sides of each, every time: asked of each it was `west` x `squares` x the inputs.
    fn west_scan(west: usize, squares: usize) -> Vec<BezPath> {
        let height = 3.0 * squares as f64;
        let mut tall = BezPath::new();
        for i in 0..west {
            let x = 2.0 * i as f64;
            tall.extend(crate::testutil::rect(x, -1.0, x + 1.0, height + 1.0).iter());
        }
        let mut inputs = vec![tall];
        inputs.extend((0..squares).map(|j| crate::testutil::rect(5_000.0, 3.0 * j as f64, 5_001.0, 3.0 * j as f64 + 1.0)));
        inputs
    }

    fn west_flags(west: usize, squares: usize) -> usize {
        let inputs = west_scan(west, squares);
        let refs: Vec<(&BezPath, FillRule)> = inputs.iter().map(|p| (p, FillRule::NonZero)).collect();
        work::reset();
        let r = try_boolean_n(&refs, |m| m.iter().skip(1).any(|&b| b)).unwrap();
        assert!((area(&r, FillRule::NonZero) - squares as f64).abs() < 1e-6, "{}", area(&r, FillRule::NonZero));
        work::take(Work::MaskFlags)
    }

    /// The predicate is asked once of each distinct winding, so the flags it is given do not grow with
    /// the scans west: with the squares and the rectangles both doubled, they grow four times (the
    /// inputs times the distinct windings, each square's own), where they grew eight times.
    #[test]
    fn the_flags_do_not_grow_with_the_scans_through_segments_that_are_no_boundary() {
        let (small, large) = (west_flags(100, 100), west_flags(200, 200));
        assert!(small > 0 && large <= 4 * small + small / 4, "{small} flags for 100 squares, {large} for 200 (eight times would be {})", 8 * small);
        // The scans alone (more rectangles to the west, the same squares) add no flag at all.
        assert_eq!(west_flags(100, 40), west_flags(400, 40));
        // And the flags are at most two for each segment of the sweep and input, which the limit counts as four.
        let inputs = west_scan(100, 100);
        let arr = Arrangement::new(inputs.iter().map(|p| (p.clone(), FillRule::NonZero)).collect()).unwrap();
        assert!(small <= 2 * arr.top.segment_indices().count() * inputs.len(), "{small} flags");
        // The pass makes the same contours: the answers are remembered, not changed.
        let direct = arr.contours_where(|w| arr.mask(w).iter().skip(1).any(|&b| b));
        let cached = arr.contours(|m| m.iter().skip(1).any(|&b| b));
        assert_eq!(direct.contours().count(), cached.contours().count());
        assert_eq!(
            direct.contours().map(|c| c.path.elements().len()).sum::<usize>(),
            cached.contours().map(|c| c.path.elements().len()).sum::<usize>()
        );
    }

    /// `n` nested squares, then `m` small squares in a column inside them, each its own input.
    fn nested_around_a_column(n: usize, m: usize) -> Vec<BezPath> {
        let r = |k: usize| 2.0 * m as f64 + k as f64;
        let mut inputs: Vec<BezPath> = (1..=n).map(|k| crate::testutil::rect(-r(k), -r(k), r(k), r(k))).collect();
        inputs.extend((0..m).map(|j| {
            let y = -(m as f64) + 2.0 * j as f64;
            crate::testutil::rect(0.0, y, 1.0, y + 1.0)
        }));
        inputs
    }

    /// The contour pass asks for the answer about the same side again and again: it scans west through
    /// the segments that are no boundary for every contour it starts, and a winding of nested shapes is a
    /// long list, so the nested squares round a column of small squares each made it hash every one (n²m
    /// entries). The answers are found by where the windings are stored: what is hashed is each stored
    /// winding once, which grows with the entries (four times for twice the squares and twice the column),
    /// and the pass finds every answer stored.
    #[test]
    fn the_answers_to_the_predicate_are_found_without_hashing_the_windings_again() {
        let keys = |n: usize, m: usize| -> (usize, usize) {
            let inputs = nested_around_a_column(n, m);
            let arr = Arrangement::new(inputs.iter().map(|p| (p.clone(), FillRule::NonZero)).collect()).unwrap();
            work::reset();
            let made = arr.contours(|f| f.iter().skip(n).any(|&b| b));
            assert_eq!(made.contours().count(), m, "{n} nested, {m} in the column");
            (work::take(Work::WindingKeys), work::take(Work::AnswerMisses))
        };
        let ((small, misses), (large, more)) = (keys(20, 20), keys(40, 40));
        assert_eq!((misses, more), (0, 0), "every answer the pass asked for was stored");
        assert!(
            small > 0 && large <= 4 * small + small / 4,
            "{small} entries hashed for 20 and 20, {large} for 40 and 40 (eight times would be {})",
            8 * small
        );
        // The predicate is asked once of each side of each segment, at most (this one, of a distinct winding each).
        let inputs = nested_around_a_column(15, 15);
        let arr = Arrangement::new(inputs.iter().map(|p| (p.clone(), FillRule::NonZero)).collect()).unwrap();
        let calls = std::cell::Cell::new(0usize);
        work::reset();
        let made = arr.contours_where(|w| {
            calls.set(calls.get() + 1);
            w.len() > 15
        });
        assert_eq!(made.contours().count(), 15);
        assert_eq!(calls.get(), 2 * arr.top.segment_indices().count());
        assert_eq!(work::take(Work::AnswerMisses), 0);
    }

    /// The check of the windings against an operand's area is made for curves of large coordinates too,
    /// not skipped as "cannot tell" because their length could not be measured.
    #[test]
    fn the_windings_of_a_large_curved_operand_are_checked() {
        for scale in [1e8, 1e10, 1e12] {
            let ring = kurbo::Circle::new((0.0, 0.0), scale).to_path(scale * 1e-6);
            let eps = eps_for([&ring]).unwrap();
            let top = Topology::<i32>::from_path(&ring, eps).unwrap();
            assert_eq!(windings_add_up(&top, |w| *w, &ring), Ok(Some(true)), "scale {scale}");
        }
    }

    /// An operand whose windings do not add up and whose recovery is over a limit gives no result,
    /// rather than the wrong one: with the pair limit between what the operation and its recovery
    /// need, the call is `TooComplex`, and otherwise the right area or a refusal for size, never a
    /// wrong area.
    #[test]
    fn a_recovery_over_a_limit_is_an_error_and_never_a_wrong_result() {
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
        let (mut right, mut refused, mut window) = (0, 0, Vec::new());
        for pairs in 0..80usize {
            let limits = crate::Limits { segment_pairs: pairs, ..crate::Limits::default() };
            let all = [
                crate::with_limits(limits, || try_boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION)),
                crate::with_limits(limits, || try_boolean_n(&[(&a, FillRule::NonZero), (&b, FillRule::NonZero)], |m| m[0] || m[1])),
            ];
            // The first arrangement alone fits: the operand pair is swept (Arrangement::new succeeds).
            let swept = Arrangement::new_budgeted(vec![(fill_bezpath(&a), FillRule::NonZero), (fill_bezpath(&b), FillRule::NonZero)], pairs).is_ok();
            for r in all {
                match r {
                    Ok(p) => {
                        assert!(
                            (area(&p, FillRule::NonZero) - 4.375).abs() < 1e-6,
                            "a wrong area, {}, with {pairs} pairs",
                            area(&p, FillRule::NonZero)
                        );
                        right += 1;
                    }
                    Err(PathOpsError::TooComplex) => {
                        refused += 1;
                        if swept {
                            window.push(pairs);
                        }
                    }
                    Err(e) => panic!("{e:?}"),
                }
            }
        }
        assert!(right > 0 && refused > 0, "{right} right, {refused} refused");
        assert!(!window.is_empty(), "the pair limits where the sweep fits and its recovery does not are refused, not answered wrongly");
        // The infallible forms give nothing for it.
        let limits = crate::Limits { segment_pairs: window[0], ..crate::Limits::default() };
        assert!(crate::with_limits(limits, || boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union)).elements().is_empty());
    }

    /// A closed star polygon {n/k}: n edges, each crossing many others.
    fn star(n: usize, k: usize, r: f64, cx: f64) -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..n {
            let a = std::f64::consts::TAU * ((i * k) % n) as f64 / n as f64;
            let p = (cx + r * a.cos(), r * a.sin());
            if i == 0 { bp.move_to(p) } else { bp.line_to(p) }
        }
        bp.close_path();
        bp
    }

    /// The contours of crossing-heavy inputs are counted as they are made and the union stops once
    /// they pass the limit: each star is small, and so are its contours alone (filled even-odd it
    /// is a cell for each of its crossings), but the contours of a dozen are not, and were all
    /// made (and kept) before the union was sized.
    #[test]
    fn the_unions_stop_once_the_contours_made_pass_the_limit() {
        let eo = FillRule::EvenOdd;
        let stars: Vec<BezPath> = (0..12).map(|i| star(41, 18, 50.0, f64::from(i) * 200.0)).collect();
        let one = try_normalize(&stars[0], eo).unwrap().elements().len();
        assert!(one > 10 * stars[0].elements().len(), "{one} elements from {}", stars[0].elements().len());
        let inputs: Vec<(&BezPath, FillRule)> = stars.iter().map(|s| (s, eo)).collect();
        // Room for the contours of three stars (and the input of all twelve).
        let mut limits = crate::Limits { elements: 4 * one + 12 * stars[0].elements().len(), ..crate::Limits::default() };
        assert!(crate::with_limits(limits, || stars.iter().all(|s| try_normalize(s, eo).is_ok())));
        work::reset();
        let tried = crate::with_limits(limits, || try_unite_all(&inputs));
        assert_eq!(tried, Err(PathOpsError::TooComplex));
        let sweeps = work::take(Work::Sweeps);
        let made = work::take(Work::ContourElements);
        assert!(sweeps <= 6, "{sweeps} sweeps for twelve stars when four stars' contours fill the limit");
        assert!(made <= 6 * one, "{made} elements of contours made, {one} for each star");
        work::reset();
        let plain = crate::with_limits(limits, || unite_all(&inputs));
        assert!(plain.elements().is_empty());
        assert!(work::take(Work::Sweeps) <= 6);
        // The same inputs with room for all of it are united.
        limits.elements = 40 * one;
        let all = crate::with_limits(limits, || try_unite_all(&inputs)).unwrap();
        assert!(area(&all, FillRule::NonZero) > 0.0);
    }

    #[test]
    fn the_match_tolerance_is_a_millionth_of_the_size_up_to_a_thousand_and_the_sweeps_noise_after() {
        assert_eq!(match_tolerance(1.0), 1e-6);
        assert_eq!(match_tolerance(101.0), 1.01e-4);
        assert_eq!(match_tolerance(1001.0), 1.001e-3);
        assert_eq!(match_tolerance(1_000_001.0), 1e-3);
        assert!(match_tolerance(1e13) > 1e-3 && match_tolerance(1e13) < 5.0, "{}", match_tolerance(1e13));
    }

    /// A thin lens a million units from the origin: the two halves of its second arch are joined
    /// again as that arch, not as the first (which lies a unit away, inside the matching
    /// tolerance that grew with the coordinates).
    #[test]
    fn the_pieces_of_one_arch_are_not_rebuilt_from_the_arch_beside_it() {
        use kurbo::{ParamCurve, ParamCurveNearest};
        let x = 1_000_000.0;
        let upper = CubicBez::new((x, 0.0), (x + 30.0, 30.0), (x + 70.0, 30.0), (x + 100.0, 0.0));
        let lower = CubicBez::new((x + 100.0, 0.0), (x + 70.0, 31.0), (x + 30.0, 31.0), (x, 0.0));
        let mut lens = BezPath::new();
        lens.move_to(upper.p0);
        lens.curve_to(upper.p1, upper.p2, upper.p3);
        lens.curve_to(lower.p1, lower.p2, lower.p3);
        lens.close_path();
        let tidy = Tidy::keeping(DEFAULT_PRECISION, [&lens]);
        let halves = [lower.subsegment(0.0..0.5), lower.subsegment(0.5..1.0)].map(|c| Seg { c, line: false });
        let merged = tidy_segments(halves.to_vec(), false, &tidy);
        assert_eq!(merged.len(), 1);
        let mid = merged[0].c.eval(0.5);
        assert!(
            lower.nearest(mid, 1e-9).distance_sq.sqrt() < 1e-3,
            "the joined arch is {} from its own",
            lower.nearest(mid, 1e-9).distance_sq.sqrt()
        );
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
        assert_eq!(arrangement_adds_up(&arr, &[&a, &b], Panic::Unknown), Ok(false), "the sweep's windings are off");
        let (ra, rb) = (resolved(&a, FillRule::NonZero).unwrap().unwrap(), resolved(&b, FillRule::NonZero).unwrap().unwrap());
        let arr = Arrangement::new(vec![(ra.clone(), FillRule::NonZero), (rb.clone(), FillRule::NonZero)]).unwrap();
        assert_eq!(arrangement_adds_up(&arr, &[&ra, &rb], Panic::Unknown), Ok(true));
        // A plain pair adds up at once.
        let (s, t) = (poly(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]), poly(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]));
        let arr = Arrangement::new(vec![(s.clone(), FillRule::NonZero), (t.clone(), FillRule::EvenOdd)]).unwrap();
        assert_eq!(arrangement_adds_up(&arr, &[&s, &t], Panic::Unknown), Ok(true));
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn a_panic_in_the_contour_pass_becomes_an_error() {
        let arr = Arrangement::new(vec![(fill_bezpath(&square(0.0, 0.0, 10.0)), FillRule::NonZero)]).unwrap();
        assert!(arr.try_contours(|m| m[0]).is_ok());
        assert_eq!(arr.try_contours(|_| panic!("simulated panic in the sweep")).err(), Some(PathOpsError::Degenerate));
    }

    /// A panic in the recovery's resolving of an operand is the call's error, as one in its last contour
    /// pass is: the first result was found wrong, and the recovery that would replace it is not there.
    #[test]
    #[cfg(panic = "unwind")]
    fn a_panic_while_an_operand_is_resolved_is_an_error_and_not_the_first_result() {
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
        let union = || try_boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION);
        let n_ary = || try_boolean_n(&[(&a, FillRule::NonZero), (&b, FillRule::NonZero)], |m| m[0] || m[1]);
        assert!(union().is_ok_and(|p| (p.area().abs() - 4.375).abs() < 1e-6), "resolved, the union is right");
        assert!(n_ary().is_ok_and(|p| (p.area().abs() - 4.375).abs() < 1e-6));
        // The sweep of an operand on its own, a contour pass of its windings check, its contour pass,
        // and a contour pass of the windings check of the resolved operands together.
        for step in [Step::Sweep, Step::Check, Step::Pass, Step::Recheck] {
            FORCE_RESOLVE_PANIC.with(|f| f.set(Some(step)));
            let (got, n) = (union(), n_ary());
            FORCE_RESOLVE_PANIC.with(|f| f.set(None));
            assert_eq!(got.err(), Some(PathOpsError::Degenerate), "{step:?}: not the first result, 5.5");
            assert_eq!(n.err(), Some(PathOpsError::Degenerate), "{step:?}");
        }
        // Operands whose windings add up are not resolved, and a panic there would not be reached.
        let (s, t) = (poly(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]), poly(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]));
        for step in [Step::Sweep, Step::Check, Step::Pass, Step::Recheck] {
            FORCE_RESOLVE_PANIC.with(|f| f.set(Some(step)));
            let plain = try_boolean(&s, FillRule::NonZero, &t, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION);
            FORCE_RESOLVE_PANIC.with(|f| f.set(None));
            assert!(plain.is_ok_and(|p| (p.area().abs() - 28.0).abs() < 1e-9), "{step:?}");
        }
    }

    /// In the windings check of the resolved operands together, a contour pass that panics is the
    /// call's error even where another operand is found off, in either order: the recovery has no
    /// result to give, and the first one was found wrong. With one off and none panicking, the first
    /// result is kept, as the documentation says.
    #[test]
    fn a_panic_in_the_recovery_check_is_an_error_beside_an_operand_found_off() {
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
        let union = || try_boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION);
        let n_ary = || try_boolean_n(&[(&a, FillRule::NonZero), (&b, FillRule::NonZero)], |m| m[0] || m[1]);
        let answering = |answers: Vec<Answer>| {
            RECHECK_ANSWERS.with(|r| *r.borrow_mut() = answers);
            let got = (union(), n_ary());
            RECHECK_ANSWERS.with(|r| r.borrow_mut().clear());
            got
        };
        let (off, panicked) = (Ok(Some(false)), Err(PathOpsError::Degenerate));
        for answers in [vec![(0, off), (1, panicked)], vec![(0, panicked), (1, off)]] {
            let (got, n) = answering(answers.clone());
            assert_eq!(got.err(), Some(PathOpsError::Degenerate), "{answers:?}");
            assert_eq!(n.err(), Some(PathOpsError::Degenerate), "{answers:?}");
        }
        // One off and the other adding up: the first result (5.5 to its refit, which is wrong), as documented.
        let (got, n) = answering(vec![(0, off), (1, Ok(Some(true)))]);
        let areas = (got.map(|p| p.area().abs()), n.map(|p| p.area().abs()));
        assert!(areas.0.is_ok_and(|a| (a - 5.5).abs() < 1e-5) && areas.1.is_ok_and(|a| (a - 5.5).abs() < 1e-5), "{areas:?}");
    }
}
