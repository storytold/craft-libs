//! Offset Path and Outline Stroke.

use craft_geom::{FillRule, PathData, SubPath};
use kurbo::{BezPath, PathEl, Stroke, StrokeOpts};

pub use kurbo::{Cap, Join};

use linesweeper::topology::{ContourIdx, Contours};

use crate::PathOpsError;
use crate::boolean::{Arrangement, DEFAULT_PRECISION, Tidy, all_contours_to_path, contours_to_path, fill_bezpath, normalize_bez};
use crate::edit::remove_redundant_points;

/// Flattening/fit tolerance handed to kurbo's stroker.
const STROKE_TOL: f64 = 1e-3;

/// Most overlapping segment pairs allowed for planar arrangement in offset path: 524,288 (1 << 19).
///
/// Rationale:
/// - The global boolean sweep cap is `MAX_SEGMENT_PAIRS = 1 << 22` (4,194,304 pairs).
/// - An offset operation decomposes paths into outer rings, inner/fill boundaries, and open
///   subpath strokes. When applied to multi-copy transformations (e.g. 25–50 copies in #4145),
///   these parallel strokes create dense self- and cross-intersections where pairwise bounding-box
///   overlaps scale quadratically with the number of segments.
/// - At 1 << 19 (524,288 segment pairs), the planar sweep memory usage is bounded to tens of MBs
///   and completes within ~100 ms on typical CPUs, preventing exponential allocation spikes and
///   application hangs while still generously accommodating legitimate complex artwork.
pub const MAX_OFFSET_SEGMENT_PAIRS: usize = 1 << 19;

/// Drop collinear and coincident spine vertices before stroking (#3446). Uses `tol` only for
/// redundancy; does not simplify or refit the spine.
fn compact_stroke_spine(bp: &BezPath, tol: f64) -> BezPath {
    if bp.elements().is_empty() {
        return BezPath::new();
    }
    let tol = if tol.is_finite() && tol > 0.0 { tol } else { STROKE_TOL };
    remove_redundant_points(&PathData::from_bezpath(bp), tol).to_bezpath()
}

