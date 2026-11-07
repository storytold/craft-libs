// Carved from VectorCraft's crates/pathops (src/offset.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Offset Path and Outline Stroke.

use kurbo::{BezPath, PathEl, Point, Stroke, StrokeCtx, StrokeOpts};

pub use kurbo::{Cap, Join};

use linesweeper::topology::{ContourIdx, Contours};

use crate::boolean::{
    Arrangement, DEFAULT_PRECISION, Tidy, all_contours_to_path, append_contours, contours_to_path, ensure_size, eps_for, normalize_bez,
    try_unite_all_with,
};
use crate::edit::remove_redundant_points;
use crate::path::{Sub, canonical, fill_bezpath, is_finite, subs};
use crate::work::{self, Work};
use crate::{FillRule, PathOpsError};

/// Flattening/fit tolerance handed to kurbo's stroker.
const STROKE_TOL: f64 = 1e-3;

/// Most overlapping segment pairs allowed for planar arrangement in offset path: 524,288 (1 << 19),
/// the default of [`crate::Limits::offset_segment_pairs`].
///
/// Rationale:
/// - The global boolean sweep cap is `MAX_SEGMENT_PAIRS = 1 << 22` (4,194,304 pairs).
/// - An offset operation decomposes paths into outer rings, inner/fill boundaries, and open
///   subpath strokes. When applied to multi-copy transformations (e.g. 25–50 copies, VectorCraft issue 4145),
///   these parallel strokes create dense self- and cross-intersections where pairwise bounding-box
///   overlaps scale quadratically with the number of segments.
/// - At 1 << 19 (524,288 segment pairs), the planar sweep memory usage is bounded to tens of MBs
///   and completes within ~100 ms on typical CPUs, preventing exponential allocation spikes and
///   application hangs while still generously accommodating legitimate complex artwork.
pub const MAX_OFFSET_SEGMENT_PAIRS: usize = 1 << 19;

/// Drop collinear and coincident spine vertices before stroking (VectorCraft issue 3446). Uses `tol` only for
/// redundancy; does not simplify or refit the spine.
fn compact_stroke_spine(bp: &BezPath, tol: f64) -> BezPath {
    if bp.elements().is_empty() {
        return BezPath::new();
    }
    let tol = if tol.is_finite() && tol > 0.0 { tol } else { STROKE_TOL };
    remove_redundant_points(bp, tol)
}

/// Most segments of one subpath that are stroked in one piece. A longer subpath is stroked in
/// windows of this many, each one segment into the one before it, so that the stroker's output can
/// be counted before it grows past the budget (kurbo gives no way to stop it, and it makes up to
/// hundreds of curves for one when the width is huge against the tolerance).
const STROKE_WINDOW: usize = 256;

