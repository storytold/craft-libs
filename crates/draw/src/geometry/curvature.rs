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
