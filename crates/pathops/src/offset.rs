// Carved from VectorCraft's crates/pathops (src/offset.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Offset Path and Outline Stroke.

use kurbo::{BezPath, PathEl, Point, Stroke, StrokeCtx, StrokeOpts};

pub use kurbo::{Cap, Join};

use linesweeper::topology::{ContourIdx, Contours};

use crate::boolean::{
    Arrangement, DEFAULT_PRECISION, Tidy, all_contours_to_path, append_contours, contours_to_path, ensure_size, eps_for, held_union,
    normalize_bez_counted, normalize_bez_within, try_unite_all_with,
};
use crate::edit::remove_redundant_points;
use crate::fit::measured_perimeter;
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
        let Some(c) = normalize_bez_within(part, FillRule::NonZero, budget.saturating_sub(laid.elements().len()))? else {
            return Err(PathOpsError::TooComplex);
        };
        append_contours(&mut laid, &c, budget)?;
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
    // One outline is held to the element limit as the union of several is ([`held_union`]): one that
    // crosses itself a great deal can make many more elements than it has.
    if let [one] = &parts[..] {
        return held_union(one, &tidy);
    }
    // Each outline is normalised on its own first, as in `stroke_region`.
    let mut all = BezPath::new();
    for p in &parts {
        let left = crate::budget::limits().elements.saturating_sub(all.elements().len());
        let Some(c) = normalize_bez_within(p, FillRule::NonZero, left)? else { return Err(PathOpsError::TooComplex) };
        append_contours(&mut all, &c, crate::budget::limits().elements)?;
    }
    if all.elements().is_empty() {
        return Ok(BezPath::new());
    }
    // Outlines that cross each other make more boundary than they hold: the union is held to the limit too.
    held_union(&all, &tidy)
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
        return held_union(one, &tidy).unwrap_or_default();
    }
    // Each outline is normalised on its own first: a hollow arrowhead may wind against the line
    // it overlaps, and the raw windings would cancel there under the non-zero rule.
    let mut all = BezPath::new();
    for p in &parts {
        let left = crate::budget::limits().elements.saturating_sub(all.elements().len());
        match normalize_bez_within(p, FillRule::NonZero, left) {
            Ok(Some(c)) => {
                if append_contours(&mut all, &c, crate::budget::limits().elements).is_err() {
                    return BezPath::new();
                }
            }
            // Over what is left: the union would be refused as too complex.
            Ok(None) => return BezPath::new(),
            Err(_) => {}
        }
    }
    if all.elements().is_empty() {
        return BezPath::new();
    }
    // Past the element limit, as the union of crossing outlines can be, or an error of the sweep: nothing.
    held_union(&all, &tidy).unwrap_or_default()
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
///
/// Told by the sweep, and checked by the contours and by the lengths of the two. A piece of `closed`
/// that the sweep cuts out of it with the area filled on both sides (a subpath inside another wound
/// the same way, or a small loop of a subpath inside its own fill), or on neither (two passes that
/// cancel), is no boundary: then the boundary is used, however short the piece (a loop a tenth of a
/// unit round and its way there and back, in a square a hundred million across, is within what the
/// lengths can tell apart). A piece the sweep cancels outright is no segment of it at all (a spur a
/// subpath goes out and back along): so every segment of `closed` must also be covered, from end to
/// end, by the segments of the boundary that lie along it, within 16 tolerances of the sweep
/// ([`covered_by`]); a point of it on the boundary is not enough, as the middle of a line out and back
/// can lie on another edge. There are fewer contours
/// than subpaths when a subpath is none of them or two are one (overlapping). (A subpath that crosses
/// itself makes more contours, and may be all boundary.) And the boundary is shorter by the edges that
/// are not on it. The lengths are measured to a
/// relative accuracy, and they agree to what that accuracy and the sweep's tolerance at each vertex
/// allow and no more: a difference that is a fraction of the length of the whole, as a unit square in a
/// square a million across is, tells of edges that are not on the boundary. Only up to
/// [`MAX_COMPARED_CURVES`] curves are measured: past that, or with a segment that cannot be measured,
/// the boundary is used, which is right for any input and differs from `closed` only in how it is cut.
/// The boundary leaves out the pieces of the sweep's contours that go straight out and back along one
/// line to within a rounding of their own coordinates ([`without_spurs`]): a cubic out and back along a
/// line is two passes the sweep keeps a rounding apart, not a retrace it cancels. A slit is not one,
/// however narrow: its sides are its width apart, whatever the sweep's tolerance.
fn filled_boundary(closed: &BezPath) -> Result<BezPath, PathOpsError> {
    let mut boundary = BezPath::new();
    let mut contours = 0usize;
    let (swept, all_boundary) = normalize_bez_counted(closed, FillRule::NonZero, usize::MAX)?.ok_or(PathOpsError::TooComplex)?;
    let eps = eps_for([closed])?;
    for contour in swept.contours() {
        // Without the pieces it goes out and back along: two passes of one curve that the sweep keeps a
        // rounding apart (a cubic out and back along a line, which it does not take for a retrace),
        // whose stroke would cut a notch in the inset. A contour that is nothing else bounds nothing.
        let Some(path) = without_spurs(&contour.path) else { continue };
        boundary.extend(path.iter());
        contours += 1;
    }
    let subpaths = closed.elements().iter().filter(|el| matches!(el, PathEl::MoveTo(_))).count();
    let curves = |p: &BezPath| p.segments().filter(|s| !matches!(s, kurbo::PathSeg::Line(_))).count();
    if !all_boundary || contours < subpaths || curves(closed).saturating_add(curves(&boundary)) > MAX_COMPARED_CURVES {
        return Ok(boundary);
    }
    if !covered_by(closed, &boundary, 16.0 * eps) {
        return Ok(boundary);
    }
    let (Some(kept), Some(whole)) = (measured_perimeter(&boundary, 1e-9), measured_perimeter(closed, 1e-9)) else { return Ok(boundary) };
    // Each length is off by a billionth of the control polygons it sums (4e-9 of the whole allows both,
    // with room), and a vertex of the sweep's output is up to its tolerance from where it was.
    let vertices = closed.elements().len().saturating_add(boundary.elements().len());
    let noise = 4e-9 * whole + 2.0 * eps * vertices as f64;
    Ok(if (whole - kept).abs() <= noise { closed.clone() } else { boundary })
}

