//! The Curvature Pen's handles through a list of anchors.
//!
//! One rule draws, previews and edits Curvature paths everywhere: the pen draft's Curvature mode,
//! `BezierTool::Curvature` on finished paths, `EditSubpath::reflow`, and an app's engine.

use std::ops::RangeInclusive;

use kurbo::Point;

/// The range a Curvature insert's segment parameter `t` is clamped to, so an anchor added right
/// at a segment's end still lands inside it (PhotoCraft's Curvature press). Add Anchor Point and
/// Auto Add take the hit's `t` as it is.
// pc: crates/ui-egui/src/tools/direct_select.rs:418 (`t.clamp(0.01, 0.99)`).
pub const CURVATURE_T_RANGE: RangeInclusive<f64> = 0.01..=0.99;

/// [`curvature_handles`] over all anchors, carrying kept knots over when the draft extends the
/// kept run of an open path: the tail from the last kept knot is recomputed as an open run, kept
/// knots keep their anchors and handles except the last kept knot's out handle and, when closed,
/// knot 0's in handle (put in line with its kept out handle, else with the next kept knot's in
/// handle, else its anchor). One entry per anchor.
pub fn carried_handles(anchors: &[Point], corners: &[bool], closed: bool, kept: &[(Point, Point)]) -> Vec<(Point, Point)> {
    // pc: crates/ui-egui/src/tools/vector_ui/curvature.rs:11-56 `curvature_refresh` (#10726, #10744).
    let mut handles = curvature_handles(anchors, corners, closed);
    let corner = |i: usize| corners.get(i).copied().unwrap_or(false);
    let last = kept.len().saturating_sub(1);
    let extended = anchors.len() > kept.len();
    if extended && !closed && !kept.is_empty() {
        // As in Photoshop, a path carried on is a new Curvature run from its end.
        let run = curvature_handles(anchors.get(last..).unwrap_or_default(), corners.get(last..).unwrap_or_default(), false);
        for (slot, fresh) in handles.iter_mut().skip(last).zip(run) {
            *slot = fresh;
        }
    }
    for (i, &(was_in, was_out)) in kept.iter().enumerate() {
        let Some(slot) = handles.get_mut(i) else { break };
        let (keep_in, keep_out) = (!(closed && i == 0), !(extended && i == last));
        if keep_in {
            slot.0 = was_in;
        }
        if keep_out {
            slot.1 = was_out;
        }
        if !keep_in && keep_out && !corner(i) {
            // An out handle on its anchor gives no direction: the kept segment leaves towards its next control point,
            // else the next anchor, so the bent in handle comes from the far side and the knot stays smooth.
            let across = kept.get(i + 1).zip(anchors.get(i + 1)).map(|(next, &anchor)| [next.0, anchor]);
            let toward = std::iter::once(slot.1).chain(across.into_iter().flatten());
            if let Some(h) = toward.filter_map(|from| in_line(anchors[i], from, slot.0)).next() {
                slot.0 = h;
            }
        }
    }
    handles
}

/// A handle as long as `bent` from `anchor`, on the far side of it from `kept`; `None` when `kept` lies on the anchor
/// and gives no direction.
fn in_line(anchor: Point, kept: Point, bent: Point) -> Option<Point> {
    let d = anchor - kept;
    let len = d.hypot();
    if !(len > 1e-9 && len.is_finite()) {
        return None;
    }
    let reach = bent.distance(anchor);
    reach.is_finite().then(|| anchor + d / len * reach)
}

/// The Curvature Pen's handles for `anchors`: for each anchor its (in, out) handle points, so the
/// curve passes smoothly through every anchor (Catmull-Rom style, PhotoCraft's chordal rule).
///
/// - An inner anchor (every anchor when `closed`) lies on handles parallel to the chord from its
///   previous to its next neighbour (so they are collinear), the in handle a third of the
///   distance to the previous anchor long, the out handle a third of the distance to the next.
///   When the neighbours coincide, the direction is from the anchor to the next one.
/// - An open path's first anchor aims its out handle at the second anchor's in handle, and the
///   last anchor its in handle at the second-to-last anchor's out handle, each a third of the
///   distance to that neighbour long. The ends' other handles stay on their anchors.
/// - `corners[i]` true keeps anchor `i`'s handles on the anchor (a corner point); an index past
///   `corners` is smooth.
/// - Fewer than three anchors give straight segments: every handle on its anchor.
/// - Coincident points give handles on the anchor, never NaN.
///
/// The result has one entry per anchor.
pub fn curvature_handles(anchors: &[Point], corners: &[bool], closed: bool) -> Vec<(Point, Point)> {
    // pc: crates/vector/src/edit.rs:281-327 `curvature_knots`.
    const ZERO: f64 = 1e-9;
    let n = anchors.len();
    let mut handles: Vec<(Point, Point)> = anchors.iter().map(|&a| (a, a)).collect();
    let unit = |a: Point, b: Point| {
        let l = a.distance(b);
        (l > ZERO).then(|| (b - a) / l)
    };
    let corner = |i: usize| corners.get(i).copied().unwrap_or(false);
    let inner = |i: usize| closed || (i > 0 && i + 1 < n);
    for (i, &p) in anchors.iter().enumerate() {
        if n < 3 || corner(i) || !inner(i) {
            continue;
        }
        let prev = anchors[(i + n - 1) % n];
        let next = anchors[(i + 1) % n];
        let Some(d) = unit(prev, next).or_else(|| unit(p, next)) else { continue };
        handles[i] = (p - d * (prev.distance(p) / 3.0), p + d * (p.distance(next) / 3.0));
    }
    if !closed && n >= 3 {
        for (end, neighbour, first) in [(0, 1, true), (n - 1, n - 2, false)] {
            if corner(end) {
                continue;
            }
            let p = anchors[end];
            let aim = if first { handles[neighbour].0 } else { handles[neighbour].1 };
            let Some(d) = unit(p, aim) else { continue };
            let h = p + d * (p.distance(anchors[neighbour]) / 3.0);
            if first {
                handles[end].1 = h;
            } else {
                handles[end].0 = h;
            }
        }
    }
    handles
}