/// The outline of the stroke of `bp`, as the stroker makes it (overlaps in it are not removed),
/// within [`crate::Limits::elements`] elements of outline in all.
fn stroke_bez(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> Result<BezPath, PathOpsError> {
    stroke_within(bp, width, cap, join, miter_limit, crate::budget::limits().elements)
}

/// [`stroke_bez`] within `budget` elements of outline: [`PathOpsError::TooComplex`] as soon as the
/// outlines made pass it.
///
/// Each subpath is stroked on its own, as the stroker does for a path with several, and one of more
/// than [`STROKE_WINDOW`] segments in windows. The outline of a window is a stroke with butt caps
/// where it meets the next, so the strokes of the windows together cover the stroke of the
/// subpath: every segment is in a window, and so is every join between two of them. A path of
/// short subpaths is stroked as before, in one outline. Otherwise each outline is made clean on its
/// own (a window may wind against the next where they overlap, and the windings would cancel) and
/// they are laid together: under the non-zero rule they cover the stroke, and this leaves the
/// union to whoever sweeps them next ([`try_stroke_region`] for [`try_outline_stroke`], the
/// arrangement for an offset), instead of sweeping and refitting it here as well.
fn stroke_within(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64, budget: usize) -> Result<BezPath, PathOpsError> {
    let mut parts = stroke_parts(bp, width, cap, join, miter_limit, budget)?;
    if parts.len() <= 1 {
        return Ok(parts.pop().unwrap_or_default());
    }
    let mut laid = BezPath::new();
    for part in &parts {
        append_contours(&mut laid, &normalize_bez(part, FillRule::NonZero)?, budget)?;
    }
    Ok(laid)
}

/// The outlines of the stroke of `bp` ([`stroke_outlines`]), closed, the one of the short subpaths
/// first: one outline when no subpath is long, else one more for each window of a long one.
fn stroke_parts(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64, budget: usize) -> Result<Vec<BezPath>, PathOpsError> {
    let (whole, windows) = stroke_outlines(bp, width, cap, join, miter_limit, budget)?;
    let mut parts: Vec<BezPath> = if whole.elements().is_empty() { Vec::new() } else { vec![close_all(&whole)] };
    parts.extend(windows);
    Ok(parts)
}

/// The outlines [`stroke_parts`] makes of the stroke of `bp`: the one outline of all its short
/// subpaths, and one for each window of a long one.
fn stroke_outlines(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64, budget: usize) -> Result<(BezPath, Vec<BezPath>), PathOpsError> {
    let spine = compact_stroke_spine(bp, STROKE_TOL);
    let style =
        |start: Cap, end: Cap| Stroke::new(width).with_join(join).with_start_cap(start).with_end_cap(end).with_miter_limit(miter_limit.max(1.0));
    let (opts, mut ctx, mut used) = (StrokeOpts::default(), StrokeCtx::default(), 0usize);
    let mut stroke = |els: Vec<PathEl>, style: &Stroke, out: &mut BezPath| -> Result<(), PathOpsError> {
        kurbo::stroke_with(els, style, &opts, STROKE_TOL, &mut ctx);
        let made = ctx.output().elements().len();
        work::add(Work::StrokeOutput, made);
        used = used.saturating_add(made);
        if used > budget {
            return Err(PathOpsError::TooComplex);
        }
        out.extend(ctx.output().iter());
        Ok(())
    };
    let (mut whole, mut windows) = (BezPath::new(), Vec::<BezPath>::new());
    for sub in subs(&spine) {
        let mut segs = sub.draws.clone();
        if sub.closed && sub.end() != sub.start {
            segs.push(PathEl::LineTo(sub.start));
        }
        let n = segs.len();
        if n <= STROKE_WINDOW {
            let mut els = vec![PathEl::MoveTo(sub.start)];
            els.extend_from_slice(&sub.draws);
            if sub.closed {
                els.push(PathEl::ClosePath);
            }
            stroke(els, &style(cap, cap), &mut whole)?;
            continue;
        }
        // Where each segment starts.
        let starts: Vec<Point> = std::iter::once(sub.start).chain(segs.iter().filter_map(PathEl::end_point)).collect();
        let mut from = 0;
        loop {
            let to = (from + STROKE_WINDOW).min(n);
            let mut els = vec![PathEl::MoveTo(starts.get(from).copied().unwrap_or(sub.start))];
            els.extend_from_slice(segs.get(from..to).unwrap_or_default());
            if sub.closed && to == n {
                // The join where the subpath closes is in the last window.
                els.extend(segs.first().copied());
            }
            let (start, end) = (if !sub.closed && from == 0 { cap } else { Cap::Butt }, if !sub.closed && to == n { cap } else { Cap::Butt });
            let mut outline = BezPath::new();
            stroke(els, &style(start, end), &mut outline)?;
            windows.push(close_all(&outline));
            if to == n {
                break;
            }
            from = to - 1;
        }
    }
    Ok((whole, windows))
}

/// Like [`stroke_bez`] but without [`compact_stroke_spine`] (regression baseline in unit tests).
#[cfg(test)]
fn stroke_bez_without_spine_compact(bp: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter_limit.max(1.0));
    close_all(&kurbo::stroke(bp.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// kurbo's stroker may leave the outline of an open path without a final `ClosePath`.
fn close_all(bp: &BezPath) -> BezPath {
    work::add(Work::Visited, bp.elements().len());
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

/// Outline Stroke: the filled area painted by stroking `path` with `width`. Overlaps produced by
/// the stroker are removed, so the result is a clean compound path. A width that is not positive
/// and finite, and a path that cannot be stroked ([`try_outline_stroke`] says why), give an empty
/// path.
pub fn outline_stroke(path: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> BezPath {
    try_outline_stroke(path, width, cap, join, miter_limit).unwrap_or_default()
}

/// Fallible [`outline_stroke`]: an error when the path has coordinates that are not finite
/// ([`PathOpsError::NonFinite`]) or too large for the sweep ([`PathOpsError::Degenerate`]), or more
/// segments than [`crate::MAX_ELEMENTS`] ([`PathOpsError::TooComplex`]), or when the stroke's own
/// outline cannot be swept. An empty path or a width that is not positive gives an empty path.
pub fn try_outline_stroke(path: &BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64) -> Result<BezPath, PathOpsError> {
    if path.elements().is_empty() {
        return Ok(BezPath::new());
    }
    ensure_size([path])?;
    if !width.is_finite() || !is_finite(path) {
        return Err(PathOpsError::NonFinite);
    }
    if width <= 0.0 {
        return Ok(BezPath::new());
    }
    eps_for([path])?;
    // The outlines go to one sweep and one refit, whether the path is short or long.
    try_stroke_region(&stroke_parts(path, width, cap, join, miter_limit, crate::budget::limits().elements)?)
}

/// [`stroke_region`] that reports an outline it could not combine (non-finite or too large
/// coordinates, too many segments) instead of leaving it out.
pub fn try_stroke_region(outlines: &[BezPath]) -> Result<BezPath, PathOpsError> {
    // The size first, before any outline is copied.
    ensure_size(outlines)?;
    let tidy = Tidy::free(DEFAULT_PRECISION);
    let parts: Vec<BezPath> = outlines.iter().map(close_all).filter(|o| !o.elements().is_empty()).collect();
    if let [one] = &parts[..] {
        return Ok(all_contours_to_path(&normalize_bez(one, FillRule::NonZero)?, &tidy));
    }
    // Each outline is normalised on its own first, as in `stroke_region`.
    let mut all = BezPath::new();
    for p in &parts {
        append_contours(&mut all, &normalize_bez(p, FillRule::NonZero)?, crate::budget::limits().elements)?;
    }
    if all.elements().is_empty() {
        return Ok(BezPath::new());
    }
    Ok(all_contours_to_path(&normalize_bez(&all, FillRule::NonZero)?, &tidy))
}

/// The area covered by stroke outlines (each filled with the non-zero rule, open subpaths
/// closed) as one clean path: overlaps removed and the stroker's pieces refitted. An outline that
/// cannot be combined is left out ([`try_stroke_region`] reports it).
pub fn stroke_region(outlines: &[BezPath]) -> BezPath {
    if ensure_size(outlines).is_err() {
        return BezPath::new();
    }
    let tidy = Tidy::free(DEFAULT_PRECISION);
    let parts: Vec<BezPath> = outlines.iter().map(close_all).filter(|o| !o.elements().is_empty()).collect();
    if let [one] = &parts[..] {
        return normalize_bez(one, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default();
    }
    // Each outline is normalised on its own first: a hollow arrowhead may wind against the line
    // it overlaps, and the raw windings would cancel there under the non-zero rule.
    let mut all = BezPath::new();
    for p in &parts {
        if let Ok(c) = normalize_bez(p, FillRule::NonZero)
            && append_contours(&mut all, &c, crate::budget::limits().elements).is_err()
        {
            return BezPath::new();
        }
    }
    if all.elements().is_empty() {
        return BezPath::new();
    }
    normalize_bez(&all, FillRule::NonZero).map(|c| all_contours_to_path(&c, &tidy)).unwrap_or_default()
}

/// Fallible [`offset_path`] that returns an error when the input is non-finite or exceeds
/// the complexity bound ([`MAX_OFFSET_SEGMENT_PAIRS`]), or when planar arrangement fails.
pub fn try_offset_path(path: &BezPath, delta: f64, join: Join, miter_limit: f64) -> Result<BezPath, PathOpsError> {
    offset_with_check(path, delta, join, miter_limit, |bp| SegmentTree::new(boxed_segments(bp)), any_within)
}

/// The subpaths of `path` that are filled (closed ones, and open ones whose ends meet) and those that
/// are only outlined, each as a path of its own.
fn split_closed_open(path: &BezPath) -> (BezPath, BezPath) {
    let (mut closed, mut open) = (BezPath::new(), BezPath::new());
    for sub in subs(path).into_iter().map(Sub::closed_if_ends_meet).filter(Sub::draws_something) {
        if sub.closed { sub.push_closed(&mut closed) } else { sub.push_open(&mut open) }
    }
    (closed, open)
}

/// The boundary of the area `closed` fills under the non-zero rule: `closed` itself when every edge
/// of it is on that boundary, else the contours of its sweep. A rectangle inside another one wound
/// the same way is a subpath of the fill but no part of its boundary, and an inset must not stroke it.
fn filled_boundary(closed: &BezPath) -> Result<BezPath, PathOpsError> {
    use kurbo::Shape;
    let mut boundary = BezPath::new();
    for contour in normalize_bez(closed, FillRule::NonZero)?.contours() {
        boundary.extend(contour.path.iter());
    }
    let (kept, whole) = (boundary.perimeter(1e-6), closed.perimeter(1e-6));
    Ok(if (whole - kept).abs() <= 1e-9 * whole { closed.clone() } else { boundary })
}

/// [`try_offset_path`] with the test of whether some segment of a path lies closer than a radius to a
/// point: `index` makes whatever `near` searches of a path's segments. The path is `path` for an
/// outset, and the boundary of the area the closed subpaths fill for an inset (its strokes are made
/// of nothing else, and the open subpaths do not take part: they are outlined whichever way the
/// offset goes).
fn offset_with_check<W>(
    path: &BezPath,
    delta: f64,
    join: Join,
    miter_limit: f64,
    index: impl Fn(&BezPath) -> W,
    near: impl Fn(&W, kurbo::Point, f64) -> bool,
) -> Result<BezPath, PathOpsError> {
    // The size first: it is one length read, and the checks after it read every element.
    ensure_size([path])?;
    if !delta.is_finite() || !is_finite(path) {
        return Err(PathOpsError::NonFinite);
    }
    if path.elements().is_empty() {
        return Ok(BezPath::new());
    }
    let (closed, open) = split_closed_open(path);
    let closed = if delta < 0.0 && !closed.elements().is_empty() { filled_boundary(&closed)? } else { closed };
    let d = delta.abs();
    let fill = fill_bezpath(&closed);
    let ring = if d > 0.0 { stroke_bez(&closed, 2.0 * d, Cap::Butt, join, miter_limit)? } else { BezPath::new() };
    let open_bp = if d > 0.0 && !open.elements().is_empty() { stroke_bez(&open, 2.0 * d, Cap::Butt, join, miter_limit)? } else { BezPath::new() };
    // Kept for an inset, which adds the outlines of the open subpaths to the rest.
    let open_outline = (delta < 0.0).then(|| open_bp.clone());
    let arr = Arrangement::new_budgeted(
        vec![(fill, FillRule::NonZero), (ring, FillRule::NonZero), (open_bp, FillRule::NonZero)],
        crate::budget::limits().offset_segment_pairs,
    )?;
    let c = if delta >= 0.0 { arr.try_contours(|m| m[0] || m[1] || m[2])? } else { arr.try_contours(|m| m[0] && !m[1])? };
    // When `d` exceeds a curvature radius the stroker's inner offset inverts and its loops cancel
    // winding, leaving faces uncovered that are really within `d` of the path. Every face of the
    // true offset is bounded by arrangement edges, so a contour is spurious iff a point deep inside
    // it lies closer than `d`: holes when growing, islands when insetting.
    let drop_outer = delta < 0.0;
    let source = std::cell::LazyCell::new(|| index(&if drop_outer { closed.clone() } else { canonical(path) }));
    let within = |p: kurbo::Point, r: f64| near(&source, p, r);
    let spurious: Vec<bool> =
        c.contours().map(|k| k.outer == drop_outer && deep_point(&k.path).is_some_and(|p| within(p, d * (1.0 - 1e-4) - 1e-6))).collect();
    let result = if spurious.contains(&true) {
        let keep = (0..spurious.len()).filter(|&i| !has_marked_ancestor(&c, i, &spurious)).map(ContourIdx);
        contours_to_path(&c, keep, &Tidy::free(DEFAULT_PRECISION))
    } else {
        all_contours_to_path(&c, &Tidy::free(DEFAULT_PRECISION))
    };
    // An inset takes the strokes of the closed subpaths away, and an open subpath is outlined by its
    // stroke whichever way the offset goes: the outlines are added to what is left.
    if let Some(outline) = open_outline.filter(|o| !o.elements().is_empty()) {
        return try_unite_all_with(&[(&result, FillRule::NonZero), (&outline, FillRule::NonZero)], DEFAULT_PRECISION);
    }
    Ok(result)
}

/// Offset Path. Positive `delta` grows the filled area, negative insets it.
/// Closed subpaths are treated as a non-zero filled region; open subpaths are outlined with a
/// stroke of width `2|delta|` (butt caps). An open subpath whose last anchor sits on its first is
/// offset as a closed one.
///
/// If planar arrangement exceeds complexity or encounters a degenerate arrangement error,
/// gracefully preserves `path` unchanged to prevent rendering crashes or disappearing artwork.
/// Non-finite input returns an empty path.
pub fn offset_path(path: &BezPath, delta: f64, join: Join, miter_limit: f64) -> BezPath {
    try_offset_path(path, delta, join, miter_limit).unwrap_or_else(|e| match e {
        PathOpsError::NonFinite => BezPath::new(),
        _ => path.clone(),
    })
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
        crate::path::flatten(bp.iter(), 0.01 * r.width().min(r.height()).max(1e-6), |el| match el {
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
    fn offset_inputs() -> Vec<BezPath> {
        let mut rng = Lcg(31);
        let mut paths = vec![];
        let from = |bp: &BezPath| bp.clone();
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
                    let by_scan = offset_with_check(&path, delta, join, 4.0, boxed_segments, |segs, p, r| {
                        let v = any_within_by_scan(segs, p, r);
                        found.set((found.get().0 + usize::from(v), found.get().1 + usize::from(!v)));
                        v
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
        let out = offset_path(&plate, 1.0, Join::Round, 4.0);
        // The outer contour and the 50 wide holes; the 50 narrow ones are gone.
        assert_eq!(subs(&out).len(), 1 + 50);
        // The holes left are the wide ones, shrunk to radius 3 and still centred on their grid point.
        let mut holes = 0;
        for sp in subs(&out).iter().skip(1) {
            let b = canonical_one(sp).bounding_box();
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
        // VectorCraft issue 1028: with linesweeper 0.4 the sweep over this miter offset never finished and
        // its memory grew without bound, freezing the app. A synthetic outline (found by search)
        // with the same failure as the reported glyph; the exact coordinates matter.
        const OUTLINE: &str = "M74.359137297823,82.83738194552835 L88.097432485874,103.04979803227248 \
            L67.06936731196308,105.95779886823588 L53.29382873396223,98.53943925153727 \
            L29.684713319694094,94.17996020306241 L41.65480824807079,75.639672165115 \
            Q47.491034578751446,52.12555333014565 46.157302824282816,55.67193079127421 \
            Q69.08365151786553,53.9608681393331 68.37499772324333,51.17242466680129 \
            Q74.4136309339628,75.13284223919301 71.3309887459222,73.69128869058812 Z";
        let path = BezPath::from_svg(OUTLINE).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let input = path.clone();
        std::thread::spawn(move || tx.send(offset_path(&input, 1.8350459854022054, Join::Miter, 4.0)));
        let Ok(out) = rx.recv_timeout(std::time::Duration::from_secs(30)) else {
            // the stuck sweep keeps allocating on its thread: end the test binary now
            eprintln!("Offset Path didn't finish in 30 s (VectorCraft issue 1028)");
            std::process::exit(101);
        };
        let (before, after) = (crate::area(&path, FillRule::NonZero), crate::area(&out, FillRule::NonZero));
        assert_eq!(subs(&out).len(), 1);
        assert!(after > before, "{before} → {after}");
    }

    /// The distance from `p` to the nearest point of `path`'s outline.
    fn distance_to(path: &BezPath, p: kurbo::Point) -> f64 {
        use kurbo::ParamCurveNearest;
        path.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
    }

    fn outline_max_deviation(a: &BezPath, b: &BezPath) -> f64 {
        use kurbo::ParamCurve;
        let mut worst: f64 = 0.0;
        for (src, dst) in [(a, b), (b, a)] {
            for seg in src.segments() {
                for i in 0..=12 {
                    worst = worst.max(distance_to(dst, seg.eval(f64::from(i) / 12.0)));
                }
            }
        }
        worst
    }

    /// A polyline through `points`, closed or open.
    fn polyline(points: &[(f64, f64)], closed: bool) -> BezPath {
        let mut bp = BezPath::new();
        for (i, &p) in points.iter().enumerate() {
            if i == 0 { bp.move_to(p) } else { bp.line_to(p) }
        }
        if closed {
            bp.close_path();
        }
        bp
    }

    /// One subpath as a path of its own.
    fn canonical_one(sub: &Sub) -> BezPath {
        let mut bp = BezPath::new();
        if sub.closed {
            sub.push_closed(&mut bp)
        } else {
            sub.push_open(&mut bp)
        }
        bp
    }

    #[test]
    fn compact_stroke_spine_drops_colinear_points_without_moving_curved_outlines() {
        let width = 10.0;
        let cap = Cap::Round;
        let join = Join::Round;
        let miter = 4.0;
        // Dense colinear spine: compact outline should match the uncompacted one closely.
        let dense: Vec<(f64, f64)> = (0..=40).map(|i| (f64::from(i) * 2.5, 50.0)).collect();
        let dense_bez = polyline(&dense, false);
        let compact = stroke_bez(&dense_bez, width, cap, join, miter).unwrap();
        let raw = stroke_bez_without_spine_compact(&dense_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&compact, &raw) <= 1e-3, "colinear compaction must not refit the stroke outline");
        let sparse = polyline(&[(0.0, 50.0), (100.0, 50.0)], false);
        let sparse_outline = outline_stroke(&sparse, width, cap, join, miter);
        let compact_region = stroke_region(&[compact]);
        assert!(outline_max_deviation(&compact_region, &sparse_outline) <= 1e-3, "compacted colinear spine should outline like the endpoints alone");
        // Sampled arc: only redundant vertices are removed; outline must track the raw stroke.
        let arc: Vec<(f64, f64)> = (0..=32)
            .map(|i| {
                let t = f64::from(i) / 32.0 * std::f64::consts::PI;
                (50.0 + 50.0 * t.cos(), 50.0 + 8.0 * t.sin())
            })
            .collect();
        let arc_bez = polyline(&arc, false);
        let arc_compact = stroke_bez(&arc_bez, width, cap, join, miter).unwrap();
        let arc_raw = stroke_bez_without_spine_compact(&arc_bez, width, cap, join, miter);
        assert!(outline_max_deviation(&arc_compact, &arc_raw) <= 1e-3, "curved spine compaction must not refit the stroke outline");
    }

    /// An open subpath of `n` cubics that bend up and down along a line.
    fn wavy(n: usize) -> BezPath {
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        for i in 0..n {
            let (x, s) = (i as f64 * 10.0, if i % 2 == 0 { 3.0 } else { -3.0 });
            b.curve_to((x + 3.0, s), (x + 6.0, -s), (x + 10.0, 0.0));
        }
        b
    }

    /// A closed subpath of `n` cubics round a ring, bending in and out.
    fn wavy_ring(n: usize) -> BezPath {
        let at = |k: usize, r: f64| {
            let a = k as f64 / n as f64 * std::f64::consts::TAU;
            kurbo::Point::new(300.0 + r * a.cos(), 300.0 + r * a.sin())
        };
        let mut b = BezPath::new();
        b.move_to(at(0, 200.0));
        for k in 0..n {
            let wave = if k % 2 == 0 { 20.0 } else { -20.0 };
            let (mid, next) = (at(k, 200.0 + wave), at(k + 1, 200.0));
            b.curve_to(mid, mid, next);
        }
        b.close_path();
        b
    }

    /// The area the outlines cover together, each made clean on its own first and the lot united,
    /// without any refit.
    fn union_area(outlines: &[BezPath]) -> f64 {
        let mut all = BezPath::new();
        for o in outlines {
            for ct in normalize_bez(o, FillRule::NonZero).unwrap().contours() {
                all.extend(ct.path.iter());
            }
        }
        use kurbo::Shape as _;
        normalize_bez(&all, FillRule::NonZero).unwrap().contours().map(|c| c.path.area()).sum::<f64>().abs()
    }

    /// kurbo's own stroke of the compacted path, whole, as the stroker made it before windows.
    fn whole_stroke(bp: &BezPath, width: f64, cap: Cap, join: Join, miter: f64) -> BezPath {
        let spine = compact_stroke_spine(bp, STROKE_TOL);
        let style = Stroke::new(width).with_join(join).with_caps(cap).with_miter_limit(miter.max(1.0));
        close_all(&kurbo::stroke(spine.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
    }

    /// The stroker's outline is counted as it is made and stopped once it passes the budget: 3,000
    /// cubics stroked 100 million wide make over 200,000 elements of outline, and the stroke is
    /// refused after about the budget's worth, not after all of it.
    #[test]
    fn the_stroke_outline_is_stopped_at_its_budget() {
        let path = wavy(3_000);
        let wide = 1e8;
        let full = kurbo::stroke(path.iter(), &Stroke::new(wide).with_join(Join::Miter), &StrokeOpts::default(), STROKE_TOL).elements().len();
        assert!(full > 150_000, "the whole stroke is {full} elements");
        work::reset();
        assert_eq!(stroke_within(&path, wide, Cap::Butt, Join::Miter, 4.0, 50_000), Err(PathOpsError::TooComplex));
        let made = work::take(Work::StrokeOutput);
        // The window that passed the budget is counted too: 256 cubics a window, up to 100 elements each.
        assert!(made > 50_000 && made < 50_000 + 256 * 100, "{made} elements made for a budget of 50,000");
        assert!(made < full / 2, "{made} of {full}");
        // With room for it, the windows make about the whole stroke, and a little over for the
        // segment each shares with the next.
        assert!(stroke_outlines(&path, wide, Cap::Butt, Join::Miter, 4.0, usize::MAX).is_ok());
        let made = work::take(Work::StrokeOutput);
        assert!(made >= full && made < full + full / 20, "{made} against {full}");
    }

    /// A subpath stroked in windows covers what the stroker covers in one piece, open or closed,
    /// for caps and joins of every kind. (Not to the last digit: where the stroker's inner joins
    /// loop, the one outline and the windows' outlines lose a different sliver, a millionth of the area.)
    #[test]
    fn windows_stroke_what_one_pass_strokes() {
        let (open, ring) = (wavy(600), wavy_ring(520));
        assert!(subs(&open)[0].draws.len() > 2 * STROKE_WINDOW && subs(&ring)[0].draws.len() > 2 * STROKE_WINDOW);
        let styles =
            [(3.0, Cap::Butt, Join::Miter), (3.0, Cap::Round, Join::Round), (20.0, Cap::Square, Join::Miter), (20.0, Cap::Butt, Join::Bevel)];
        for (path, name) in [(&open, "open"), (&ring, "closed")] {
            for (width, cap, join) in styles {
                let whole = union_area(&[whole_stroke(path, width, cap, join, 4.0)]);
                let (small, windows) = stroke_outlines(path, width, cap, join, 4.0, usize::MAX).unwrap();
                assert!(small.elements().is_empty() && windows.len() > 2, "{} windows", windows.len());
                let in_windows = union_area(&windows.iter().map(close_all).collect::<Vec<_>>());
                assert!(
                    (whole - in_windows).abs() <= 1e-5 * whole,
                    "{name} {width} wide, {cap:?} caps, {join:?} joins: {in_windows} against {whole}"
                );
            }
        }
    }

    /// Outlines past the limit together are refused before any is copied.
    #[test]
    fn many_outlines_past_the_limit_are_refused_before_they_are_copied() {
        let outline = crate::testutil::ellipse(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0));
        let n = outline.elements().len();
        let outlines: Vec<BezPath> = (0..50).map(|_| outline.clone()).collect();
        let mut few = crate::Limits { elements: 10 * n, ..crate::Limits::default() };
        work::reset();
        let (tried, plain) = crate::with_limits(few, || (try_stroke_region(&outlines), stroke_region(&outlines)));
        assert_eq!(tried, Err(PathOpsError::TooComplex));
        assert!(plain.elements().is_empty());
        assert_eq!(work::take(Work::Visited), 0, "no outline was read");
        assert_eq!(work::take(Work::Sweeps), 0);
        // Within the limit they are united.
        few.elements = 100 * n;
        assert!(crate::with_limits(few, || try_stroke_region(&outlines)).is_ok());
    }

    /// A long stroke is swept once for each window and once for the union, and refitted once: the
    /// outline for an offset is only laid together, with the union left to the arrangement.
    #[test]
    fn a_long_stroke_is_made_clean_once() {
        let path = wavy(600);
        let (cap, join) = (Cap::Round, Join::Round);
        let (small, windows) = stroke_outlines(&path, 3.0, cap, join, 4.0, usize::MAX).unwrap();
        assert!(small.elements().is_empty() && windows.len() > 2);
        let k = windows.len();
        work::reset();
        let outline = try_outline_stroke(&path, 3.0, cap, join, 4.0).unwrap();
        assert_eq!(work::take(Work::Sweeps), k + 1, "a sweep for each of the {k} windows, one for the union");
        let whole = union_area(&[whole_stroke(&path, 3.0, cap, join, 4.0)]);
        // The refit to the default precision moves the outline by up to that, a thousandth of the area here.
        let area = union_area(&[outline]);
        assert!((area - whole).abs() <= 1e-3 * whole, "{area} against {whole}");
        // For an offset: the windows each made clean and laid together.
        work::reset();
        let laid = stroke_within(&path, 3.0, cap, join, 4.0, usize::MAX).unwrap();
        assert_eq!(work::take(Work::Sweeps), k, "a sweep for each window and no more");
        let area = union_area(&[laid]);
        assert!((area - whole).abs() <= 1e-5 * whole, "{area} against {whole}");
    }

    /// Short subpaths are stroked as the stroker strokes them whole: one outline, the same elements.
    #[test]
    fn short_subpaths_are_stroked_in_one_outline_as_before() {
        let mut path = wavy(100);
        let mut other = BezPath::new();
        other.move_to((0.0, 50.0));
        other.curve_to((20.0, 90.0), (40.0, 10.0), (60.0, 50.0));
        other.close_path();
        path.extend(other.iter());
        let (whole, windows) = stroke_outlines(&path, 4.0, Cap::Round, Join::Round, 4.0, usize::MAX).unwrap();
        assert!(windows.is_empty());
        assert_eq!(close_all(&whole), whole_stroke(&path, 4.0, Cap::Round, Join::Round, 4.0));
    }

    /// 300 squares, each shifted a little from the one before: far more overlapping segment pairs than
    /// the offset allows.
    fn dense_squares() -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..300 {
            let offset = f64::from(i) * 0.1;
            let rect = kurbo::Rect::new(offset, offset, offset + 100.0, offset + 100.0);
            bp.extend(rect.to_path(0.1).iter());
        }
        bp
    }

    #[test]
    fn offset_path_recovers_gracefully_when_complexity_exceeds_budget() {
        // VectorCraft issue 4145: dense overlapping geometry must not hang or panic; if complexity passes
        // the offset cap, try_offset_path reports PathOpsError::TooComplex and offset_path
        // preserves the input path unchanged.
        let path = dense_squares();
        assert_eq!(try_offset_path(&path, 5.0, Join::Round, 4.0), Err(PathOpsError::TooComplex));
        let res = offset_path(&path, 5.0, Join::Round, 4.0);
        assert_eq!(res, path);
    }

    #[test]
    fn offset_path_returns_empty_on_non_finite() {
        let rect = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = rect.to_path(0.1);
        assert_eq!(try_offset_path(&path, f64::NAN, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert_eq!(try_offset_path(&path, f64::INFINITY, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert!(offset_path(&path, f64::NAN, Join::Round, 4.0).elements().is_empty());
        assert!(offset_path(&path, f64::INFINITY, Join::Round, 4.0).elements().is_empty());

        // Even with dense overlapping geometry, non-finite points return NonFinite error and empty rather than fallback.
        let mut dense_non_finite = dense_squares();
        dense_non_finite.extend(polyline(&[(f64::NAN, 0.0), (10.0, 10.0)], true).iter());
        assert_eq!(try_offset_path(&dense_non_finite, 5.0, Join::Round, 4.0), Err(PathOpsError::NonFinite));
        assert!(offset_path(&dense_non_finite, 5.0, Join::Round, 4.0).elements().is_empty());
    }
}