/// Is every segment of `path` covered by the segments of `on` that lie along it, to within `tol`? Each
/// segment of `on` whose ends and middle lie within `tol` of a segment of `path` covers that segment
/// between the points nearest its ends; a segment of `path` is covered when those stretches leave no
/// gap of more than twice `tol` from its start to its end. A piece the sweep cancelled (a line out
/// and back, a curve that folds onto itself) has nothing of `on` along it, wherever its middle or its
/// far points lie. The segments of `path` are looked up by an index of their pieces' bounds, at the
/// middle of each segment of `on`.
///
/// The nearest point on a curve is found to a relative accuracy (kurbo's put a point of a piece of a
/// circle of radius 2.5e7, the same curve to a unit in the last place, 7e-5 off it), so each segment
/// allows a ten-billionth of its control polygon besides `tol`.
fn covered_by(path: &BezPath, on: &BezPath, tol: f64) -> bool {
    use crate::planar::index::{RectIndex, segment_footprint};
    use kurbo::{ParamCurve, ParamCurveNearest, PathSeg};
    let segs: Vec<PathSeg> = path.segments().collect();
    let allow: Vec<f64> = segs.iter().map(|&s| tol + 1e-10 * polygon_length(s)).collect();
    let items: Vec<(kurbo::Rect, (usize, PathSeg))> = segs.iter().enumerate().map(|(i, &s)| (kurbo::Shape::bounding_box(&s), (i, s))).collect();
    let index = RectIndex::with_footprint(items, |r, (_, s), cell| segment_footprint(r, s, cell));
    let mut stretches: Vec<Vec<(f64, f64)>> = vec![Vec::new(); segs.len()];
    for b in on.segments() {
        let (start, end, middle) = (b.start(), b.end(), b.eval(0.5));
        for &(i, s) in index.at(middle, tol) {
            let a = allow.get(i).copied().unwrap_or(tol);
            let a_sq = a * a;
            let (ns, ne) = (s.nearest(start, 1e-9), s.nearest(end, 1e-9));
            if ns.distance_sq <= a_sq
                && ne.distance_sq <= a_sq
                && s.nearest(middle, 1e-9).distance_sq <= a_sq
                && let Some(list) = stretches.get_mut(i)
            {
                list.push((ns.t.min(ne.t), ns.t.max(ne.t)));
            }
        }
    }
    segs.iter().zip(allow).zip(stretches).all(|((s, a), mut list)| {
        // A stretch of the segment left uncovered is a gap when the curve over it reaches further than
        // twice the allowance: measured by the bounds of that part of it, not between its ends (a curve
        // out and back ends where it starts).
        let gap = |t0: f64, t1: f64| {
            let b = kurbo::ParamCurveExtrema::bounding_box(&s.subsegment(t0..t1));
            b.width().hypot(b.height()) > 2.0 * a
        };
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        // How far along the segment the stretches so far reach, unbroken from its start.
        let mut reach = 0.0;
        for (t0, t1) in list {
            if t0 > reach && gap(reach, t0) {
                return false;
            }
            reach = f64::max(reach, t1);
        }
        reach >= 1.0 || !gap(reach, 1.0)
    })
}