fn stroke_bez(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    let spine = compact_stroke_spine(bp, STROKE_TOL);
    let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter_limit.max(1.0));
    close_all(&kurbo::stroke(spine.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// Like [`stroke_bez`] but without [`compact_stroke_spine`] (regression baseline in unit tests).
#[cfg(test)]
fn stroke_bez_without_spine_compact(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter_limit.max(1.0));
    close_all(&kurbo::stroke(bp.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// kurbo's stroker may leave the outline of an open path without a final `ClosePath`.
fn close_all(bp: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in bp.iter() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(el);
    }
    if open {
        out.close_path();
    }
    out
}

/// Object → Path → Outline Stroke: the filled area painted by stroking `path` with `width`.
/// Overlaps produced by the stroker are removed, so the result is a clean compound path.
pub fn outline_stroke(path: &PathData, width: f64, cap: Cap, join: Join, miter_limit: f64) -> PathData {
    if width <= 0.0 || !width.is_finite() || path.is_empty() {
        return PathData::default();
    }
    stroke_region(&[stroke_bez(&path.to_bezpath(), width, cap, join, miter_limit)])
}

/// The area covered by stroke outlines (each filled with the non-zero rule, open subpaths
/// closed) as one clean path: overlaps removed and the stroker's pieces refitted.
pub fn stroke_region(outlines: &[BezPath]) -> PathData {
    match crate::sweep_axis::turned_outlines(outlines) {
        Some(turned) => crate::sweep_axis::unturn(stroke_region_upright(&turned)),
        None => stroke_region_upright(outlines),
    }
}

/// [`stroke_region`] with the sweep run on `outlines` as they are.
pub(crate) fn stroke_region_upright(outlines: &[BezPath]) -> PathData {
    let tidy = Tidy::free(DEFAULT_PRECISION);
    let parts: Vec<BezPath> = outlines.iter().map(close_all).filter(|o| !o.elements().is_empty()).collect();
    if let [one] = &parts[..] {
        return normalize_bez(one, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default();
    }
    // Each outline is normalised on its own first: a hollow arrowhead may wind against the line
    // it overlaps, and the raw windings would cancel there under the non-zero rule.
    let mut all = BezPath::new();
    for p in &parts {
        if let Ok(c) = normalize_bez(p, FillRule::NonZero) {
            for ct in c.contours() {
                all.extend(ct.path.iter());
            }
        }
    }
    if all.elements().is_empty() {
        return PathData::default();
    }
    normalize_bez(&all, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default()
}

/// Validate that all anchor coordinates and control handles of `path` are finite.
fn path_is_finite(path: &PathData) -> bool {
    path.subpaths.iter().all(|sp| {
        sp.anchors.iter().all(|a| {
            a.p.x.is_finite() && a.p.y.is_finite() && a.h_in.x.is_finite() && a.h_in.y.is_finite() && a.h_out.x.is_finite() && a.h_out.y.is_finite()
        })
    })
}

/// Fallible [`offset_path`] that returns an error when the input is non-finite or exceeds
/// the complexity bound ([`MAX_OFFSET_SEGMENT_PAIRS`]), or when planar arrangement fails.
pub fn try_offset_path(path: &PathData, delta: f64, join: Join, miter_limit: f64) -> Result<PathData, PathOpsError> {
    offset_with_check(path, delta, join, miter_limit, |part| {
        let bp = part.to_bezpath();
        let src = std::cell::LazyCell::new(move || SegmentTree::new(boxed_segments(&bp)));
        move |p, r| any_within(&src, p, r)
    })
}

/// [`try_offset_path`] with `within_of(part)`, the test of whether some segment of `part` lies
/// closer than a radius to a point.
fn offset_with_check<W: Fn(kurbo::Point, f64) -> bool>(
    path: &PathData,
    delta: f64,
    join: Join,
    miter_limit: f64,
    within_of: impl Fn(&PathData) -> W,
) -> Result<PathData, PathOpsError> {
    if !delta.is_finite() || !path_is_finite(path) {
        return Err(PathOpsError::NonFinite);
    }
    // An inset keeps only faces inside the closed fill, so it would drop the outline of every
    // open subpath (#6427). Inset the closed subpaths, outline the open ones as a positive offset
    // does, and unite the two.
    if delta < 0.0 && path.subpaths.iter().any(|s| s.anchors.len() > 1 && !close_if_ends_meet(s).closed) {
        let (closed, open): (Vec<SubPath>, Vec<SubPath>) = path.subpaths.iter().cloned().partition(|s| close_if_ends_meet(s).closed);
        let (closed, open) = (PathData::new(closed), PathData::new(open));
        let inset = offset_region(&closed, delta, join, miter_limit, within_of(&closed))?;
        let outline = offset_region(&open, -delta, join, miter_limit, within_of(&open))?;
        if inset.is_empty() {
            return Ok(outline);
        }
        return crate::boolean::try_unite_all(&[(&inset, FillRule::NonZero), (&outline, FillRule::NonZero)]);
    }
    offset_region(path, delta, join, miter_limit, within_of(path))
}

/// [`offset_with_check`] of a finite `path` with the `within` test of its own segments.
fn offset_region(
    path: &PathData,
    delta: f64,
    join: Join,
    miter_limit: f64,
    within: impl Fn(kurbo::Point, f64) -> bool,
) -> Result<PathData, PathOpsError> {
    if path.is_empty() {
        return Ok(PathData::default());
    }
    let subpaths: Vec<SubPath> = path.subpaths.iter().map(close_if_ends_meet).collect();
    let closed = PathData::new(subpaths.iter().filter(|s| s.closed && s.anchors.len() > 1).cloned().collect());
    let open: Vec<SubPath> = subpaths.into_iter().filter(|s| !s.closed && s.anchors.len() > 1).collect();
    let d = delta.abs();
    let fill = fill_bezpath(&closed);
    let ring = if d > 0.0 { stroke_bez(&closed.to_bezpath(), 2.0 * d, Cap::Butt, join, miter_limit) } else { BezPath::new() };
    let open_bp = if d > 0.0 && !open.is_empty() {
        stroke_bez(&PathData::new(open).to_bezpath(), 2.0 * d, Cap::Butt, join, miter_limit)
    } else {
        BezPath::new()
    };
    let parts = vec![fill, ring, open_bp];
    match crate::sweep_axis::turned_outlines(&parts) {
        Some(turned) => offset_faces(turned, delta, |p, r| within(crate::sweep_axis::unturn_point(p), r)).map(crate::sweep_axis::unturn),
        None => offset_faces(parts, delta, within),
    }
}

/// The faces of the offset by `delta` from `parts` (the closed fill, the ring stroked around it and
/// the stroke of the open subpaths, in that order), less the spurious ones that `within` finds.
fn offset_faces(parts: Vec<BezPath>, delta: f64, within: impl Fn(kurbo::Point, f64) -> bool) -> Result<PathData, PathOpsError> {
    let d = delta.abs();
    let arr = Arrangement::new_budgeted(parts.into_iter().map(|p| (p, FillRule::NonZero)).collect(), MAX_OFFSET_SEGMENT_PAIRS)?;
    let c = if delta >= 0.0 { arr.contours_covering(|c| !c.is_empty()) } else { arr.contours_covering(|c| c.contains(&0) && !c.contains(&1)) };
    // When `d` exceeds a curvature radius the stroker's inner offset inverts and its loops cancel
    // winding, leaving faces uncovered that are really within `d` of the path. Every face of the
    // true offset is bounded by arrangement edges, so a contour is spurious iff a point deep inside
    // it lies closer than `d`: holes when growing, islands when insetting.
    let drop_outer = delta < 0.0;
    let spurious: Vec<bool> =
        c.contours().map(|k| k.outer == drop_outer && deep_point(&k.path).is_some_and(|p| within(p, d * (1.0 - 1e-4) - 1e-6))).collect();
    if !spurious.contains(&true) {
        return Ok(all_contours_to_path(&c, &Tidy::free(DEFAULT_PRECISION)));
    }
    let keep = (0..spurious.len()).filter(|&i| !has_marked_ancestor(&c, i, &spurious)).map(ContourIdx);
    Ok(contours_to_path(&c, keep, &Tidy::free(DEFAULT_PRECISION)))
}

/// Object → Path → Offset Path. Positive `delta` grows the filled area, negative insets it.
/// Closed subpaths are treated as a non-zero filled region; open subpaths are outlined with a
/// stroke of width `2|delta|` (butt caps). An open subpath whose last anchor sits on its first is
/// offset as a closed one.
///
/// If planar arrangement exceeds complexity or encounters a degenerate arrangement error,
/// gracefully preserves `path` unchanged to prevent rendering crashes or disappearing artwork (#4145).
/// Non-finite input returns an empty path.
pub fn offset_path(path: &PathData, delta: f64, join: Join, miter_limit: f64) -> PathData {
    try_offset_path(path, delta, join, miter_limit).unwrap_or_else(|e| match e {
        PathOpsError::NonFinite => PathData::default(),
        _ => path.clone(),
    })
}

/// An open subpath of three or more anchors whose last anchor lies within 1e-6 of its first (the
/// test Object → Path → Join applies to one path) as a closed subpath: the last anchor is merged
/// into the first, which takes its in-handle. Any other subpath is returned unchanged.
fn close_if_ends_meet(sp: &SubPath) -> SubPath {
    let mut sp = sp.clone();
    if !sp.closed
        && sp.anchors.len() > 2
        && sp.anchors.first().zip(sp.anchors.last()).is_some_and(|(f, l)| f.p.distance(l.p) < 1e-6)
        && let Some(last) = sp.anchors.pop()
        && let Some(first) = sp.anchors.first_mut()
    {
        first.h_in = last.h_in;
        sp.closed = true;
    }
    sp
}

fn has_marked_ancestor(c: &Contours, mut i: usize, marked: &[bool]) -> bool {
    loop {
        if marked[i] {
            return true;
        }
        match c[ContourIdx(i)].parent {
            Some(ContourIdx(p)) => i = p,
            None => return false,
        }
    }
}

/// A point well inside a closed contour: the midpoint of the widest span on the horizontal line
/// through its vertical centre.
fn deep_point(bp: &BezPath) -> Option<kurbo::Point> {
    use kurbo::Shape;
    let r = bp.bounding_box();
    if !(r.width() > 0.0 && r.height() > 0.0) {
        return None;
    }
    let mut best: Option<(f64, kurbo::Point)> = None;
    for f in [0.5, 0.3, 0.7] {
        let y = r.y0 + r.height() * f;
        let mut xs = vec![];
        let mut prev: Option<kurbo::Point> = None;
        let mut start = kurbo::Point::ZERO;
        let edge = |a: kurbo::Point, b: kurbo::Point, xs: &mut Vec<f64>| {
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        };
        kurbo::flatten(bp.iter(), 0.01 * r.width().min(r.height()).max(1e-6), |el| match el {
            PathEl::MoveTo(p) => {
                start = p;
                prev = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(a) = prev {
                    edge(a, p, &mut xs);
                }
                prev = Some(p);
            }
            PathEl::ClosePath => {
                if let Some(a) = prev {
                    edge(a, start, &mut xs);
                }
                prev = Some(start);
            }
            _ => {}
        });
        xs.sort_by(f64::total_cmp);
        for w in xs.as_chunks::<2>().0 {
            let span = w[1] - w[0];
            if best.is_none_or(|(b, _)| span > b) {
                best = Some((span, kurbo::Point::new((w[0] + w[1]) / 2.0, y)));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Each segment of `bp` with its bounding box.
fn boxed_segments(bp: &BezPath) -> Vec<(kurbo::Rect, kurbo::PathSeg)> {
    use kurbo::ParamCurveExtrema;
    bp.segments().map(|s| (s.bounding_box(), s)).collect()
}

/// Whether a segment with bounding box `b` could lie within `r` of `p`: the box is inflated by `r`
/// and by slack for rounding in the box and in the curve evaluation, which grows with the coordinates.
fn box_may_reach(b: &kurbo::Rect, p: kurbo::Point, r: f64) -> bool {
    let reach = r + 1e-6 + 1e-9 * p.x.abs().max(p.y.abs());
    p.x >= b.x0 - reach && p.x <= b.x1 + reach && p.y >= b.y0 - reach && p.y <= b.y1 + reach
}

/// Most segments in a leaf of a [`SegmentTree`].
const TREE_LEAF: usize = 8;

/// A path's segments in a tree of bounding boxes, built once and searched for the segments near a point.
struct SegmentTree {
    /// The segments, ordered so that each leaf holds a run of them.
    segs: Vec<(kurbo::Rect, kurbo::PathSeg)>,
    /// The nodes, each one followed by the nodes below it.
    nodes: Vec<TreeNode>,
}

struct TreeNode {
    /// The union of the boxes below this node. A box holding NaN never passes [`box_may_reach`],
    /// and the union leaves its NaN out.
    bounds: kurbo::Rect,
    /// The segments of a leaf; empty for a node with nodes below it.
    leaf: std::ops::Range<usize>,
    /// The first node after this one that is not below it.
    next: usize,
}

impl SegmentTree {
    fn new(mut segs: Vec<(kurbo::Rect, kurbo::PathSeg)>) -> Self {
        let mut nodes = Vec::new();
        Self::split(&mut segs, 0, &mut nodes);
        Self { segs, nodes }
    }

    /// Adds the nodes for `segs`, which start at index `start` of the whole list: past
    /// [`TREE_LEAF`] segments, halves them along the wider side of their bounds.
    fn split(segs: &mut [(kurbo::Rect, kurbo::PathSeg)], start: usize, nodes: &mut Vec<TreeNode>) {
        let empty = kurbo::Rect::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let bounds = segs.iter().fold(empty, |u, (b, _)| u.union(*b));
        let at = nodes.len();
        let leaf = if segs.len() <= TREE_LEAF { start..start + segs.len() } else { start..start };
        nodes.push(TreeNode { bounds, leaf, next: at + 1 });
        if segs.len() > TREE_LEAF {
            let wide = bounds.width() >= bounds.height();
            let center = |b: &kurbo::Rect| if wide { b.x0 + b.x1 } else { b.y0 + b.y1 };
            let mid = segs.len() / 2;
            segs.select_nth_unstable_by(mid, |a, b| center(&a.0).total_cmp(&center(&b.0)));
            let (low, high) = segs.split_at_mut(mid);
            Self::split(low, start, nodes);
            Self::split(high, start + mid, nodes);
            let end = nodes.len();
            if let Some(n) = nodes.get_mut(at) {
                n.next = end;
            }
        }
    }

    /// Whether `test` holds for a segment in a leaf whose bounds [`box_may_reach`] allows. Stops at
    /// the first segment it holds for.
    fn any_near(&self, p: kurbo::Point, r: f64, mut test: impl FnMut(&(kurbo::Rect, kurbo::PathSeg)) -> bool) -> bool {
        let mut i = 0;
        while let Some(n) = self.nodes.get(i) {
            if !box_may_reach(&n.bounds, p, r) {
                i = n.next;
            } else if n.leaf.is_empty() {
                i += 1;
            } else if self.segs.get(n.leaf.clone()).is_some_and(|run| run.iter().any(&mut test)) {
                return true;
            } else {
                i = n.next;
            }
        }
        false
    }
}

/// Whether some segment lies closer than `r` to `p`. A segment whose box is too far away can't, so
/// it is skipped without the costly nearest-point search, and the search stops at the first
/// segment that does.
fn any_within(tree: &SegmentTree, p: kurbo::Point, r: f64) -> bool {
    use kurbo::ParamCurveNearest;
    r > 0.0 && tree.any_near(p, r, |(b, s)| box_may_reach(b, p, r) && s.nearest(p, 1e-6).distance_sq.sqrt() < r)
}

#[cfg(test)]
mod tests {
    use kurbo::Shape;

    use super::*;

    /// The check as it was: the nearest distance over every segment, then compared.
    fn dist_to(bp: &BezPath, p: kurbo::Point) -> f64 {
        use kurbo::ParamCurveNearest;
        bp.segments().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        /// A coordinate on a coarse grid, so segments touch, coincide and nest often.
        fn grid(&mut self, n: f64) -> f64 {
            (self.next() * n).floor()
        }
    }

    #[test]
    fn nearby_segment_check_agrees_with_the_full_scan() {
        let mut rng = Lcg(7);
        let (mut hits, mut skipped) = (0, 0);
        for case in 0..400 {
            let n = [3.0, 6.0, 20.0][case % 3];
            let mut bp = BezPath::new();
            for _ in 0..1 + (rng.next() * 12.0) as usize {
                let pt = |rng: &mut Lcg| kurbo::Point::new(rng.grid(n), rng.grid(n));
                let a = pt(&mut rng);
                bp.move_to(a);
                match (rng.next() * 5.0) as u32 {
                    0 => bp.line_to(a), // zero length
                    1 => bp.line_to(pt(&mut rng)),
                    2 => bp.quad_to(pt(&mut rng), pt(&mut rng)),
                    3 => bp.curve_to(pt(&mut rng), pt(&mut rng), pt(&mut rng)),
                    _ => {
                        // A circle, touching or nested in others.
                        let r = 1.0 + rng.grid(n / 2.0);
                        bp.extend(kurbo::Circle::new(a, r).to_path(0.01));
                    }
                }
                if rng.next() < 0.3 {
                    bp.close_path();
                }
            }
            // The same geometry far from the origin too, where the rounding slack matters.
            let shift = [0.0, 1e6, 1e9][case % 3];
            let bp = kurbo::Affine::translate((shift, -shift)) * bp;
            let segs = boxed_segments(&bp);
            let tree = SegmentTree::new(segs.clone());
            for _ in 0..40 {
                // On the grid (on segments and their ends) and off it.
                let p = if rng.next() < 0.5 {
                    kurbo::Point::new(rng.grid(n), rng.grid(n))
                } else {
                    kurbo::Point::new(rng.next() * (n + 2.0) - 1.0, rng.next() * (n + 2.0) - 1.0)
                };
                let p = kurbo::Point::new(p.x + shift, p.y - shift);
                let nearest = dist_to(&bp, p);
                // Radii around the true distance (where a wrong skip would show), just outside a
                // segment's box, and the usual kind.
                let box_gap =
                    segs.iter().map(|(b, _)| (b.x0 - p.x).max(p.x - b.x1).max(b.y0 - p.y).max(p.y - b.y1).max(0.0)).fold(f64::INFINITY, f64::min);
                for r in [
                    nearest,
                    nearest * (1.0 - 1e-9),
                    nearest * (1.0 + 1e-9),
                    nearest + 1e-6,
                    box_gap + 5e-7,
                    box_gap + 2e-6,
                    0.0,
                    -1.0,
                    1e-7,
                    rng.next() * 3.0,
                    1e6,
                ] {
                    assert_eq!(any_within(&tree, p, r), nearest < r, "case {case} p {p:?} r {r} nearest {nearest}");
                    hits += usize::from(nearest < r);
                    // Every segment the box test skips is really out of reach.
                    for (b, s) in &segs {
                        if r > 0.0 && !box_may_reach(b, p, r) {
                            skipped += 1;
                            use kurbo::ParamCurveNearest;
                            assert!(s.nearest(p, 1e-6).distance_sq.sqrt() >= r, "case {case} skipped a segment within {r} of {p:?}");
                        }
                    }
                }
            }
        }
        assert!(hits > 1000 && skipped > 10_000, "{hits} {skipped}");
        assert!(!any_within(&SegmentTree::new(vec![]), kurbo::Point::ZERO, 1.0));
    }

    /// The test over every segment in path order, with no tree.
    fn any_within_by_scan(segs: &[(kurbo::Rect, kurbo::PathSeg)], p: kurbo::Point, r: f64) -> bool {
        use kurbo::ParamCurveNearest;
        r > 0.0 && segs.iter().any(|(b, s)| box_may_reach(b, p, r) && s.nearest(p, 1e-6).distance_sq.sqrt() < r)
    }

    /// A grid of `n` by `n` unit lines, 10 apart.
    fn line_grid(n: usize) -> Vec<(kurbo::Rect, kurbo::PathSeg)> {
        let mut bp = BezPath::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f64 * 10.0, j as f64 * 10.0);
                bp.move_to((x, y));
                bp.line_to((x + 1.0, y));
            }
        }
        boxed_segments(&bp)
    }

    #[test]
    fn the_tree_tests_only_the_segments_near_the_point() {
        let tree = SegmentTree::new(line_grid(100));
        for (p, r) in [((500.5, 500.0), 0.25), ((503.0, 505.0), 2.0), ((-50.0, -50.0), 1.0)] {
            let p = kurbo::Point::new(p.0, p.1);
            let mut tested = 0;
            tree.any_near(p, r, |_| {
                tested += 1;
                false
            });
            assert!(tested <= 4 * TREE_LEAF, "{tested} of 10000 segments tested at {p:?} within {r}");
        }
        // A radius that takes in the whole grid tests every segment.
        let mut tested = 0;
        tree.any_near(kurbo::Point::new(500.0, 500.0), 1e6, |_| {
            tested += 1;
            false
        });
        assert_eq!(tested, 10_000);
    }

    #[test]
    fn the_tree_holds_every_segment_once_at_any_size() {
        for n in [0, 1, TREE_LEAF - 1, TREE_LEAF, TREE_LEAF + 1, 2 * TREE_LEAF + 1, 100, 1000] {
            let mut bp = BezPath::new();
            for i in 0..n {
                bp.move_to((i as f64, 0.0));
                bp.line_to((i as f64, 1.0));
            }
            let tree = SegmentTree::new(boxed_segments(&bp));
            let mut xs = vec![];
            tree.any_near(kurbo::Point::new(0.0, 0.0), 1e9, |(b, _)| {
                xs.push(b.x0);
                false
            });
            xs.sort_by(f64::total_cmp);
            assert_eq!(xs, (0..n).map(|i| i as f64).collect::<Vec<_>>());
        }
    }

    #[test]
    fn the_tree_answers_like_the_scan_when_boxes_are_not_finite() {
        // Boxes with NaN or infinite sides, as extreme coordinates can give, among ordinary ones.
        let line = |x: f64| kurbo::PathSeg::Line(kurbo::Line::new((x, 0.0), (x + 1.0, 0.0)));
        let mut segs: Vec<(kurbo::Rect, kurbo::PathSeg)> =
            (0..40).map(|i| (line(f64::from(i) * 5.0).bounding_box(), line(f64::from(i) * 5.0))).collect();
        for (k, b) in [
            kurbo::Rect::new(f64::NAN, 0.0, 1.0, 1.0),
            kurbo::Rect::new(0.0, f64::NAN, 1.0, 1.0),
            kurbo::Rect::new(0.0, 0.0, f64::NAN, 1.0),
            kurbo::Rect::new(0.0, 0.0, 1.0, f64::NAN),
            kurbo::Rect::new(f64::NEG_INFINITY, -1.0, f64::INFINITY, 1.0),
            kurbo::Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN),
        ]
        .into_iter()
        .enumerate()
        {
            segs.insert(k * 6, (b, line(1000.0 + k as f64 * 5.0)));
        }
        for every_box_is_nan in [false, true] {
            if every_box_is_nan {
                segs.iter_mut().for_each(|(b, _)| *b = kurbo::Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN));
            }
            let tree = SegmentTree::new(segs.clone());
            for x in [-3.0, 0.5, 7.0, 100.0, 1000.5, 1012.0, 1e6] {
                for r in [0.5, 2.0, 50.0, 1e7] {
                    let p = kurbo::Point::new(x, 0.25);
                    assert_eq!(any_within(&tree, p, r), any_within_by_scan(&segs, p, r), "{every_box_is_nan} {p:?} {r}");
                }
            }
        }
    }

    /// Paths across the kinds of input the offset treats differently.
    fn offset_inputs() -> Vec<PathData> {
        let mut rng = Lcg(31);
        let mut paths = vec![];
        let from = |bp: &BezPath| PathData::from_bezpath(bp);
        // Separate shapes with no overlaps.
        let mut islands = BezPath::new();
        for i in 0..5 {
            for j in 0..5 {
                let (x, y) = (f64::from(i) * 20.0, f64::from(j) * 20.0);
                islands.extend(kurbo::Rect::new(x, y, x + 12.0, y + 12.0).to_path(0.1));
            }
        }
        paths.push(from(&islands));
        // A compound path with holes, some of a radius equal to the offsets below.
        for small in [0.8, 1.0, 2.0] {
            let mut plate = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
            for i in 0..4 {
                for j in 0..4 {
                    let r = if (i + j) % 2 == 0 { small } else { 4.0 };
                    plate.extend(kurbo::Circle::new((12.5 + 25.0 * f64::from(i), 12.5 + 25.0 * f64::from(j)), r).to_path(0.01).reverse_subpaths());
                }
            }
            paths.push(from(&plate));
        }
        // A self-crossing star, closed and open.
        let star = |close: bool| {
            let mut bp = BezPath::new();
            for i in 0..=7 {
                let t = f64::from(i) * 3.0 / 7.0 * std::f64::consts::TAU;
                let p = (50.0 + 40.0 * t.cos(), 50.0 + 40.0 * t.sin());
                if i == 0 { bp.move_to(p) } else { bp.line_to(p) }
            }
            if close {
                bp.close_path();
            }
            bp
        };
        paths.push(from(&star(true)));
        paths.push(from(&star(false)));
        // Short paths: one line, one curve, a single anchor, and a path whose ends meet.
        for svg in ["M0,0 L10,0", "M0,0 C5,10 10,-10 15,0", "M3,3", "M0,0 L10,0 L10,10 L0,0"] {
            paths.push(from(&BezPath::from_svg(svg).unwrap()));
        }
        // An open zigzag whose stroke leaves many small holes.
        let mut zigzag = BezPath::new();
        for row in 0..4 {
            for col in 0..40 {
                let c = if row % 2 == 0 { col } else { 39 - col };
                let p = (f64::from(c) * 1.6, f64::from(row) * 6.0 + if col % 2 == 0 { 0.0 } else { 2.0 });
                if row == 0 && col == 0 { zigzag.move_to(p) } else { zigzag.line_to(p) }
            }
        }
        paths.push(from(&zigzag));
        // Random curves that cross themselves and each other, near the origin and far from it.
        for case in 0..18 {
            let n = [6.0, 40.0][case % 2];
            let mut bp = BezPath::new();
            for _ in 0..1 + case % 4 {
                let pt = |rng: &mut Lcg| kurbo::Point::new(rng.grid(n), rng.grid(n));
                bp.move_to(pt(&mut rng));
                for _ in 0..2 + (rng.next() * 5.0) as usize {
                    match (rng.next() * 3.0) as u32 {
                        0 => bp.line_to(pt(&mut rng)),
                        1 => bp.quad_to(pt(&mut rng), pt(&mut rng)),
                        _ => bp.curve_to(pt(&mut rng), pt(&mut rng), pt(&mut rng)),
                    }
                }
                if rng.next() < 0.6 {
                    bp.close_path();
                }
            }
            let shift = [0.0, 0.0, 1e6][case % 3];
            paths.push(from(&(kurbo::Affine::translate((shift, -shift)) * bp)));
        }
        paths
    }

    #[test]
    fn offsets_with_the_tree_are_identical_to_offsets_with_the_scan() {
        let (mut checks, mut hits, mut misses) = (0, 0, 0);
        for path in offset_inputs() {
            for delta in [0.0, 0.05, 1.0, 5.0, 60.0, -0.05, -1.0, -5.0, -60.0] {
                for join in [Join::Round, Join::Miter, Join::Bevel] {
                    let found = std::cell::Cell::new((0, 0));
                    let by_scan = offset_with_check(&path, delta, join, 4.0, |part| {
                        let segs = boxed_segments(&part.to_bezpath());
                        let found = &found;
                        move |p, r| {
                            let v = any_within_by_scan(&segs, p, r);
                            found.set((found.get().0 + usize::from(v), found.get().1 + usize::from(!v)));
                            v
                        }
                    });
                    checks += 1;
                    hits += found.get().0;
                    misses += found.get().1;
                    assert_eq!(try_offset_path(&path, delta, join, 4.0), by_scan, "{path:?} by {delta} with {join:?}");
                }
            }
        }
        assert!(checks > 500 && hits > 200 && misses > 200, "{checks} {hits} {misses}");
    }

    #[test]
    fn growing_a_plate_with_many_holes_keeps_the_holes_that_stay() {
        // A plate with a grid of round holes: growing by 1 closes the narrow ones (radius <= 1)
        // and keeps the wide ones.
        let mut plate = BezPath::new();
        plate.extend(kurbo::Rect::new(0.0, 0.0, 200.0, 200.0).to_path(0.1));
        for i in 0..10 {
            for j in 0..10 {
                let r = if (i + j) % 2 == 0 { 0.8 } else { 4.0 };
                plate.extend(kurbo::Circle::new((10.0 + 20.0 * f64::from(i), 10.0 + 20.0 * f64::from(j)), r).to_path(0.01).reverse_subpaths());
            }
        }
        let out = offset_path(&PathData::from_bezpath(&plate), 1.0, Join::Round, 4.0);
        // The outer contour and the 50 wide holes; the 50 narrow ones are gone.
        assert_eq!(out.subpaths.len(), 1 + 50);
        // The holes left are the wide ones, shrunk to radius 3 and still centred on their grid point.
        let mut holes = 0;
        for sp in out.subpaths.iter().skip(1) {
            let b = PathData::single(sp.clone()).to_bezpath().bounding_box();
            let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            assert!((b.width() - 6.0).abs() < 0.05 && (b.height() - 6.0).abs() < 0.05, "{b:?}");
            let (i, j) = (((cx - 10.0) / 20.0).round(), ((cy - 10.0) / 20.0).round());
            assert!((cx - 10.0 - 20.0 * i).abs() < 0.05 && (cy - 10.0 - 20.0 * j).abs() < 0.05 && (i + j) % 2.0 != 0.0, "{b:?}");
            holes += 1;
        }
        assert_eq!(holes, 50);
    }

    #[test]
    fn stroke_region_unites_outlines_whatever_their_winding() {
        // A bar, and a square ring wound the other way that the bar runs into.
        let bar = kurbo::Rect::new(0.0, 0.0, 20.0, 4.0).to_path(0.1);
        let mut ring = kurbo::Rect::new(10.0, -8.0, 26.0, 12.0).to_path(0.1).reverse_subpaths();
        ring.extend(kurbo::Rect::new(14.0, -4.0, 22.0, 8.0).to_path(0.1).iter());
        let region = stroke_region(&[bar, ring]);
        let area = crate::area(&region, FillRule::NonZero);
        // Ring 16·20 − 8·12 = 224, plus the bar outside it (10·4) and inside its hole (6·4).
        assert!((area - (224.0 + 40.0 + 24.0)).abs() < 1e-6, "{area}");
    }

    #[test]
    fn miter_offset_finishes_where_the_sweep_used_to_loop() {
        // Issue #1028: with linesweeper 0.4 the sweep over this miter offset never finished and
        // its memory grew without bound, freezing the app. A synthetic outline (found by search)
        // with the same failure as the reported glyph; the exact coordinates matter.
        const OUTLINE: &str = "M74.359137297823,82.83738194552835 L88.097432485874,103.04979803227248 \
            L67.06936731196308,105.95779886823588 L53.29382873396223,98.53943925153727 \
            L29.684713319694094,94.17996020306241 L41.65480824807079,75.639672165115 \
            Q47.491034578751446,52.12555333014565 46.157302824282816,55.67193079127421 \
            Q69.08365151786553,53.9608681393331 68.37499772324333,51.17242466680129 \
            Q74.4136309339628,75.13284223919301 71.3309887459222,73.69128869058812 Z";
        let path = PathData::from_bezpath(&BezPath::from_svg(OUTLINE).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let input = path.clone();
        std::thread::spawn(move || tx.send(offset_path(&input, 1.8350459854022054, Join::Miter, 4.0)));
        let Ok(out) = rx.recv_timeout(std::time::Duration::from_secs(30)) else {
            // the stuck sweep keeps allocating on its thread: end the test binary now
            eprintln!("Offset Path didn't finish in 30 s (issue #1028)");
            std::process::exit(101);
        };
        let (before, after) = (crate::area(&path, FillRule::NonZero), crate::area(&out, FillRule::NonZero));
        assert_eq!(out.subpaths.len(), 1);
        assert!(after > before, "{before} → {after}");
    }

    fn outline_max_deviation(a: &BezPath, b: &BezPath) -> f64 {
        use craft_geom::Point;
        use kurbo::ParamCurve;
        let pa = PathData::from_bezpath(a);
        let pb = PathData::from_bezpath(b);
        let mut worst: f64 = 0.0;
        for (src, dst) in [(a, &pb), (b, &pa)] {
            for seg in src.segments() {
                for i in 0..=12 {
                    let p = seg.eval(i as f64 / 12.0);
                    let pt = Point::new(p.x, p.y);
                    worst = worst.max(dst.nearest(pt).unwrap().4);
                }
            }
        }
        worst
    }

    #[test]
    fn compact_stroke_spine_drops_colinear_points_without_moving_curved_outlines() {
        let width = 10.0;
        let cap = Cap::Round;
        let join = Join::Round;
        let miter = 4.0;
        // Dense colinear spine: compact outline should match the uncompacted one closely.
        let dense: Vec<craft_geom::Point> = (0..=40).map(|i| craft_geom::Point::new(i as f64 * 2.5, 50.0)).collect();
        let dense_path = PathData::single(SubPath::polyline(&dense, false));
        let dense_bez = dense_path.to_bezpath();
        let compact = stroke_bez(&dense_bez, width, cap, join, miter);
        let raw = stroke_bez_without_spine_compact(&dense_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&compact, &raw) <= 1e-3, "colinear compaction must not refit the stroke outline");
        let sparse = PathData::single(SubPath::polyline(&[craft_geom::Point::new(0.0, 50.0), craft_geom::Point::new(100.0, 50.0)], false));
        let sparse_outline = outline_stroke(&sparse, width, cap, join, miter);
        let compact_region = stroke_region(&[compact]);
        assert!(
            outline_max_deviation(&compact_region.to_bezpath(), &sparse_outline.to_bezpath()) <= 1e-3,
            "compacted colinear spine should outline like the endpoints alone"
        );
        // Sampled arc: only redundant vertices are removed; outline must track the raw stroke.
        let arc: Vec<craft_geom::Point> = (0..=32)
            .map(|i| {
                let t = i as f64 / 32.0 * std::f64::consts::PI;
                craft_geom::Point::new(50.0 + 50.0 * t.cos(), 50.0 + 8.0 * t.sin())
            })
            .collect();
        let arc_bez = PathData::single(SubPath::polyline(&arc, false)).to_bezpath();
        let arc_compact = stroke_bez(&arc_bez, width, cap, join, miter);
        let arc_raw = stroke_bez_without_spine_compact(&arc_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&arc_compact, &arc_raw) <= 1e-3, "curved spine compaction must not refit the stroke outline");
    }

    #[test]
    fn offset_path_recovers_gracefully_when_complexity_exceeds_budget() {
        // Issue #4145: dense overlapping geometry must not hang or panic; if complexity passes
        // the offset cap, try_offset_path reports PathOpsError::TooComplex and offset_path
        // preserves the input path unchanged.
        let mut bp = BezPath::new();
        for i in 0..300 {
            let offset = i as f64 * 0.1;
            let rect = kurbo::Rect::new(offset, offset, offset + 100.0, offset + 100.0);
            bp.extend(rect.to_path(0.1).iter());
        }
        let path = PathData::from_bezpath(&bp);
        assert_eq!(try_offset_path(&path, 5.0, Join::Round, 4.0), Err(PathOpsError::TooComplex));
        let res = offset_path(&path, 5.0, Join::Round, 4.0);
        assert_eq!(res, path);
    }

    #[test]
    fn offset_path_returns_empty_on_non_finite() {
        let rect = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = PathData::from_bezpath(&rect.to_path(0.1));
        assert_eq!(try_offset_path(&path, f64::NAN, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert_eq!(try_offset_path(&path, f64::INFINITY, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert!(offset_path(&path, f64::NAN, Join::Round, 4.0).is_empty());
        assert!(offset_path(&path, f64::INFINITY, Join::Round, 4.0).is_empty());

        // Even with dense overlapping geometry, non-finite points return NonFinite error and empty rather than fallback.
        let mut bp = BezPath::new();
        for i in 0..300 {
            let offset = i as f64 * 0.1;
            let rect = kurbo::Rect::new(offset, offset, offset + 100.0, offset + 100.0);
            bp.extend(rect.to_path(0.1).iter());
        }
        let mut dense_non_finite = PathData::from_bezpath(&bp);
        dense_non_finite.subpaths.push(SubPath::polyline(&[craft_geom::Point::new(f64::NAN, 0.0), craft_geom::Point::new(10.0, 10.0)], true));
        assert_eq!(try_offset_path(&dense_non_finite, 5.0, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert!(offset_path(&dense_non_finite, 5.0, Join::Round, 4.0).is_empty());
    }
}