/// The closed contour `contour` without the pieces it goes straight out and back along: a piece that
/// goes out and back by itself ([`folds`]), and a piece and the next when the next comes back over it
/// ([`retraces`]), as often as that leaves such a pair, round the end of the contour to its start too.
/// `None` when nothing is left. A slit, however narrow, is not one: its sides are its width apart,
/// where the two passes of a retraced curve are a rounding of their coordinates apart.
fn without_spurs(contour: &BezPath) -> Option<BezPath> {
    use kurbo::ParamCurve;
    let mut kept: std::collections::VecDeque<kurbo::PathSeg> = std::collections::VecDeque::new();
    for seg in contour.segments() {
        if folds(seg) {
            continue;
        }
        if kept.back().is_some_and(|&last| retraces(last, seg)) {
            kept.pop_back();
        } else {
            kept.push_back(seg);
        }
    }
    while let (Some(&first), Some(&last)) = (kept.front(), kept.back()) {
        if kept.len() < 2 || !retraces(last, first) {
            break;
        }
        kept.pop_back();
        kept.pop_front();
    }
    let start = kept.front()?.start();
    let mut path = BezPath::new();
    path.move_to(start);
    for seg in &kept {
        path.push(seg.as_path_el());
    }
    path.close_path();
    Some(path)
}

/// The length of the control polygon of the segment `s` (a line's is its length): at least its length.
fn polygon_length(s: kurbo::PathSeg) -> f64 {
    seg_points(s).windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The points of the segment `s`, control points too.
fn seg_points(s: kurbo::PathSeg) -> Vec<Point> {
    match s {
        kurbo::PathSeg::Line(l) => vec![l.p0, l.p1],
        kurbo::PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
        kurbo::PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    }
}

/// A rounding of the coordinates of `points`: 64 units in the last place of the largest of them.
fn rounding_of(points: &[Point]) -> f64 {
    64.0 * f64::EPSILON * points.iter().fold(0.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()))
}

/// Do all of `points` lie within `tol` of the line from `a` to `b`?
fn all_near_line(points: &[Point], a: Point, b: Point, tol: f64) -> bool {
    let chord = b - a;
    let len_sq = chord.hypot2();
    points.iter().all(|&p| {
        let t = if len_sq > 0.0 { ((p - a).dot(chord) / len_sq).clamp(0.0, 1.0) } else { 0.0 };
        (p - (a + chord * t)).hypot() <= tol
    })
}

/// Does the segment `s` go straight out and back by itself: end where it starts, with all its points
/// within a rounding of their coordinates of the line from its start to the point of it furthest away?
/// (Or is it a point.) It then encloses nothing.
fn folds(s: kurbo::PathSeg) -> bool {
    use kurbo::ParamCurve;
    let pts = seg_points(s);
    let rounding = rounding_of(&pts);
    let a = s.start();
    if s.end().distance(a) > rounding {
        return false;
    }
    let far = pts.iter().copied().fold(a, |f, p| if p.distance(a) > f.distance(a) { p } else { f });
    all_near_line(&pts, a, far, rounding)
}

/// Does `back` come straight back over `out`: from where `out` ends to where it starts, both within a
/// rounding of their coordinates ([`rounding_of`]) of the line between those two points, control points
/// too? Each curve then lies along that line from one end to the other, so the two are one curve to
/// within twice that, and enclose nothing between them.
fn retraces(out: kurbo::PathSeg, back: kurbo::PathSeg) -> bool {
    use kurbo::ParamCurve;
    let (a, b) = (out.start(), out.end());
    let all: Vec<Point> = seg_points(out).into_iter().chain(seg_points(back)).collect();
    let rounding = rounding_of(&all);
    let span = a.distance(b);
    if span.is_nan() || span <= rounding {
        return false;
    }
    back.start().distance(b) <= rounding && back.end().distance(a) <= rounding && all_near_line(&all, a, b, rounding)
}

/// Most curves (of the two paths together) whose lengths [`filled_boundary`] measures.
const MAX_COMPARED_CURVES: usize = 4096;

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
    let open_outline = (delta < 0.0 && !open_bp.elements().is_empty()).then(|| open_bp.clone());
    let arr = Arrangement::new_budgeted(
        vec![(fill, FillRule::NonZero), (ring, FillRule::NonZero), (open_bp, FillRule::NonZero)],
        crate::budget::limits().offset_segment_pairs,
    )?;
    let c = if delta >= 0.0 { arr.try_contours(|m| m[0] || m[1] || m[2])? } else { arr.try_contours(|m| m[0] && !m[1])? };
    // When `d` exceeds a curvature radius the stroker's inner offset inverts and its loops cancel
    // winding, leaving faces uncovered that are really within `d` of the path. Every face of the
    // true offset is bounded by arrangement edges, so a contour is spurious iff a point deep inside
    // its face (the contours directly inside it left out) lies closer than `d`: holes when growing,
    // islands when insetting.
    let drop_outer = delta < 0.0;
    let source = std::cell::LazyCell::new(|| index(&if drop_outer { closed.clone() } else { canonical(path) }));
    let result = without_spurious(&c, drop_outer, d, |p, r| near(&source, p, r));
    // An inset takes the strokes of the closed subpaths away, and an open subpath is outlined by its
    // stroke whichever way the offset goes: the outlines are added to what is left. They are cleaned
    // of the stroker's holes first, as the arrangement of a growing offset cleans them, or an open
    // curve that turns inside the radius would leave a hole in an inset and not in an outset.
    if let Some(raw) = open_outline {
        let near_open = std::cell::LazyCell::new(|| index(&open));
        let alone = Arrangement::new_budgeted(vec![(raw, FillRule::NonZero)], crate::budget::limits().offset_segment_pairs)?;
        let outline = without_spurious(&alone.try_contours(|m| m[0])?, false, d, |p, r| near(&near_open, p, r));
        if !outline.elements().is_empty() {
            return try_unite_all_with(&[(&result, FillRule::NonZero), (&outline, FillRule::NonZero)], DEFAULT_PRECISION);
        }
    }
    Ok(result)
}

/// The contours `c` of a stroke's arrangement, without those that are spurious: an outer contour
/// (`drop_outer`) or a hole (not) with a point deep inside its face that `within` finds closer than `d`
/// to the path that was stroked, and everything inside such a contour.
///
/// The face is the contour with the contours directly inside it left out (the holes of an outer
/// contour, the islands of a hole): a point inside the contour alone can lie in one of them, by an
/// edge of the path, and a face that is right was dropped (a square with a hole whose edge runs
/// through the square's middle had an inset of nothing).
fn without_spurious(c: &Contours, drop_outer: bool, d: f64, within: impl Fn(kurbo::Point, f64) -> bool) -> BezPath {
    let mut inside: Vec<Vec<usize>> = vec![Vec::new(); c.contours().count()];
    for (i, k) in c.contours().enumerate() {
        if let Some(list) = k.parent.and_then(|ContourIdx(p)| inside.get_mut(p)) {
            list.push(i);
        }
    }
    let face = |i: usize, k: &linesweeper::topology::Contour| {
        let mut face = k.path.clone();
        for &j in inside.get(i).into_iter().flatten() {
            face.extend(c[ContourIdx(j)].path.iter());
        }
        face
    };
    let spurious: Vec<bool> = c
        .contours()
        .enumerate()
        .map(|(i, k)| k.outer == drop_outer && deep_point(&face(i, k)).is_some_and(|p| within(p, d * (1.0 - 1e-4) - 1e-6)))
        .collect();
    if spurious.contains(&true) {
        let keep = (0..spurious.len()).filter(|&i| !has_marked_ancestor(c, i, &spurious)).map(ContourIdx);
        contours_to_path(c, keep, &Tidy::free(DEFAULT_PRECISION))
    } else {
        all_contours_to_path(c, &Tidy::free(DEFAULT_PRECISION))
    }
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

/// Is the contour `i`, or a contour it lies in, marked?
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

/// A point well inside the area `bp` bounds by the even-odd rule (a contour, or a face: a contour with
/// the contours directly inside it): the midpoint of the widest span inside it on one of three
/// horizontal lines, through its vertical centre and three tenths above and below.
///
/// The spans are found on `bp` flattened to a hundredth of its size, which can put the middle of a
/// span outside the area where the area is narrower than that (a ring a tenth wide round a hole a
/// hundred across): so a middle is taken only where the winding of `bp` itself is odd, the widest of
/// the first few spans that is. `None` when none is (and the contour is then not taken for spurious).
fn deep_point(bp: &BezPath) -> Option<kurbo::Point> {
    use kurbo::Shape;
    let r = bp.bounding_box();
    if !(r.width() > 0.0 && r.height() > 0.0) {
        return None;
    }
    let mut spans = spans_across(bp, r, (0.01 * r.width().min(r.height())).max(1e-6));
    // The widest first, the first found of equal ones first.
    spans.sort_by(|a, b| b.0.total_cmp(&a.0));
    spans.into_iter().take(MAX_DEEP_TESTS).map(|(_, p)| p).find(|&p| bp.winding(p) % 2 != 0)
}

/// Most middles of spans [`deep_point`] tests against the winding of the path.
const MAX_DEEP_TESTS: usize = 8;

/// The spans inside `bp` (even-odd) on the horizontal lines through the middle of its bounds `r` and
/// three tenths above and below, on `bp` flattened to `tolerance`: each span's width and middle, in
/// the order of the lines and from left to right.
fn spans_across(bp: &BezPath, r: kurbo::Rect, tolerance: f64) -> Vec<(f64, kurbo::Point)> {
    let mut spans = Vec::new();
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
        crate::path::flatten(bp.iter(), tolerance, |el| match el {
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
            spans.push((w[1] - w[0], kurbo::Point::new((w[0] + w[1]) / 2.0, y)));
        }
    }
    spans
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
    use crate::boolean::normalize_bez;

    /// A contour that starts at the tip of a spur has the spur's two passes at its two ends, the way
    /// out last and the way back first: they are taken out round the end, as often as that leaves such
    /// a pair (a spur of two pieces each way, slanted), and what is left is the contour without it.
    #[test]
    fn a_spur_at_the_start_of_a_contour_is_taken_out_round_its_end() {
        let (o, mid, tip) = (Point::new(50.0, 0.0), Point::new(50.1, 0.1), Point::new(50.3, 0.3));
        let mut contour = BezPath::new();
        contour.move_to(tip);
        for p in [mid, o, Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0), Point::new(0.0, 0.0), o, mid, tip] {
            contour.line_to(p);
        }
        contour.close_path();
        let mut square = BezPath::new();
        square.move_to(o);
        for p in [Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0), Point::new(0.0, 0.0), o] {
            square.line_to(p);
        }
        square.close_path();
        assert_eq!(without_spurs(&contour), Some(square));
        // A contour that is nothing but a spur, starting at its tip, is nothing.
        let mut spur = BezPath::new();
        spur.move_to(tip);
        for p in [mid, o, mid, tip] {
            spur.line_to(p);
        }
        spur.close_path();
        assert_eq!(without_spurs(&spur), None);
    }

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

    /// One subpath that goes back and forth across a square, first along it and then up and down it:
    /// 2m passes joined at their ends, more than a window of segments for 70, and a stroke whose
    /// windows each cross themselves in a lattice.
    fn back_and_forth(m: usize) -> BezPath {
        let side = 10.0 * m as f64;
        let mut pts = Vec::new();
        for i in 0..m {
            let y = 10.0 * i as f64;
            let (a, b) = if i % 2 == 0 { (0.0, side) } else { (side, 0.0) };
            pts.extend([(a, y), (b, y)]);
        }
        for i in 0..m {
            let x = 10.0 * i as f64 + 5.0;
            let (a, b) = if i % 2 == 0 { (0.0, side) } else { (side, 0.0) };
            pts.extend([(x, a), (x, b)]);
        }
        polyline(&pts, false)
    }

    /// The contours of a window of a stroke are counted before they are made, against the elements left
    /// of the budget of the stroke: windows that cross themselves in a lattice make far more boundary
    /// than outline, and a budget that fits the outline refuses them before any contour is made.
    #[test]
    fn the_windows_of_a_stroke_are_counted_before_their_contours_are_made() {
        let path = back_and_forth(70);
        assert!(subs(&path)[0].draws.len() > STROKE_WINDOW + 10);
        work::reset();
        assert!(stroke_outlines(&path, 1.0, Cap::Butt, Join::Miter, 4.0, usize::MAX).is_ok());
        let outline = work::take(Work::StrokeOutput);
        // Room for the outline of the stroke, which is made, but not for the lattice the sweep cuts it into.
        work::reset();
        assert_eq!(stroke_within(&path, 1.0, Cap::Butt, Join::Miter, 4.0, outline), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::StrokeOutput), outline, "the whole outline was made");
        assert_eq!(work::take(Work::ContourElements), 0, "no contour was");
        assert!(work::take(Work::BoundarySegments) > 2 * outline, "the boundary of the first window alone is far more than the outline");
        // With room for them they are made.
        assert!(stroke_within(&path, 1.0, Cap::Butt, Join::Miter, 4.0, 100 * outline).is_ok());
        assert!(work::take(Work::ContourElements) > outline);
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

    /// `n` circles of radius 5 apart, as one path of 4n cubics.
    fn circles(n: usize) -> BezPath {
        let mut p = BezPath::new();
        for i in 0..n {
            p.extend(kurbo::Circle::new((f64::from(i as u32) * 20.0, 0.0), 5.0).to_path(0.01).iter());
        }
        p
    }

    /// The boundary is the art when every edge is on it, at any size, and is the sweep's contours when
    /// some edge is not however small beside the rest: a unit square inside one a million across (its
    /// 4 units of edge are a millionth of the perimeter), a square of a ten-thousandth inside a
    /// thousand-unit one, one in a billion-unit square, and shapes that cross or overlap (whose edges
    /// inside each other are no boundary).
    #[test]
    fn the_boundary_of_ordinary_art_is_the_art_and_an_edge_inside_it_is_not() {
        use crate::testutil::{ellipse, rect, subpath_count};
        let scaled = |p: &BezPath, k: f64| {
            let mut q = p.clone();
            q.apply_affine(kurbo::Affine::scale(k));
            q
        };
        let mut ring = rect(0.0, 0.0, 100.0, 100.0);
        // A hole, wound the other way.
        ring.extend(polyline(&[(30.0, 30.0), (30.0, 70.0), (70.0, 70.0), (70.0, 30.0)], true).iter());
        let mut apart = ellipse(kurbo::Rect::new(0.0, 0.0, 80.0, 50.0));
        let mut below = circles(3);
        below.apply_affine(kurbo::Affine::translate((0.0, -100.0)));
        apart.extend(below.iter());
        let blob = {
            let mut b = BezPath::new();
            b.move_to((0.0, 0.0));
            b.curve_to((40.0, -30.0), (80.0, 30.0), (100.0, 0.0));
            b.curve_to((130.0, 50.0), (70.0, 90.0), (50.0, 100.0));
            b.curve_to((10.0, 80.0), (-30.0, 30.0), (0.0, 0.0));
            b.close_path();
            b
        };
        for k in [1.0, 1e3, 1e6, 1e8] {
            for (name, art) in [("ring", &ring), ("ellipse and circles", &apart), ("blob", &blob)] {
                let art = scaled(art, k);
                let closed = crate::path::canonical(&art);
                assert_eq!(filled_boundary(&closed).unwrap(), closed, "{name} at {k}");
            }
        }
        for (outer, inner) in [(1e6, 1.0), (1e3, 1e-4), (1e8, 1.0)] {
            let mut nested = rect(0.0, 0.0, outer, outer);
            nested.extend(rect(outer / 3.0, outer / 3.0, outer / 3.0 + inner, outer / 3.0 + inner).iter());
            let boundary = filled_boundary(&nested).unwrap();
            assert!(
                boundary != nested && subpath_count(&boundary) == 1,
                "a square of {inner} in one of {outer}: {} subpaths",
                subpath_count(&boundary)
            );
        }
        // Told by the contours alone: a square of a thousandth in one a billion across is a few parts in a
        // trillion of the length, under what the lengths can be told apart by.
        let mut speck = rect(0.0, 0.0, 1e9, 1e9);
        speck.extend(rect(4e8, 4e8, 4e8 + 1e-3, 4e8 + 1e-3).iter());
        assert!(filled_boundary(&speck).unwrap() != speck, "a square of a thousandth in one of a billion");
        // And told by the lengths alone, with as many contours as subpaths: a square that goes round twice
        // in one subpath, and one that has a small loop inside it, wound the same way, in the same subpath.
        let twice = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true);
        let boundary = filled_boundary(&twice).unwrap();
        assert!(boundary != twice && subpath_count(&boundary) == 1, "a square drawn twice");
        let loop_inside = |s: f64| polyline(&[(0.0, 0.0), (s, 0.0), (s, s), (0.0, s), (0.0, 0.0), (5.0, 1.0), (5.0, 5.0)], true);
        for s in [100.0, 1e7] {
            let one = loop_inside(s);
            assert_eq!(subpath_count(&one), 1);
            assert!(filled_boundary(&one).unwrap() != one, "a loop of 20 square units inside a square of {s}");
        }
        // A rectangle across another: the edges inside are no boundary, and the contours are one.
        let mut crossing = rect(0.0, 0.0, 100.0, 100.0);
        crossing.extend(rect(50.0, 50.0, 150.0, 150.0).iter());
        let boundary = filled_boundary(&crossing).unwrap();
        assert_eq!(subpath_count(&boundary), 1);
        assert!(boundary != crossing);
    }

    /// The boundary an inset strokes is told from the path by measuring the curves of both, and
    /// that is limited: past [`MAX_COMPARED_CURVES`] of them nothing is measured and the boundary is used. (Measuring to an absolute accuracy took a cubic of large
    /// coordinates the most levels of subdivision there are.)
    #[test]
    fn the_curves_an_inset_measures_are_counted_and_limited() {
        // Few curves, apart: all measured (the cubics of the path and of its boundary), the path kept.
        let few = circles(100);
        work::reset();
        assert_eq!(filled_boundary(&few).unwrap(), few);
        let measured = work::take(Work::Measured);
        assert!((400..=2 * 8 * 100).contains(&measured), "{measured} curves measured");
        // Too many: none measured, and the boundary (the same circles, cut) is what comes back.
        let many = circles(1_100);
        work::reset();
        let boundary = filled_boundary(&many).unwrap();
        assert_eq!(work::take(Work::Measured), 0, "past the limit nothing is measured");
        assert!(boundary != many && (crate::area(&boundary, FillRule::NonZero) - crate::area(&many, FillRule::NonZero)).abs() < 1e-6);
        // Lines only: the lengths are chords, no curve is counted, and a rectangle in another is told.
        let mut nested = crate::testutil::rect(0.0, 0.0, 100.0, 100.0);
        nested.extend(crate::testutil::rect(30.0, 30.0, 70.0, 70.0).iter());
        work::reset();
        assert!(filled_boundary(&nested).unwrap() != nested);
        assert_eq!(work::take(Work::Measured), 0);
        // A curve of large coordinates is measured like any other (to a relative accuracy), up to the sweep's own limit.
        let huge = {
            let mut p = BezPath::new();
            p.extend(kurbo::Circle::new((0.0, 0.0), 3e12).to_path(1.0).iter());
            p
        };
        work::reset();
        assert_eq!(filled_boundary(&huge).unwrap(), huge);
        assert!(work::take(Work::Measured) > 0);
        // Nothing the measure does depends on the size of the coordinates.
        for scale in [1.0, 1e3, 1e6, 1e8] {
            let ring = kurbo::Circle::new((0.0, 0.0), scale).to_path(scale * 1e-4);
            let len = crate::fit::measured_perimeter(&ring, 1e-9).unwrap();
            let exact = kurbo::Shape::perimeter(&ring, scale * 1e-12);
            assert!((len - exact).abs() < 1e-8 * exact, "scale {scale}: {len} against {exact}");
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
